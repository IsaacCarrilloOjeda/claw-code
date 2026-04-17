//! Chat dispatcher — lightweight path for no-prefix messages.
//!
//! Loads the core context file from `GHOST_CORE_CONTEXT_PATH`, appends any
//! relevant memory notes (Phase 2 semantic search), then calls Claude Haiku.
//! After responding, spawns a fire-and-forget task to extract and store new
//! notes from the conversation turn.

use std::time::Duration;

use crate::constants::{ANTHROPIC_MESSAGES_URL, HAIKU_MODEL, OPUS_MODEL, SONNET_MODEL};

const AI_TIMEOUT_SECS: u64 = 60;
const MEMORY_SEARCH_LIMIT: i64 = 5;
const SCHOLAR_SEARCH_LIMIT: i64 = 3;
const SCHOLAR_SOLUTION_THRESHOLD: f64 = 0.15;
const SCHOLAR_FAILED_THRESHOLD: f64 = 0.25;
const SCHOLAR_DEDUP_THRESHOLD: f64 = 0.10;

/// Dispatch a no-prefix message to Claude Haiku. Returns the response text.
///
/// `history` is a slice of prior `{role, content}` message objects (last N exchanges).
/// These are prepended to the messages array so GHOST maintains conversational context.
///
/// If `pool` is provided, semantic memory search runs before the call and
/// note extraction runs asynchronously after.
pub async fn dispatch(
    message: &str,
    history: &[serde_json::Value],
    _job_id: &str,
    pool: Option<&sqlx::PgPool>,
) -> Result<String, String> {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    // Strip filler greetings/sign-offs before any API call.
    let cleaned = crate::compress::strip_filler(message);

    // Intake polish: rewrite ambiguous/complex prompts into tight specs.
    // Failure is never fatal — falls through with cleaned message.
    let polished = crate::compress::intake_polish(&cleaned)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[ghost intake] failed: {e}, using cleaned message");
            cleaned.clone()
        });

    // Apply project-aware abbreviations (Phase E2).
    let polished = crate::compress::apply_abbreviations(&polished);

    let core_context = load_core_context();

    // Embed the polished message once; reuse for memory search + scholar search.
    let query_embedding = crate::memory::embed(&polished).await.ok();

    let memory_context = load_memory_context_with_embedding(query_embedding.as_deref(), pool).await;
    let scholar_context = if let Some(ref emb) = query_embedding {
        load_scholar_context(&polished, emb, pool).await
    } else {
        String::new()
    };

    let do_search = should_search(&polished);
    eprintln!("[ghost chat] should_search={do_search} for: {message}");
    let web_context = if do_search {
        load_web_context(&polished).await
    } else {
        String::new()
    };

    let mut system = core_context;
    if !memory_context.is_empty() {
        system.push_str("\n\n## What you remember about Isaac\n<memory_notes>\n");
        // Cap the memory block to prevent a poisoned store from bloating the prompt
        let capped = if memory_context.len() > 4096 {
            &memory_context[..4096]
        } else {
            &memory_context
        };
        system.push_str(capped);
        system.push_str("\n</memory_notes>");
    }
    if !scholar_context.is_empty() {
        system.push_str("\n\n");
        system.push_str(&scholar_context);
    }
    if web_context.is_empty() {
        eprintln!("[ghost chat] web_context is empty — nothing to inject");
    } else {
        eprintln!(
            "[ghost chat] injecting {} bytes of web context",
            web_context.len()
        );
        system.push_str(
            "\n\n## Current web search results\nUse these results to answer the user's question:\n",
        );
        system.push_str(&web_context);
    }

    let mut messages: Vec<serde_json::Value> = history.to_vec();
    messages.push(serde_json::json!({"role": "user", "content": polished}));

    // Cascade: Haiku → Sonnet → Opus (escalate on low-confidence responses)
    let body = build_request_body(HAIKU_MODEL, &system, &messages);
    let mut response = call_anthropic(&api_key, &body).await?;
    let mut tier = "haiku";

    if crate::compress::is_low_confidence(&response) {
        eprintln!("[ghost chat] haiku low-confidence, escalating to sonnet");
        let body = build_request_body(SONNET_MODEL, &system, &messages);
        response = call_anthropic(&api_key, &body).await?;
        tier = "sonnet";

        if crate::compress::is_low_confidence(&response) {
            eprintln!("[ghost chat] sonnet low-confidence, escalating to opus");
            let body = build_request_body(OPUS_MODEL, &system, &messages);
            response = call_anthropic(&api_key, &body).await?;
            tier = "opus";
        }
    }

    eprintln!("[ghost chat] tier={tier} for: {message}");

    // Fire-and-forget: extract facts from this turn and store them.
    // Uses the original message (not cleaned) for accurate memory extraction.
    if let Some(p) = pool {
        let pool_owned = p.clone();
        let msg = message.to_string();
        let resp = response.clone();
        tokio::spawn(async move {
            crate::memory::extract_and_store(pool_owned, msg, resp).await;
        });

        // Store problem+solution in scholar DB for future cache hits.
        let pool_scholar = p.clone();
        let msg_scholar = polished.clone();
        let resp_scholar = response.clone();
        tokio::spawn(async move {
            store_scholar_solution(pool_scholar, msg_scholar, resp_scholar).await;
        });
    }

    Ok(response)
}

/// Embed the incoming message and pull the top-N most relevant memory notes.
/// Returns a formatted string to inject into the system prompt, or empty string
/// if the pool is absent, embedding fails, or no notes exist yet.
pub(crate) async fn load_memory_context(message: &str, pool: Option<&sqlx::PgPool>) -> String {
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

/// Like `load_memory_context` but takes a pre-computed embedding to avoid
/// redundant API calls when the caller already embedded the message.
async fn load_memory_context_with_embedding(
    embedding: Option<&[f32]>,
    pool: Option<&sqlx::PgPool>,
) -> String {
    let Some(pool) = pool else {
        return String::new();
    };
    let Some(embedding) = embedding else {
        return String::new();
    };

    let notes = crate::db::search_notes(pool, embedding, MEMORY_SEARCH_LIMIT).await;
    if notes.is_empty() {
        return String::new();
    }

    notes
        .iter()
        .map(|n| format!("- [{}] {}", n.category, n.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Search the scholar DB for previously solved similar problems and known-bad
/// approaches. Returns formatted context to inject into the system prompt.
async fn load_scholar_context(
    message: &str,
    embedding: &[f32],
    pool: Option<&sqlx::PgPool>,
) -> String {
    let Some(pool) = pool else {
        return String::new();
    };

    let results =
        crate::db::search_scholar_with_distance(pool, embedding, SCHOLAR_SEARCH_LIMIT).await;
    if results.is_empty() {
        return String::new();
    }

    let mut sections = Vec::new();

    // Inject successful solutions (high similarity, well-tested)
    for (sol, distance) in &results {
        if *distance < SCHOLAR_SOLUTION_THRESHOLD && sol.success_count >= 3 {
            let mut block = format!(
                "## Previously solved similar problem\nProblem: {}\n",
                sol.problem_sig
            );
            block.push_str(&sol.solution);
            sections.push(block);
            eprintln!(
                "[ghost scholar] solution hit (dist={distance:.3}, count={}): {}",
                sol.success_count, sol.problem_sig
            );
            break; // only inject the best match
        }
    }

    // Inject failed attempts (looser threshold)
    let mut failed_lines = Vec::new();
    for (sol, distance) in &results {
        if *distance < SCHOLAR_FAILED_THRESHOLD && !sol.failed_attempts.is_empty() {
            for attempt in &sol.failed_attempts {
                failed_lines.push(format!("- {attempt}"));
            }
            eprintln!(
                "[ghost scholar] failed-attempts hit (dist={distance:.3}): {} attempt(s)",
                sol.failed_attempts.len()
            );
        }
    }
    if !failed_lines.is_empty() {
        let mut block = "## Known bad approaches for similar problems\nDO NOT TRY:\n".to_string();
        block.push_str(&failed_lines.join("\n"));
        sections.push(block);
    }

    let _ = message; // used for logging context if needed later
    sections.join("\n\n")
}

/// Store a problem+solution pair in the scholar DB (fire-and-forget).
async fn store_scholar_solution(pool: sqlx::PgPool, message: String, response: String) {
    let embedding = match crate::memory::embed(&message).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[ghost scholar] embed failed, skipping store: {e}");
            return;
        }
    };

    // Check for near-duplicate — if very similar problem exists, bump its count
    let existing = crate::db::search_scholar_with_distance(&pool, &embedding, 1).await;
    if let Some((sol, distance)) = existing.first() {
        if *distance < SCHOLAR_DEDUP_THRESHOLD {
            eprintln!(
                "[ghost scholar] dedup hit (dist={distance:.3}), incrementing {}",
                sol.id
            );
            crate::db::increment_scholar_success(&pool, &sol.id).await;
            return;
        }
    }

    let stored = crate::db::insert_scholar(
        &pool,
        &message,
        Some(&embedding),
        &response,
        None, // solution_lang
        None, // context_file
    )
    .await;

    if stored {
        eprintln!("[ghost scholar] stored new solution for: {message}");
    }
}

/// Simple heuristic: returns `true` if the message looks like it would benefit
/// from a web search. Avoids burning Brave Search quota on greetings / short
/// confirmations.
fn should_search(message: &str) -> bool {
    const SEARCH_KEYWORDS: &[&str] = &[
        "search", "look up", "find", "what is", "what are", "who is", "who are", "when is",
        "when was", "where is", "how to", "how do", "latest", "current", "news", "weather",
        "price", "stock", "define", "explain",
    ];

    if message.contains('?') {
        return true;
    }

    let lower = message.to_ascii_lowercase();
    for kw in SEARCH_KEYWORDS {
        if lower.contains(kw) {
            return true;
        }
    }

    false
}

/// Run a Brave web search for the message and format results for injection.
/// Returns empty string if `BRAVE_API_KEY` is unset or the search returns nothing.
async fn load_web_context(message: &str) -> String {
    match crate::search::web_search(message).await {
        Ok(results) if !results.is_empty() => {
            eprintln!("[ghost search] {} result(s) for: {message}", results.len());
            crate::search::format_results(&results)
        }
        Ok(_) => String::new(),
        Err(e) => {
            eprintln!("[ghost search] search failed: {e}");
            String::new()
        }
    }
}

/// Load the core context file. Falls back to a minimal default if the env var
/// is unset or the file cannot be read.
pub(crate) fn load_core_context() -> String {
    let path = match std::env::var("GHOST_CORE_CONTEXT_PATH") {
        Ok(p) if !p.is_empty() => p,
        _ => return "You are GHOST, a personal AI assistant.".to_string(),
    };
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("[ghost chat] failed to read core context from {path}: {e}");
        "You are GHOST, a personal AI assistant.".to_string()
    })
}

fn build_request_body(
    model: &str,
    system: &str,
    messages: &[serde_json::Value],
) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "max_tokens": 1024,
        "system": system,
        "messages": messages,
    })
}

async fn call_anthropic(api_key: &str, body: &serde_json::Value) -> Result<String, String> {
    let client = crate::http_client::shared_client();

    let resp = client
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(AI_TIMEOUT_SECS))
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
