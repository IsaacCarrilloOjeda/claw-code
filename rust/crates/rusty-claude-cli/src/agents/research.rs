//! Research agent — Brave Search → page fetch → Haiku summary.
//!
//! Wave 4 scope: "brief" mode only (top-5 Brave results, best-effort body
//! fetch for top 3, single Haiku summary call). Everything is stateless
//! per-call — no caching, no memory, no Gerald.
//!
// TODO(wave 5): deep mode — Sonnet-driven multi-step research via
// `orchestrator.rs`. Not wired here.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use sqlx::PgPool;

use super::{Agent, AgentRequest, AgentResponse, ModelTier, Usage};
use crate::constants::{ANTHROPIC_MESSAGES_URL, HAIKU_MODEL};
use crate::http_client::shared_client;

const BRAVE_SEARCH_URL: &str = "https://api.search.brave.com/res/v1/web/search";
const BRAVE_RESULT_COUNT: u8 = 5;
const PAGE_FETCH_COUNT: usize = 3;
const PAGE_FETCH_TIMEOUT_SECS: u64 = 5;
const PAGE_BODY_BYTES: usize = 8 * 1024;
const PAGE_BODY_CHARS: usize = 4_000;
const HAIKU_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone)]
struct ResearchHit {
    title: String,
    url: String,
    snippet: String,
    body: Option<String>,
}

pub struct Research;

impl Research {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Research {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Agent for Research {
    fn name(&self) -> &'static str {
        "research"
    }

    fn declared_tier(&self) -> ModelTier {
        ModelTier::Fast
    }

    fn requires_approval(&self, _req: &AgentRequest) -> bool {
        false
    }

    async fn handle(&self, req: AgentRequest, _pool: &PgPool) -> Result<AgentResponse, String> {
        let query = req.message.trim();
        if query.is_empty() {
            return Err("research query is empty".to_string());
        }

        let brave_key = require_brave_key(std::env::var("BRAVE_API_KEY").ok())?;

        let mut hits = brave_search(&brave_key, query).await?;

        if hits.is_empty() {
            return Ok(AgentResponse {
                text: format!("No results for '{query}'"),
                usage: Usage::default(),
                tier: ModelTier::Fast,
            });
        }

        // Best-effort body fetch for the top N — failures are logged and skipped.
        for hit in hits.iter_mut().take(PAGE_FETCH_COUNT) {
            match fetch_page_excerpt(&hit.url).await {
                Ok(body) => hit.body = Some(body),
                Err(e) => eprintln!("[research] fetch {} failed: {e}", hit.url),
            }
        }

        let anthropic_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

        let (summary, usage) = haiku_summarise(&anthropic_key, query, &hits)
            .await
            .map_err(|e| format!("research summary failed: {e}"))?;

        Ok(AgentResponse {
            text: summary,
            usage,
            tier: ModelTier::Fast,
        })
    }
}

/// Validate a `BRAVE_API_KEY` read from the environment. Trims whitespace and
/// treats an empty/blank key as missing. Split out so we can unit-test the
/// missing-key error path without mutating process env (forbidden under
/// `forbid(unsafe_code)`).
fn require_brave_key(raw: Option<String>) -> Result<String, String> {
    raw.map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .ok_or_else(|| "BRAVE_API_KEY not set — research agent disabled".to_string())
}

// ---------------------------------------------------------------------------
// Brave search
// ---------------------------------------------------------------------------

async fn brave_search(api_key: &str, query: &str) -> Result<Vec<ResearchHit>, String> {
    let resp = shared_client()
        .get(BRAVE_SEARCH_URL)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", api_key)
        .query(&[
            ("q", query),
            ("count", &BRAVE_RESULT_COUNT.to_string()),
            ("safesearch", "moderate"),
        ])
        .send()
        .await
        .map_err(|e| format!("Brave request failed: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("read Brave body: {e}"))?;

    if !status.is_success() {
        return Err(format!("Brave search error {status}: {body}"));
    }

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("parse Brave json: {e}"))?;

    let items = match json["web"]["results"].as_array() {
        Some(arr) => arr.clone(),
        None => return Ok(Vec::new()),
    };

    let hits = items
        .iter()
        .filter_map(|item| {
            let title = item["title"].as_str()?.to_string();
            let url = item["url"].as_str()?.to_string();
            let snippet = item["description"]
                .as_str()
                .or_else(|| item["snippet"].as_str())
                .unwrap_or("")
                .to_string();
            Some(ResearchHit {
                title,
                url,
                snippet,
                body: None,
            })
        })
        .take(BRAVE_RESULT_COUNT as usize)
        .collect();

    Ok(hits)
}

// ---------------------------------------------------------------------------
// Page fetch
// ---------------------------------------------------------------------------

async fn fetch_page_excerpt(url: &str) -> Result<String, String> {
    let resp = shared_client()
        .get(url)
        .timeout(Duration::from_secs(PAGE_FETCH_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("status {}", resp.status()));
    }

    let bytes = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;

    let cut = bytes.len().min(PAGE_BODY_BYTES);
    let snippet = match std::str::from_utf8(&bytes[..cut]) {
        Ok(s) => s.to_string(),
        Err(e) => {
            // Try the valid prefix up to the first invalid byte.
            let valid = e.valid_up_to();
            if valid == 0 {
                return Err("non-utf8 body".to_string());
            }
            std::str::from_utf8(&bytes[..valid])
                .unwrap_or("")
                .to_string()
        }
    };

    let cleaned = strip_html(&snippet);
    Ok(truncate_chars(&cleaned, PAGE_BODY_CHARS))
}

/// Hand-rolled `<[^>]+>` stripper (regex isn't a dependency). Also collapses
/// runs of whitespace to single spaces and trims.
fn strip_html(input: &str) -> String {
    let mut stripped = String::with_capacity(input.len());
    let mut in_tag = false;
    for c in input.chars() {
        match c {
            '<' => {
                in_tag = true;
                stripped.push(' ');
            }
            '>' if in_tag => {
                in_tag = false;
                stripped.push(' ');
            }
            _ if !in_tag => stripped.push(c),
            _ => {}
        }
    }

    let mut out = String::with_capacity(stripped.len());
    let mut prev_ws = false;
    for c in stripped.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(c);
            prev_ws = false;
        }
    }

    out.trim().to_string()
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// Haiku summary
// ---------------------------------------------------------------------------

async fn haiku_summarise(
    api_key: &str,
    query: &str,
    hits: &[ResearchHit],
) -> Result<(String, Usage), String> {
    let mut user_prompt = format!("Research query: {query}\n\n");
    for hit in hits {
        user_prompt.push_str("## ");
        user_prompt.push_str(&hit.title);
        user_prompt.push('\n');
        user_prompt.push_str(&hit.url);
        user_prompt.push('\n');
        if !hit.snippet.is_empty() {
            user_prompt.push_str(&hit.snippet);
            user_prompt.push('\n');
        }
        if let Some(body) = &hit.body {
            if !body.is_empty() {
                user_prompt.push_str(body);
                user_prompt.push('\n');
            }
        }
        user_prompt.push('\n');
    }

    let system = "Summarise the following research results for Isaac in 3–5 bullet points. Cite source URLs inline.";

    let body = json!({
        "model": HAIKU_MODEL,
        "max_tokens": 1024,
        "system": system,
        "messages": [{ "role": "user", "content": user_prompt }],
    });

    let resp = shared_client()
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(HAIKU_TIMEOUT_SECS))
        .header("x-api-key", api_key)
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

// ---------------------------------------------------------------------------
// Tests — pure pieces only. Network paths are exercised manually with a
// real `BRAVE_API_KEY` / `ANTHROPIC_API_KEY`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Source;

    fn req(msg: &str) -> AgentRequest {
        AgentRequest {
            message: msg.to_string(),
            history: Vec::new(),
            source: Source::Dashboard,
            job_id: "test-job".to_string(),
            sender_phone: None,
        }
    }

    fn dead_pool() -> PgPool {
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .test_before_acquire(false)
            .connect_lazy("postgres://invalid:invalid@127.0.0.1:1/invalid")
            .expect("lazy connect should not fail")
    }

    #[tokio::test]
    async fn empty_query_returns_error_without_network() {
        let pool = dead_pool();
        let agent = Research::new();
        // Whitespace-only also counts as empty after trim.
        let err = agent
            .handle(req("   "), &pool)
            .await
            .expect_err("empty query must error");
        assert!(err.contains("research query is empty"), "got: {err}");
    }

    #[test]
    fn require_brave_key_rejects_missing_and_blank() {
        // All three "empty" shapes must produce the same error message the
        // handler surfaces to the user, regardless of why the env was absent.
        for input in [None, Some(String::new()), Some("   ".to_string())] {
            let err = require_brave_key(input).expect_err("should reject");
            assert!(err.contains("BRAVE_API_KEY not set"), "got: {err}");
        }
    }

    #[test]
    fn require_brave_key_accepts_real_key() {
        let k = require_brave_key(Some("  secret  ".to_string())).expect("accept");
        assert_eq!(k, "secret");
    }

    #[test]
    fn strip_html_removes_tags_and_collapses_whitespace() {
        let input = "<p>Hello  <b>world</b>\n  <a href=\"x\">link</a></p>";
        let out = strip_html(input);
        assert_eq!(out, "Hello world link");
    }

    #[test]
    fn strip_html_handles_unclosed_tags() {
        // A stray `<` with no matching `>` should swallow the rest — preferable
        // to leaking raw markup into the summary prompt.
        let input = "safe <broken and nothing closes";
        let out = strip_html(input);
        assert_eq!(out, "safe");
    }

    #[test]
    fn strip_html_preserves_plain_text_unchanged() {
        let out = strip_html("just plain text with spaces");
        assert_eq!(out, "just plain text with spaces");
    }

    #[test]
    fn truncate_chars_respects_char_boundaries() {
        // Multi-byte chars: "é" is 2 bytes but 1 char.
        let s = "éééé";
        assert_eq!(truncate_chars(s, 2), "éé");
        assert_eq!(truncate_chars(s, 10), "éééé");
    }
}
