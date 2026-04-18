-- Add loadbearing flag to sms_history.
-- Loadbearing messages are always included in context regardless of recency.
ALTER TABLE sms_history ADD COLUMN IF NOT EXISTS loadbearing BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX IF NOT EXISTS idx_sms_history_loadbearing
    ON sms_history (phone, loadbearing) WHERE loadbearing = TRUE;
