//! Chat dispatcher — lightweight path for no-prefix SMS messages.
//!
//! Loads the core context file from `GHOST_CORE_CONTEXT_PATH` and calls
//! Claude Haiku with `[core context + message]`. No streaming — waits for
//! the full response before returning.

use std::time::Duration;

const HAIKU_MODEL: &str = "claude-haiku-4-5-20251001";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const AI_TIMEOUT_SECS: u64 = 60;

/// Dispatch a no-prefix message to Claude Haiku. Returns the response text.
pub async fn dispatch(message: &str, _job_id: &str) -> Result<String, String> {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    let system = load_core_context();

    let request_body = serde_json::json!({
        "model": HAIKU_MODEL,
        "max_tokens": 1024,
        "system": system,
        "messages": [{"role": "user", "content": message}],
    });

    call_anthropic(&api_key, &request_body).await
}

/// Load the core context file. Falls back to a minimal default if the env var
/// is unset or the file cannot be read.
fn load_core_context() -> String {
    let path = match std::env::var("GHOST_CORE_CONTEXT_PATH") {
        Ok(p) if !p.is_empty() => p,
        _ => return "You are GHOST, a personal AI assistant.".to_string(),
    };
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("[ghost chat] failed to read core context from {path}: {e}");
        "You are GHOST, a personal AI assistant.".to_string()
    })
}

async fn call_anthropic(api_key: &str, body: &serde_json::Value) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(AI_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("HTTP client build error: {e}"))?;

    let resp = client
        .post(ANTHROPIC_MESSAGES_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(body)
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
