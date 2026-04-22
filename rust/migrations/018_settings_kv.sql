-- Runtime-mutable settings. A small JSONB key-value store the daemon can
-- read/write at runtime, used by the provider router, the coder agent's
-- budget/flags, and the kill switch. See `db::get_setting` / `set_setting`.

CREATE TABLE IF NOT EXISTS settings_kv (
    key        TEXT PRIMARY KEY,
    value      JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO settings_kv (key, value) VALUES
    ('provider.default',            '"openrouter"'::jsonb),
    ('provider.per_agent',          '{}'::jsonb),
    ('coder.budget_cents_per_day',  '200'::jsonb),
    ('coder.auto_apply',            'false'::jsonb),
    ('coder.summarize_as_you_go',   'true'::jsonb),
    ('coder.kill_switch',           'false'::jsonb)
ON CONFLICT (key) DO NOTHING;
