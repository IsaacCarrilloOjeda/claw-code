-- Per-contact SMS conversation history for multi-turn context.
CREATE TABLE IF NOT EXISTS sms_history (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    phone       TEXT NOT NULL,
    role        TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sms_history_phone_time
    ON sms_history (phone, created_at DESC);
