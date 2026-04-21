-- Scheduled triggers table (Wave 4).
--
-- One row per recurring agent fire. The `infra/scheduler` tokio task polls
-- this table every 30s, selects `WHERE enabled = TRUE AND next_fire_at <= now()`,
-- dispatches each due row via Dispatcher::dispatch, and rewrites `next_fire_at`
-- from `cron_expr` using the `cron` crate.
--
-- NOTE: `cron_expr` uses the `cron` crate's 6-field format:
--   <sec> <min> <hr> <day-of-month> <month> <day-of-week>
-- e.g., `0 0 9 * * *` = every day at 09:00:00. This is NOT classic 5-field
-- cron — the leading seconds field is mandatory.

CREATE TABLE IF NOT EXISTS scheduled_triggers (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL,
    cron_expr     TEXT NOT NULL,
    agent         TEXT NOT NULL,
    payload       TEXT NOT NULL,
    enabled       BOOLEAN NOT NULL DEFAULT TRUE,
    last_fired_at TIMESTAMPTZ,
    next_fire_at  TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_scheduled_triggers_next_fire
    ON scheduled_triggers (next_fire_at)
    WHERE enabled = TRUE;
