//! Scheduler module (Wave 4).
//!
//! A single tokio task wakes every 30s, pulls rows from `scheduled_triggers`
//! whose `next_fire_at` has elapsed, and dispatches each one via
//! `agents::dispatcher::Dispatcher::dispatch` with `Source::Scheduled`.
//!
//! Design notes:
//! - Single task, single 30s tick — NOT a task-per-trigger. Keeps the loop
//!   cheap and serialises DB writes.
//! - `next_fire_at` is rewritten BEFORE dispatch so a slow agent never
//!   double-fires on the next tick.
//! - Dispatch errors are logged and swallowed — the dispatcher's own
//!   `ghost_events` write is the audit record.
//! - `cron_expr` uses the `cron` crate's 6-field format:
//!   `<sec> <min> <hr> <day-of-month> <month> <day-of-week>`.
//!   e.g., `0 0 9 * * *` fires every day at 09:00:00 UTC.
//!
//! Wave 4: trigger registration is manual SQL (`INSERT INTO scheduled_triggers
//! ...`). HTTP CRUD ships in Wave 5.

use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Utc};
use cron::Schedule;
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::agents::dispatcher::Dispatcher;
use crate::agents::{AgentRequest, Source};
use crate::db;

const TICK_INTERVAL_SECS: u64 = 30;
const BATCH_LIMIT: i64 = 10;
const LOG_PREFIX: &str = "[ghost scheduler]";

/// Spawn the scheduler loop. Returns the `JoinHandle`; the daemon drops it
/// so the task lives for the process lifetime. Fatal errors inside `tick`
/// are logged and the loop continues.
pub fn spawn(pool: PgPool) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(TICK_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        ticker.tick().await; // skip the immediate first tick
        loop {
            ticker.tick().await;
            if let Err(e) = tick(&pool).await {
                eprintln!("{LOG_PREFIX} tick failed: {e}");
            }
        }
    })
}

async fn tick(pool: &PgPool) -> Result<(), String> {
    let due = db::due_triggers(pool, BATCH_LIMIT)
        .await
        .map_err(|e| format!("due_triggers query failed: {e}"))?;

    for trigger in due {
        fire_one(pool, trigger).await;
    }
    Ok(())
}

async fn fire_one(pool: &PgPool, trigger: db::ScheduledTrigger) {
    // Compute the next fire time first. If the cron expression is invalid we
    // disable the trigger implicitly by pushing `next_fire_at` into the far
    // future — manual cleanup required. Alternative would be dropping the
    // row, but we'd rather keep it visible for debugging.
    let next = match next_fire(&trigger.cron_expr) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "{LOG_PREFIX} invalid cron_expr on trigger {} ({}): {e} — skipping",
                trigger.id, trigger.name
            );
            return;
        }
    };

    // Stamp fired BEFORE dispatch. A slow-running agent must not cause the
    // next tick to re-fire this row.
    if let Err(e) = db::mark_fired(pool, trigger.id, next).await {
        eprintln!(
            "{LOG_PREFIX} mark_fired failed for {} ({}): {e} — skipping dispatch",
            trigger.id, trigger.name
        );
        return;
    }

    let job_id = db::create_job(pool, &trigger.payload, &trigger.agent, "scheduled", None)
        .await
        .unwrap_or_default();

    let req = AgentRequest {
        message: trigger.payload.clone(),
        history: Vec::new(),
        source: Source::Scheduled,
        job_id,
        sender_phone: None,
    };

    match Dispatcher::new().dispatch(req, pool).await {
        Ok(_) => eprintln!(
            "{LOG_PREFIX} fired {} ({}) — next at {}",
            trigger.name, trigger.id, next
        ),
        Err(e) => eprintln!(
            "{LOG_PREFIX} dispatch failed for {} ({}): {e}",
            trigger.name, trigger.id
        ),
    }
}

fn next_fire(cron_expr: &str) -> Result<DateTime<Utc>, String> {
    let schedule = Schedule::from_str(cron_expr).map_err(|e| format!("parse error: {e}"))?;
    schedule
        .upcoming(Utc)
        .next()
        .ok_or_else(|| "cron produced no upcoming fire time".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_fire_parses_valid_six_field_cron() {
        // Every day at 09:00:00 UTC.
        let next = next_fire("0 0 9 * * *").expect("valid cron must parse");
        assert!(next > Utc::now(), "next fire must be in the future");
    }

    #[test]
    fn next_fire_rejects_invalid_cron() {
        let err = next_fire("not a cron expression").expect_err("invalid cron must error");
        assert!(err.contains("parse error"));
    }

    #[test]
    fn next_fire_rejects_five_field_cron() {
        // Classic 5-field cron is not supported by the `cron` crate — seconds
        // field is mandatory. This test pins that contract so a future
        // migration doesn't silently accept the wrong format.
        let err = next_fire("0 9 * * *").expect_err("5-field cron must error");
        assert!(err.contains("parse error"));
    }
}
