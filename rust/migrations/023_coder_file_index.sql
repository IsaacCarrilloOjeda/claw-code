-- Semantic file index for the coder agent.
--
-- One row per tracked file in the repo. `signature_summary` holds extracted
-- fn/struct/class signatures + a short raw-body fingerprint; that text is what
-- gets embedded (not the whole file), keeping embed cost cheap. On each coder
-- turn, `search_files(query, k)` does a pgvector nearest-neighbor lookup and
-- returns the top-k paths so the agent knows where to look without reading
-- every file. Embedding dim is 1024 to match the rest of GHOST (Voyage
-- voyage-3 / OpenAI text-embedding-3-small with dimensions=1024, see
-- migration 002).

CREATE TABLE IF NOT EXISTS coder_file_index (
    path              TEXT PRIMARY KEY,
    signature_summary TEXT NOT NULL,
    file_size_bytes   INT  NOT NULL,
    sha256            TEXT NOT NULL,
    embedding         VECTOR(1024),
    indexed_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS coder_file_index_embedding
    ON coder_file_index
    USING ivfflat (embedding vector_cosine_ops)
    WITH (lists = 32);
