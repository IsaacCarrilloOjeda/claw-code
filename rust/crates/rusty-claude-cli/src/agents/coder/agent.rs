//! `CoderAgent` — the feature-shipping agent.
//!
//! Owns its own wire format (`tool_use` for Anthropic, `tool_calls` for the
//! OpenAI-compatible `OpenRouter` path) because the shared `call_model` layer
//! is text-in/text-out by design. Spend-recording invariant: every upstream
//! model call lands in `coder_spend` via `db::record_spend`. Gate runs
//! before the next turn starts, not mid-turn.
//!
//! Fallback chain (advance only on 5xx/429, 2× malformed tool args, or
//! explicit refusal): `DeepSeek` → Haiku → Sonnet → `MiMo` → refuse.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::time::timeout;
use uuid::Uuid;

use crate::agents::tools::{self, Tool, ToolCtx};
use crate::agents::{Agent, AgentRequest, AgentResponse, ModelTier, Usage};
use crate::constants::ANTHROPIC_MESSAGES_URL;
use crate::db;
use crate::http_client::shared_client;
use crate::infra::cache::build_cached_system;
use crate::infra::provider::{provider_for, Provider};

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const CALL_TIMEOUT_SECS: u64 = 120;
const MAX_TOOL_TURNS: u32 = 10;
const MAX_TOKENS_PER_TURN: u32 = 4096;
const MAX_MALFORMED_RETRIES: u32 = 2;
const DEFAULT_BUDGET_CENTS: i64 = 200;
const DEFAULT_MIMO_SLUG: &str = "xiaomi/mimo-7b-rl";

const FALLBACK_CHAIN: &[FallbackStep] = &[
    FallbackStep {
        provider: Provider::OpenRouter,
        model: "deepseek/deepseek-chat",
        tier: ModelTier::Code,
    },
    FallbackStep {
        provider: Provider::Anthropic,
        model: "claude-haiku-4-5-20251001",
        tier: ModelTier::Fast,
    },
    FallbackStep {
        provider: Provider::Anthropic,
        model: "claude-sonnet-4-6",
        tier: ModelTier::Mid,
    },
    // MiMo slot — the model slug is read from settings_kv so we can flip it
    // without a redeploy when OpenRouter renames the catalog entry.
    FallbackStep {
        provider: Provider::OpenRouter,
        model: "__mimo__",
        tier: ModelTier::Code,
    },
];

#[derive(Clone, Copy)]
struct FallbackStep {
    provider: Provider,
    model: &'static str,
    tier: ModelTier,
}

pub struct CoderAgent {
    pub repo_root: PathBuf,
    pub chat_id: Uuid,
}

impl CoderAgent {
    pub fn new(repo_root: PathBuf, chat_id: Uuid) -> Self {
        Self { repo_root, chat_id }
    }
}

#[async_trait]
impl Agent for CoderAgent {
    fn name(&self) -> &'static str {
        "coder"
    }
    fn declared_tier(&self) -> ModelTier {
        ModelTier::Code
    }
    fn requires_approval(&self, _req: &AgentRequest) -> bool {
        // Diff-apply governance is enforced inside `tools::diff`, which reads
        // `coder.auto_apply` live. The Agent-trait `requires_approval` is
        // dispatcher-level gating (should we pause before calling the agent
        // at all?) — coder never pauses at that level.
        false
    }

    #[allow(clippy::too_many_lines)]
    async fn handle(&self, req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String> {
        if is_killed(pool).await {
            return Err("kill_switched".into());
        }

        let cap = db::get_setting::<i64>(pool, "coder.budget_cents_per_day")
            .await
            .unwrap_or(DEFAULT_BUDGET_CENTS);
        let spent = i64::from(db::spend_today(pool, "coder").await.unwrap_or(0));
        if spent >= cap {
            return Err(format!("budget_exhausted: {spent}¢ of {cap}¢ (coder)"));
        }

        let job_uuid = Uuid::parse_str(&req.job_id).ok();
        let auto_apply = db::get_setting::<bool>(pool, "coder.auto_apply")
            .await
            .unwrap_or(false);

        let registry = tools::registry();
        let ctx = ToolCtx {
            repo_root: self.repo_root.clone(),
            pool: pool.clone(),
            auto_apply,
            chat_id: self.chat_id,
        };

        let stable = build_stable_prefix(&self.repo_root, &registry).await;
        let dynamic = build_dynamic_suffix(pool, self.chat_id, &req.message).await;

        // Resolve the starting step based on the per-agent provider setting:
        // settings_kv says Anthropic → skip DeepSeek and start at Haiku.
        let start_idx = match provider_for("coder", pool).await {
            Provider::Anthropic => 1,
            Provider::OpenRouter => 0,
        };

        let mimo_slug = db::get_setting::<String>(pool, "coder.fallback.mimo_model")
            .await
            .unwrap_or_else(|| DEFAULT_MIMO_SLUG.to_string());

        let mut accumulated = Usage::default();
        let mut messages_anthropic: Vec<Value> = Vec::new();
        let mut messages_openai: Vec<Value> = Vec::new();
        append_history(&mut messages_anthropic, &req.history, false);
        append_history(&mut messages_openai, &req.history, true);
        messages_anthropic.push(json!({ "role": "user", "content": req.message }));
        messages_openai.push(json!({ "role": "user", "content": req.message }));

        let mut fallback_idx = start_idx;
        let mut last_err: Option<String> = None;

        'fallback: while fallback_idx < FALLBACK_CHAIN.len() {
            let step = FALLBACK_CHAIN[fallback_idx];
            let model = if step.model == "__mimo__" {
                mimo_slug.as_str()
            } else {
                step.model
            };

            // Per-step messages: we always restart from the initial user turn
            // because mid-chain model switches shouldn't carry the partial
            // tool-use state of a crashed model forward.
            let mut turn_anthropic = messages_anthropic.clone();
            let mut turn_openai = messages_openai.clone();

            for _turn in 0..MAX_TOOL_TURNS {
                // Pre-turn budget gate.
                let spent_now = i64::from(db::spend_today(pool, "coder").await.unwrap_or(0));
                if spent_now >= cap {
                    return Err(format!("budget_exhausted: {spent_now}¢ of {cap}¢ (coder)"));
                }

                let turn_result = match step.provider {
                    Provider::Anthropic => {
                        anthropic_turn(model, &stable, &dynamic, &turn_anthropic, &registry).await
                    }
                    Provider::OpenRouter => {
                        openrouter_turn(model, &stable, &dynamic, &mut turn_openai, &registry).await
                    }
                };

                let turn = match turn_result {
                    Ok(t) => t,
                    Err(TurnError::Transient(msg)) => {
                        last_err = Some(msg);
                        fallback_idx += 1;
                        continue 'fallback;
                    }
                    Err(TurnError::Fatal(msg)) => {
                        return Err(format!("coder turn failed: {msg}"));
                    }
                };

                // Spend-recording invariant — one record_spend per upstream
                // call, with actual tokens + provider.
                let cost = cost_cents_for(step.tier, turn.tokens_in, turn.tokens_out);
                if let Err(e) = db::record_spend(
                    pool,
                    "coder",
                    model,
                    step.provider.as_str(),
                    turn.tokens_in,
                    turn.tokens_out,
                    turn.cache_read,
                    i32::try_from(cost).unwrap_or(i32::MAX),
                    job_uuid,
                )
                .await
                {
                    eprintln!("[coder] record_spend failed: {e}");
                }
                accumulated = Usage {
                    tokens_in: accumulated.tokens_in.saturating_add(turn.tokens_in),
                    tokens_out: accumulated.tokens_out.saturating_add(turn.tokens_out),
                };

                if turn.refused {
                    last_err = Some("model refused".into());
                    fallback_idx += 1;
                    continue 'fallback;
                }

                match step.provider {
                    Provider::Anthropic => {
                        // Append assistant turn for continuity.
                        turn_anthropic.push(
                            json!({ "role": "assistant", "content": turn.content_blocks.clone() }),
                        );
                        if turn.tool_calls.is_empty() {
                            return Ok(AgentResponse {
                                text: turn.text,
                                usage: accumulated,
                                tier: ModelTier::Code,
                            });
                        }
                        let mut results: Vec<Value> = Vec::with_capacity(turn.tool_calls.len());
                        for call in &turn.tool_calls {
                            let result = execute_tool(&registry, &ctx, &call.name, &call.args)
                                .await
                                .unwrap_or_else(|e| format!("tool error: {e}"));
                            results.push(json!({
                                "type": "tool_result",
                                "tool_use_id": call.id,
                                "content": result
                            }));
                        }
                        turn_anthropic.push(json!({ "role": "user", "content": results }));
                    }
                    Provider::OpenRouter => {
                        // Append assistant turn exactly as returned.
                        turn_openai.push(turn.assistant_raw.clone().unwrap_or(json!({
                            "role": "assistant", "content": turn.text.clone()
                        })));
                        if turn.tool_calls.is_empty() {
                            return Ok(AgentResponse {
                                text: turn.text,
                                usage: accumulated,
                                tier: ModelTier::Code,
                            });
                        }
                        for call in &turn.tool_calls {
                            let result = execute_tool(&registry, &ctx, &call.name, &call.args)
                                .await
                                .unwrap_or_else(|e| format!("tool error: {e}"));
                            turn_openai.push(json!({
                                "role": "tool",
                                "tool_call_id": call.id,
                                "content": result
                            }));
                        }
                    }
                }
            }

            // Hit MAX_TOOL_TURNS without finishing — cut the losses on this
            // step and let the fallback chain try a different model.
            last_err = Some(format!("exceeded {MAX_TOOL_TURNS} tool turns"));
            fallback_idx += 1;
        }

        Err(format!(
            "all providers exhausted (last: {})",
            last_err.unwrap_or_else(|| "unknown".into())
        ))
    }
}

async fn is_killed(pool: &PgPool) -> bool {
    if std::env::var("GHOST_CODING_AGENT")
        .ok()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("off"))
    {
        return true;
    }
    db::get_setting::<bool>(pool, "coder.kill_switch")
        .await
        .unwrap_or(false)
}

fn append_history(dest: &mut Vec<Value>, history: &[Value], _openai_shape: bool) {
    // Only keep the last 6 turns of {role, content} shape; reject anything
    // else. History comes from the dashboard and we don't fully trust it.
    let tail: Vec<&Value> = history.iter().rev().take(6).collect();
    for turn in tail.into_iter().rev() {
        let role = turn.get("role").and_then(Value::as_str).unwrap_or("");
        let content = turn.get("content").and_then(Value::as_str).unwrap_or("");
        if role.is_empty() || content.is_empty() {
            continue;
        }
        if role != "user" && role != "assistant" {
            continue;
        }
        dest.push(json!({ "role": role, "content": content }));
    }
}

async fn build_stable_prefix(repo_root: &std::path::Path, registry: &[Box<dyn Tool>]) -> String {
    let mut out = String::from(
        "You are GHOST's Coder agent. You ship small, reviewable changes. \
         You MUST use the provided tools to read the repo before proposing \
         edits. The `diff` tool's output tells you whether your change was \
         applied or queued for Isaac's approval — either way, stop guessing \
         about file contents and keep reading with `read`/`grep`/`list_dir`. \
         When you've finished the task, respond with a short plain-text \
         summary — no further tool calls.\n\n",
    );
    out.push_str("## Tools available\n");
    for tool in registry {
        let schema = tool.schema();
        let desc = schema
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("(no description)");
        let _ = writeln!(out, "- `{}` — {desc}", tool.name());
    }
    out.push_str("\n## CLAUDE.md\n");
    out.push_str(&read_optional(repo_root, "CLAUDE.md").await);
    out.push_str("\n## ARCHITECTURE.md\n");
    out.push_str(&read_optional(repo_root, "ARCHITECTURE.md").await);
    out
}

async fn read_optional(repo_root: &std::path::Path, rel: &str) -> String {
    match tokio::fs::read_to_string(repo_root.join(rel)).await {
        Ok(s) => s,
        Err(_) => format!("({rel} not found)"),
    }
}

async fn build_dynamic_suffix(pool: &PgPool, chat_id: Uuid, user_msg: &str) -> String {
    let mut out = String::new();

    // Potentially relevant files (Prompt C). Degrade silently to empty.
    if let Ok(hits) = super::index::search_files(pool, user_msg, 5).await {
        if !hits.is_empty() {
            out.push_str("## Potentially relevant files\n");
            for hit in hits {
                let _ = writeln!(out, "- {} — {}", hit.path, first_line(&hit.summary));
            }
            out.push('\n');
        }
    }

    // Earlier in this chat (summarizer). Returns Vec<String> infallibly.
    let hits = crate::agents::summarizer::relevant_condensate(pool, &chat_id, user_msg, 3).await;
    if !hits.is_empty() {
        out.push_str("## Earlier in this chat\n");
        for h in hits {
            let _ = writeln!(out, "- {h}");
        }
        out.push('\n');
    }

    out
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

fn cost_cents_for(tier: ModelTier, tokens_in: u32, tokens_out: u32) -> i64 {
    crate::infra::budget::cost_cents(tier, i64::from(tokens_in), i64::from(tokens_out))
}

// ----- turn wire format ---------------------------------------------------

struct TurnOutcome {
    text: String,
    tokens_in: u32,
    tokens_out: u32,
    cache_read: u32,
    refused: bool,
    tool_calls: Vec<ParsedToolCall>,
    /// Anthropic: raw content-blocks array for the assistant turn.
    content_blocks: Vec<Value>,
    /// `OpenRouter`: full assistant message (includes `tool_calls`) to append verbatim.
    assistant_raw: Option<Value>,
}

struct ParsedToolCall {
    id: String,
    name: String,
    args: Value,
}

enum TurnError {
    /// 5xx, 429, malformed tool args after retries, timeouts — advance fallback.
    Transient(String),
    /// 4xx (besides 429), auth failure, unparseable body — bubble up now.
    Fatal(String),
}

async fn anthropic_turn(
    model: &str,
    stable: &str,
    dynamic: &str,
    messages: &[Value],
    registry: &[Box<dyn Tool>],
) -> Result<TurnOutcome, TurnError> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| TurnError::Fatal("ANTHROPIC_API_KEY not set".into()))?;
    let tools_json: Vec<Value> = registry.iter().map(|t| t.schema()).collect();

    let body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS_PER_TURN,
        "system": build_cached_system(stable, dynamic),
        "messages": messages,
        "tools": tools_json,
    });

    let send = shared_client()
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(CALL_TIMEOUT_SECS))
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send();
    let resp = timeout(Duration::from_secs(CALL_TIMEOUT_SECS + 5), send)
        .await
        .map_err(|_| TurnError::Transient("anthropic timeout".into()))?
        .map_err(|e| classify_http_err(&e.to_string(), 0))?;

    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_http_err(&body, status));
    }
    let v: Value = resp
        .json()
        .await
        .map_err(|e| TurnError::Transient(format!("anthropic parse: {e}")))?;

    let usage = &v["usage"];
    let tokens_in = u32::try_from(usage["input_tokens"].as_u64().unwrap_or(0)).unwrap_or(u32::MAX);
    let tokens_out =
        u32::try_from(usage["output_tokens"].as_u64().unwrap_or(0)).unwrap_or(u32::MAX);
    let cache_read =
        u32::try_from(usage["cache_read_input_tokens"].as_u64().unwrap_or(0)).unwrap_or(u32::MAX);

    let content_blocks = v["content"].as_array().cloned().unwrap_or_default();
    let mut text = String::new();
    let mut tool_calls: Vec<ParsedToolCall> = Vec::new();
    for block in &content_blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(t);
                }
            }
            Some("tool_use") => {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let args = block.get("input").cloned().unwrap_or(Value::Null);
                tool_calls.push(ParsedToolCall { id, name, args });
            }
            _ => {}
        }
    }

    let refused = v
        .get("stop_reason")
        .and_then(Value::as_str)
        .is_some_and(|r| r == "refusal");

    Ok(TurnOutcome {
        text,
        tokens_in,
        tokens_out,
        cache_read,
        refused,
        tool_calls,
        content_blocks,
        assistant_raw: None,
    })
}

#[allow(clippy::too_many_lines)]
async fn openrouter_turn(
    model: &str,
    stable: &str,
    dynamic: &str,
    messages: &mut Vec<Value>,
    registry: &[Box<dyn Tool>],
) -> Result<TurnOutcome, TurnError> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| TurnError::Fatal("OPENAI_API_KEY not set".into()))?;

    // Flatten cached-system array into plain text for OpenAI-compat path.
    let system_text = if dynamic.is_empty() {
        stable.to_string()
    } else {
        format!("{stable}\n\n{dynamic}")
    };
    let tools_json: Vec<Value> = registry
        .iter()
        .map(|t| anthropic_to_openai_tool(&t.schema()))
        .collect();

    let mut malformed_retries = 0_u32;
    loop {
        let mut full_messages = Vec::with_capacity(messages.len() + 1);
        full_messages.push(json!({ "role": "system", "content": system_text }));
        full_messages.extend(messages.iter().cloned());

        let body = json!({
            "model": model,
            "max_tokens": MAX_TOKENS_PER_TURN,
            "messages": full_messages,
            "tools": tools_json,
        });

        let send = shared_client()
            .post(OPENROUTER_URL)
            .timeout(Duration::from_secs(CALL_TIMEOUT_SECS))
            .bearer_auth(&api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send();
        let resp = timeout(Duration::from_secs(CALL_TIMEOUT_SECS + 5), send)
            .await
            .map_err(|_| TurnError::Transient("openrouter timeout".into()))?
            .map_err(|e| classify_http_err(&e.to_string(), 0))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.text().await.unwrap_or_default();
            return Err(classify_http_err(&body, status));
        }

        let v: Value = resp
            .json()
            .await
            .map_err(|e| TurnError::Transient(format!("openrouter parse: {e}")))?;

        let usage = &v["usage"];
        let tokens_in =
            u32::try_from(usage["prompt_tokens"].as_u64().unwrap_or(0)).unwrap_or(u32::MAX);
        let tokens_out =
            u32::try_from(usage["completion_tokens"].as_u64().unwrap_or(0)).unwrap_or(u32::MAX);

        let choice = &v["choices"][0];
        let msg = &choice["message"];
        let text = msg
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let raw_calls = msg.get("tool_calls").and_then(Value::as_array);

        let refused = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .is_some_and(|r| r == "content_filter");

        let mut parsed: Vec<ParsedToolCall> = Vec::new();
        let mut malformed: Vec<String> = Vec::new();
        if let Some(calls) = raw_calls {
            for c in calls {
                let id = c
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let fn_obj = c.get("function").cloned().unwrap_or(Value::Null);
                let name = fn_obj
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let args_str = fn_obj
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match serde_json::from_str::<Value>(args_str) {
                    Ok(args) => parsed.push(ParsedToolCall { id, name, args }),
                    Err(e) => malformed.push(format!("{name}: {e}")),
                }
            }
        }

        if !malformed.is_empty() && malformed_retries < MAX_MALFORMED_RETRIES {
            malformed_retries += 1;
            messages.push(msg.clone());
            messages.push(json!({
                "role": "user",
                "content": format!(
                    "Your previous response had malformed tool_call arguments: {}. \
                     Please retry with valid JSON.",
                    malformed.join("; ")
                )
            }));
            continue;
        }
        if !malformed.is_empty() {
            return Err(TurnError::Transient(format!(
                "malformed tool_calls after {MAX_MALFORMED_RETRIES} retries: {}",
                malformed.join("; ")
            )));
        }

        return Ok(TurnOutcome {
            text,
            tokens_in,
            tokens_out,
            cache_read: 0,
            refused,
            tool_calls: parsed,
            content_blocks: Vec::new(),
            assistant_raw: Some(msg.clone()),
        });
    }
}

fn anthropic_to_openai_tool(schema: &Value) -> Value {
    let name = schema.get("name").cloned().unwrap_or(Value::Null);
    let description = schema.get("description").cloned().unwrap_or(Value::Null);
    let parameters = schema
        .get("input_schema")
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}

fn classify_http_err(body: &str, status: u16) -> TurnError {
    if status == 429 || (500..600).contains(&status) || status == 0 {
        TurnError::Transient(format!("http {status}: {body}"))
    } else {
        TurnError::Fatal(format!("http {status}: {body}"))
    }
}

async fn execute_tool(
    registry: &[Box<dyn Tool>],
    ctx: &ToolCtx,
    name: &str,
    args: &Value,
) -> Result<String, String> {
    let tool = registry
        .iter()
        .find(|t| t.name() == name)
        .ok_or_else(|| format!("unknown tool: {name}"))?;
    tool.run(args.clone(), ctx)
        .await
        .map_err(|e| format!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_to_openai_tool_shape() {
        let input = json!({
            "name": "read",
            "description": "read a file",
            "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}
        });
        let out = anthropic_to_openai_tool(&input);
        assert_eq!(out["type"], "function");
        assert_eq!(out["function"]["name"], "read");
        assert_eq!(out["function"]["description"], "read a file");
        assert_eq!(out["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn classify_http_err_5xx_is_transient() {
        assert!(matches!(
            classify_http_err("boom", 503),
            TurnError::Transient(_)
        ));
        assert!(matches!(
            classify_http_err("ratelimit", 429),
            TurnError::Transient(_)
        ));
        assert!(matches!(
            classify_http_err("bad auth", 401),
            TurnError::Fatal(_)
        ));
    }

    #[test]
    fn append_history_keeps_last_six_and_filters() {
        let mut dest: Vec<Value> = Vec::new();
        let hist = vec![
            json!({"role": "user", "content": "a"}),
            json!({"role": "assistant", "content": "b"}),
            json!({"role": "bogus", "content": "c"}),
            json!({"role": "user"}),
            json!({"role": "user", "content": "d"}),
        ];
        append_history(&mut dest, &hist, false);
        assert_eq!(dest.len(), 3);
        assert_eq!(dest[0]["content"], "a");
        assert_eq!(dest[1]["content"], "b");
        assert_eq!(dest[2]["content"], "d");
    }
}
