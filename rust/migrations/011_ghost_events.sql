-- Per-turn event log. The self-correction substrate. Every routed message
-- writes a row. Mirrors weekly-condensed summaries to Gerald (Wave 4+).

CREATE TABLE IF NOT EXISTS ghost_events (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id            UUID REFERENCES jobs(id) ON DELETE SET NULL,
    agent             TEXT NOT NULL,
    tier              TEXT NOT NULL,
    input             TEXT,
    output            TEXT,
    tokens_in         INTEGER NOT NULL DEFAULT 0,
    tokens_out        INTEGER NOT NULL DEFAULT 0,
    cost_cents        INTEGER NOT NULL DEFAULT 0,
    outcome           TEXT NOT NULL,
    human_correction  TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_ghost_events_created_at ON ghost_events(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_ghost_events_agent ON ghost_events(agent);
CREATE INDEX IF NOT EXISTS idx_ghost_events_job_id ON ghost_events(job_id);
