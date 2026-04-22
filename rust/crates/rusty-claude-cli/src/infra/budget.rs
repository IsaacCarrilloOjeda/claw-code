//! Per-agent daily budget enforcement.
//!
//! Before a dispatcher calls an agent it asks [`check`] whether today's spend
//! has hit the hard cap. After a successful call it calls [`debit`] with the
//! real token counts. The hardcoded cap table here is a floor — callers that
//! plug in a model + provider via [`debit_with_provider`] also mirror the
//! spend into the `coder_spend` audit ledger.
//!
//! For coder-group agents the cap can be overridden at runtime via
//! `settings_kv.coder.budget_cents_per_day`; see [`cap_for_async`].

use crate::agents::ModelTier;
use crate::db;
use sqlx::PgPool;

/// Per-agent daily cap in US cents. Unknown agents fall back to
/// [`DEFAULT_CAP_CENTS`]. These numbers are deliberately conservative — the
/// cap is a guardrail against runaway spend, not a spending target.
const DAILY_CAP_CENTS: &[(&str, i64)] = &[
    ("chat_dispatcher", 100), // $1.00/day — Haiku is cheap
    ("director", 500),        // $5.00/day — Sonnet
    ("research", 300),
    ("calendar", 100),
    ("docs", 100),
    ("chief_of_staff", 200),
    ("dreamer", 200),
    ("email", 100),
    ("coder", 200),
    ("brainstorm", 100),
    ("orchestrator", 200),
    ("law", 300),
    ("it_guide", 200),
    ("alarm", 10),
];

const DEFAULT_CAP_CENTS: i64 = 100;

/// Agents whose cap can be overridden by `settings_kv.coder.budget_cents_per_day`.
const CODER_AGENTS: &[&str] = &["coder", "brainstorm", "orchestrator"];

/// Look up the hardcoded daily cap for an agent. Unknown agents get the
/// default cap. Use [`cap_for_async`] to honor the runtime settings override
/// for coder-group agents.
pub fn cap_for(agent: &str) -> i64 {
    DAILY_CAP_CENTS
        .iter()
        .find_map(|(name, cents)| (*name == agent).then_some(*cents))
        .unwrap_or(DEFAULT_CAP_CENTS)
}

/// Settings-aware cap lookup. Falls back to [`cap_for`] for non-coder agents
/// or when the setting is absent / invalid.
pub async fn cap_for_async(pool: &PgPool, agent: &str) -> i64 {
    if !CODER_AGENTS.contains(&agent) {
        return cap_for(agent);
    }
    match db::get_setting::<i64>(pool, "coder.budget_cents_per_day").await {
        Some(v) if v > 0 => v,
        _ => cap_for(agent),
    }
}

/// Cost of a single call in cents given tier + token counts. Rates are per 1M
/// tokens and rounded up so fractional cents don't silently leak past the cap.
pub fn cost_cents(tier: ModelTier, tokens_in: i64, tokens_out: i64) -> i64 {
    let (in_per_m, out_per_m) = match tier {
        ModelTier::Fast => (15, 60),     // Haiku cents per 1M
        ModelTier::Code => (14, 28),     // DeepSeek cents per 1M
        ModelTier::Mid => (300, 1500),   // Sonnet cents per 1M
        ModelTier::Full => (1500, 7500), // Opus cents per 1M
    };
    (tokens_in * in_per_m + tokens_out * out_per_m + 999_999) / 1_000_000
}

#[derive(Debug, Clone)]
pub struct BudgetStatus {
    pub agent: String,
    pub spent_cents: i64,
    pub cap_cents: i64,
    pub calls_today: i32,
}

impl BudgetStatus {
    pub fn remaining_cents(&self) -> i64 {
        self.cap_cents - self.spent_cents
    }

    pub fn is_blown(&self) -> bool {
        self.spent_cents >= self.cap_cents
    }
}

/// Check today's budget for an agent. Always returns a status — if no spend
/// row exists yet, spent = 0 and calls = 0. For coder-group agents the cap
/// is sourced from `settings_kv` when set.
pub async fn check(pool: &PgPool, agent: &str) -> Result<BudgetStatus, sqlx::Error> {
    let (spent_cents, calls_today) = db::get_agent_spend_today(pool, agent).await?;
    Ok(BudgetStatus {
        agent: agent.to_string(),
        spent_cents,
        cap_cents: cap_for_async(pool, agent).await,
        calls_today,
    })
}

/// Debit the budget after a successful agent call. Upserts today's aggregate
/// row and, when `model`/`provider` are provided (coder path), appends a row
/// to the `coder_spend` audit ledger.
pub async fn debit(
    pool: &PgPool,
    agent: &str,
    tokens_in: i64,
    tokens_out: i64,
    cost_cents: i64,
) -> Result<(), sqlx::Error> {
    db::upsert_agent_spend(pool, agent, tokens_in, tokens_out, cost_cents).await
}

/// Debit + audit ledger. Use this from the provider-routed paths (coder,
/// brainstorm, orchestrator) so each model call lands in `coder_spend` with
/// the exact model name and provider it hit. `cache_read` is recorded for
/// Anthropic cache hits — pass 0 on providers that don't surface it.
#[allow(clippy::too_many_arguments)]
pub async fn debit_with_provider(
    pool: &PgPool,
    agent: &str,
    model: &str,
    provider: &str,
    tokens_in: u32,
    tokens_out: u32,
    cache_read: u32,
    cost_cents: i64,
    job_id: Option<uuid::Uuid>,
) -> Result<(), sqlx::Error> {
    db::upsert_agent_spend(
        pool,
        agent,
        i64::from(tokens_in),
        i64::from(tokens_out),
        cost_cents,
    )
    .await?;
    let cost_i32 = i32::try_from(cost_cents).unwrap_or(i32::MAX);
    db::record_spend(
        pool, agent, model, provider, tokens_in, tokens_out, cache_read, cost_i32, job_id,
    )
    .await
}

/// Today's spend across every agent that has called. Used by the upcoming
/// `/agents/budget` endpoint; caps are joined in via [`cap_for`].
pub async fn today(pool: &PgPool) -> Result<Vec<BudgetStatus>, sqlx::Error> {
    let rows = db::list_agent_spend_today(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(agent, spent_cents, calls_today)| BudgetStatus {
            cap_cents: cap_for(&agent),
            agent,
            spent_cents,
            calls_today,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_for_known_agents() {
        assert_eq!(cap_for("chat_dispatcher"), 100);
        assert_eq!(cap_for("director"), 500);
        assert_eq!(cap_for("coder"), 200);
        assert_eq!(cap_for("brainstorm"), 100);
        assert_eq!(cap_for("orchestrator"), 200);
        assert_eq!(cap_for("alarm"), 10);
    }

    #[test]
    fn cap_for_unknown_agent_is_default() {
        assert_eq!(cap_for("nonexistent_agent"), DEFAULT_CAP_CENTS);
        assert_eq!(cap_for(""), DEFAULT_CAP_CENTS);
    }

    #[test]
    fn cost_cents_rounds_up() {
        // 1 input token on Haiku is a tiny fraction of a cent — must still
        // round up to 1 cent so we don't silently undercount.
        assert_eq!(cost_cents(ModelTier::Fast, 1, 0), 1);
        assert_eq!(cost_cents(ModelTier::Fast, 0, 1), 1);
    }

    #[test]
    fn cost_cents_zero_tokens_zero_cost() {
        assert_eq!(cost_cents(ModelTier::Fast, 0, 0), 0);
        assert_eq!(cost_cents(ModelTier::Mid, 0, 0), 0);
        assert_eq!(cost_cents(ModelTier::Full, 0, 0), 0);
    }

    #[test]
    fn cost_cents_one_million_tokens_matches_rate() {
        assert_eq!(cost_cents(ModelTier::Fast, 1_000_000, 0), 15);
        assert_eq!(cost_cents(ModelTier::Fast, 0, 1_000_000), 60);
        assert_eq!(cost_cents(ModelTier::Mid, 1_000_000, 1_000_000), 1800);
        assert_eq!(cost_cents(ModelTier::Full, 1_000_000, 1_000_000), 9000);
        assert_eq!(cost_cents(ModelTier::Code, 1_000_000, 1_000_000), 42);
    }

    #[test]
    fn budget_status_is_blown_at_cap() {
        let under = BudgetStatus {
            agent: "x".into(),
            spent_cents: 99,
            cap_cents: 100,
            calls_today: 3,
        };
        assert!(!under.is_blown());
        assert_eq!(under.remaining_cents(), 1);

        let at = BudgetStatus {
            agent: "x".into(),
            spent_cents: 100,
            cap_cents: 100,
            calls_today: 5,
        };
        assert!(at.is_blown());
        assert_eq!(at.remaining_cents(), 0);

        let over = BudgetStatus {
            agent: "x".into(),
            spent_cents: 150,
            cap_cents: 100,
            calls_today: 7,
        };
        assert!(over.is_blown());
        assert_eq!(over.remaining_cents(), -50);
    }
}
