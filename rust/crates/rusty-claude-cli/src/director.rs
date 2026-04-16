//! Director — handles `!`-prefixed messages with Claude Sonnet.
//!
//! Now loads core context and memory (same as `chat_dispatcher`) for a consistent
//! experience across both routing paths.

use std::time::Duration;

use crate::constants::{ANTHROPIC_MESSAGES_URL, SONNET_MODEL};

const AI_TIMEOUT_SECS: u64 = 60;

/// Handle a Director-routed message (! prefix, stripped before call).
/// Returns the response text.
pub async fn handle(
    message: &str,
    _job_id: &str,
    pool: Option<&sqlx::PgPool>,
) -> Result<String, String> {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    // Load core context and memory — same as chat_dispatcher for consistency.
    let core_context = crate::chat_dispatcher::load_core_context();
    let memory_context = crate::chat_dispatcher::load_memory_context(message, pool).await;

    let mut system = core_context;
    if !memory_context.is_empty() {
        system.push_str("\n\n## What you remember about Isaac\n<memory_notes>\n");
        let capped = if memory_context.len() > 4096 {
            &memory_context[..4096]
        } else {
            &memory_context
        };
        system.push_str(capped);
        system.push_str("\n</memory_notes>");
    }

    let request_body = serde_json::json!({
        "model": SONNET_MODEL,
        "max_tokens": 1024,
        "system": system,
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

    Ok(text)
}
