-- Summarize-as-you-go storage. One row per KEEP-classified coder turn,
-- holding a 2-sentence summary + embedding for later injection as
-- "## Earlier in this chat" context.
--
-- Embedding dim is 1024 to match the rest of GHOST (Voyage voyage-3 and
-- OpenAI text-embedding-3-small with dimensions=1024; see migration 002).

CREATE TABLE IF NOT EXISTS coder_condensate (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    chat_id     UUID NOT NULL,
    summary     TEXT NOT NULL,
    embedding   VECTOR(1024),
    turn_idx    INT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS coder_condensate_chat ON coder_condensate(chat_id);
CREATE INDEX IF NOT EXISTS coder_condensate_embedding
    ON coder_condensate USING ivfflat (embedding vector_cosine_ops);
