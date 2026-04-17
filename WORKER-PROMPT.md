# GHOST Bible Agent — Worker Prompts

> **How to use:** Open a new Claude Code session in this repo, paste the HEADER + one PHASE block.
> One phase per session. Phases must be done in order (B1 depends on B0, etc).

---

## HEADER (paste this at the top of every worker session)

You are working on GHOST, a personal AI operating system built in Rust. The repo is at the current working directory. Before doing anything, read these files in order:

1. `CLAUDE.md` — repo conventions, verification commands, gotchas
2. `rust/crates/rusty-claude-cli/src/chat_dispatcher.rs` — current chat dispatch logic (your integration target)
3. `rust/crates/rusty-claude-cli/src/db.rs` — all database functions (you will add Bible functions here)
4. `rust/crates/rusty-claude-cli/src/memory.rs` — embedding functions (`embed()` is what you call for vectors)
5. `rust/crates/rusty-claude-cli/src/constants.rs` — model string constants
6. `rust/crates/rusty-claude-cli/src/main.rs` — module declarations (you will add `mod bible;`)
7. `rust/crates/rusty-claude-cli/src/daemon.rs` — HTTP daemon (you will add Bible endpoints in Phase B3)
8. `rust/migrations/` — existing migrations (003, 004 are the most recent — your new ones start at 005)

After reading, run `cargo test -p rusty-claude-cli --bins` to confirm baseline. Then implement.

### Verification (run after EVERY phase)

```bash
cd rust
cargo fmt
cargo clippy -p rusty-claude-cli -p plugins --bins -- -D warnings
cargo test -p rusty-claude-cli --bins
```

All three must pass. Full-workspace clippy is blocked by Unix-only code in `runtime` crate — that's expected, don't try to fix it.

### Universal Rules

- Do NOT refactor existing code that works. Only touch what's needed for this phase.
- Do NOT add comments or docstrings to code you didn't write.
- Do NOT change the existing memory system, scholar DB, response cache, or orchestrator.
- Do NOT add new crate dependencies unless the phase explicitly says to. `toml`, `sqlx`, `serde_json`, `reqwest`, `tokio`, `uuid` are already available.
- Do NOT create README files or documentation files.
- Do NOT commit. Leave changes uncommitted for review.
- The existing `crate::memory::embed()` function returns `Vec<f32>` (1024-dim via Voyage AI). Use it for all embedding needs — do NOT build a separate embedding path.
- The existing `crate::db::vec_to_pgvector()` converts `&[f32]` to the string format pgvector expects. Use it for all vector inserts.
- Follow the patterns in `db.rs` exactly: runtime-checked `sqlx::query` (not compile-time macros), `try_get` for column extraction, `uuid::Uuid::new_v4().to_string()` for IDs.

---

# ============================================================
# PHASE B0 — Bible Database Schema + Core DB Functions
# STATUS: TODO
# ESTIMATED: ~400 lines (migration + db functions + structs)
# DEPENDS ON: nothing (can start immediately)
# ============================================================

## What You Are Building

The database layer for the Bible Agent: four new Postgres tables (`bible_verses`, `bible_pericopes`, `bible_cross_refs`, `bible_lexicon`) and the Rust functions to query them. This phase creates the schema and all CRUD/search operations. No data is ingested yet — that's Phase B1.

## 1. New Migration: `rust/migrations/005_bible_schema.sql`

```sql
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
```

## 2. New structs and functions in `db.rs`

Add a new section at the bottom of `db.rs` (before the circuit breaker section), following the exact same patterns as `ScholarSolution` and `CachedResponse`.

### Structs

```rust
// ---------------------------------------------------------------------------
// Bible Agent models (Phase B0)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct BibleVerse {
    pub id: String,
    pub book: String,
    pub chapter: i32,
    pub verse: i32,
    pub book_order: i32,
    pub testament: String,
    pub original_lang: String,
    pub original_text: String,
    pub kjv_text: String,
    pub web_text: Option<String>,
    pub strongs: Vec<String>,
    pub morphology: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BiblePericope {
    pub id: String,
    pub title: String,
    pub start_book: String,
    pub start_chapter: i32,
    pub start_verse: i32,
    pub end_book: String,
    pub end_chapter: i32,
    pub end_verse: i32,
    pub genre: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BibleCrossRef {
    pub id: String,
    pub source_book: String,
    pub source_chapter: i32,
    pub source_verse: i32,
    pub target_book: String,
    pub target_chapter: i32,
    pub target_verse: i32,
    pub rel_type: String,
    pub source_dataset: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LexiconEntry {
    pub strongs_id: String,
    pub original_word: String,
    pub transliteration: String,
    pub lang: String,
    pub root: Option<String>,
    pub definition: String,
    pub usage_count: i32,
    pub semantic_range: Vec<String>,
}
```

### Query functions (all `pub async fn`)

**Semantic search (the primary query path):**

```rust
/// Semantic search on bible_verses by embedding similarity. Returns (verse, distance).
pub async fn search_bible_verses(
    pool: &PgPool,
    embedding: &[f32],
    limit: i64,
) -> Vec<(BibleVerse, f64)>
```
- Query: `SELECT ... , (embedding <=> $1::vector) AS distance FROM bible_verses WHERE embedding IS NOT NULL ORDER BY embedding <=> $1::vector LIMIT $2`
- Follow the exact pattern from `search_scholar_with_distance`

```rust
/// Semantic search on bible_pericopes by embedding similarity.
pub async fn search_bible_pericopes(
    pool: &PgPool,
    embedding: &[f32],
    limit: i64,
) -> Vec<(BiblePericope, f64)>
```
- Same pattern, on `bible_pericopes` table

**Reference lookup (exact verse fetch):**

```rust
/// Fetch a single verse by book/chapter/verse.
pub async fn get_bible_verse(
    pool: &PgPool,
    book: &str,
    chapter: i32,
    verse: i32,
) -> Option<BibleVerse>
```

```rust
/// Fetch a range of verses (e.g., Romans 8:28-30). Ordered by book_order, chapter, verse.
pub async fn get_bible_verse_range(
    pool: &PgPool,
    book: &str,
    start_chapter: i32,
    start_verse: i32,
    end_chapter: i32,
    end_verse: i32,
) -> Vec<BibleVerse>
```
- `WHERE book = $1 AND ((chapter = $2 AND verse >= $3) OR (chapter > $2 AND chapter < $4) OR (chapter = $4 AND verse <= $5)) ORDER BY chapter, verse`
- Handle same-chapter case (start_chapter == end_chapter) as: `chapter = $2 AND verse >= $3 AND verse <= $5`

**Cross-reference traversal:**

```rust
/// Get all cross-references FROM a given verse (1-hop outbound).
pub async fn get_cross_refs_from(
    pool: &PgPool,
    book: &str,
    chapter: i32,
    verse: i32,
) -> Vec<BibleCrossRef>
```

```rust
/// Get all cross-references TO a given verse (1-hop inbound).
pub async fn get_cross_refs_to(
    pool: &PgPool,
    book: &str,
    chapter: i32,
    verse: i32,
) -> Vec<BibleCrossRef>
```

**Strong's / Lexicon queries:**

```rust
/// Look up a Strong's number (e.g., "H1254" or "G26").
pub async fn get_lexicon_entry(pool: &PgPool, strongs_id: &str) -> Option<LexiconEntry>
```

```rust
/// Find all verses containing a specific Strong's number.
pub async fn search_verses_by_strongs(
    pool: &PgPool,
    strongs_id: &str,
    limit: i64,
) -> Vec<BibleVerse>
```
- `WHERE $1 = ANY(strongs) ORDER BY book_order, chapter, verse LIMIT $2`
- GIN index on `strongs` makes this fast

```rust
/// Find all lexicon entries sharing the same root (e.g., all words from Hebrew root "ברא").
pub async fn search_lexicon_by_root(pool: &PgPool, root: &str) -> Vec<LexiconEntry>
```

**Bulk insert functions (used by the ingestion pipeline in Phase B1):**

```rust
/// Insert a verse. Returns true on success. Skips if (book, chapter, verse) already exists.
pub async fn insert_bible_verse(
    pool: &PgPool,
    book: &str,
    chapter: i32,
    verse: i32,
    book_order: i32,
    testament: &str,
    original_lang: &str,
    original_text: &str,
    kjv_text: &str,
    web_text: Option<&str>,
    strongs: &[String],
    morphology: Option<&serde_json::Value>,
    embedding: Option<&[f32]>,
    pericope_id: Option<&str>,
) -> bool
```
- Use `INSERT ... ON CONFLICT (book, chapter, verse) DO NOTHING`
- If `embedding` is Some, bind it via `vec_to_pgvector`. If None, omit the column (same pattern as `insert_scholar`).

```rust
/// Insert a pericope. Returns the generated UUID.
pub async fn insert_bible_pericope(
    pool: &PgPool,
    title: &str,
    start_book: &str, start_chapter: i32, start_verse: i32,
    end_book: &str, end_chapter: i32, end_verse: i32,
    genre: Option<&str>,
    summary: Option<&str>,
    embedding: Option<&[f32]>,
) -> Option<String>
```

```rust
/// Insert a cross-reference. Skips duplicates silently.
pub async fn insert_bible_cross_ref(
    pool: &PgPool,
    source_book: &str, source_chapter: i32, source_verse: i32,
    target_book: &str, target_chapter: i32, target_verse: i32,
    rel_type: &str,
    source_dataset: &str,
) -> bool
```

```rust
/// Insert a lexicon entry. Upserts on strongs_id.
pub async fn insert_lexicon_entry(
    pool: &PgPool,
    strongs_id: &str,
    original_word: &str,
    transliteration: &str,
    lang: &str,
    root: Option<&str>,
    definition: &str,
    usage_count: i32,
    semantic_range: &[String],
) -> bool
```
- Use `INSERT ... ON CONFLICT (strongs_id) DO UPDATE SET definition = EXCLUDED.definition, usage_count = EXCLUDED.usage_count, semantic_range = EXCLUDED.semantic_range`

**Stats function (for health check / dashboard):**

```rust
/// Return counts of all Bible tables for health/status display.
pub async fn bible_stats(pool: &PgPool) -> (i64, i64, i64, i64)
```
- Returns `(verse_count, pericope_count, crossref_count, lexicon_count)`
- Four COUNT queries (can be one query with subqueries)

## 3. Unit tests

Add tests to `db.rs` or a new `tests/bible_db_test.rs` — but since the existing db.rs doesn't have tests (it's all integration against Postgres), add unit tests for any pure helper functions you extract. At minimum, test:

- The SQL strings compile (no syntax errors) by calling the query builders (they'll fail to execute without a pool, but sqlx will parse them)
- Actually — the existing db.rs has NO unit tests because everything needs a live pool. Follow the same pattern: no unit tests in db.rs. The schema correctness is validated by `sqlx::migrate!` running at startup.

Instead, write tests in the new `bible.rs` module (Phase B2) for the parsing/formatting logic.

## IMPORTANT: Do not create the `bible.rs` module yet. Do not add `mod bible;` to main.rs yet. That's Phase B2. This phase is schema + db functions only.

---

# ============================================================
# PHASE B1 — Bible Data Ingestion Pipeline
# STATUS: TODO
# ESTIMATED: ~500 lines (ingestion module + data parsing + CLI command)
# DEPENDS ON: B0 (schema + db functions must exist)
# ============================================================

## What You Are Building

A CLI subcommand (`claw bible-ingest`) and the ingestion pipeline that downloads, parses, and loads Bible data into the tables from Phase B0. This runs once (or on-demand to re-ingest), not at runtime.

The data sources are all public domain and available as structured files. The ingestion pipeline downloads them (or reads from local files), parses into the schema, embeds composite strings via `crate::memory::embed()`, and bulk-inserts.

## 1. New file: `rust/crates/rusty-claude-cli/src/bible_ingest.rs`

### Data sources

The ingestion pipeline reads from JSON/TSV/XML files placed in a `.ghost/bible-data/` directory. The user downloads them manually (or a future helper script does it). The ingestion code reads local files — it does NOT download from the internet at runtime.

Expected directory structure:
```
.ghost/bible-data/
  kjv.json            -- KJV text, verse-aligned
  web.json            -- WEB text, verse-aligned (optional, skipped if missing)
  hebrew-wlc.json     -- Westminster Leningrad Codex, verse-aligned with morph tags
  greek-ugnt.json     -- unfoldingWord Greek NT, verse-aligned with morph tags
  strongs-hebrew.json -- Strong's Hebrew lexicon entries
  strongs-greek.json  -- Strong's Greek lexicon entries
  cross-refs.tsv      -- Treasury of Scripture Knowledge cross-references
  pericopes.json      -- Pericope boundaries + titles
```

The ingestion code must be TOLERANT of missing files. If `web.json` is missing, ingest without WEB text. If `cross-refs.tsv` is missing, skip cross-references. Only `kjv.json` is strictly required.

### Expected JSON formats

**kjv.json / web.json:**
```json
[
  {"book": "Genesis", "chapter": 1, "verse": 1, "text": "In the beginning..."},
  ...
]
```

**hebrew-wlc.json:**
```json
[
  {
    "book": "Genesis", "chapter": 1, "verse": 1,
    "text": "בְּרֵאשִׁית בָּרָא אֱלֹהִים...",
    "strongs": ["H7225", "H1254", "H430", "H853", "H8064", "H853", "H776"],
    "morphology": {"words": [{"word": "בְּרֵאשִׁית", "strongs": "H7225", "morph": "Ncfsa"}]}
  },
  ...
]
```

**greek-ugnt.json:** same structure, with Greek text and G-prefixed Strong's numbers.

**strongs-hebrew.json / strongs-greek.json:**
```json
[
  {
    "strongs_id": "H1254",
    "original_word": "בָּרָא",
    "transliteration": "bara",
    "definition": "to create, shape, form",
    "root": "ברא",
    "semantic_range": ["create", "shape", "form", "make", "cut down"]
  },
  ...
]
```

**cross-refs.tsv:**
```
source_book	source_chapter	source_verse	target_book	target_chapter	target_verse	rel_type
Genesis	1	1	John	1	1	thematic
Matthew	1	23	Isaiah	7	14	quotation
...
```

**pericopes.json:**
```json
[
  {
    "title": "The Creation",
    "start_book": "Genesis", "start_chapter": 1, "start_verse": 1,
    "end_book": "Genesis", "end_chapter": 2, "end_verse": 3,
    "genre": "narrative"
  },
  ...
]
```

### Book metadata constant

Define a constant lookup table for the 66 books:

```rust
const BOOKS: &[(&str, i32, &str, &str)] = &[
    // (name, order, testament, original_lang)
    ("Genesis", 1, "OT", "hebrew"),
    ("Exodus", 2, "OT", "hebrew"),
    // ... all 66 books ...
    ("Daniel", 27, "OT", "hebrew"),  // note: parts are Aramaic, but primary is Hebrew
    // ... NT ...
    ("Matthew", 40, "NT", "greek"),
    // ... through ...
    ("Revelation", 66, "NT", "greek"),
];
```

For Daniel, the ingestion code should check if the hebrew-wlc.json data marks specific verses as Aramaic and set `original_lang` accordingly. If the data doesn't distinguish, default to "hebrew" for Daniel (it's mostly Hebrew, the Aramaic portions will still be in the WLC text).

Same for Ezra — Ezra 4:8-6:18 and 7:12-26 are Aramaic.

### Ingestion flow

```rust
pub async fn ingest_bible(pool: &sqlx::PgPool) -> Result<IngestStats, String>
```

Runs these steps in order:

1. **Read and parse data files** from `.ghost/bible-data/` (or `GHOST_BIBLE_DATA_PATH` env override)
2. **Ingest lexicon entries** first (strongs-hebrew.json + strongs-greek.json) — these are referenced by verses
3. **Ingest pericopes** — insert all pericope boundaries, get back the generated UUIDs
4. **Ingest verses** — for each verse:
   a. Look up the book metadata (order, testament, original_lang)
   b. Merge KJV text + WEB text (if available) + original text (if available) + Strong's numbers (if available)
   c. Build the composite embedding string: `"{book} {chapter}:{verse} | {original_text} | {kjv_text} | {strongs_joined}"`
   d. Call `crate::memory::embed(&composite)` — this is the expensive step (API calls)
   e. Match the verse to its pericope (by checking if it falls within any pericope's range)
   f. Call `crate::db::insert_bible_verse(...)`
5. **Ingest cross-references** — parse TSV, insert each row
6. **Generate pericope summaries** — for each pericope, collect its verses' KJV text, call Haiku to generate a 2-3 sentence summary, embed the summary, update the pericope row
7. **Print stats** — counts per table

### Embedding batching

~31,102 verses * 1 embedding API call each = way too many sequential calls. Batch them:

- Voyage AI accepts batches of up to 128 inputs per request
- Build a batch embedding function: `embed_batch(texts: &[String]) -> Result<Vec<Vec<f32>>, String>`
- Add this to `memory.rs` as a new public function alongside the existing `embed()`
- Voyage request body: `{"model": "voyage-3", "input": ["text1", "text2", ...], "output_dimension": 1024}`
- The response has `data: [{embedding: [...]}, {embedding: [...]}]` — one per input, in order
- Process verses in batches of 128, calling `embed_batch` for each batch
- Add a small delay between batches (100ms) to respect rate limits
- If batch embedding fails, fall back to individual `embed()` calls for that batch

### Rate limiting and progress

- Print progress every 500 verses: `[bible ingest] {n}/31102 verses embedded...`
- If a verse fails to embed, store it with NULL embedding (it still works for exact-reference lookups, just not semantic search)
- Track and report: verses ingested, verses with embeddings, verses without, pericopes, cross-refs, lexicon entries

### Return struct

```rust
pub struct IngestStats {
    pub verses_total: u32,
    pub verses_embedded: u32,
    pub pericopes: u32,
    pub cross_refs: u32,
    pub lexicon_entries: u32,
    pub elapsed_secs: u64,
}
```

## 2. Add `embed_batch()` to `memory.rs`

```rust
/// Batch-embed multiple texts. Tries Voyage first (supports up to 128 inputs per call).
/// Falls back to sequential individual `embed()` calls if batch API fails.
pub async fn embed_batch(texts: &[String]) -> Result<Vec<Vec<f32>>, String>
```

Implementation:
- If `VOYAGE_API_KEY` is set: POST to Voyage with `"input": texts` (array), parse array of embeddings from response
- If not, or if Voyage fails: loop through texts calling `embed()` individually
- OpenAI also supports batch embedding, so if falling back to OpenAI, use the same batch pattern

## 3. CLI subcommand: add `bible-ingest` to `main.rs`

In the argument parsing section of `main.rs`, add a new subcommand:

```
claw bible-ingest [--data-dir PATH]
```

- Default data dir: `.ghost/bible-data/` relative to cwd (or `GHOST_BIBLE_DATA_PATH` env)
- Must have `DATABASE_URL` set (needs Postgres)
- Calls `crate::bible_ingest::ingest_bible(pool).await`
- Prints the `IngestStats` at the end

Look at how other subcommands (like `claw daemon`) are dispatched in `main.rs` and follow the same pattern.

### Add `mod bible_ingest;` to `main.rs`

## 4. Data preparation script

Create a small helper: `scripts/download-bible-data.ps1`

This PowerShell script downloads the public-domain data files into `.ghost/bible-data/`. It should:

1. Create `.ghost/bible-data/` if it doesn't exist
2. Download KJV JSON from a public API or GitHub dataset (there are several — use one that provides verse-aligned JSON)
3. Download WEB JSON similarly
4. Download Hebrew WLC data (Open Scriptures Hebrew Bible on GitHub)
5. Download Greek UGNT data (unfoldingWord on GitHub)
6. Download Strong's lexicon data
7. Download TSK cross-references

For each download: check if the file already exists, skip if it does. Print progress.

**NOTE:** The exact URLs will depend on what's currently available. The script should be a starting point that Isaac can adjust. Use well-known sources:
- `https://github.com/nicholasgasior/bible-api-go` has KJV JSON
- `https://github.com/openscriptures/morphhb` has Hebrew morphological data  
- `https://github.com/unfoldingWord/en_ugnt` has Greek NT data
- `https://github.com/nicholasgasior/bible-api-go` or similar for cross-refs

If you can't find a perfect URL, leave a `# TODO: update URL` comment and use a placeholder. The script structure matters more than the exact URLs.

## 5. Unit tests

Test the parsing logic (not the DB inserts — those need a live pool):

- Parse a KJV JSON array entry → correct book/chapter/verse/text
- Parse a Hebrew verse entry with Strong's numbers → correct strongs array
- Parse a lexicon entry → correct fields
- Parse a cross-ref TSV line → correct source/target
- Build composite embedding string → correct format
- Book metadata lookup → correct order/testament/lang for known books
- Missing optional file (web.json) → ingestion continues without error

---

# ============================================================
# PHASE B2 — Bible Agent Module (Query Engine + AI Integration)
# STATUS: TODO
# ESTIMATED: ~450 lines (bible.rs module + chat dispatcher integration)
# DEPENDS ON: B0 (schema), B1 (data must be ingested)
# ============================================================

## What You Are Building

The Bible Agent: a new Rust module (`bible.rs`) that receives a user query, determines what kind of Bible question it is, retrieves relevant data from all four tables, and assembles a rich context block for the AI to use when generating a response. This gets wired into the existing chat dispatcher alongside memory/scholar/web search.

## 1. New file: `rust/crates/rusty-claude-cli/src/bible.rs`

### Query classification

```rust
#[derive(Debug, PartialEq)]
pub enum BibleQueryType {
    /// Direct verse reference: "John 3:16", "Romans 8:28-30", "Genesis 1"
    Reference(VerseRef),
    /// Word study: "what does agape mean", "Strong's H1254", "Hebrew word for love"
    WordStudy(String),
    /// Topical: "what does the Bible say about patience"
    Topical(String),
    /// Not a Bible query
    NotBible,
}

#[derive(Debug, PartialEq)]
pub struct VerseRef {
    pub book: String,
    pub start_chapter: i32,
    pub start_verse: Option<i32>,    // None = whole chapter
    pub end_chapter: Option<i32>,    // None = same as start
    pub end_verse: Option<i32>,      // None = same as start
}
```

```rust
/// Classify a user message into a Bible query type.
/// Returns NotBible if the message doesn't look Bible-related.
pub fn classify_query(message: &str) -> BibleQueryType
```

Classification logic (check in this order):

1. **Reference detection** — regex or pattern match for:
   - `Book Chapter:Verse` (e.g., "John 3:16", "1 Corinthians 13:4")
   - `Book Chapter:Start-End` (e.g., "Romans 8:28-30")
   - `Book Chapter` (e.g., "Genesis 1", "Psalm 23")
   - Must handle multi-word book names: "1 Samuel", "Song of Solomon", "1 Corinthians"
   - Must handle common abbreviations: "Gen", "Ex", "Lev", "Rom", "1Cor", "Rev", etc.
   - Return `BibleQueryType::Reference(VerseRef {...})`

2. **Word study detection** — check for:
   - Strong's number pattern: `H\d{1,4}` or `G\d{1,4}` (case-insensitive)
   - Phrases: "what does X mean" where context is biblical, "hebrew word for", "greek word for", "original word", "root word"
   - Return `BibleQueryType::WordStudy(the_term)`

3. **Topical detection** — check for:
   - Phrases like "what does the bible say about", "scripture about", "biblical view", "verse about", "bible and"
   - The `!bible` prefix (if present, strip it and treat the rest as topical)
   - Return `BibleQueryType::Topical(the_topic)`

4. **NotBible** — if none of the above match

### Book name normalization

```rust
/// Normalize a book name/abbreviation to the canonical form used in the database.
/// "Gen" → "Genesis", "1Cor" → "1 Corinthians", "Ps" → "Psalms", etc.
/// Returns None if not a recognized book name.
pub fn normalize_book_name(input: &str) -> Option<String>
```

Build a static lookup table covering:
- Full names: "Genesis", "Exodus", ...
- Common abbreviations: "Gen", "Ex", "Lev", "Num", "Deut", "Josh", "Judg", "1Sam", "2Sam", ...
- Common alternates: "Psalm" → "Psalms", "Song of Songs" → "Song of Solomon"
- Case-insensitive matching

### Context assembly

```rust
/// Build a Bible context block to inject into the system prompt.
/// This is the main entry point called from chat_dispatcher.
pub async fn load_bible_context(
    message: &str,
    query_embedding: Option<&[f32]>,
    pool: Option<&sqlx::PgPool>,
) -> String
```

Returns an empty string if:
- pool is None
- classify_query returns NotBible (unless `!bible` prefix is present)
- no results found

For each query type, the context assembly strategy differs:

**Reference queries:**
1. Fetch the exact verses via `get_bible_verse` or `get_bible_verse_range`
2. For each verse: fetch outbound cross-references (limit 5)
3. For each Strong's number in the verses: fetch the lexicon entry
4. Format into a context block (see formatting below)

**Word study queries:**
1. If a Strong's number was detected: `get_lexicon_entry` → then `search_verses_by_strongs` (limit 10)
2. If a word was detected: embed the word, `search_bible_verses` by embedding (limit 10), also `search_lexicon_by_root` if it looks like a Hebrew/Greek term
3. Collect cross-references for the top verse hits
4. Format into a context block

**Topical queries:**
1. Use the query_embedding (already computed by chat_dispatcher) to search `bible_verses` (limit 8) and `bible_pericopes` (limit 3)
2. For top verse hits: fetch cross-references (limit 3 per verse)
3. For pericope hits: fetch the summary and genre
4. Format into a context block

### Context formatting

```rust
/// Format Bible data into a system prompt context block.
fn format_bible_context(
    query_type: &BibleQueryType,
    verses: &[(BibleVerse, Option<f64>)],  // verse + optional distance
    pericopes: &[(BiblePericope, Option<f64>)],
    cross_refs: &[BibleCrossRef],
    lexicon: &[LexiconEntry],
) -> String
```

Output format:

```
## Bible study context
Query type: [reference/word study/topical]

### Verses
**Genesis 1:1**
Original (Hebrew): בְּרֵאשִׁית בָּרָא אֱלֹהִים אֵת הַשָּׁמַיִם וְאֵת הָאָרֶץ
KJV: In the beginning God created the heaven and the earth.
WEB: In the beginning, God created the heavens and the earth.
Strong's: H7225 (reshith: beginning, first) · H1254 (bara: to create) · H430 (elohim: God) · H8064 (shamayim: heavens) · H776 (erets: earth)

**Genesis 1:2**
...

### Cross-references
- Genesis 1:1 → John 1:1 (thematic) — "In the beginning was the Word"
- Genesis 1:1 → Isaiah 65:17 (thematic) — "For, behold, I create new heavens"
- Genesis 1:1 → Hebrews 11:3 (allusion) — "the worlds were framed by the word of God"

### Word study: H1254 (bara, בָּרָא)
Root: ברא
Definition: to create, shape, form
Semantic range: create, shape, form, make, cut down
Occurrences: 54 times in OT
Related words from same root: H1254a (beriah: creation), ...

### Thematic context
Pericope: "The Creation" (Genesis 1:1 — 2:3) [narrative]
Summary: The creation account describes God bringing the universe into existence over six days...
```

**CRITICAL FORMATTING RULES:**
- Always show original language text when available
- Always show Strong's numbers with transliteration and short gloss inline
- Cross-references show the target verse's KJV text (truncated to ~100 chars)
- Cap total context to 6000 chars to avoid blowing up the system prompt
- Prefer depth on fewer verses over shallow coverage of many

### The `!bible` prefix handler

```rust
/// Strip the `!bible` prefix from a message if present. Returns (stripped_message, was_bible_prefix).
pub fn strip_bible_prefix(message: &str) -> (String, bool)
```

Check for: `!bible `, `!bible:`, `!Bible ` (case-insensitive on "bible"). Strip the prefix, return the remainder and a flag. The flag is used by chat_dispatcher to force Bible context loading even if `classify_query` returns NotBible.

## 2. Wire into `chat_dispatcher.rs`

In the `dispatch()` function, after the existing context loading (memory, scholar, cache, web search), add Bible context:

```rust
// After web_context is loaded...

let (bible_msg, bible_forced) = crate::bible::strip_bible_prefix(&polished);
let bible_context = if bible_forced {
    // !bible prefix — always load Bible context using the stripped message
    crate::bible::load_bible_context(&bible_msg, query_embedding.as_deref(), pool).await
} else {
    // No prefix — only load if the message looks Bible-related
    crate::bible::load_bible_context(&polished, query_embedding.as_deref(), pool).await
};

// When injecting into system prompt:
if !bible_context.is_empty() {
    eprintln!("[ghost chat] injecting {} bytes of Bible context", bible_context.len());
    system.push_str("\n\n");
    system.push_str(&bible_context);
}
```

**If `bible_forced` is true**, also prepend a Bible-specific preamble to the system prompt:

```rust
if bible_forced && !bible_context.is_empty() {
    system.push_str("\n\n## Bible study mode\n\
        You are GHOST's Bible study assistant. You have access to the original Hebrew, \
        Aramaic, and Greek texts alongside KJV and WEB English translations.\n\
        When answering Bible questions:\n\
        1. Always cite the specific verse reference (Book Chapter:Verse)\n\
        2. Show the original language text when it adds insight to the English\n\
        3. Reference Strong's numbers for key terms (e.g., G26 agape)\n\
        4. Note when English translations obscure or flatten the original meaning\n\
        5. Use cross-references to show how themes connect across scripture\n\
        6. Distinguish between what the text says (factual) and what it means (interpretive)\n\
        7. For interpretive claims, note major scholarly positions rather than picking one\n\
        8. The lexicon data is from public-domain sources (BDB, Thayer's) — treat as reference, not infallible");
}
```

**If using `bible_msg` (prefix was stripped)**, use `bible_msg` instead of `polished` in the messages array sent to the AI. The user's `!bible` prefix shouldn't be in the message the model sees.

### Add `mod bible;` to `main.rs`

## 3. Unit tests in `bible.rs`

**classify_query tests:**
- `"John 3:16"` → Reference with book="John", chapter=3, verse=16
- `"Romans 8:28-30"` → Reference with range
- `"Genesis 1"` → Reference with whole chapter (verse=None)
- `"1 Corinthians 13:4-7"` → Reference with multi-word book
- `"Psalm 23"` → Reference (singular "Psalm" normalized to "Psalms")
- `"Gen 1:1"` → Reference with abbreviation
- `"what does agape mean"` → WordStudy("agape")
- `"Strong's H1254"` → WordStudy("H1254")
- `"hebrew word for love"` → WordStudy("love")
- `"what does the bible say about patience"` → Topical("patience")
- `"verses about forgiveness"` → Topical("forgiveness")
- `"what's the weather"` → NotBible
- `"help me debug this function"` → NotBible

**normalize_book_name tests:**
- `"Gen"` → Some("Genesis")
- `"1Cor"` → Some("1 Corinthians")
- `"Ps"` → Some("Psalms")
- `"Psalm"` → Some("Psalms")
- `"Song of Songs"` → Some("Song of Solomon")
- `"Rev"` → Some("Revelation")
- `"Foobar"` → None

**strip_bible_prefix tests:**
- `"!bible what is love"` → ("what is love", true)
- `"!Bible John 3:16"` → ("John 3:16", true)
- `"what is love"` → ("what is love", false)

**VerseRef parsing tests:**
- `"John 3:16"` → book="John", start_chapter=3, start_verse=Some(16), end=None
- `"Romans 8:28-30"` → book="Romans", start_chapter=8, start_verse=Some(28), end_verse=Some(30)
- `"Genesis 1"` → book="Genesis", start_chapter=1, start_verse=None

**format_bible_context tests:**
- Given a verse with original text + Strong's → output contains Hebrew/Greek text
- Given cross-refs → output contains "Cross-references" section
- Given lexicon entries → output contains "Word study" section
- Empty inputs → empty string returned
- Context cap: given 50 verses → output is ≤ 6000 chars

---

# ============================================================
# PHASE B3 — Bible Dashboard Endpoints + Data Prep Scripts
# STATUS: TODO
# ESTIMATED: ~300 lines (daemon endpoints + scripts + dashboard wiring)
# DEPENDS ON: B0 (schema), B1 (ingestion), B2 (bible.rs module)
# ============================================================

## What You Are Building

HTTP endpoints on the daemon for the dashboard to query Bible data directly, plus the finishing touches: dashboard UI tab, data preparation scripts for sourcing the actual public-domain files, and an update to CLAUDE.md documenting the Bible Agent.

## 1. New daemon endpoints in `daemon.rs`

Add these endpoints following the existing patterns (CORS headers, JSON responses, 503 if no pool):

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/bible/stats` | open | Returns `{verses, pericopes, cross_refs, lexicon}` counts |
| GET | `/bible/verse/:book/:chapter/:verse` | open | Single verse with full data (original, KJV, WEB, Strong's, morphology) |
| GET | `/bible/range/:book/:startChap/:startVerse/:endChap/:endVerse` | open | Verse range |
| GET | `/bible/search?q=...` | open | Semantic search — embeds the query, returns top 10 verse matches |
| GET | `/bible/strongs/:id` | open | Lexicon entry for a Strong's number |
| GET | `/bible/strongs/:id/verses` | open | All verses containing that Strong's number (limit 50) |
| GET | `/bible/crossrefs/:book/:chapter/:verse` | open | Cross-references from a verse |
| GET | `/bible/pericope/:id` | open | Pericope by ID with summary |

### Implementation notes

- For `/bible/search?q=...`: embed the query text using `crate::memory::embed()`, then call `crate::db::search_bible_verses()`. Return the top 10 results as JSON with distance scores.
- For `/bible/verse/:book/:chapter/:verse`: URL-decode the book name (it may have spaces → `%20` or `+`). Use `crate::db::get_bible_verse()`.
- All responses: include the original language text, KJV, WEB, and Strong's numbers. The dashboard can display these as tabs or expandable sections.
- Follow the existing pattern in daemon.rs: match on the path, parse params, call db function, serialize to JSON, write_response.

### Response JSON formats

**Single verse:**
```json
{
  "book": "Genesis",
  "chapter": 1,
  "verse": 1,
  "testament": "OT",
  "original_lang": "hebrew",
  "original_text": "בְּרֵאשִׁית בָּרָא...",
  "kjv_text": "In the beginning God created...",
  "web_text": "In the beginning, God created...",
  "strongs": ["H7225", "H1254", "H430", "H853", "H8064", "H853", "H776"],
  "morphology": { ... }
}
```

**Search results:**
```json
{
  "results": [
    {
      "verse": { ...same as above... },
      "distance": 0.234,
      "pericope": { "title": "The Creation", "genre": "narrative" }
    }
  ],
  "query": "what does the bible say about creation"
}
```

**Cross-refs:**
```json
{
  "source": "Genesis 1:1",
  "cross_refs": [
    {
      "target": "John 1:1",
      "rel_type": "thematic",
      "target_verse": { "kjv_text": "In the beginning was the Word..." }
    }
  ]
}
```

## 2. Dashboard UI (optional — add only if time allows)

In `dashboard/src/App.jsx`, add a "Bible" tab alongside Chat, Memory, etc. This is a stretch goal — the daemon endpoints are the priority.

If implemented, the Bible tab should have:
- A search bar that calls `/bible/search?q=...`
- Results displayed as verse cards showing reference, original text, KJV, and Strong's links
- Clicking a Strong's number calls `/bible/strongs/:id` and shows the lexicon entry
- Cross-reference links that load the target verse inline

## 3. Update CLAUDE.md

Add a new section documenting the Bible Agent:

```markdown
---

## Bible Agent (Phase B)

**Files:**
- `rust/crates/rusty-claude-cli/src/bible.rs` — query classification, context assembly
- `rust/crates/rusty-claude-cli/src/bible_ingest.rs` — ingestion pipeline
- `rust/crates/rusty-claude-cli/src/db.rs` — Bible query functions (search_bible_*, insert_bible_*, get_bible_*, get_lexicon_*, etc.)
- `rust/migrations/005_bible_schema.sql` — bible_verses, bible_pericopes, bible_cross_refs, bible_lexicon tables

### Data sources (all public domain)
- KJV text — verse-aligned JSON
- WEB (World English Bible) — verse-aligned JSON
- Westminster Leningrad Codex — Hebrew/Aramaic OT with morphological tags
- unfoldingWord Greek NT — Greek NT with morphological tags
- Strong's Concordance — Hebrew + Greek lexicon entries
- Treasury of Scripture Knowledge — ~63,000 cross-references

### How it works
1. User sends a message (with or without `!bible` prefix)
2. `classify_query()` determines if it's a Bible reference, word study, topical question, or not Bible-related
3. For Bible queries: retrieves relevant verses (semantic search + exact reference), cross-references (graph traversal), and lexicon entries (Strong's lookup)
4. Assembles a rich context block showing original Hebrew/Greek text, multiple English translations (KJV + WEB), Strong's numbers with definitions, and cross-references
5. AI generates response grounded in the retrieved data — original language analysis is based on stored lexical data, not hallucinated

### Ingestion
Run `claw bible-ingest` after placing data files in `.ghost/bible-data/`. Downloads are manual (see `scripts/download-bible-data.ps1`).

### Embedding strategy
Composite embedding per verse: `"{book} {chapter}:{verse} | {original_text} | {kjv_text} | {strongs_joined}"`. One vector captures reference, original language, English gloss, and lexical identifiers. Queried via Voyage AI's multilingual embedding model.

### Endpoints
| Method | Path | Description |
|--------|------|-------------|
| GET | `/bible/stats` | Table counts |
| GET | `/bible/verse/:book/:chapter/:verse` | Single verse |
| GET | `/bible/range/:book/:startChap/:startVerse/:endChap/:endVerse` | Verse range |
| GET | `/bible/search?q=...` | Semantic search |
| GET | `/bible/strongs/:id` | Lexicon entry |
| GET | `/bible/strongs/:id/verses` | Verses by Strong's number |
| GET | `/bible/crossrefs/:book/:chapter/:verse` | Cross-references |
```

## 4. Smoke test the full pipeline

After all phases are complete, verify the full flow works end-to-end:

1. `claw bible-ingest` — should ingest data and print stats
2. Chat: "!bible John 3:16" — should return verse with original Greek, KJV, WEB, Strong's, cross-refs
3. Chat: "what does agape mean" — should auto-detect as Bible word study, return G26 lexicon entry + verses
4. Chat: "what does the bible say about patience" — should return topical results with multiple verses + cross-refs
5. Chat: "what's the weather" — should NOT trigger Bible context
6. `GET /bible/stats` — should return non-zero counts
7. `GET /bible/search?q=love` — should return verse results with distances

---

# ============================================================
# BUILD ORDER SUMMARY
# ============================================================

| Phase | Name | Depends On | Estimated Lines | Sessions |
|-------|------|-----------|-----------------|----------|
| **B0** | Bible DB schema + core DB functions | — | ~400 | 1 |
| **B1** | Bible data ingestion pipeline | B0 | ~500 | 1-2 |
| **B2** | Bible Agent module (query + AI integration) | B0, B1* | ~450 | 1 |
| **B3** | Daemon endpoints + dashboard + docs | B0, B1, B2 | ~300 | 1 |

*B2 code can be written without data, but testing requires B1 to have run.

**Total: 4-5 worker sessions.**

### Data acquisition (manual step between B0 and B1)

Before running Phase B1, Isaac needs to source the data files. The download script from B1 helps, but some datasets may need manual download from GitHub repos:

- **KJV JSON:** many sources, e.g., github.com/thiagobodruk/bible or similar
- **WEB JSON:** worldenglishbible.org or similar GitHub mirrors
- **Hebrew WLC:** github.com/openscriptures/morphhb → JSON exports
- **Greek UGNT:** github.com/unfoldingWord/en_ugnt → JSON exports
- **Strong's lexicon:** Various GitHub repos with parsed Strong's data
- **TSK cross-references:** github.com/nicholasgasior/bible-api-go or openbible.info/labs/cross-references

All are public domain or freely licensed. The exact JSON format may need light transformation to match the expected schema — the download script or a small Python script can handle this.
