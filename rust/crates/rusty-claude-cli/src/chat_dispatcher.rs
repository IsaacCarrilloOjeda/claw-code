//! Chat dispatcher — lightweight path for no-prefix messages.
//!
//! Loads the core context file from `GHOST_CORE_CONTEXT_PATH`, appends any
//! relevant memory notes (Phase 2 semantic search), then calls Claude Haiku.
//! After responding, spawns a fire-and-forget task to extract and store new
//! notes from the conversation turn.

use std::time::Duration;

const HAIKU_MODEL: &str = "claude-haiku-4-5-20251001";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const AI_TIMEOUT_SECS: u64 = 60;
const MEMORY_SEARCH_LIMIT: i64 = 5;

/// Dispatch a no-prefix message to Claude Haiku. Returns the response text.
///
/// If `pool` is provided, semantic memory search runs before the call and
/// note extraction runs asynchronously after.
pub async fn dispatch(
    message: &str,
    _job_id: &str,
    pool: Option<&sqlx::PgPool>,
) -> Result<String, String> {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    let core_context = load_core_context();
    let memory_context = load_memory_context(message, pool).await;

    let system = if memory_context.is_empty() {
        core_context
    } else {
        format!("{core_context}\n\n## What you remember about Isaac\n{memory_context}")
    };

    let request_body = serde_json::json!({
        "model": HAIKU_MODEL,
        "max_tokens": 1024,
        "system": system,
        "messages": [{"role": "user", "content": message}],
    });

    let response = call_anthropic(&api_key, &request_body).await?;

    // Fire-and-forget: extract facts from this turn and store them.
    if let Some(p) = pool {
        let pool_owned = p.clone();
        let msg = message.to_string();
        let resp = response.clone();
        tokio::spawn(async move {
            crate::memory::extract_and_store(pool_owned, msg, resp).await;
        });
    }

    Ok(response)
}

/// Embed the incoming message and pull the top-N most relevant memory notes.
/// Returns a formatted string to inject into the system prompt, or empty string
/// if the pool is absent, embedding fails, or no notes exist yet.
async fn load_memory_context(message: &str, pool: Option<&sqlx::PgPool>) -> String {
    let Some(pool) = pool else {
        return String::new();
    };

    let embedding = match crate::memory::embed(message).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[ghost memory] embed failed for query: {e}");
            return String::new();
        }
    };

    let notes = crate::db::search_notes(pool, &embedding, MEMORY_SEARCH_LIMIT).await;
    if notes.is_empty() {
        return String::new();
    }

    notes
        .iter()
        .map(|n| format!("- [{}] {}", n.category, n.content))
        .collect::<Vec<_>>()
        .join("\n")
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
