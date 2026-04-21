-- Public facts blob — shareable facts about Isaac that both the rep
-- (chat_dispatcher) and the outbound guard consult. Singleton row so
-- the UI can treat it as one textarea + one save button.
CREATE TABLE IF NOT EXISTS sms_facts (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    content     TEXT NOT NULL DEFAULT '',
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO sms_facts (id, content) VALUES (1, '')
ON CONFLICT (id) DO NOTHING;
