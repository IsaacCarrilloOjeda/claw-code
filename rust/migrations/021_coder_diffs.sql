-- Pending diff queue for the coder agent. When `coder.auto_apply` is false,
-- the diff tool inserts a row here instead of writing to disk; Isaac approves
-- or rejects via dashboard (/code/diffs/:id/{apply,reject}). Rows stay around
-- after resolution for audit.

CREATE TABLE IF NOT EXISTS coder_pending_diffs (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    chat_id     UUID NOT NULL,
    path        TEXT NOT NULL,
    search      TEXT NOT NULL,
    replace     TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'pending',  -- pending | applied | rejected
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS coder_pending_diffs_chat
    ON coder_pending_diffs(chat_id) WHERE status = 'pending';

-- MiMo fallback slug (last-resort provider in the coder's fallback chain).
-- Overridable via settings_kv at runtime if OpenRouter renames the model.
INSERT INTO settings_kv (key, value) VALUES
    ('coder.fallback.mimo_model', '"xiaomi/mimo-7b-rl"'::jsonb)
ON CONFLICT (key) DO NOTHING;
