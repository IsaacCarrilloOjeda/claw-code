//! Chief of Staff — Sonnet-driven orchestrator.
//!
//! Decomposes a compound ask into a plan of sub-agent calls, executes each
//! leg by re-entering `Dispatcher::dispatch` (so budget/events are recorded
//! per leg), then composes a final reply from the aggregated outputs.
//!
//! Budget/cost model: this agent's `declared_tier` is `Mid` (Sonnet). The
//! dispatcher debits the `chief_of_staff` line for THIS call's Sonnet
//! tokens only. Sub-legs (Research, Calendar, Docs) debit their own lines
//! via their own dispatcher entry, same as if Isaac had called them
//! directly — double-counting is intentional and correct.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;

use super::dispatcher::Dispatcher;
use super::{Agent, AgentRequest, AgentResponse, ModelTier, Usage};
use crate::constants::{ANTHROPIC_MESSAGES_URL, SONNET_MODEL};
use crate::http_client::shared_client;

const SONNET_TIMEOUT_SECS: u64 = 60;
const PLAN_MAX_TOKENS: u32 = 512;
const COMPOSE_MAX_TOKENS: u32 = 1024;

const PLAN_SYSTEM_PROMPT: &str =
    "You are the Chief of Staff orchestrator for Isaac's GHOST system.\n\
Given a compound request, produce a JSON plan that decomposes it into\n\
ordered sub-agent calls. Available sub-agents:\n\
- research — web search + summary. Use for \"look up\", \"find out\", \"what is\".\n\
- calendar — Google Calendar. Use for list/create events. Supports:\n\
    list (\"what's on my calendar today\")\n\
    create \"Summary\" at <RFC3339> for <duration>\n\
- docs — Google Docs. Supports create/read/append. For create, pass\n\
    'create \"Title\"'. For append, pass the doc URL followed by the content.\n\
\n\
Respond with ONLY a JSON object. No prose.\n\
\n\
Schema:\n\
{\n\
  \"goal\": \"<one-line restatement of the overall ask>\",\n\
  \"legs\": [\n\
    { \"agent\": \"research|calendar|docs\", \"prompt\": \"<plain text payload>\" }\n\
  ]\n\
}\n\
\n\
Keep legs minimal — 1 to 3 max. If the ask needs only one agent, still\n\
emit a single-leg plan. If none of the above agents fit, emit { \"goal\": \"...\", \"legs\": [] }.";

const COMPOSE_SYSTEM_PROMPT: &str =
    "You are the Chief of Staff. You just orchestrated sub-agents to answer\n\
Isaac's request. Produce a single concise reply that combines what the\n\
sub-agents found. If a leg errored, surface the error briefly. No preamble,\n\
no \"as an AI\"; write like Isaac's chief of staff briefing him in 3–6 lines.";

#[derive(Debug, Clone, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub legs: Vec<PlanLeg>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanLeg {
    pub agent: String,
    pub prompt: String,
}

pub struct ChiefOfStaff;

impl ChiefOfStaff {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ChiefOfStaff {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Agent for ChiefOfStaff {
    fn name(&self) -> &'static str {
        "chief_of_staff"
    }

    fn declared_tier(&self) -> ModelTier {
        ModelTier::Mid
    }

    fn requires_approval(&self, _req: &AgentRequest) -> bool {
        false
    }

    async fn handle(&self, req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String> {
        let trimmed = req.message.trim();
        if trimmed.is_empty() {
            return Err("chief of staff needs an actual request".into());
        }

        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

        let (plan, plan_usage) = build_plan(&api_key, trimmed).await?;

        let mut leg_outputs: Vec<(String, String)> = Vec::new();
        for leg in &plan.legs {
            let prefixed = format_leg_message(leg);
            let sub = AgentRequest {
                message: prefixed,
                history: Vec::new(),
                source: req.source,
                job_id: format!("{}-{}", req.job_id, leg.agent),
                sender_phone: req.sender_phone.clone(),
            };
            match Dispatcher::new().dispatch(sub, pool).await {
                Ok(resp) => leg_outputs.push((leg.agent.clone(), resp.text)),
                Err(e) => leg_outputs.push((leg.agent.clone(), format!("(error: {e})"))),
            }
        }

        let (summary, compose_usage) =
            compose_reply(&api_key, trimmed, &plan, &leg_outputs).await?;

        let usage = Usage {
            tokens_in: plan_usage.tokens_in.saturating_add(compose_usage.tokens_in),
            tokens_out: plan_usage
                .tokens_out
                .saturating_add(compose_usage.tokens_out),
        };

        Ok(AgentResponse {
            text: summary,
            usage,
            tier: ModelTier::Mid,
        })
    }
}

/// Prefix a leg's payload so the dispatcher re-classifies it to the right
/// agent. Unknown agents (including `chief_of_staff` itself — the recursion
/// guard) fall through to plain Chat.
pub(crate) fn format_leg_message(leg: &PlanLeg) -> String {
    match leg.agent.as_str() {
        "research" => format!("?{}", leg.prompt),
        "calendar" => format!("@{}", leg.prompt),
        "docs" => format!("&{}", leg.prompt),
        _ => leg.prompt.clone(),
    }
}

/// Strip a leading Markdown code fence (```json or bare ```) and any trailing
/// ``` fence that wraps the body. Sonnet sometimes wraps JSON in a fenced
/// block despite the system prompt asking for raw JSON.
pub(crate) fn strip_code_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    let after_open: &str = if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.trim_start_matches('\n')
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.trim_start_matches('\n')
    } else {
        trimmed
    };
    let end_trimmed = after_open.trim_end();
    let body = end_trimmed.strip_suffix("```").unwrap_or(end_trimmed);
    body.trim().to_string()
}

pub(crate) fn parse_plan(raw: &str) -> Result<Plan, String> {
    let cleaned = strip_code_fence(raw);
    serde_json::from_str::<Plan>(&cleaned)
        .map_err(|e| format!("could not parse Chief of Staff plan: {e}: raw = {raw}"))
}

async fn build_plan(api_key: &str, user_msg: &str) -> Result<(Plan, Usage), String> {
    let body = json!({
        "model": SONNET_MODEL,
        "max_tokens": PLAN_MAX_TOKENS,
        "system": PLAN_SYSTEM_PROMPT,
        "messages": [{ "role": "user", "content": user_msg }],
    });

    let (text, usage) = anthropic_call(api_key, &body).await?;
    let plan = parse_plan(&text)?;
    Ok((plan, usage))
}

async fn compose_reply(
    api_key: &str,
    original: &str,
    plan: &Plan,
    leg_outputs: &[(String, String)],
) -> Result<(String, Usage), String> {
    let plan_json = serde_json::to_string(&json!({
        "goal": plan.goal,
        "legs": plan.legs.iter().map(|l| json!({
            "agent": l.agent,
            "prompt": l.prompt,
        })).collect::<Vec<_>>(),
    }))
    .unwrap_or_else(|_| "{}".to_string());

    let mut user_prompt =
        format!("Original request: {original}\n\nPlan: {plan_json}\n\nLeg results:\n");
    if leg_outputs.is_empty() {
        user_prompt.push_str("(no sub-agents matched)\n");
    } else {
        for (agent, output) in leg_outputs {
            user_prompt.push_str("- ");
            user_prompt.push_str(agent);
            user_prompt.push_str(": ");
            user_prompt.push_str(output);
            user_prompt.push('\n');
        }
    }

    let body = json!({
        "model": SONNET_MODEL,
        "max_tokens": COMPOSE_MAX_TOKENS,
        "system": COMPOSE_SYSTEM_PROMPT,
        "messages": [{ "role": "user", "content": user_prompt }],
    });

    anthropic_call(api_key, &body).await
}

async fn anthropic_call(
    api_key: &str,
    body: &serde_json::Value,
) -> Result<(String, Usage), String> {
    let resp = shared_client()
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(SONNET_TIMEOUT_SECS))
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Anthropic request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Anthropic error {status}: {text}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("parse Anthropic json: {e}"))?;

    let text = json["content"][0]["text"]
        .as_str()
        .unwrap_or("(empty response)")
        .to_string();

    let usage_json = &json["usage"];
    let input = usage_json["input_tokens"].as_u64().unwrap_or(0);
    let cache_read = usage_json["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let output = usage_json["output_tokens"].as_u64().unwrap_or(0);
    let usage = Usage {
        tokens_in: u32::try_from(input.saturating_add(cache_read)).unwrap_or(u32::MAX),
        tokens_out: u32::try_from(output).unwrap_or(u32::MAX),
    };

    Ok((text, usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Source;

    fn leg(agent: &str, prompt: &str) -> PlanLeg {
        PlanLeg {
            agent: agent.to_string(),
            prompt: prompt.to_string(),
        }
    }

    #[test]
    fn format_leg_message_prefixes_known_agents() {
        assert_eq!(
            format_leg_message(&leg("research", "what is rust")),
            "?what is rust"
        );
        assert_eq!(
            format_leg_message(&leg("calendar", "what's on today")),
            "@what's on today"
        );
        assert_eq!(
            format_leg_message(&leg("docs", "create \"Notes\"")),
            "&create \"Notes\""
        );
    }

    #[test]
    fn format_leg_message_passes_through_unknown_agents() {
        // Unknown agents — including the recursion-guard case — drop through
        // to plain Chat (no prefix added).
        assert_eq!(
            format_leg_message(&leg("chief_of_staff", "nested")),
            "nested"
        );
        assert_eq!(format_leg_message(&leg("weather", "forecast")), "forecast");
    }

    #[test]
    fn parse_plan_accepts_representative_json() {
        let raw = r#"{
            "goal": "look up rust lifetimes and drop an agenda on my calendar",
            "legs": [
                { "agent": "research", "prompt": "rust lifetimes basics" },
                { "agent": "calendar", "prompt": "create \"Study Rust\" at 2026-05-01T10:00:00Z for 1h" }
            ]
        }"#;
        let plan = parse_plan(raw).expect("plan should parse");
        assert_eq!(plan.legs.len(), 2);
        assert_eq!(plan.legs[0].agent, "research");
        assert_eq!(plan.legs[1].agent, "calendar");
        assert!(plan.goal.contains("rust lifetimes"));
    }

    #[test]
    fn parse_plan_strips_json_code_fence() {
        let raw = "```json\n{ \"goal\": \"g\", \"legs\": [] }\n```";
        let plan = parse_plan(raw).expect("fenced plan should parse");
        assert_eq!(plan.goal, "g");
        assert!(plan.legs.is_empty());
    }

    #[test]
    fn parse_plan_strips_bare_code_fence() {
        let raw = "```\n{ \"goal\": \"bare\", \"legs\": [] }\n```";
        let plan = parse_plan(raw).expect("bare-fenced plan should parse");
        assert_eq!(plan.goal, "bare");
    }

    #[test]
    fn parse_plan_surfaces_raw_on_failure() {
        let raw = "not json at all";
        let err = parse_plan(raw).expect_err("should fail");
        assert!(err.contains("raw = not json at all"), "got: {err}");
    }

    #[test]
    fn requires_approval_is_always_false() {
        let agent = ChiefOfStaff::new();
        let make = |msg: &str| AgentRequest {
            message: msg.to_string(),
            history: Vec::new(),
            source: Source::Dashboard,
            job_id: "t".to_string(),
            sender_phone: None,
        };
        assert!(!agent.requires_approval(&make("plan my week")));
        assert!(!agent.requires_approval(&make("")));
        assert!(!agent.requires_approval(&make("delete everything")));
    }
}
