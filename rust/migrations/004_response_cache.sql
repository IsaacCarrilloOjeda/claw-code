-- Response cache: stores query+response pairs with embeddings for token recycling.
-- Similar future queries get the cached response injected as context so the model
-- edits instead of re-deriving from scratch.

CREATE TABLE IF NOT EXISTS response_cache (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    query_text      TEXT NOT NULL,
    query_embed     VECTOR(1024),
    response_text   TEXT NOT NULL,
    hit_count       INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_hit_at     TIMESTAMPTZ
);
