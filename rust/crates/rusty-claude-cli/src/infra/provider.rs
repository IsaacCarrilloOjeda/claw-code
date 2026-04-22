//! Provider router — dispatches model calls to Anthropic or `OpenRouter`.
//!
//! The settings table (`settings_kv`) controls which provider handles a given
//! agent's calls. `provider.default` is the global fallback; `provider.per_agent`
//! overrides per agent name. Settings are cached in memory and refreshed every
//! 60s by a background task spawned from `daemon_main`.
//!
//! Anthropic path accepts the `system` JSON value as-is — callers may pass the
//! cached array form built by [`crate::infra::cache::build_cached_system`] to
//! get prompt caching. That cache API is **Anthropic-only**; on the `OpenRouter`
//! path we flatten the array to plain text and prepend it as a `system` role
//! message. `DeepSeek` does its own automatic caching, so do NOT send
//! `cache_control` blocks to `OpenRouter`.
//!
//! Kill switch: setting the `GHOST_CODING_AGENT` env var to `"off"` causes
//! `call_model` to short-circuit with `ProviderError::KillSwitched` for the
//! coder, brainstorm, and orchestrator agents. Other agents are unaffected so
//! the kill switch only grounds the new cost-exposed code paths.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::constants::ANTHROPIC_MESSAGES_URL;
use crate::http_client::shared_client;

const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const CALL_TIMEOUT_SECS: u64 = 90;
const REFRESH_INTERVAL_SECS: u64 = 60;

/// Coder-path agents that honor the `GHOST_CODING_AGENT=off` kill switch.
/// Everything else (`chat_dispatcher`, director, research, …) is unaffected.
const KILL_SWITCHED_AGENTS: &[&str] = &["coder", "brainstorm", "orchestrator"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenRouter,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenRouter => "openrouter",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Some(Provider::Anthropic),
            "openrouter" => Some(Provider::OpenRouter),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read: u32,
    pub cache_write: u32,
    /// Actual model string that served the response. For Anthropic + `OpenRouter`
    /// this matches the request; future fallbacks may surface a different name.
    pub model: String,
}

#[derive(Debug)]
pub enum ProviderError {
    /// `GHOST_CODING_AGENT=off` is set and the agent is in the coder group.
    KillSwitched,
    /// HTTP error from the upstream provider, including 4xx and 5xx.
    Http(u16, String),
    /// Request timed out before the provider responded.
    Timeout,
    /// Provider returned a body we couldn't parse or interpret.
    Parse(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::KillSwitched => {
                write!(f, "coding agent is disabled (GHOST_CODING_AGENT=off)")
            }
            ProviderError::Http(code, body) => write!(f, "provider HTTP {code}: {body}"),
            ProviderError::Timeout => write!(f, "provider request timed out"),
            ProviderError::Parse(s) => write!(f, "provider parse error: {s}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// In-memory snapshot of the routing settings, rebuilt every 60s from
/// `settings_kv`. Callers hit this via the `PROVIDER_CONFIG` static.
#[derive(Debug, Clone)]
struct ProviderConfig {
    default: Provider,
    per_agent: HashMap<String, Provider>,
    /// Mirror of `settings_kv.coder.kill_switch` — or-combined with the
    /// `GHOST_CODING_AGENT=off` env var by [`is_kill_switched`] so the
    /// dashboard can flip the switch without a daemon restart.
    kill_switch: bool,
}

impl ProviderConfig {
    fn bootstrap() -> Self {
        Self {
            default: Provider::OpenRouter,
            per_agent: HashMap::new(),
            kill_switch: false,
        }
    }

    fn resolve(&self, agent: &str) -> Provider {
        self.per_agent.get(agent).copied().unwrap_or(self.default)
    }
}

static PROVIDER_CONFIG: OnceLock<Arc<RwLock<ProviderConfig>>> = OnceLock::new();

/// Initialize the cached provider config and spawn a 60s refresh task.
/// Called once from `daemon_main` at startup. Safe to call when the pool is
/// not yet connected — the first refresh tick will populate from DB.
pub fn init(pool: Arc<PgPool>) {
    let arc = Arc::new(RwLock::new(ProviderConfig::bootstrap()));
    if PROVIDER_CONFIG.set(arc.clone()).is_err() {
        // Already initialized — probably a test or duplicate call. Nothing to do.
        return;
    }

    // Load once synchronously-ish (spawn a task to populate immediately, then
    // keep refreshing). Readers before the first load see the bootstrap
    // defaults (openrouter, no overrides) which matches the seeded row.
    let pool_for_bg = pool;
    tokio::spawn(async move {
        // First load: don't wait the full interval.
        refresh_config(&pool_for_bg, &arc).await;

        let mut interval = tokio::time::interval(Duration::from_secs(REFRESH_INTERVAL_SECS));
        interval.tick().await; // skip immediate tick
        loop {
            interval.tick().await;
            refresh_config(&pool_for_bg, &arc).await;
        }
    });
}

async fn refresh_config(pool: &PgPool, slot: &Arc<RwLock<ProviderConfig>>) {
    let default: Option<String> = crate::db::get_setting(pool, "provider.default").await;
    let per_agent: Option<HashMap<String, String>> =
        crate::db::get_setting(pool, "provider.per_agent").await;
    let kill_switch: Option<bool> = crate::db::get_setting(pool, "coder.kill_switch").await;

    let default = default
        .as_deref()
        .and_then(Provider::parse)
        .unwrap_or(Provider::OpenRouter);

    let per_agent = per_agent
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| Provider::parse(&v).map(|p| (k, p)))
        .collect();

    *slot.write().await = ProviderConfig {
        default,
        per_agent,
        kill_switch: kill_switch.unwrap_or(false),
    };
}

/// Resolve the provider for `agent`. Uses the cached config — falls back to
/// `OpenRouter` if `init` hasn't run yet (tests, early startup).
pub async fn provider_for(agent: &str, _pool: &PgPool) -> Provider {
    let Some(lock) = PROVIDER_CONFIG.get() else {
        return Provider::OpenRouter;
    };
    lock.read().await.resolve(agent)
}

/// Execute a chat completion against the chosen provider. `system` is passed
/// through on the Anthropic path; it is flattened to a leading `system`
/// message on the `OpenRouter` path. Pre-built Anthropic cache blocks are safe
/// here — we detect the array shape and lift the text.
pub async fn call_model(
    provider: Provider,
    agent: &str,
    model: &str,
    system: Value,
    messages: Vec<Value>,
    max_tokens: u32,
) -> Result<ModelResponse, ProviderError> {
    if is_kill_switched(agent).await {
        return Err(ProviderError::KillSwitched);
    }
    match provider {
        Provider::Anthropic => call_anthropic(model, system, messages, max_tokens).await,
        Provider::OpenRouter => call_openrouter(model, system, messages, max_tokens).await,
    }
}

/// True when a coder-group agent should refuse to call out. OR of the
/// `GHOST_CODING_AGENT=off` env var and `settings_kv.coder.kill_switch`
/// (cached in `ProviderConfig`), so the dashboard can flip the switch
/// without a restart.
async fn is_kill_switched(agent: &str) -> bool {
    if !KILL_SWITCHED_AGENTS.contains(&agent) {
        return false;
    }
    let env_off = std::env::var("GHOST_CODING_AGENT")
        .ok()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("off"));
    if env_off {
        return true;
    }
    let Some(lock) = PROVIDER_CONFIG.get() else {
        return false;
    };
    lock.read().await.kill_switch
}

async fn call_anthropic(
    model: &str,
    system: Value,
    messages: Vec<Value>,
    max_tokens: u32,
) -> Result<ModelResponse, ProviderError> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| ProviderError::Parse("ANTHROPIC_API_KEY not set".into()))?;

    let body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": system,
        "messages": messages,
    });

    let resp = shared_client()
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(CALL_TIMEOUT_SECS))
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| map_reqwest_err(&e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ProviderError::Http(status.as_u16(), text));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| ProviderError::Parse(format!("anthropic json: {e}")))?;

    let text = json["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let usage = &json["usage"];
    let input_tokens =
        u32::try_from(usage["input_tokens"].as_u64().unwrap_or(0)).unwrap_or(u32::MAX);
    let cache_read =
        u32::try_from(usage["cache_read_input_tokens"].as_u64().unwrap_or(0)).unwrap_or(u32::MAX);
    let cache_write = u32::try_from(usage["cache_creation_input_tokens"].as_u64().unwrap_or(0))
        .unwrap_or(u32::MAX);
    let output_tokens =
        u32::try_from(usage["output_tokens"].as_u64().unwrap_or(0)).unwrap_or(u32::MAX);

    Ok(ModelResponse {
        text,
        input_tokens,
        output_tokens,
        cache_read,
        cache_write,
        model: model.to_string(),
    })
}

async fn call_openrouter(
    model: &str,
    system: Value,
    mut messages: Vec<Value>,
    max_tokens: u32,
) -> Result<ModelResponse, ProviderError> {
    #[derive(Deserialize)]
    struct OpenRouterUsage {
        #[serde(default)]
        prompt_tokens: u64,
        #[serde(default)]
        completion_tokens: u64,
    }

    let api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| ProviderError::Parse("OPENAI_API_KEY not set".into()))?;

    let system_text = anthropic_system_to_plain(&system);
    if !system_text.is_empty() {
        messages.insert(0, json!({ "role": "system", "content": system_text }));
    }

    let body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages,
    });

    let resp = shared_client()
        .post(OPENROUTER_URL)
        .timeout(Duration::from_secs(CALL_TIMEOUT_SECS))
        .bearer_auth(&api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| map_reqwest_err(&e))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ProviderError::Http(status.as_u16(), text));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| ProviderError::Parse(format!("openrouter json: {e}")))?;

    let text = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let usage: OpenRouterUsage =
        serde_json::from_value(json["usage"].clone()).unwrap_or(OpenRouterUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
        });

    let input_tokens = u32::try_from(usage.prompt_tokens).unwrap_or(u32::MAX);
    let output_tokens = u32::try_from(usage.completion_tokens).unwrap_or(u32::MAX);

    Ok(ModelResponse {
        text,
        input_tokens,
        output_tokens,
        cache_read: 0,
        cache_write: 0,
        model: model.to_string(),
    })
}

/// Flatten the Anthropic `system` value into plain text for providers that
/// don't understand content-block arrays. Accepts either a plain string or
/// the cached-array form `[{type:"text", text:"...", ...}, ...]`.
fn anthropic_system_to_plain(system: &Value) -> String {
    if let Some(s) = system.as_str() {
        return s.to_string();
    }
    if let Some(arr) = system.as_array() {
        let mut out = String::new();
        for block in arr {
            if let Some(t) = block.get("text").and_then(Value::as_str) {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str(t);
            }
        }
        return out;
    }
    String::new()
}

fn map_reqwest_err(e: &reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Http(0, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Tests that mutate `GHOST_CODING_AGENT` run serially against this mutex
    // so they can't clobber each other when cargo test runs in parallel.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn provider_parse_accepts_known_values() {
        assert_eq!(Provider::parse("anthropic"), Some(Provider::Anthropic));
        assert_eq!(Provider::parse("OpenRouter"), Some(Provider::OpenRouter));
        assert_eq!(Provider::parse("  ANTHROPIC "), Some(Provider::Anthropic));
        assert_eq!(Provider::parse("unknown"), None);
        assert_eq!(Provider::parse(""), None);
    }

    #[test]
    fn provider_config_resolves_per_agent_then_default() {
        let mut cfg = ProviderConfig::bootstrap();
        cfg.default = Provider::Anthropic;
        cfg.per_agent
            .insert("coder".to_string(), Provider::OpenRouter);
        assert_eq!(cfg.resolve("coder"), Provider::OpenRouter);
        assert_eq!(cfg.resolve("research"), Provider::Anthropic);
    }

    #[tokio::test]
    async fn kill_switch_only_fires_for_coder_group() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("GHOST_CODING_AGENT", "off");
        assert!(is_kill_switched("coder").await);
        assert!(is_kill_switched("brainstorm").await);
        assert!(is_kill_switched("orchestrator").await);
        assert!(!is_kill_switched("research").await);
        assert!(!is_kill_switched("chat_dispatcher").await);
        std::env::remove_var("GHOST_CODING_AGENT");
        assert!(!is_kill_switched("coder").await);
    }

    #[tokio::test]
    async fn kill_switch_case_insensitive() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::set_var("GHOST_CODING_AGENT", "OFF");
        assert!(is_kill_switched("coder").await);
        std::env::set_var("GHOST_CODING_AGENT", "on");
        assert!(!is_kill_switched("coder").await);
        std::env::remove_var("GHOST_CODING_AGENT");
    }

    #[test]
    fn flatten_system_string_passthrough() {
        let v = json!("hello world");
        assert_eq!(anthropic_system_to_plain(&v), "hello world");
    }

    #[test]
    fn flatten_system_array_joins_text_blocks() {
        let v = json!([
            {"type": "text", "text": "stable core", "cache_control": {"type": "ephemeral"}},
            {"type": "text", "text": "dynamic turn context"},
        ]);
        assert_eq!(
            anthropic_system_to_plain(&v),
            "stable core\n\ndynamic turn context"
        );
    }

    #[test]
    fn flatten_system_empty_for_null_or_object() {
        assert_eq!(anthropic_system_to_plain(&json!(null)), "");
        assert_eq!(anthropic_system_to_plain(&json!({"x": 1})), "");
    }
}
