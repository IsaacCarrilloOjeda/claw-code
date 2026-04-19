-- Per-contact notes visible to GHOST in the system prompt.
ALTER TABLE sms_contacts ADD COLUMN IF NOT EXISTS notes TEXT;
