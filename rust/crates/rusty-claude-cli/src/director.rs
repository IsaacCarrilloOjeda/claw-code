//! Director — `!`-prefixed messages served by Claude Sonnet.
//!
//! Wave 2: this file is a thin facade. Routing lives in `agents::dispatcher`,
//! prefix parsing lives in `agents::intent`. The real Sonnet call is kept
//! here as `sonnet_reply` so the dispatcher can call it without a circular
//! module dependency.

use std::time::Duration;

use crate::agents::Usage;
use crate::constants::{ANTHROPIC_MESSAGES_URL, SONNET_MODEL};

const AI_TIMEOUT_SECS: u64 = 60;

/// Run the Sonnet call for a Director-routed message (prefix already stripped).
/// Loads core context + memory and returns the assistant text together with
/// the token usage reported by Anthropic (used by the dispatcher to debit the
/// per-agent budget).
pub(crate) async fn sonnet_reply(
    message: &str,
    _job_id: &str,
    pool: Option<&sqlx::PgPool>,
) -> Result<(String, Usage), String> {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    let stable = crate::chat_dispatcher::load_core_context();
    let memory_context = crate::chat_dispatcher::load_memory_context(message, pool).await;

    let mut dynamic = String::new();
    if !memory_context.is_empty() {
        dynamic.push_str("## What you remember about Isaac\n<memory_notes>\n");
        let capped = if memory_context.len() > 4096 {
            &memory_context[..4096]
        } else {
            &memory_context
        };
        dynamic.push_str(capped);
        dynamic.push_str("\n</memory_notes>");
    }

    let request_body = serde_json::json!({
        "model": SONNET_MODEL,
        "max_tokens": 1024,
        "system": crate::infra::cache::build_cached_system(&stable, &dynamic),
        "messages": [{"role": "user", "content": message}],
    });

    let client = crate::http_client::shared_client();

    let resp = client
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(AI_TIMEOUT_SECS))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("Anthropic API request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {status}: {text}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse API response: {e}"))?;

    let text = json["content"][0]["text"]
        .as_str()
        .unwrap_or("(empty response)")
        .to_string();

    let usage = extract_usage(&json);
    Ok((text, usage))
}

/// Pull `input_tokens` + `output_tokens` (and any cache-read tokens, which we
/// count conservatively) from the Anthropic `usage` block.
fn extract_usage(json: &serde_json::Value) -> Usage {
    let usage = &json["usage"];
    let input = usage["input_tokens"].as_u64().unwrap_or(0);
    let cache_read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let output = usage["output_tokens"].as_u64().unwrap_or(0);
    Usage {
        tokens_in: u32::try_from(input.saturating_add(cache_read)).unwrap_or(u32::MAX),
        tokens_out: u32::try_from(output).unwrap_or(u32::MAX),
    }
}

/// Facade over `agents::dispatcher`. Re-prefixes the message with `!` so the
/// dispatcher's intent classifier routes it back here via `sonnet_reply`.
pub async fn handle(
    message: &str,
    job_id: &str,
    pool: Option<&sqlx::PgPool>,
) -> Result<String, String> {
    let pool = pool.ok_or_else(|| "director requires a db pool".to_string())?;
    let req = crate::agents::AgentRequest {
        message: format!("!{message}"),
        history: Vec::new(),
        source: crate::agents::Source::Sms,
        job_id: job_id.to_string(),
        sender_phone: None,
    };
    let dispatcher = crate::agents::dispatcher::Dispatcher::new();
    let resp = dispatcher.dispatch(req, pool).await?;
    Ok(resp.text)
}
