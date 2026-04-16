-- Voyage AI voyage-3 only supports output_dimension=1024 (not 1536).
-- Shrink the embedding column to match. Existing 1536-dim embeddings
-- are dropped (set to NULL) — notes remain, they just lose vector search
-- ranking until re-embedded.

ALTER TABLE director_notes
    ALTER COLUMN embedding TYPE VECTOR(1024)
    USING NULL;
