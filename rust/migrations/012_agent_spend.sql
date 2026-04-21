-- Daily per-agent token + dollar counters. One row per (agent, day).
-- Budget enforcement reads + writes this table.

CREATE TABLE IF NOT EXISTS agent_spend (
    agent       TEXT NOT NULL,
    day         DATE NOT NULL,
    tokens_in   BIGINT NOT NULL DEFAULT 0,
    tokens_out  BIGINT NOT NULL DEFAULT 0,
    cost_cents  BIGINT NOT NULL DEFAULT 0,
    calls       INTEGER NOT NULL DEFAULT 0,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (agent, day)
);

CREATE INDEX IF NOT EXISTS idx_agent_spend_day ON agent_spend(day DESC);
