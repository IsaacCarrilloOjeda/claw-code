-- Per-call spend ledger for the coder path. Complement to `agent_spend`:
-- `agent_spend` is the fast aggregate counter (agent, day) used for the
-- in-flight budget check; `coder_spend` is the audit trail — one row per
-- model call with the exact model, provider, and job_id.

CREATE TABLE IF NOT EXISTS coder_spend (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent             TEXT NOT NULL,
    day               DATE NOT NULL
                          DEFAULT ((now() AT TIME ZONE 'America/Denver')::date),
    input_tokens      INT NOT NULL DEFAULT 0,
    output_tokens     INT NOT NULL DEFAULT 0,
    cache_read_tokens INT NOT NULL DEFAULT 0,
    cost_cents        INT NOT NULL DEFAULT 0,
    model             TEXT NOT NULL,
    provider          TEXT NOT NULL,
    job_id            UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS coder_spend_agent_day ON coder_spend(agent, day);
CREATE INDEX IF NOT EXISTS coder_spend_created_at ON coder_spend(created_at DESC);
