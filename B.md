# Worker B (Wave 5) — Dreamer agent + `interest_nodes` migration

One-line task: ship `agents/dreamer.rs` (an agent that reads recent ghost_events + director_notes, runs a Sonnet reflection pass, emits "interest nodes" with embeddings), plus migration `015_interest_nodes.sql`, plus the db.rs helpers for the new table.

**Do not register the agent with the dispatcher yet.** That's Wave 5.5 — mirrors how Calendar (Wave 3) and Docs (Wave 4) landed. Worker A owns `agents/intent.rs` and `agents/dispatcher.rs` this wave and does NOT add Dreamer — that lands in a post-merge one-liner once Isaac decides on a prefix (most likely `~`).

---

## Read first

1. `CLAUDE.md` + `ARCHITECTURE.md`.
2. `rust/crates/rusty-claude-cli/src/agents/research.rs` — the closest shape to what you're building (composes a Haiku/Sonnet call with pulled context). Copy its structure.
3. `rust/crates/rusty-claude-cli/src/agents/mod.rs` — `Agent` trait + `AgentRequest`/`AgentResponse`/`Usage`/`Source`.
4. `rust/crates/rusty-claude-cli/src/memory.rs` — the embedding helper `embed(text) -> Option<Vec<f32>>` and how notes get written with pgvector. Reuse `embed`; do NOT reimplement.
5. `rust/crates/rusty-claude-cli/src/db.rs` — style for CRUD helpers (`insert_note`, `list_notes`, `search_notes`). Append your `interest_nodes` helpers at the end, following the same style.
6. `rust/crates/rusty-claude-cli/src/infra/events.rs` — the `ghost_events` reader you'll pull recent rows from. If it doesn't expose a "recent events" query, add one there — but ONLY if necessary; prefer a one-off SQL in db.rs.
7. Any recent migration (`014_scheduled_triggers.sql` is closest; also `001_initial.sql` for the `vector(1536)` column type) — match the style.
8. `rust/crates/rusty-claude-cli/src/constants.rs` — `HAIKU_MODEL`, `SONNET_MODEL`, `ANTHROPIC_API_URL`.
9. `rust/crates/rusty-claude-cli/src/http_client.rs` — `shared_client()` only.

---

## Scope — exactly these files

| Path | Action |
|---|---|
| `rust/crates/rusty-claude-cli/src/agents/dreamer.rs` | **Create** |
| `rust/crates/rusty-claude-cli/src/agents/mod.rs` | **Edit** — add `pub mod dreamer;` (one line; A also appends here — trivial merge) |
| `rust/migrations/015_interest_nodes.sql` | **Create** (renumber if 015 is taken — see below) |
| `rust/crates/rusty-claude-cli/src/db.rs` | **Edit (append only)** — `InterestNode` struct + helpers |

**Do NOT touch:** `agents/intent.rs`, `agents/dispatcher.rs` (A owns both), `daemon.rs` (C owns it), any other agent file, any `infra/*` file, dashboard, `memory.rs`. Your edits are additive only.

---

## Design (already decided)

### Migration 015 — `interest_nodes` table

**Numbering check:** `ls rust/migrations/` before writing. If `015` is taken, use the next sequential free number. The filename MUST be `0NN_interest_nodes.sql`.

```sql
-- Wave 5: Dreamer's reflective write surface.
-- Dreamer reads recent ghost_events + director_notes, condenses recurring
-- themes into "interest nodes" with embeddings. Chat dispatcher and Chief
-- of Staff can pgvector-query this table to surface what Isaac is thinking
-- about lately without re-reading raw events.
CREATE TABLE IF NOT EXISTS interest_nodes (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    topic        TEXT NOT NULL,           -- short label ("react-native build")
    summary      TEXT NOT NULL,           -- 1–3 sentence description
    weight       FLOAT NOT NULL DEFAULT 1.0,  -- decayed over time
    embedding    VECTOR(1536),            -- NULL if no embedding provider configured
    source_refs  JSONB NOT NULL DEFAULT '[]'::jsonb,  -- [{kind, id}] pointers
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_interest_nodes_weight
    ON interest_nodes (weight DESC);

-- Mirrors director_notes — pgvector requires an explicit ivfflat/hnsw index
-- only once the table has real data. Skip the ANN index this wave; a table
-- scan over ~hundreds of rows is fine.
```

### db.rs helpers (append only)

```rust
pub struct InterestNode {
    pub id: Uuid,
    pub topic: String,
    pub summary: String,
    pub weight: f32,
    pub embedding: Option<Vec<f32>>,
    pub source_refs: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn insert_interest_node(
    pool: &PgPool,
    topic: &str,
    summary: &str,
    embedding: Option<&[f32]>,
    source_refs: &serde_json::Value,
) -> Result<Uuid, sqlx::Error> { ... }

pub async fn list_interest_nodes(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<InterestNode>, sqlx::Error> { ... }

/// Recent activity window the Dreamer reflects on. Pulls the last N
/// ghost_events rows AND the last M director_notes. Returned as plain
/// formatted strings — Dreamer doesn't need structured rows.
pub async fn dreamer_window(
    pool: &PgPool,
    events_limit: i64,
    notes_limit: i64,
) -> Result<Vec<String>, sqlx::Error> { ... }
```

The `dreamer_window` return is a `Vec<String>` where each entry is one event/note formatted like `[event 2026-04-19 research] Isaac asked about: <input>` — Dreamer concatenates them as the user prompt. Keep the total window text bounded at ~8 KiB (caller responsibility, not SQL — truncate in Rust after pulling).

Use `sqlx::query_as!` or manual `sqlx::query` — match whatever style the existing file uses (Wave 4's `due_triggers` / `mark_fired` are the style precedent).

### `agents/dreamer.rs` — `Agent` trait impl

```rust
//! Dreamer — offline reflection agent.
//!
//! Wave 5 scope: one verb. Reads recent ghost_events + director_notes,
//! asks Sonnet to identify 3–8 recurring topics with 1–3 sentence summaries,
//! embeds each, upserts into `interest_nodes`. Returns a human-readable
//! summary for the caller (usually a scheduled_triggers row Isaac added
//! manually that fires at 03:00 UTC daily).
//!
//! This file intentionally does NOT register itself with the dispatcher —
//! that's a one-liner in Wave 5.5 after all Wave 5 branches merge. Isaac
//! will pick a prefix post-merge (likely `~`).
//!
//! Auth: none external. Just `ANTHROPIC_API_KEY` for Sonnet + an embedding
//! key (VOYAGE_API_KEY or OPENAI_API_KEY) for the `embed()` call.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;

use super::{Agent, AgentRequest, AgentResponse, ModelTier, Usage};
use crate::constants::{ANTHROPIC_API_URL, SONNET_MODEL};
use crate::http_client::shared_client;
use crate::{db, memory};
```

#### Structure

```rust
pub struct Dreamer;

impl Dreamer {
    pub fn new() -> Self { Self }
}
impl Default for Dreamer { fn default() -> Self { Self::new() } }

#[async_trait]
impl Agent for Dreamer {
    fn name(&self) -> &'static str { "dreamer" }
    fn declared_tier(&self) -> ModelTier { ModelTier::Mid }
    fn requires_approval(&self, _req: &AgentRequest) -> bool { false }
    async fn handle(&self, _req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String> {
        let window = db::dreamer_window(pool, 200, 50)
            .await
            .map_err(|e| format!("dreamer window query failed: {e}"))?;
        if window.is_empty() {
            return Ok(AgentResponse {
                text: "dreamer: no recent activity to reflect on".into(),
                usage: Usage::default(),
                tier: ModelTier::Mid,
            });
        }
        let (topics, usage) = reflect(&window).await?;
        let mut wrote = 0u32;
        for t in &topics {
            let emb = memory::embed(&format!("{}\n{}", t.topic, t.summary)).await;
            let refs = json!([]);
            if db::insert_interest_node(
                pool,
                &t.topic,
                &t.summary,
                emb.as_deref(),
                &refs,
            ).await.is_ok() {
                wrote += 1;
            }
        }
        Ok(AgentResponse {
            text: format!("dreamer: wrote {wrote} interest node(s)"),
            usage,
            tier: ModelTier::Mid,
        })
    }
}
```

#### The reflect call

```rust
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Topic {
    pub topic: String,
    pub summary: String,
}

async fn reflect(window_lines: &[String]) -> Result<(Vec<Topic>, Usage), String> { ... }
```

System prompt:
```
You are the Dreamer — a reflection process that reads Isaac's recent GHOST
activity and names what's been occupying his attention. Output a JSON array
of 3 to 8 topics:

[{ "topic": "<3–5 word label>", "summary": "<1–3 sentences>" }, ...]

Pick recurring themes, not one-off mentions. No preamble, no code fence.
```

User prompt: join `window_lines` with `\n` (bounded to ~8 KiB — truncate from the START to keep the freshest entries; oldest drops first).

Model: `SONNET_MODEL`, `max_tokens: 1024`. Parse the `content[*].text` as a JSON array; strip a leading ```json``` fence if present. On parse failure: `Err(format!("could not parse dreamer topics: {e}: raw = {raw}"))`.

Extract `Usage` from `usage.input_tokens` + `usage.output_tokens` same as `chat_dispatcher`.

#### Edge cases

- No embedding key set → `memory::embed` returns `None` — write the row with `NULL` embedding, same as `director_notes` does today.
- Zero topics back from Sonnet (parse ok, empty array) → return `"dreamer: no themes emerged"`, `usage` from the call.
- Any single `insert_interest_node` error → log and continue; do NOT fail the whole run. The return string reports how many succeeded.
- `ANTHROPIC_API_KEY` missing → `Err("ANTHROPIC_API_KEY not set — dreamer disabled".into())`.

### Unit tests (at least these four)

1. `Topic` parses from a representative JSON array.
2. Code-fence stripping: wrapping ```json``` is removed before `serde_json::from_str`.
3. Oldest-first truncation: a helper like `fn truncate_from_start(lines: &[String], max_bytes: usize) -> String` keeps the newest and drops the oldest. Write a 3-line test that verifies the oldest line is dropped when total exceeds the cap.
4. `requires_approval` is false for any input.

No network, no DB integration tests. Pure pieces only — match Wave 4 Docs test style.

---

## Constraints — do NOT

- Do NOT register the agent with the dispatcher. No `agents/intent.rs` edits. No `agents/dispatcher.rs` edits. Wave 5.5 handles it.
- Do NOT add new crate dependencies. `async_trait`, `serde`, `serde_json`, `sqlx`, `reqwest` are in tree.
- Do NOT touch `memory.rs` — reuse `memory::embed`. If it's not `pub`, flag it and stop rather than changing visibility unilaterally.
- Do NOT add a "dreamer_runs" audit table. The dispatcher already writes `ghost_events` when Dreamer is wired in Wave 5.5; until then a manual trigger invocation will just run silently.
- Do NOT implement weight decay this wave. The column exists; a future cron touches it. Write every new row with `weight = 1.0`.
- Do NOT create an ivfflat/hnsw index in the migration — the table is empty and the index can land after there's real data.
- Do NOT modify the `Agent` trait or `AgentRequest`/`AgentResponse` shapes.
- Do NOT touch `daemon.rs`, `dispatcher.rs`, `intent.rs`, `chat_dispatcher.rs`, `director.rs`, `infra/*`.

---

## Implementation order

1. Write migration `015_interest_nodes.sql` (renumber if taken — see above; flag Isaac in your PR note if you had to renumber).
2. Append `db.rs` helpers: `InterestNode` struct + `insert_interest_node` + `list_interest_nodes` + `dreamer_window`.
3. Create `agents/dreamer.rs` with `Agent` trait impl + `reflect` + tests.
4. Add `pub mod dreamer;` to `agents/mod.rs`.
5. `cargo fmt && cargo clippy -p rusty-claude-cli --bins -- -D warnings && cargo test -p rusty-claude-cli --bins`.
6. All green → done.

---

## Verification

```bash
cd rust
cargo fmt
cargo clippy -p rusty-claude-cli --bins -- -D warnings
cargo test -p rusty-claude-cli --bins
```

All green. Pre-existing plugin-lifecycle failures don't count (see CLAUDE.md).

Manual smoke (optional, if Postgres handy):
```sql
SELECT id, topic, weight, created_at FROM interest_nodes ORDER BY created_at DESC LIMIT 5;
```

---

## Done criteria

- Migration `015_interest_nodes.sql` (or next-free number) exists and runs on daemon startup via `sqlx::migrate!`.
- `agents/dreamer.rs` exists, implements `Agent`, compiles clippy-clean.
- `db.rs` has `InterestNode` + `insert_interest_node` + `list_interest_nodes` + `dreamer_window`.
- `agents/mod.rs` has `pub mod dreamer;`.
- At least 4 unit tests covering the cases listed above.
- NOT registered with the dispatcher — that's explicitly Wave 5.5.
- Scoped clippy green, scoped tests green.

---

## If you hit ambiguity

Stop and flag:
- If `memory::embed` isn't `pub` or its signature differs from `embed(&str) -> Option<Vec<f32>>`, flag it — do NOT make it public without Isaac's sign-off.
- If `dreamer_window` requires data from `ghost_events` columns that don't exist, flag it — check `infra/events.rs` for the `ghost_events` schema and match reality.
- If a migration with number 015 already exists from another branch, use the next free slot and note the renumber in your final report.
- If `SONNET_MODEL` isn't the right tier (budget concerns), flag it — don't silently switch to Haiku.
- If C's `db.rs` append (for `insert_scheduled_trigger`) happens to land in the same hunk you're editing, standard git-merge append — both land cleanly.
