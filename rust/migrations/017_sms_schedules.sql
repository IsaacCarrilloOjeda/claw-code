-- Event-driven auto-reply: availability schedules (slots A/B/C) + sleep mode.
--
-- Semantics:
--   - Each contact may be assigned to one of 3 schedule slots (A, B, C) or NULL (none).
--   - A slot has 0..N windows (weekly recurring or one-off date).
--   - While "now" (in America/Denver) falls inside any window of a contact's slot,
--     that contact's auto_reply is forced ON. Outside all windows → OFF.
--   - Sleep mode is a manual toggle: user presses "sleep" with an awake-by time;
--     while active, every contact in `sms_sleep_contacts` gets auto_reply=TRUE
--     regardless of schedule. Sleep auto-clears when `now() >= awake_by`.
--   - Manual toggle of auto_reply via the contact row also sets schedule_slot=NULL,
--     so the scheduler stops touching that contact until the user reassigns a slot.
--
-- Evaluated every 60s by the daemon's background polling task.

CREATE TABLE IF NOT EXISTS sms_schedules (
    slot        CHAR(1) PRIMARY KEY CHECK (slot IN ('A', 'B', 'C')),
    name        TEXT NOT NULL DEFAULT '',
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO sms_schedules (slot, name) VALUES ('A', ''), ('B', ''), ('C', '')
    ON CONFLICT (slot) DO NOTHING;

-- weekday_mask bit layout matches PostgreSQL EXTRACT(DOW): Sun=bit0, Mon=bit1, ..., Sat=bit6.
-- Mon-Fri = 0b0111110 = 62.
CREATE TABLE IF NOT EXISTS sms_schedule_windows (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slot         CHAR(1) NOT NULL REFERENCES sms_schedules(slot) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('weekly', 'oneoff')),
    weekday_mask INTEGER,               -- required for 'weekly'; bits 0..6 = Sun..Sat
    day_date     DATE,                  -- required for 'oneoff'
    start_time   TIME NOT NULL,
    end_time     TIME NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (end_time > start_time),
    CHECK (
        (kind = 'weekly' AND weekday_mask IS NOT NULL AND day_date IS NULL)
        OR
        (kind = 'oneoff' AND day_date IS NOT NULL AND weekday_mask IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_sms_schedule_windows_slot ON sms_schedule_windows (slot);

ALTER TABLE sms_contacts
    ADD COLUMN IF NOT EXISTS schedule_slot CHAR(1)
    REFERENCES sms_schedules(slot) ON DELETE SET NULL;

-- Singleton sleep-mode row.
CREATE TABLE IF NOT EXISTS sms_sleep_mode (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    active      BOOLEAN NOT NULL DEFAULT FALSE,
    asleep_at   TIMESTAMPTZ,
    awake_by    TIMESTAMPTZ,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO sms_sleep_mode (id, active) VALUES (1, FALSE)
    ON CONFLICT (id) DO NOTHING;

-- Contacts that get auto_reply forced ON while sleep mode is active.
CREATE TABLE IF NOT EXISTS sms_sleep_contacts (
    phone       TEXT PRIMARY KEY,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
