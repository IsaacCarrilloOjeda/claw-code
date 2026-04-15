//! Memory system for GHOST — embeddings, semantic search, and note extraction.
//!
//! `embed()` calls the `OpenAI` `text-embedding-3-small` model (1536 dims, matches DB schema).
//! `extract_and_store()` is fire-and-forget: call via `tokio::spawn` after a chat
//! response so it never blocks the user-facing latency path.

use std::time::Duration;

const OPENAI_EMBED_URL: &str = "https://api.openai.com/v1/embeddings";
const EMBED_MODEL: &str = "text-embedding-3-small";
const EMBED_TIMEOUT_SECS: u64 = 10;
const EXTRACT_TIMEOUT_SECS: u64 = 15;
const HAIKU_MODEL: &str = "claude-haiku-4-5-20251001";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";

/// Call the `OpenAI` `text-embedding-3-small` model and return a 1536-dim vector.
/// Returns `Err` if `OPENAI_API_KEY` is unset or the API call fails.
pub async fn embed(text: &str) -> Result<Vec<f32>, String> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| "OPENAI_API_KEY not set".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(EMBED_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let body = serde_json::json!({
        "model": EMBED_MODEL,
        "input": text,
    });

    let resp = client
        .post(OPENAI_EMBED_URL)
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("OpenAI embed request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("OpenAI embed error {status}: {body}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse embed response: {e}"))?;

    let embedding = json["data"][0]["embedding"]
        .as_array()
        .ok_or_else(|| "missing embedding array in response".to_string())?
        .iter()
        .map(|v| {
            #[allow(clippy::cast_possible_truncation)]
            let f = v.as_f64().unwrap_or(0.0) as f32;
            f
        })
        .collect();

    Ok(embedding)
}

/// Fire-and-forget: extract factual notes from a conversation turn and store
/// them in Postgres with embeddings. Intended to be called via `tokio::spawn`
/// so it runs after the response is already sent to the user.
///
/// Calls Haiku to extract 0-3 notes in `category|content` format, embeds each
/// note, and inserts into `director_notes`. Silent on failure.
pub async fn extract_and_store(pool: sqlx::PgPool, message: String, response: String) {
    let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") else {
        return;
    };

    let prompt = format!(
        "Extract 0-3 factual notes from this conversation worth remembering long-term about the user.\n\
         Output one note per line: category|content\n\
         Categories: personal, social, code, projects, style, calendar\n\
         Only extract concrete facts (name, preference, project, deadline, habit).\n\
         If nothing is worth remembering, output nothing at all.\n\
         \n\
         User: {message}\n\
         Assistant: {response}"
    );

    let body = serde_json::json!({
        "model": HAIKU_MODEL,
        "max_tokens": 256,
        "messages": [{"role": "user", "content": prompt}],
    });

    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(EXTRACT_TIMEOUT_SECS))
        .build()
    else {
        return;
    };

    let resp = match client
        .post(ANTHROPIC_MESSAGES_URL)
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ghost memory] extract request failed: {e}");
            return;
        }
    };

    if !resp.status().is_success() {
        return;
    }

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return,
    };

    let text = match json["content"][0]["text"].as_str() {
        Some(t) => t.to_string(),
        None => return,
    };

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(2, '|').collect();
        if parts.len() != 2 {
            continue;
        }
        let category = parts[0].trim();
        let content = parts[1].trim();
        if category.is_empty() || content.is_empty() {
            continue;
        }

        match embed(content).await {
            Ok(embedding) => {
                crate::db::insert_note(&pool, category, content, &embedding).await;
                eprintln!("[ghost memory] stored note [{category}]: {content}");
            }
            Err(e) => {
                eprintln!("[ghost memory] embed failed for note '{content}': {e}");
            }
        }
    }
}
