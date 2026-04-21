//! Per-turn event log — the self-correction substrate.
//!
//! Every routed message will eventually write one row to `ghost_events`. This
//! module exposes typed wrappers over the raw `db.rs` helpers so callers don't
//! have to deal with stringly-typed outcome values. Wave 3 wires this into the
//! dispatcher; nothing in the hot path calls it yet.

use crate::db;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Fallback,
    Refused,
    Error,
    Escalated,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::Fallback => "fallback",
            Outcome::Refused => "refused",
            Outcome::Error => "error",
            Outcome::Escalated => "escalated",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "success" => Ok(Outcome::Success),
            "fallback" => Ok(Outcome::Fallback),
            "refused" => Ok(Outcome::Refused),
            "error" => Ok(Outcome::Error),
            "escalated" => Ok(Outcome::Escalated),
            other => Err(format!("unknown outcome: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewEvent<'a> {
    pub job_id: Option<Uuid>,
    pub agent: &'a str,
    pub tier: &'a str,
    pub input: Option<&'a str>,
    pub output: Option<&'a str>,
    pub tokens_in: i32,
    pub tokens_out: i32,
    pub cost_cents: i32,
    pub outcome: Outcome,
}

#[derive(Debug, Clone)]
pub struct EventRecord {
    pub id: Uuid,
    pub job_id: Option<Uuid>,
    pub agent: String,
    pub tier: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub tokens_in: i32,
    pub tokens_out: i32,
    pub cost_cents: i32,
    pub outcome: Outcome,
    pub human_correction: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl TryFrom<db::GhostEventRow> for EventRecord {
    type Error = String;

    fn try_from(row: db::GhostEventRow) -> Result<Self, Self::Error> {
        Ok(EventRecord {
            id: row.id,
            job_id: row.job_id,
            agent: row.agent,
            tier: row.tier,
            input: row.input,
            output: row.output,
            tokens_in: row.tokens_in,
            tokens_out: row.tokens_out,
            cost_cents: row.cost_cents,
            outcome: Outcome::parse(&row.outcome)?,
            human_correction: row.human_correction,
            created_at: row.created_at,
        })
    }
}

/// Append an event. Returns the new event's UUID.
pub async fn record(pool: &PgPool, event: NewEvent<'_>) -> Result<Uuid, sqlx::Error> {
    db::insert_ghost_event(
        pool,
        &db::NewGhostEventRow {
            job_id: event.job_id,
            agent: event.agent,
            tier: event.tier,
            input: event.input,
            output: event.output,
            tokens_in: event.tokens_in,
            tokens_out: event.tokens_out,
            cost_cents: event.cost_cents,
            outcome: event.outcome.as_str(),
        },
    )
    .await
}

/// Most recent N events, newest first. Rows with unparseable outcome strings
/// are skipped silently — they shouldn't exist (the column is written via
/// [`Outcome::as_str`]) but we don't want a corrupt row to poison the list.
pub async fn recent(pool: &PgPool, limit: i64) -> Result<Vec<EventRecord>, sqlx::Error> {
    let rows = db::list_recent_ghost_events(pool, limit).await?;
    Ok(rows.into_iter().filter_map(|r| r.try_into().ok()).collect())
}

/// Most recent N events for a specific agent, newest first.
pub async fn for_agent(
    pool: &PgPool,
    agent: &str,
    limit: i64,
) -> Result<Vec<EventRecord>, sqlx::Error> {
    let rows = db::list_ghost_events_by_agent(pool, agent, limit).await?;
    Ok(rows.into_iter().filter_map(|r| r.try_into().ok()).collect())
}

/// Attach a human correction to an event. Used when Isaac edits or overrides
/// a response — the correction becomes training signal for the dreamer.
pub async fn attach_correction(
    pool: &PgPool,
    event_id: Uuid,
    correction: &str,
) -> Result<(), sqlx::Error> {
    db::update_ghost_event_correction(pool, event_id, correction).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_round_trip_every_variant() {
        for variant in [
            Outcome::Success,
            Outcome::Fallback,
            Outcome::Refused,
            Outcome::Error,
            Outcome::Escalated,
        ] {
            let s = variant.as_str();
            let parsed = Outcome::parse(s).expect("parse must succeed for known variant");
            assert_eq!(parsed, variant, "round-trip failed for {s}");
        }
    }

    #[test]
    fn outcome_parse_rejects_unknown() {
        let err = Outcome::parse("not-a-real-outcome").unwrap_err();
        assert!(err.contains("not-a-real-outcome"));
    }

    #[test]
    fn outcome_parse_is_case_sensitive() {
        assert!(Outcome::parse("Success").is_err());
        assert!(Outcome::parse("SUCCESS").is_err());
    }
}
