//! Orchestrator agent. Receives a feature spec, breaks it into up to 5
//! independent tasks, and queues them in `coder_tasks`. Execution is fire-
//! and-return: `POST /code/orchestrate/:id/run` spawns a worker pool and
//! comes back immediately with `status: "running"`; workers update
//! `coder_tasks.status` and `worker_output` as they finish, and the
//! dashboard polls `GET /code/orchestrate/:id` to paint task cards live.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::db;
use crate::infra::provider::{call_model, provider_for, Provider, ProviderError};

use super::coder::CoderAgent;
use super::{Agent, AgentRequest, AgentResponse, ModelTier, Source, Usage};

const SYSTEM_PROMPT: &str = "You receive a feature spec. Break it into at most 5 independent tasks that can be executed in parallel by coder workers. Each task must be self-contained — the worker will not see other tasks. For each task, produce: files_to_read[], files_to_modify[], prompt (a self-contained instruction for the coder including acceptance criteria), verify_command (the single bash command that proves the task is done). Return a JSON array of task objects. Do not write code yourself. Respond with ONLY a JSON array — no prose, no markdown fences.";

const DEFAULT_MODEL_ANTHROPIC: &str = "claude-sonnet-4-6";
const DEFAULT_MODEL_OPENROUTER: &str = "anthropic/claude-sonnet-4";
const MAX_TASKS: usize = 5;
const MAX_TOKENS: u32 = 4096;
const DEFAULT_BUDGET_CENTS: i64 = 200;

pub struct OrchestratorAgent {
    pub repo_root: PathBuf,
    pub chat_id: Option<Uuid>,
}

impl OrchestratorAgent {
    pub fn new(repo_root: PathBuf, chat_id: Option<Uuid>) -> Self {
        Self { repo_root, chat_id }
    }
}

/// Structured task row as returned by the planner. Stored as one row in
/// `coder_tasks`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlannedTask {
    pub prompt: String,
    #[serde(default)]
    pub files_to_read: Vec<String>,
    #[serde(default)]
    pub files_to_modify: Vec<String>,
    #[serde(default)]
    pub verify_command: Option<String>,
}

/// What the orchestrator endpoint returns after planning.
pub struct PlanOutcome {
    pub orchestration_id: Uuid,
    pub tasks: Vec<(Uuid, PlannedTask)>,
    pub usage: Usage,
}

impl OrchestratorAgent {
    /// Plan the spec into tasks and persist everything. Does not execute.
    pub async fn plan(&self, spec: &str, pool: &PgPool) -> Result<PlanOutcome, String> {
        let cap = db::get_setting::<i64>(pool, "orchestrator.budget_cents_per_day")
            .await
            .unwrap_or(DEFAULT_BUDGET_CENTS);
        let spent = i64::from(db::spend_today(pool, "orchestrator").await.unwrap_or(0));
        if spent >= cap {
            return Err(format!(
                "budget_exhausted: {spent}¢ of {cap}¢ (orchestrator)"
            ));
        }

        let provider = provider_for("orchestrator", pool).await;
        let model = match provider {
            Provider::Anthropic => DEFAULT_MODEL_ANTHROPIC,
            Provider::OpenRouter => DEFAULT_MODEL_OPENROUTER,
        };

        let system = Value::String(SYSTEM_PROMPT.to_string());
        let messages = vec![json!({ "role": "user", "content": spec })];
        let resp = call_model(
            provider,
            "orchestrator",
            model,
            system,
            messages,
            MAX_TOKENS,
        )
        .await
        .map_err(map_provider_err)?;

        let tier = ModelTier::Mid;
        let cost = crate::infra::budget::cost_cents(
            tier,
            i64::from(resp.input_tokens),
            i64::from(resp.output_tokens),
        );
        if let Err(e) = db::record_spend(
            pool,
            "orchestrator",
            &resp.model,
            provider.as_str(),
            resp.input_tokens,
            resp.output_tokens,
            resp.cache_read,
            i32::try_from(cost).unwrap_or(i32::MAX),
            None,
        )
        .await
        {
            eprintln!("[orchestrator] record_spend failed: {e}");
        }

        let tasks = parse_plan(&resp.text)?;
        if tasks.is_empty() {
            return Err("planner returned no tasks".into());
        }

        let orch_id: Uuid = sqlx::query_scalar(
            "INSERT INTO coder_orchestrations (spec, chat_id, status)
             VALUES ($1, $2, 'planned') RETURNING id",
        )
        .bind(spec)
        .bind(self.chat_id)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("insert orchestration: {e}"))?;

        let mut stored: Vec<(Uuid, PlannedTask)> = Vec::with_capacity(tasks.len());
        for task in tasks {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO coder_tasks
                   (orchestration_id, task_prompt, files_to_read, files_to_modify, verify_command)
                 VALUES ($1, $2, $3, $4, $5) RETURNING id",
            )
            .bind(orch_id)
            .bind(&task.prompt)
            .bind(json!(task.files_to_read))
            .bind(json!(task.files_to_modify))
            .bind(task.verify_command.clone())
            .fetch_one(pool)
            .await
            .map_err(|e| format!("insert task: {e}"))?;
            stored.push((id, task));
        }

        Ok(PlanOutcome {
            orchestration_id: orch_id,
            tasks: stored,
            usage: Usage {
                tokens_in: resp.input_tokens,
                tokens_out: resp.output_tokens,
            },
        })
    }
}

#[async_trait]
impl Agent for OrchestratorAgent {
    fn name(&self) -> &'static str {
        "orchestrator"
    }
    fn declared_tier(&self) -> ModelTier {
        ModelTier::Mid
    }
    fn requires_approval(&self, _req: &AgentRequest) -> bool {
        false
    }
    async fn handle(&self, req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String> {
        let outcome = self.plan(&req.message, pool).await?;
        let text = format!(
            "planned {} task(s). orchestration_id={}. run with POST /code/orchestrate/{}/run",
            outcome.tasks.len(),
            outcome.orchestration_id,
            outcome.orchestration_id
        );
        Ok(AgentResponse {
            text,
            usage: outcome.usage,
            tier: ModelTier::Mid,
        })
    }
}

fn parse_plan(text: &str) -> Result<Vec<PlannedTask>, String> {
    let trimmed = strip_code_fences(text);
    let v: Value = serde_json::from_str(&trimmed)
        .map_err(|e| format!("planner output is not JSON: {e}. raw: {trimmed}"))?;

    // Accept either a bare array or an object containing a `tasks` array.
    let arr = if let Some(a) = v.as_array() {
        a.clone()
    } else if let Some(a) = v.get("tasks").and_then(Value::as_array) {
        a.clone()
    } else {
        return Err("planner output must be a JSON array or {tasks: [...]}".into());
    };

    let mut out: Vec<PlannedTask> = Vec::with_capacity(arr.len());
    for (i, item) in arr.into_iter().enumerate() {
        let task: PlannedTask = serde_json::from_value(item)
            .map_err(|e| format!("task {i} failed to deserialize: {e}"))?;
        if task.prompt.trim().is_empty() {
            return Err(format!("task {i} has empty prompt"));
        }
        out.push(task);
    }

    if out.len() > MAX_TASKS {
        out.truncate(MAX_TASKS);
    }
    Ok(out)
}

fn strip_code_fences(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        return rest.trim_end_matches("```").trim().to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        return rest.trim_end_matches("```").trim().to_string();
    }
    trimmed.to_string()
}

fn map_provider_err(e: ProviderError) -> String {
    match e {
        ProviderError::KillSwitched => "kill_switched".into(),
        other => format!("provider error: {other}"),
    }
}

/// Fire-and-return worker executor. Spawns one coder per task; each worker
/// flips its task row to `running` → `done`/`failed` and stores its final
/// text in `worker_output`. Does NOT await completion — the caller hands
/// control back to the HTTP response immediately.
pub fn spawn_workers(orch_id: Uuid, repo_root: PathBuf, pool: PgPool) {
    tokio::spawn(async move {
        if let Err(e) =
            sqlx::query("UPDATE coder_orchestrations SET status = 'running' WHERE id = $1")
                .bind(orch_id)
                .execute(&pool)
                .await
        {
            eprintln!("[orchestrator] mark running failed: {e}");
        }

        let rows = match sqlx::query(
            "SELECT id, task_prompt FROM coder_tasks
             WHERE orchestration_id = $1 AND status = 'pending'
             ORDER BY created_at",
        )
        .bind(orch_id)
        .fetch_all(&pool)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[orchestrator] load tasks failed: {e}");
                return;
            }
        };

        let mut set: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
        for row in rows {
            let task_id: Uuid = row.try_get("id").unwrap_or_else(|_| Uuid::nil());
            let prompt: String = row.try_get("task_prompt").unwrap_or_default();
            let pool_c = pool.clone();
            let root_c = repo_root.clone();
            set.spawn(async move {
                run_single_task(task_id, prompt, root_c, pool_c).await;
            });
        }
        while set.join_next().await.is_some() {}

        // Final status: done if every task is done, else failed.
        let final_status = match sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM coder_tasks
             WHERE orchestration_id = $1 AND status <> 'done'",
        )
        .bind(orch_id)
        .fetch_one(&pool)
        .await
        {
            Ok(0) => "done",
            _ => "failed",
        };
        if let Err(e) = sqlx::query("UPDATE coder_orchestrations SET status = $1 WHERE id = $2")
            .bind(final_status)
            .bind(orch_id)
            .execute(&pool)
            .await
        {
            eprintln!("[orchestrator] final status update failed: {e}");
        }
    });
}

async fn run_single_task(task_id: Uuid, prompt: String, repo_root: PathBuf, pool: PgPool) {
    let _ = sqlx::query("UPDATE coder_tasks SET status = 'running' WHERE id = $1")
        .bind(task_id)
        .execute(&pool)
        .await;

    let worker_chat_id = Uuid::new_v4();
    let coder = CoderAgent::new(repo_root, worker_chat_id);
    let req = AgentRequest {
        message: prompt,
        history: Vec::new(),
        source: Source::Dashboard,
        job_id: worker_chat_id.to_string(),
        sender_phone: None,
    };

    match coder.handle(req, &pool).await {
        Ok(resp) => {
            let _ = sqlx::query(
                "UPDATE coder_tasks
                   SET status = 'done', worker_output = $2, completed_at = now()
                 WHERE id = $1",
            )
            .bind(task_id)
            .bind(&resp.text)
            .execute(&pool)
            .await;
        }
        Err(e) => {
            let _ = sqlx::query(
                "UPDATE coder_tasks
                   SET status = 'failed', worker_output = $2, completed_at = now()
                 WHERE id = $1",
            )
            .bind(task_id)
            .bind(&e)
            .execute(&pool)
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plan_accepts_bare_array() {
        let text = r#"[{"prompt":"add ping", "files_to_modify":["a.rs"]}]"#;
        let out = parse_plan(text).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].prompt, "add ping");
        assert_eq!(out[0].files_to_modify, vec!["a.rs"]);
    }

    #[test]
    fn parse_plan_accepts_object_with_tasks_key() {
        let text = r#"{"tasks":[{"prompt":"x"}, {"prompt":"y"}]}"#;
        let out = parse_plan(text).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn parse_plan_strips_code_fences() {
        let text = "```json\n[{\"prompt\":\"a\"}]\n```";
        let out = parse_plan(text).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn parse_plan_caps_at_five() {
        let tasks: Vec<Value> = (0..8).map(|i| json!({"prompt": format!("t{i}")})).collect();
        let text = serde_json::to_string(&tasks).unwrap();
        let out = parse_plan(&text).unwrap();
        assert_eq!(out.len(), MAX_TASKS);
    }

    #[test]
    fn parse_plan_rejects_empty_prompt() {
        let text = r#"[{"prompt":""}]"#;
        assert!(parse_plan(text).is_err());
    }
}
