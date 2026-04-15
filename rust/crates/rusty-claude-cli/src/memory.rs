//! Memory system for GHOST — embeddings, semantic search, and note extraction.
//!
//! `embed()` tries providers in order:
//!   1. `VOYAGE_API_KEY` → Voyage AI `voyage-3` with `output_dimension=1536`
//!   2. `OPENAI_API_KEY` → `OpenAI` `text-embedding-3-small` (1536 dims)
//!
//! Both produce 1536-dim vectors matching the DB schema. When neither key is
//! set, `embed()` returns `Err` and notes are stored without a vector.
//!
//! `extract_and_store()` is fire-and-forget: call via `tokio::spawn` after a
//! chat response so it never blocks the user-facing latency path.

use std::time::Duration;

const VOYAGE_EMBED_URL: &str = "https://api.voyageai.com/v1/embeddings";
const VOYAGE_EMBED_MODEL: &str = "voyage-3";
const VOYAGE_OUTPUT_DIM: u32 = 1536; // match DB VECTOR(1536)

const OPENAI_EMBED_URL: &str = "https://api.openai.com/v1/embeddings";
const OPENAI_EMBED_MODEL: &str = "text-embedding-3-small";

const EMBED_TIMEOUT_SECS: u64 = 10;
const EXTRACT_TIMEOUT_SECS: u64 = 15;
const HAIKU_MODEL: &str = "claude-haiku-4-5-20251001";
const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";

/// Embed `text` as a 1536-dim vector.
///
/// Tries Voyage AI first (`VOYAGE_API_KEY`), then `OpenAI` (`OPENAI_API_KEY`).
/// Returns `Err` if no key is available or the API call fails.
pub async fn embed(text: &str) -> Result<Vec<f32>, String> {
    if let Ok(key) = std::env::var("VOYAGE_API_KEY") {
        return embed_voyage(text, &key).await;
    }
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        return embed_openai(text, &key).await;
    }
    Err("no embedding key set (VOYAGE_API_KEY or OPENAI_API_KEY)".to_string())
}

async fn embed_voyage(text: &str, api_key: &str) -> Result<Vec<f32>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(EMBED_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let body = serde_json::json!({
        "model": VOYAGE_EMBED_MODEL,
        "input": text,
        "output_dimension": VOYAGE_OUTPUT_DIM,
    });

    let resp = client
        .post(VOYAGE_EMBED_URL)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Voyage embed request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Voyage embed error {status}: {body}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Voyage response: {e}"))?;

    parse_embedding(&json["data"][0]["embedding"], "Voyage")
}

async fn embed_openai(text: &str, api_key: &str) -> Result<Vec<f32>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(EMBED_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let body = serde_json::json!({
        "model": OPENAI_EMBED_MODEL,
        "input": text,
    });

    let resp = client
        .post(OPENAI_EMBED_URL)
        .bearer_auth(api_key)
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
        .map_err(|e| format!("Failed to parse OpenAI response: {e}"))?;

    parse_embedding(&json["data"][0]["embedding"], "OpenAI")
}

fn parse_embedding(value: &serde_json::Value, provider: &str) -> Result<Vec<f32>, String> {
    let arr = value
        .as_array()
        .ok_or_else(|| format!("missing embedding array in {provider} response"))?;
    let vec = arr
        .iter()
        .map(|v| {
            #[allow(clippy::cast_possible_truncation)]
            let f = v.as_f64().unwrap_or(0.0) as f32;
            f
        })
        .collect();
    Ok(vec)
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
                crate::db::insert_note(&pool, category, content, Some(&embedding)).await;
                eprintln!("[ghost memory] stored note [{category}]: {content}");
            }
            Err(e) => {
                eprintln!("[ghost memory] embed failed, storing without vector: {e}");
                crate::db::insert_note(&pool, category, content, None).await;
            }
        }
    }
}
