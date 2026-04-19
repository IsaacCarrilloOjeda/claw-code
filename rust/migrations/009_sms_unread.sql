-- Track when a conversation was last viewed to compute unread counts.
ALTER TABLE sms_contacts ADD COLUMN IF NOT EXISTS last_read_at TIMESTAMPTZ;
