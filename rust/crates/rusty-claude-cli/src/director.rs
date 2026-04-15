//! Director stub — Phase 1.
//!
//! Handles `!`-prefixed SMS messages. Uses Claude Sonnet with a fixed system
//! prompt. No specialist agent spawning yet — that is Phase 3.

use std::time::Duration;

const DIRECTOR_MODEL: &str = "claude-sonnet-4-6";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const AI_TIMEOUT_SECS: u64 = 60;
const DIRECTOR_SYSTEM: &str = "You are GHOST, a personal AI assistant.";

/// Handle a Director-routed message (! prefix, stripped before call).
/// Returns the response text.
pub async fn handle(message: &str, _job_id: &str) -> Result<String, String> {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    let request_body = serde_json::json!({
        "model": DIRECTOR_MODEL,
        "max_tokens": 1024,
        "system": DIRECTOR_SYSTEM,
        "messages": [{"role": "user", "content": message}],
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(AI_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("HTTP client build error: {e}"))?;

    let resp = client
        .post(ANTHROPIC_MESSAGES_URL)
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
