-- Bible Agent schema: original-language-first Bible database.
-- Stores Hebrew/Aramaic/Greek source text alongside KJV and WEB English,
-- with Strong's concordance numbers, morphological tags, cross-references,
-- and a full lexicon. Embeddings use composite strings for multilingual search.

-- Verses: one row per Bible verse (~31,102 total)
CREATE TABLE IF NOT EXISTS bible_verses (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    book            TEXT NOT NULL,
    chapter         INTEGER NOT NULL,
    verse           INTEGER NOT NULL,
    book_order      INTEGER NOT NULL,
    testament       TEXT NOT NULL CHECK (testament IN ('OT', 'NT')),
    original_lang   TEXT NOT NULL CHECK (original_lang IN ('hebrew', 'aramaic', 'greek')),
    original_text   TEXT NOT NULL,
    kjv_text        TEXT NOT NULL,
    web_text        TEXT,
    strongs         TEXT[] DEFAULT '{}',
    morphology      JSONB,
    embedding       VECTOR(1024),
    pericope_id     UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (book, chapter, verse)
);

CREATE INDEX IF NOT EXISTS idx_bible_verses_ref ON bible_verses(book, chapter, verse);
CREATE INDEX IF NOT EXISTS idx_bible_verses_book_order ON bible_verses(book_order, chapter, verse);
CREATE INDEX IF NOT EXISTS idx_bible_verses_strongs ON bible_verses USING GIN(strongs);
CREATE INDEX IF NOT EXISTS idx_bible_verses_pericope ON bible_verses(pericope_id);
CREATE INDEX IF NOT EXISTS idx_bible_verses_testament ON bible_verses(testament);

-- Pericopes: thematic units grouping verses (~1,200 total)
CREATE TABLE IF NOT EXISTS bible_pericopes (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    title           TEXT NOT NULL,
    start_book      TEXT NOT NULL,
    start_chapter   INTEGER NOT NULL,
    start_verse     INTEGER NOT NULL,
    end_book        TEXT NOT NULL,
    end_chapter     INTEGER NOT NULL,
    end_verse       INTEGER NOT NULL,
    genre           TEXT CHECK (genre IN ('narrative', 'poetry', 'law', 'prophecy', 'epistle', 'apocalyptic', 'wisdom', 'gospel')),
    summary         TEXT,
    embedding       VECTOR(1024),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Cross-references: directed edges between verses (~63,000+ from TSK + lexical)
CREATE TABLE IF NOT EXISTS bible_cross_refs (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    source_book     TEXT NOT NULL,
    source_chapter  INTEGER NOT NULL,
    source_verse    INTEGER NOT NULL,
    target_book     TEXT NOT NULL,
    target_chapter  INTEGER NOT NULL,
    target_verse    INTEGER NOT NULL,
    rel_type        TEXT NOT NULL CHECK (rel_type IN ('quotation', 'allusion', 'thematic', 'lexical', 'parallel', 'contrast')),
    source_dataset  TEXT NOT NULL DEFAULT 'TSK',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_bible_xref_source ON bible_cross_refs(source_book, source_chapter, source_verse);
CREATE INDEX IF NOT EXISTS idx_bible_xref_target ON bible_cross_refs(target_book, target_chapter, target_verse);
CREATE INDEX IF NOT EXISTS idx_bible_xref_type ON bible_cross_refs(rel_type);

-- Lexicon: Strong's concordance entries (~8,674 total)
CREATE TABLE IF NOT EXISTS bible_lexicon (
    strongs_id      TEXT PRIMARY KEY,
    original_word   TEXT NOT NULL,
    transliteration TEXT NOT NULL,
    lang            TEXT NOT NULL CHECK (lang IN ('hebrew', 'aramaic', 'greek')),
    root            TEXT,
    definition      TEXT NOT NULL,
    usage_count     INTEGER DEFAULT 0,
    semantic_range  TEXT[] DEFAULT '{}',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_bible_lexicon_lang ON bible_lexicon(lang);
CREATE INDEX IF NOT EXISTS idx_bible_lexicon_root ON bible_lexicon(root);
