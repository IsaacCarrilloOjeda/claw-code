-- Orchestrator plan + worker ledger. `coder_orchestrations` is one row per
-- spec-fragmentation call. `coder_tasks` is N rows per orchestration, one
-- per independent worker. Workers write `status` and `worker_output` back
-- as they finish so the dashboard can poll GET /code/orchestrate/:id to
-- paint task cards live.

CREATE TABLE IF NOT EXISTS coder_orchestrations (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    spec       TEXT NOT NULL,
    chat_id    UUID,
    status     TEXT NOT NULL DEFAULT 'planned',   -- planned | running | done | failed
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS coder_tasks (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    orchestration_id UUID NOT NULL REFERENCES coder_orchestrations(id) ON DELETE CASCADE,
    task_prompt      TEXT NOT NULL,
    files_to_read    JSONB NOT NULL DEFAULT '[]'::jsonb,
    files_to_modify  JSONB NOT NULL DEFAULT '[]'::jsonb,
    verify_command   TEXT,
    status           TEXT NOT NULL DEFAULT 'pending',  -- pending | running | done | failed
    worker_output    TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at     TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS coder_tasks_orch
    ON coder_tasks(orchestration_id);
