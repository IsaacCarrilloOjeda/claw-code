-- Wave 5: Dreamer's reflective write surface.
-- Dreamer reads recent ghost_events + director_notes, condenses recurring
-- themes into "interest nodes" with embeddings. Chat dispatcher and Chief
-- of Staff can pgvector-query this table to surface what Isaac is thinking
-- about lately without re-reading raw events.
--
-- Embedding dim 1024 matches director_notes (see migration 002) — Voyage
-- voyage-3 outputs 1024-dim vectors.
CREATE TABLE IF NOT EXISTS interest_nodes (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    topic        TEXT NOT NULL,
    summary      TEXT NOT NULL,
    weight       FLOAT NOT NULL DEFAULT 1.0,
    embedding    VECTOR(1024),
    source_refs  JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_interest_nodes_weight
    ON interest_nodes (weight DESC);

-- Mirrors director_notes — pgvector requires an explicit ivfflat/hnsw index
-- only once the table has real data. Skip the ANN index this wave; a table
-- scan over ~hundreds of rows is fine.
