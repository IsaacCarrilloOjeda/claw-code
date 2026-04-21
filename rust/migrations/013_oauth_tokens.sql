-- Per-provider OAuth refresh-token storage. One row per (provider, account_label).
-- Shared by Calendar (Wave 3), Docs (Wave 4), Gmail (later).

CREATE TABLE IF NOT EXISTS oauth_tokens (
    provider      TEXT NOT NULL,    -- 'google_calendar' | 'google_docs' | 'gmail'
    account_label TEXT NOT NULL,    -- 'primary' | 'kynesystems' | etc.
    refresh_token TEXT NOT NULL,
    access_token  TEXT,
    expires_at    TIMESTAMPTZ,
    scopes        TEXT[] NOT NULL DEFAULT '{}',
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, account_label)
);
