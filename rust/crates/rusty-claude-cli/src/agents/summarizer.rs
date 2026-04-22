//! Summarize-as-you-go — per-turn KEEP/DROP classifier + 2-sentence summary.
//!
//! Fires in the background after a coder turn (`tokio::spawn`). The classifier
//! decides whether the exchange is worth remembering; KEEP turns get a short
//! summary, embed, and insert into `coder_condensate` keyed by `chat_id`. The
//! coder agent (Prompt B) calls [`relevant_condensate`] on each new turn to
//! pull the top-K matches for injection as "## Earlier in this chat".
//!
//! Routed through the provider router so the classifier/summarizer follow the
//! same Anthropic ↔ `OpenRouter` switch as the coder itself. `DeepSeek` via
//! `OpenRouter` is the cost-optimized default.

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::infra::provider::{self, Provider, ProviderError};

/// Default `OpenRouter` model for classify + summarize. Overridable via the
/// `GHOST_SUMMARIZER_MODEL` env var for experiments.
const DEFAULT_OPENROUTER_MODEL: &str = "deepseek/deepseek-chat";
/// Fallback model used when the resolved provider is Anthropic — we still
/// want the classifier to be cheap.
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-haiku-4-5-20251001";

const CLASSIFY_SYSTEM: &str = "Classify this exchange. Return exactly one word. \
KEEP if it is substantive project work (design decisions, code changes, bugs \
resolved, non-obvious facts). DROP if it is ephemeral (greetings, IT help, \
typo fixes, clarifications).";

const SUMMARIZE_SYSTEM: &str =
    "Summarize in exactly 2 sentences: what was decided, what changed. No preamble.";

/// Fire-and-forget classifier + summarizer. Never panics or returns — errors
/// are logged via `eprintln!`. Called via `tokio::spawn` from the coder path
/// immediately after the assistant reply is built.
pub async fn classify_and_condense(
    chat_id: Uuid,
    turn_idx: i32,
    user_msg: String,
    assistant_msg: String,
    pool: PgPool,
) {
    let enabled: Option<bool> = crate::db::get_setting(&pool, "coder.summarize_as_you_go").await;
    if enabled != Some(true) {
        return;
    }

    let exchange = format_exchange(&user_msg, &assistant_msg);

    let verdict = match classify(&exchange).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[ghost summarizer] classify failed: {e}");
            return;
        }
    };

    if verdict != Verdict::Keep {
        return;
    }

    let summary = match summarize(&exchange).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[ghost summarizer] summarize failed: {e}");
            return;
        }
    };

    let embedding = crate::memory::embed(&summary).await.ok();

    if let Err(e) = crate::db::insert_coder_condensate(
        &pool,
        &chat_id,
        turn_idx,
        &summary,
        embedding.as_deref(),
    )
    .await
    {
        eprintln!("[ghost summarizer] insert failed: {e}");
    }
}

/// Pull top-K condensate summaries from this chat by cosine similarity to
/// `query`. Empty Vec when the pool is unavailable, the embed provider is
/// absent, or no rows exist yet for this chat.
pub async fn relevant_condensate(
    pool: &PgPool,
    chat_id: &Uuid,
    query: &str,
    k: i64,
) -> Vec<String> {
    let embedding = match crate::memory::embed(query).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[ghost summarizer] embed failed for query: {e}");
            return Vec::new();
        }
    };
    crate::db::search_coder_condensate(pool, chat_id, &embedding, k).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Keep,
    Drop,
}

fn parse_verdict(raw: &str) -> Verdict {
    let trimmed = raw.trim().to_ascii_uppercase();
    if trimmed.starts_with("KEEP") {
        Verdict::Keep
    } else {
        Verdict::Drop
    }
}

fn format_exchange(user_msg: &str, assistant_msg: &str) -> String {
    format!("USER: {user_msg}\n\nASSISTANT: {assistant_msg}")
}

async fn classify(exchange: &str) -> Result<Verdict, ProviderError> {
    let resp = call_summarizer(CLASSIFY_SYSTEM, exchange, 4).await?;
    Ok(parse_verdict(&resp.text))
}

async fn summarize(exchange: &str) -> Result<String, ProviderError> {
    let resp = call_summarizer(SUMMARIZE_SYSTEM, exchange, 256).await?;
    Ok(resp.text.trim().to_string())
}

async fn call_summarizer(
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<crate::infra::provider::ModelResponse, ProviderError> {
    // Prefer OpenRouter (cheap) but route through whatever the user configured
    // for the "summarizer" logical agent — with OpenRouter as the default.
    let provider = resolve_summarizer_provider();
    let model = resolve_summarizer_model(provider);

    let messages: Vec<Value> = vec![json!({ "role": "user", "content": user })];
    provider::call_model(
        provider,
        "summarizer",
        &model,
        json!(system),
        messages,
        max_tokens,
    )
    .await
}

fn resolve_summarizer_provider() -> Provider {
    // Classifier is a coder-adjacent utility, not a user-visible agent. Lean
    // on OpenRouter unless the caller overrode via env.
    if std::env::var("GHOST_SUMMARIZER_PROVIDER")
        .ok()
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("anthropic"))
    {
        Provider::Anthropic
    } else {
        Provider::OpenRouter
    }
}

fn resolve_summarizer_model(provider: Provider) -> String {
    if let Ok(m) = std::env::var("GHOST_SUMMARIZER_MODEL") {
        if !m.trim().is_empty() {
            return m;
        }
    }
    match provider {
        Provider::Anthropic => DEFAULT_ANTHROPIC_MODEL.to_string(),
        Provider::OpenRouter => DEFAULT_OPENROUTER_MODEL.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_verdict_recognizes_keep_variants() {
        assert_eq!(parse_verdict("KEEP"), Verdict::Keep);
        assert_eq!(parse_verdict(" keep "), Verdict::Keep);
        assert_eq!(parse_verdict("Keep."), Verdict::Keep);
        assert_eq!(parse_verdict("KEEP - it's a design call"), Verdict::Keep);
    }

    #[test]
    fn parse_verdict_defaults_to_drop_on_ambiguity() {
        assert_eq!(parse_verdict("DROP"), Verdict::Drop);
        assert_eq!(parse_verdict(""), Verdict::Drop);
        assert_eq!(parse_verdict("idk"), Verdict::Drop);
        assert_eq!(parse_verdict("maybe keep"), Verdict::Drop);
    }

    #[test]
    fn format_exchange_includes_both_sides() {
        let s = format_exchange("hi", "hello back");
        assert!(s.contains("USER: hi"));
        assert!(s.contains("ASSISTANT: hello back"));
    }

    #[test]
    fn resolve_summarizer_model_honors_env_override() {
        std::env::set_var("GHOST_SUMMARIZER_MODEL", "deepseek/deepseek-coder");
        assert_eq!(
            resolve_summarizer_model(Provider::OpenRouter),
            "deepseek/deepseek-coder"
        );
        std::env::remove_var("GHOST_SUMMARIZER_MODEL");
        assert_eq!(
            resolve_summarizer_model(Provider::OpenRouter),
            DEFAULT_OPENROUTER_MODEL
        );
        assert_eq!(
            resolve_summarizer_model(Provider::Anthropic),
            DEFAULT_ANTHROPIC_MODEL
        );
    }
}
