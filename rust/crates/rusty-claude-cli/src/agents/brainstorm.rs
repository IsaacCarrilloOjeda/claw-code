//! Brainstorm PM agent. Takes Isaac's rambling spec, asks ≤3 targeted
//! questions, then emits a clean one-pager. Pure chat — no tools. Default
//! model: `DeepSeek`-chat via `OpenRouter`; the coder/brainstorm kill switch
//! is honored inside `call_model`.

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db;
use crate::infra::provider::{call_model, provider_for, Provider, ProviderError};

use super::{Agent, AgentRequest, AgentResponse, ModelTier, Usage};

const SYSTEM_PROMPT: &str = "You are a skeptical product manager working with Isaac on GHOST. He will describe a feature idea — often incomplete. Ask at most 3 targeted clarifying questions (numbered, with a 'my gut' default for each). Wait for answers. Then emit a clean one-page spec with these sections: **Goal** · **Non-goals** · **Constraints** · **Acceptance criteria** · **Open questions (if any)**. Do not write code. Do not propose files. Push back on scope creep.";

const DEFAULT_OPENROUTER_MODEL: &str = "deepseek/deepseek-chat";
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-6";
const MAX_TOKENS: u32 = 2048;
const DEFAULT_BUDGET_CENTS: i64 = 100;

pub struct BrainstormAgent;

impl BrainstormAgent {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for BrainstormAgent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Agent for BrainstormAgent {
    fn name(&self) -> &'static str {
        "brainstorm"
    }
    fn declared_tier(&self) -> ModelTier {
        ModelTier::Code
    }
    fn requires_approval(&self, _req: &AgentRequest) -> bool {
        false
    }

    async fn handle(&self, req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String> {
        let cap = db::get_setting::<i64>(pool, "brainstorm.budget_cents_per_day")
            .await
            .unwrap_or(DEFAULT_BUDGET_CENTS);
        let spent = i64::from(db::spend_today(pool, "brainstorm").await.unwrap_or(0));
        if spent >= cap {
            return Err(format!("budget_exhausted: {spent}¢ of {cap}¢ (brainstorm)"));
        }

        let provider = provider_for("brainstorm", pool).await;
        let model = match provider {
            Provider::Anthropic => DEFAULT_ANTHROPIC_MODEL,
            Provider::OpenRouter => DEFAULT_OPENROUTER_MODEL,
        };

        let system = Value::String(SYSTEM_PROMPT.to_string());
        let mut messages: Vec<Value> = req
            .history
            .iter()
            .filter_map(sanitize_history_entry)
            .collect();
        messages.push(json!({ "role": "user", "content": req.message }));

        let resp = call_model(provider, "brainstorm", model, system, messages, MAX_TOKENS)
            .await
            .map_err(map_provider_err)?;

        let job_uuid = Uuid::parse_str(&req.job_id).ok();
        let tier = ModelTier::Code;
        let cost = crate::infra::budget::cost_cents(
            tier,
            i64::from(resp.input_tokens),
            i64::from(resp.output_tokens),
        );
        if let Err(e) = db::record_spend(
            pool,
            "brainstorm",
            &resp.model,
            provider.as_str(),
            resp.input_tokens,
            resp.output_tokens,
            resp.cache_read,
            i32::try_from(cost).unwrap_or(i32::MAX),
            job_uuid,
        )
        .await
        {
            eprintln!("[brainstorm] record_spend failed: {e}");
        }

        Ok(AgentResponse {
            text: resp.text,
            usage: Usage {
                tokens_in: resp.input_tokens,
                tokens_out: resp.output_tokens,
            },
            tier,
        })
    }
}

fn sanitize_history_entry(v: &Value) -> Option<Value> {
    let role = v.get("role").and_then(Value::as_str)?;
    let content = v.get("content").and_then(Value::as_str)?;
    if role != "user" && role != "assistant" {
        return None;
    }
    Some(json!({ "role": role, "content": content }))
}

fn map_provider_err(e: ProviderError) -> String {
    match e {
        ProviderError::KillSwitched => "kill_switched".into(),
        other => format!("provider error: {other}"),
    }
}
