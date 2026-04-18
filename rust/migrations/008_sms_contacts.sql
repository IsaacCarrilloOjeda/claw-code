-- Per-contact SMS settings: auto-reply toggle, display name override.
CREATE TABLE IF NOT EXISTS sms_contacts (
    phone        TEXT PRIMARY KEY,
    display_name TEXT,
    auto_reply   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Schedule / commitments context injected into GHOST's SMS system prompt.
-- Two types:
--   'daily' -- applies to a specific day (e.g., "Math test 3rd period")
--   'persistent' -- always active until deleted (e.g., "Honors English Mon-Fri 8:00-9:30 AM")
CREATE TABLE IF NOT EXISTS sms_schedule (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    kind        TEXT NOT NULL CHECK (kind IN ('daily', 'persistent')),
    day_date    DATE,             -- required for 'daily', NULL for 'persistent'
    content     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sms_schedule_kind_date
    ON sms_schedule (kind, day_date);
