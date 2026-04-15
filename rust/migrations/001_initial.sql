-- Phase 0 initial schema for GHOST.
-- pgvector extension is needed for Phase 2 semantic search on director_notes.

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS vector;

-- Every request becomes a trackable job.
CREATE TABLE IF NOT EXISTS jobs (
    id                    UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    status                TEXT NOT NULL DEFAULT 'pending',
    input                 TEXT NOT NULL,
    output                TEXT,
    agent                 TEXT,
    source                TEXT,
    phone_from            TEXT,
    requires_confirmation BOOLEAN NOT NULL DEFAULT false,
    confirmation_token    TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at          TIMESTAMPTZ
);

-- Singleton row: controls which models the Director uses.
CREATE TABLE IF NOT EXISTS director_config (
    id               INTEGER PRIMARY KEY DEFAULT 1,
    primary_model    TEXT NOT NULL DEFAULT 'claude-sonnet-4-6',
    fallback_model   TEXT NOT NULL DEFAULT 'gpt-4o',
    primary_healthy  BOOLEAN NOT NULL DEFAULT true,
    fallback_healthy BOOLEAN NOT NULL DEFAULT true,
    last_health_check TIMESTAMPTZ,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Director memory notes (schema locked now; embedding populated in Phase 2).
CREATE TABLE IF NOT EXISTS director_notes (
    id               UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    category         TEXT NOT NULL,
    content          TEXT NOT NULL,
    embedding        VECTOR(1536),
    confidence       FLOAT NOT NULL DEFAULT 1.0,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_accessed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at       TIMESTAMPTZ
);

-- Seed the singleton director_config row (no-op if already present).
INSERT INTO director_config (id, primary_model, fallback_model, primary_healthy, fallback_healthy)
VALUES (1, 'claude-sonnet-4-6', 'gpt-4o', true, true)
ON CONFLICT (id) DO NOTHING;
