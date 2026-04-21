//! Dreamer — offline reflection agent.
//!
//! Wave 5 scope: one verb. Reads recent `ghost_events` + `director_notes`,
//! asks Sonnet to identify 3–8 recurring topics with 1–3 sentence summaries,
//! embeds each, upserts into `interest_nodes`. Returns a human-readable
//! summary for the caller (usually a `scheduled_triggers` row Isaac added
//! manually that fires at 03:00 UTC daily).
//!
//! This file intentionally does NOT register itself with the dispatcher —
//! that's a one-liner in Wave 5.5 after all Wave 5 branches merge. Isaac
//! will pick a prefix post-merge (likely `~`).
//!
//! Auth: none external. Just `ANTHROPIC_API_KEY` for Sonnet + an embedding
//! key (`VOYAGE_API_KEY` or `OPENAI_API_KEY`) for the `embed()` call.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;

use super::{Agent, AgentRequest, AgentResponse, ModelTier, Usage};
use crate::constants::{ANTHROPIC_MESSAGES_URL, SONNET_MODEL};
use crate::http_client::shared_client;
use crate::{db, memory};

const SONNET_TIMEOUT_SECS: u64 = 60;
const WINDOW_MAX_BYTES: usize = 8 * 1024;

pub struct Dreamer;

impl Dreamer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Dreamer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Agent for Dreamer {
    fn name(&self) -> &'static str {
        "dreamer"
    }

    fn declared_tier(&self) -> ModelTier {
        ModelTier::Mid
    }

    fn requires_approval(&self, _req: &AgentRequest) -> bool {
        false
    }

    async fn handle(&self, _req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String> {
        let window = db::dreamer_window(pool, 200, 50)
            .await
            .map_err(|e| format!("dreamer window query failed: {e}"))?;

        if window.is_empty() {
            return Ok(AgentResponse {
                text: "dreamer: no recent activity to reflect on".into(),
                usage: Usage::default(),
                tier: ModelTier::Mid,
            });
        }

        let (topics, usage) = reflect(&window).await?;

        if topics.is_empty() {
            return Ok(AgentResponse {
                text: "dreamer: no themes emerged".into(),
                usage,
                tier: ModelTier::Mid,
            });
        }

        let mut wrote = 0u32;
        for t in &topics {
            let emb = memory::embed(&format!("{}\n{}", t.topic, t.summary))
                .await
                .ok();
            let refs = json!([]);
            match db::insert_interest_node(pool, &t.topic, &t.summary, emb.as_deref(), &refs).await
            {
                Ok(_) => wrote += 1,
                Err(e) => eprintln!("[dreamer] insert failed for '{}': {e}", t.topic),
            }
        }

        Ok(AgentResponse {
            text: format!("dreamer: wrote {wrote} interest node(s)"),
            usage,
            tier: ModelTier::Mid,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Topic {
    pub topic: String,
    pub summary: String,
}

async fn reflect(window_lines: &[String]) -> Result<(Vec<Topic>, Usage), String> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| "ANTHROPIC_API_KEY not set — dreamer disabled".to_string())?;

    let user_prompt = truncate_from_start(window_lines, WINDOW_MAX_BYTES);

    let system = "You are the Dreamer — a reflection process that reads Isaac's recent GHOST \
                  activity and names what's been occupying his attention. Output a JSON array \
                  of 3 to 8 topics:\n\n\
                  [{ \"topic\": \"<3–5 word label>\", \"summary\": \"<1–3 sentences>\" }, ...]\n\n\
                  Pick recurring themes, not one-off mentions. No preamble, no code fence.";

    let body = json!({
        "model": SONNET_MODEL,
        "max_tokens": 1024,
        "system": system,
        "messages": [{ "role": "user", "content": user_prompt }],
    });

    let resp = shared_client()
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(SONNET_TIMEOUT_SECS))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
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

    let raw = json["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let usage_json = &json["usage"];
    let input = usage_json["input_tokens"].as_u64().unwrap_or(0);
    let cache_read = usage_json["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let output = usage_json["output_tokens"].as_u64().unwrap_or(0);
    let usage = Usage {
        tokens_in: u32::try_from(input.saturating_add(cache_read)).unwrap_or(u32::MAX),
        tokens_out: u32::try_from(output).unwrap_or(u32::MAX),
    };

    let cleaned = strip_code_fence(&raw);
    let topics: Vec<Topic> = serde_json::from_str(&cleaned)
        .map_err(|e| format!("could not parse dreamer topics: {e}: raw = {raw}"))?;

    Ok((topics, usage))
}

/// Strip a leading markdown fence (with or without a `json` tag) and trailing
/// fence from `raw`. Inner whitespace is left to `serde_json::from_str`.
fn strip_code_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_open = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let without_close = without_open
        .trim_end()
        .strip_suffix("```")
        .unwrap_or(without_open);
    without_close.trim().to_string()
}

/// Join `lines` with `\n` and cap at `max_bytes`. When the joined text exceeds
/// the cap, oldest lines (front of the slice) are dropped first — the Dreamer
/// sees the freshest window at the expense of history.
fn truncate_from_start(lines: &[String], max_bytes: usize) -> String {
    let joined = lines.join("\n");
    if joined.len() <= max_bytes {
        return joined;
    }

    let mut start = 0;
    while start < lines.len() {
        let remaining = lines[start..].join("\n");
        if remaining.len() <= max_bytes {
            return remaining;
        }
        start += 1;
    }

    // All individual lines alone still exceed the cap — return the very last
    // line truncated to max_bytes at a char boundary rather than dropping
    // everything.
    let last = lines.last().map_or("", String::as_str);
    let mut cut = max_bytes.min(last.len());
    while cut > 0 && !last.is_char_boundary(cut) {
        cut -= 1;
    }
    last[..cut].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Source;

    fn req(msg: &str) -> AgentRequest {
        AgentRequest {
            message: msg.to_string(),
            history: Vec::new(),
            source: Source::Scheduled,
            job_id: "test-job".to_string(),
            sender_phone: None,
        }
    }

    #[test]
    fn topic_parses_from_representative_json() {
        let raw = r#"[
            {"topic": "react-native build", "summary": "Isaac has been debugging metro bundler."},
            {"topic": "ghost phase 3", "summary": "Specialist agents landing this week."}
        ]"#;
        let topics: Vec<Topic> = serde_json::from_str(raw).expect("parse");
        assert_eq!(topics.len(), 2);
        assert_eq!(topics[0].topic, "react-native build");
        assert!(topics[1].summary.contains("Specialist"));
    }

    #[test]
    fn code_fence_stripping_removes_json_fence() {
        let raw = "```json\n[{\"topic\": \"a\", \"summary\": \"b\"}]\n```";
        let cleaned = strip_code_fence(raw);
        let topics: Vec<Topic> = serde_json::from_str(&cleaned).expect("parse");
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0].topic, "a");
    }

    #[test]
    fn code_fence_stripping_handles_plain_fence() {
        let raw = "```\n[]\n```";
        let cleaned = strip_code_fence(raw);
        assert_eq!(cleaned, "[]");
    }

    #[test]
    fn code_fence_stripping_noop_without_fence() {
        let raw = "[{\"topic\":\"x\",\"summary\":\"y\"}]";
        let cleaned = strip_code_fence(raw);
        assert_eq!(cleaned, raw);
    }

    #[test]
    fn truncate_from_start_drops_oldest_lines_first() {
        // "old" is the oldest (front of the slice); "fresh" is newest (back).
        let lines = vec![
            "old line that will be dropped".to_string(),
            "middle line".to_string(),
            "fresh line".to_string(),
        ];
        // Cap small enough that the first line must be dropped but the latter
        // two still fit with a newline between them.
        let cap = "middle line".len() + "\n".len() + "fresh line".len();
        let out = truncate_from_start(&lines, cap);
        assert!(
            !out.contains("old line"),
            "oldest line must be dropped, got: {out}"
        );
        assert!(out.contains("middle line"));
        assert!(out.contains("fresh line"));
    }

    #[test]
    fn truncate_from_start_returns_whole_input_when_under_cap() {
        let lines = vec!["a".to_string(), "b".to_string()];
        assert_eq!(truncate_from_start(&lines, 1024), "a\nb");
    }

    #[test]
    fn requires_approval_is_always_false() {
        let agent = Dreamer::new();
        assert!(!agent.requires_approval(&req("anything")));
        assert!(!agent.requires_approval(&req("")));
    }
}
