# GHOST Pipeline — Worker Prompts (All Phases)

> **How to use:** Open a new Claude Code session in this repo, paste the header + the specific phase prompt below.
> Always paste the header first, then the phase. One phase per session.

---

## HEADER (paste this at the top of every worker session)

You are working on GHOST, a personal AI operating system built in Rust. The repo is at the current working directory. Before doing anything, read these files in order:

1. `CLAUDE.md` — repo conventions, verification commands, gotchas
2. `VISION.md` — full system architecture and roadmap
3. `PIPELINE.md` — the cost-intelligence pipeline design (your implementation spec)
4. `rust/crates/rusty-claude-cli/src/chat_dispatcher.rs` — current chat dispatch logic
5. `rust/crates/rusty-claude-cli/src/compress.rs` — filler stripping + confidence heuristics
6. `rust/crates/rusty-claude-cli/src/constants.rs` — model string constants
7. `rust/crates/rusty-claude-cli/src/db.rs` — database functions
8. `rust/crates/rusty-claude-cli/src/daemon.rs` — HTTP daemon with endpoints
9. `rust/crates/rusty-claude-cli/src/memory.rs` — embedding + note extraction
10. `rust/crates/rusty-claude-cli/src/routing.rs` — keyword routing

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

- Do NOT refactor existing code that works. Only touch what's needed.
- Do NOT add comments or docstrings to code you didn't write.
- Do NOT change the daemon endpoints or HTTP layer unless the phase explicitly says to.
- Do NOT modify the memory system, embedding pipeline, or database schema unless the phase explicitly says to.
- Do NOT add new dependencies unless absolutely necessary (prefer std library).
- Do NOT create README files or documentation.
- Do NOT commit. Leave changes uncommitted for review.

---

# ============================================================
# PHASE A — Cascade Routing + Filler Stripping
# STATUS: COMPLETE (2026-04-16)
# ============================================================

Phase A has been implemented. See `compress.rs` for `strip_filler()` + `is_low_confidence()` and `chat_dispatcher.rs` for the Haiku → Sonnet → Opus cascade.

**Files created/modified:**
- `rust/crates/rusty-claude-cli/src/compress.rs` (NEW — ~200 lines + 14 tests)
- `rust/crates/rusty-claude-cli/src/chat_dispatcher.rs` (MODIFIED — cascade logic)
- `rust/crates/rusty-claude-cli/src/constants.rs` (MODIFIED — added OPUS_MODEL)
- `rust/crates/rusty-claude-cli/src/main.rs` (MODIFIED — added `mod compress`)

---

# ============================================================
# PHASE B — Intake Model (Prompt Polisher)
# STATUS: COMPLETE (2026-04-16)
# ============================================================

## What You Are Building

A cheap preprocessing step that detects ambiguous or complex prompts and rewrites them into tight specs before the expensive cascade fires. This sits between filler stripping and the cascade.

**This is NOT conversational.** It's a single Haiku call that either passes the message through unchanged or rewrites it. No back-and-forth, no UX change.

## 1. Add to `compress.rs`

### `needs_intake(message: &str) -> bool` (pure heuristic, no API)

```rust
fn needs_intake(message: &str) -> bool {
    let word_count = message.split_whitespace().count();
    
    // Short, direct messages don't need polishing
    if word_count < 15 {
        return false;
    }
    
    // Long messages (50+ words) almost always benefit
    if word_count >= 50 {
        return true;
    }
    
    // 15-49 words: check for ambiguity signals
    let lower = message.to_lowercase();
    const AMBIGUITY_SIGNALS: &[&str] = &[
        "or something", "kind of", "sort of", "maybe", "i think",
        "i guess", "not sure", "somehow", "whatever", "stuff",
        "things", "the thing", "you know", "like ", "basically",
        "idk", "i dunno",
    ];
    
    AMBIGUITY_SIGNALS.iter().any(|s| lower.contains(s))
}
```

### `intake_polish(message: &str) -> Result<String, String>` (async, calls Haiku)

```rust
pub async fn intake_polish(message: &str) -> Result<String, String> {
    if !needs_intake(message) {
        return Ok(message.to_string());
    }

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    let system = "You are a prompt polisher. Your ONLY job is to rewrite the user's message \
        as a clear, unambiguous request. Rules:\n\
        1. Remove filler, hedging, and repeated ideas\n\
        2. Make implicit requests explicit\n\
        3. If the user mentions multiple things, list them as numbered items\n\
        4. Keep the user's intent exactly — do NOT add requirements they didn't mention\n\
        5. Return ONLY the rewritten message. No preamble, no quotes, no explanation.\n\
        6. If the message is already clear, return it unchanged.\n\
        7. Never exceed 2x the original word count.";

    let body = serde_json::json!({
        "model": crate::constants::HAIKU_MODEL,
        "max_tokens": 512,
        "system": system,
        "messages": [{"role": "user", "content": message}],
    });

    let client = crate::http_client::shared_client();
    let resp = client
        .post(crate::constants::ANTHROPIC_MESSAGES_URL)
        .timeout(std::time::Duration::from_secs(15))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("intake API failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("intake API error: {}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await
        .map_err(|e| format!("intake parse failed: {e}"))?;

    let polished = json["content"][0]["text"]
        .as_str()
        .unwrap_or(message)
        .to_string();

    // Safety: if polished is way longer, model hallucinated — use original
    if polished.split_whitespace().count() > message.split_whitespace().count() * 2 {
        eprintln!("[ghost intake] polished too long, using original");
        return Ok(message.to_string());
    }

    eprintln!("[ghost intake] polished: {} -> {} words",
        message.split_whitespace().count(),
        polished.split_whitespace().count());

    Ok(polished)
}
```

### Unit tests for `needs_intake` (add to existing `#[cfg(test)]` block)

- Short message (< 15 words) → false
- Long message (50+ words) → true
- Medium message WITH ambiguity signal ("maybe", "sort of") → true
- Medium message WITHOUT ambiguity signal → false
- Exactly 15 words, no signal → false
- Exactly 50 words → true

## 2. Wire into `chat_dispatcher.rs`

Current dispatch flow:
```
cleaned = strip_filler(message)
→ embed cleaned, pull memory
→ cascade
```

New flow:
```
cleaned = strip_filler(message)
polished = intake_polish(&cleaned).await.unwrap_or_else(|e| {
    eprintln!("[ghost intake] failed: {e}, using cleaned message");
    cleaned.clone()
})
→ embed polished, pull memory (using polished)
→ cascade (using polished in messages array)
→ memory extraction still uses ORIGINAL message (not polished)
```

**Key:** intake failure is NEVER fatal. Always fall through with cleaned message. Memory extraction at the end still uses the original `message` param.

## Estimated scope: ~120-180 lines changed/added

---

# ============================================================
# PHASE C — Scholar Database
# STATUS: COMPLETE (2026-04-16)
# ============================================================

## What You Are Building

A Postgres table that caches successful solutions and tracks failed approaches. When a similar problem comes in, the system checks the scholar DB first — cache hit means $0.00 API cost. Failed attempts are injected as "DO NOT TRY" warnings.

This phase does NOT wire into the full orchestrator — it hooks into the existing chat dispatcher as a context enhancement (similar to how memory notes are injected today).

## 1. New Migration: `rust/migrations/003_scholar_solutions.sql`

```sql
-- Scholar DB: caches successful solutions, tracks failed approaches.
-- Prevents repeated mistakes and skips API calls for solved problems.

CREATE TABLE IF NOT EXISTS scholar_solutions (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    problem_sig     TEXT NOT NULL,
    problem_embed   VECTOR(1024),
    failed_attempts TEXT[] DEFAULT '{}',
    solution        TEXT NOT NULL,
    solution_lang   TEXT,
    context_file    TEXT,
    success_count   INTEGER NOT NULL DEFAULT 1,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

## 2. New functions in `db.rs`

### `search_scholar(pool, embedding, limit) -> Vec<ScholarSolution>`

Query by cosine similarity on `problem_embed`, ordered by `problem_embed <=> $1::vector`, limited to `limit` rows. Return struct:

```rust
pub struct ScholarSolution {
    pub id: String,
    pub problem_sig: String,
    pub solution: String,
    pub solution_lang: Option<String>,
    pub context_file: Option<String>,
    pub failed_attempts: Vec<String>,
    pub success_count: i32,
    pub last_used_at: String,
}
```

### `insert_scholar(pool, problem_sig, problem_embed, solution, solution_lang, context_file) -> bool`

Insert a new solution. Embed the problem signature for future matching.

### `increment_scholar_success(pool, id)`

Bump `success_count` by 1 and update `last_used_at` to `now()`.

### `add_scholar_failed_attempt(pool, id, attempt: &str)`

Append to the `failed_attempts` array: `UPDATE scholar_solutions SET failed_attempts = array_append(failed_attempts, $2) WHERE id = $1::uuid`.

## 3. Wire into `chat_dispatcher.rs`

After memory context is loaded but before the cascade, add scholar context injection:

```
load_memory_context(...)
load_scholar_context(&cleaned, pool)  // NEW
→ if scholar hit with success_count >= 3 AND high similarity:
    inject "## Previously solved similar problem\n<solution>" into system prompt
→ if scholar hit with failed_attempts:
    inject "## Known bad approaches for similar problems\nDO NOT TRY:\n- ..." into system prompt
→ cascade runs with this extra context
```

Add a new function `load_scholar_context(message: &str, pool: Option<&PgPool>) -> String` similar to `load_memory_context`. Embed the message, search scholar DB, format results.

**Similarity threshold:** Only inject solutions with cosine distance < 0.15 (i.e., cosine sim > 0.85). Failed attempts inject at a looser threshold (distance < 0.25).

## 4. Post-response: store new solutions

After a successful cascade response (in the fire-and-forget section alongside `extract_and_store`), also store the problem+solution pair in the scholar DB:

```rust
tokio::spawn(async move {
    // Existing memory extraction
    crate::memory::extract_and_store(pool_clone, msg, resp.clone()).await;
    // New: store in scholar DB
    store_scholar_solution(pool_clone2, msg2, resp).await;
});
```

`store_scholar_solution` should:
1. Embed the message
2. Check if a very similar problem already exists (cosine distance < 0.1)
3. If exists: `increment_scholar_success` (don't duplicate)
4. If new: `insert_scholar`

## Estimated scope: ~200-250 lines (migration + db functions + dispatcher wiring)

---

# ============================================================
# PHASE D1 — Orchestrator Module (Planning)
# STATUS: COMPLETE (2026-04-16)
# ============================================================

## What You Are Building

The first half of the orchestrator-worker pattern. This phase builds the orchestrator — the smart model that reads a spec and produces a plan with CRITICAL (do-it-myself) and DELEGATE (worker prompt) items.

**This does NOT replace the existing chat dispatch path.** It's a new module that the Director (or a future `!` prefix handler) can invoke for complex tasks. The chat dispatcher continues to handle simple messages.

## 1. New file: `rust/crates/rusty-claude-cli/src/orchestrator.rs`

### Core types

```rust
pub enum PlanItem {
    Critical {
        description: String,
        output: Option<String>,  // filled in after orchestrator writes it
    },
    Delegate {
        description: String,
        worker_prompt: String,   // hyper-specific prompt for cheap model
        output: Option<String>,  // filled in after worker executes
        status: WorkerStatus,
    },
}

pub enum WorkerStatus {
    Pending,
    Running,
    Done,
    Failed(String),
    Escalated,
}

pub struct Plan {
    pub task_description: String,
    pub items: Vec<PlanItem>,
    pub created_at: std::time::Instant,
}
```

### `orchestrate(spec: &str, pool: Option<&PgPool>) -> Result<Plan, String>`

1. Call Opus (or Sonnet for specs < 100 tokens) with a system prompt that instructs it to produce a plan.
2. The system prompt must tell the model to output in a parseable format:

```
[DO] <description of critical item>
[DELEGATE] <description> ||| <hyper-specific worker prompt>
```

3. Parse the response into a `Plan` with `Critical` and `Delegate` items.
4. For each `Critical` item: the orchestrator's response text IS the output (it already wrote it).
5. Return the plan — execution happens in Phase D2.

### Orchestrator system prompt

```
You are a task planner. Given a specification, produce a plan.

For each item, decide:
- [DO] — you handle this yourself (architecture, design, glue logic, anything requiring full context)
- [DELEGATE] — a cheap, fast model handles this (isolated code, simple transforms, boilerplate)

DELEGATE prompts must be hyper-specific:
- Exact function signature if applicable
- Numbered behavior steps
- What NOT to add (no extra error handling, no comments, no logging)
- Expected output length hint

Format each line:
[DO] description of what you're doing, then your actual output
[DELEGATE] description ||| the exact prompt to send to the worker model

Produce 3-15 items. If the task is simple enough for one step, output one [DO] item.
```

### Parse function

`parse_plan(response: &str) -> Vec<PlanItem>` — splits response by lines starting with `[DO]` or `[DELEGATE]`, extracts the `|||` separator for delegate prompts. Tolerant of formatting variation (extra whitespace, missing delimiters fall back to CRITICAL).

### Add `mod orchestrator;` to `main.rs`

## 2. Unit tests

- Parse a well-formatted plan → correct PlanItem variants
- Parse a single `[DO]` item → one Critical item
- Parse with missing `|||` delimiter → falls back to Critical (not crash)
- Parse empty response → empty plan
- Parse mixed DO and DELEGATE → correct ordering preserved

## Estimated scope: ~200-250 lines (module + types + parse + system prompt + tests)

---

# ============================================================
# PHASE D2 — Worker Execution + Checkpoints
# STATUS: COMPLETE (2026-04-16)
# ============================================================

## What You Are Building

The execution engine: takes a `Plan` from Phase D1, runs DELEGATE items in parallel via cheap model calls, runs checkpoints on outputs, and assembles the final result.

## 1. Add to `orchestrator.rs`

### `execute_plan(plan: &mut Plan, pool: Option<&PgPool>) -> Result<String, String>`

```
For each DELEGATE item in plan.items (in parallel via tokio::spawn):
    1. Check scholar DB for cached solution (Phase C)
       - Hit with success_count >= 3 → use cached, mark Done, skip API call
       - Hit with failed_attempts → inject "DO NOT TRY" into worker prompt
    2. Call Haiku with the worker_prompt (max_tokens: 1024)
    3. Run checkpoint on output:
       - If worker_prompt mentions code: check for balanced braces, no obvious syntax errors
       - If text output: check length > 20 chars, no hedging phrases
    4. Checkpoint pass → mark Done, store in scholar DB
    5. Checkpoint fail → retry ONCE with error context appended to prompt
    6. Second fail → mark Escalated, will be handled by orchestrator

For CRITICAL items: output is already filled from orchestrate() call.

For ESCALATED items: call Sonnet with the original worker_prompt + failed output context.

Assemble all outputs in plan order → return combined result.
```

### `call_worker(prompt: &str) -> Result<String, String>`

Simple Haiku call with a minimal system prompt: "You are a precise code/text generator. Follow the instructions exactly. Output only what is requested — no preamble, no explanation."

### `checkpoint(output: &str, prompt: &str) -> bool`

Heuristic check:
- If prompt contains "function", "fn ", "struct", "impl": check for balanced `{}` and `()`
- If output is empty or < 20 chars: fail
- If output contains hedging: fail
- Otherwise: pass

### Cheap reviewer (optional, if time allows)

After all items assembled, one final Haiku call: "Review this combined output for missing imports, duplicate names, type mismatches, cross-file integration errors. List any issues found."

If issues found → targeted Sonnet micro-fix calls.

## 2. Wire into dispatch

Add a new function in `chat_dispatcher.rs` or a new endpoint:

```rust
pub async fn dispatch_complex(
    spec: &str,
    pool: Option<&sqlx::PgPool>,
) -> Result<String, String> {
    let mut plan = orchestrate(spec, pool).await?;
    execute_plan(&mut plan, pool).await
}
```

For now, this is called only when explicitly triggered (future: Director routes complex tasks here). Do NOT replace the existing simple `dispatch()` path.

## Estimated scope: ~250-300 lines (execution loop + worker calls + checkpoint + assembly)

---

# ============================================================
# PHASE E1 — Code Macro System
# STATUS: COMPLETE (2026-04-16)
# ============================================================

## What You Are Building

A macro library that lets AI outputs use shorthand references instead of writing common code patterns line by line. A post-processor expands macros into real code.

## 1. Macro definition file: `.ghost/macros.toml`

Create the directory `.ghost/` at repo root if it doesn't exist. Create `macros.toml`:

```toml
# GHOST Code Macros — shorthand patterns for AI output
# AI writes: MACRO_NAME("param1", "param2")
# Post-processor expands to full code block with params substituted.

[macros.CORS_PREFLIGHT]
params = ["allowed_origins"]
expansion = """
fn write_cors_headers(response: &mut Response, origin: &str) {
    response.headers_mut().insert("Access-Control-Allow-Origin", origin.parse().unwrap());
    response.headers_mut().insert("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS".parse().unwrap());
    response.headers_mut().insert("Access-Control-Allow-Headers", "Content-Type, Authorization, X-Claw-Key".parse().unwrap());
    response.headers_mut().insert("Access-Control-Allow-Private-Network", "true".parse().unwrap());
}
"""

[macros.AUTH_BEARER]
params = ["key_var"]
expansion = """
let expected = std::env::var("{key_var}").unwrap_or_default();
if !validate_bearer(&auth_header, &expected) {
    return write_response(stream, 401, r#"{{"error":"unauthorized"}}"#, origin).await;
}
"""

[macros.DB_QUERY_EMBED]
params = ["table", "embed_col", "limit"]
expansion = """
sqlx::query(
    &format!("SELECT * FROM {table} WHERE {embed_col} IS NOT NULL ORDER BY {embed_col} <=> $1::vector LIMIT {limit}")
)
.bind(&embedding_str)
.fetch_all(pool)
.await
.unwrap_or_default()
"""

[macros.HAIKU_CALL]
params = ["system_prompt", "max_tokens"]
expansion = """
let body = serde_json::json!({{
    "model": crate::constants::HAIKU_MODEL,
    "max_tokens": {max_tokens},
    "system": {system_prompt},
    "messages": messages,
}});
let client = crate::http_client::shared_client();
let resp = client
    .post(crate::constants::ANTHROPIC_MESSAGES_URL)
    .timeout(std::time::Duration::from_secs(60))
    .header("x-api-key", &api_key)
    .header("anthropic-version", "2023-06-01")
    .json(&body)
    .send()
    .await
    .map_err(|e| format!("API failed: {{e}}"))?;
"""
```

## 2. New file: `rust/crates/rusty-claude-cli/src/macros.rs`

### `load_macros() -> HashMap<String, Macro>`

Read `.ghost/macros.toml` from the repo root (or `GHOST_MACROS_PATH` env). Parse TOML into:

```rust
pub struct Macro {
    pub name: String,
    pub params: Vec<String>,
    pub expansion: String,
}
```

Use the `toml` crate (add to Cargo.toml if not already present — this is one of the rare cases where a new dep is justified).

### `expand_macros(text: &str, macros: &HashMap<String, Macro>) -> String`

Scan `text` for patterns matching `MACRO_NAME("param1", "param2", ...)`. For each match:
1. Look up the macro by name
2. Substitute `{param_name}` placeholders in the expansion with the provided arguments
3. Replace the macro call with the expanded code

Use a regex: `([A-Z_]+)\(([^)]*)\)` — match uppercase macro names followed by parenthesized args.

### `inject_macro_dictionary(prompt: &str, macros: &HashMap<String, Macro>) -> String`

Prepend a preamble to worker prompts listing available macros:

```
Available code macros (use these instead of writing the code):
- AUTH_BEARER("key_var") — bearer auth check against env var
- DB_QUERY_EMBED("table", "embed_col", "limit") — cosine search query
- HAIKU_CALL("system_prompt", "max_tokens") — Haiku API call
...
When you can use a macro, write MACRO_NAME("args") on its own line.
```

### Add `mod macros;` to `main.rs`

## 3. Wire into orchestrator (Phase D)

In `execute_plan`:
- Before sending each DELEGATE prompt: call `inject_macro_dictionary` to add available macros
- After receiving each worker output: call `expand_macros` to expand any macro references

## 4. Unit tests

- Load macros from TOML string → correct HashMap
- Expand single macro with params → correct substitution
- Expand macro in surrounding text → only macro replaced, rest preserved
- Unknown macro name → left as-is (no crash)
- Macro with wrong param count → left as-is
- Empty macros file → empty HashMap, no errors

## Estimated scope: ~200-250 lines (TOML parsing + expansion + injection + tests)

**Note:** Add `toml = "0.8"` to `[dependencies]` in `rust/crates/rusty-claude-cli/Cargo.toml`.

---

# ============================================================
# PHASE E2 — Prompt Compression (Project-Aware Abbreviation)
# STATUS: COMPLETE (2026-04-16)
# ============================================================

## What You Are Building

A lookup table that maps Isaac's natural language to project-specific terms, reducing input tokens. This is Layer B from PIPELINE.md Component 6.

## 1. Abbreviation file: `.ghost/abbreviations.toml`

```toml
# Project-aware abbreviations — maps Isaac's natural language to precise terms.
# Applied before any API call. Grows as GHOST learns Isaac's vocabulary.

[abbreviations]
"the daemon file" = "daemon.rs"
"the daemon" = "daemon.rs"
"the dashboard" = "dashboard/src/App.jsx"
"the memory stuff" = "director_notes table + memory.rs"
"the memory thing" = "director_notes table + memory.rs"
"the auth thing" = "validate_bearer in daemon.rs"
"the auth" = "validate_bearer in daemon.rs"
"the routing" = "routing.rs"
"the routing thing" = "routing.rs"
"the search" = "search.rs (Brave Search)"
"the search thing" = "search.rs (Brave Search)"
"the context file" = "ghost-context.txt (GHOST_CORE_CONTEXT_PATH)"
"the pipeline" = "PIPELINE.md cost-intelligence pipeline"
"scholar db" = "scholar_solutions table"
"the scholar" = "scholar_solutions table"
```

## 2. Add to `compress.rs`

### `load_abbreviations() -> Vec<(String, String)>`

Read `.ghost/abbreviations.toml`. Return as a sorted-by-length-descending list of (pattern, replacement) pairs so longer patterns match first.

Cache the loaded abbreviations in a `once_cell::sync::Lazy` or `std::sync::OnceLock` so the file is only read once per daemon lifetime.

### `apply_abbreviations(message: &str) -> String`

Case-insensitive replacement of all known abbreviations. Longest match first to avoid partial replacements.

## 3. Wire into `chat_dispatcher.rs`

Call `apply_abbreviations` after `strip_filler` and `intake_polish`:

```
cleaned = strip_filler(message)
polished = intake_polish(&cleaned).await...
abbreviated = apply_abbreviations(&polished)
→ use abbreviated for embedding, memory search, cascade
```

## 4. Unit tests

- Known abbreviation replaced → correct term
- Case insensitive → "The Daemon" maps same as "the daemon"
- Unknown phrase → unchanged
- Longer pattern takes priority → "the memory stuff" matches before "the memory"
- Abbreviation in sentence context → only the phrase replaced, not surrounding words

## Estimated scope: ~80-120 lines

---

# ============================================================
# PHASE F1 — Token Recycling (Response Cache)
# STATUS: COMPLETE (2026-04-16)
# ============================================================

## What You Are Building

A response cache: when GHOST generates a good response, cache it with its embedding. Future similar queries get the cached response injected as context so the model edits instead of re-deriving.

## 1. New Migration: `rust/migrations/004_response_cache.sql`

```sql
CREATE TABLE IF NOT EXISTS response_cache (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    query_text      TEXT NOT NULL,
    query_embed     VECTOR(1024),
    response_text   TEXT NOT NULL,
    hit_count       INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_hit_at     TIMESTAMPTZ
);
```

## 2. New functions in `db.rs`

### `search_response_cache(pool, embedding, limit) -> Vec<CachedResponse>`

```rust
pub struct CachedResponse {
    pub id: String,
    pub query_text: String,
    pub response_text: String,
    pub hit_count: i32,
}
```

Cosine search on `query_embed`, return top matches.

### `insert_response_cache(pool, query_text, query_embed, response_text) -> bool`

### `increment_cache_hit(pool, id)`

Bump `hit_count`, update `last_hit_at`.

## 3. Wire into `chat_dispatcher.rs`

Before the cascade:
1. Embed the polished message
2. Search `response_cache` — if hit with cosine distance < 0.05 (very similar) and `hit_count >= 2`:
   - Inject into system prompt: `"## Similar previous response\nYou previously answered a similar question:\n<cached response>\nAdapt this for the current request if applicable."`
3. The cascade model sees the cached response and edits it instead of generating from scratch → fewer output tokens

After the cascade:
- Store the new query+response pair in the cache (fire-and-forget alongside memory extraction)
- If a very similar query already exists (distance < 0.03), `increment_cache_hit` instead of inserting a duplicate

## 4. Cache cleanup

Add to the existing 24-hour decay background task: delete response_cache entries older than 90 days with `hit_count < 3` (rarely-used responses aren't worth keeping).

## Estimated scope: ~150-200 lines (migration + db functions + dispatcher wiring + cleanup)

---

# ============================================================
# PHASE F2 — Speculative Parallel Execution
# STATUS: COMPLETE (2026-04-16)
# ============================================================

## What You Are Building

When the orchestrator is uncertain about the correct approach, it forks into parallel cheap model calls exploring different paths. Whichever path succeeds first wins. The others are dropped.

## 1. Add to `orchestrator.rs`

### New plan item variant

```rust
pub enum PlanItem {
    Critical { ... },
    Delegate { ... },
    Speculative {
        description: String,
        branches: Vec<SpecBranch>,
        winner: Option<usize>,  // index of winning branch after execution
    },
}

pub struct SpecBranch {
    pub hypothesis: String,
    pub worker_prompt: String,
    pub output: Option<String>,
    pub status: WorkerStatus,
}
```

### Orchestrator plan format update

Add a new output format for the orchestrator:

```
[SPECULATE] description ||| hypothesis_a: prompt_a ||| hypothesis_b: prompt_b
```

The orchestrator uses this when it's unsure which approach is correct but verification is cheap.

### `execute_speculative(spec: &Speculative, pool: ...) -> Result<String, String>`

1. Spawn all branches in parallel via `tokio::spawn`
2. Each branch: call Haiku with the worker prompt
3. Run checkpoint on each result
4. First branch to pass checkpoint → winner
5. Cancel remaining branches (drop the JoinHandles)
6. If no branch passes: escalate all branch outputs to Sonnet

Use `tokio::select!` or collect futures and take the first `Ok` result.

### Wire into `execute_plan`

When processing `Speculative` items, call `execute_speculative` instead of the normal worker path.

## 2. Update orchestrator system prompt

Add to the orchestrator's system prompt:
```
[SPECULATE] — when you're unsure between 2-3 approaches and verification is cheap.
Format: [SPECULATE] description ||| hypothesis_a: worker_prompt_a ||| hypothesis_b: worker_prompt_b
```

## 3. Unit tests

- Parse `[SPECULATE]` line with two branches → correct Speculative variant
- Parse with three branches → correct
- Parse with one branch → falls back to Delegate (speculation needs 2+)

## Estimated scope: ~150-200 lines

---

# ============================================================
# BUILD ORDER SUMMARY
# ============================================================

| Phase | Name | Depends On | Scope | Sessions |
|-------|------|-----------|-------|----------|
| **A** | Cascade routing + filler stripping | — | ~200 lines | DONE |
| **B** | Intake model (prompt polisher) | A | ~150 lines | 1 |
| **C** | Scholar database | A | ~250 lines | 1 |
| **D1** | Orchestrator module (planning) | A, B | ~250 lines | DONE |
| **D2** | Worker execution + checkpoints | D1, C | ~300 lines | DONE |
| **E1** | Code macro system | D1, D2 | ~250 lines | DONE |
| **E2** | Prompt compression (abbreviations) | A | ~100 lines | DONE |
| **F1** | Token recycling (response cache) | C | ~200 lines | DONE |
| **F2** | Speculative parallel execution | D1, D2 | ~200 lines | DONE |

**Total remaining: 8 worker sessions.**

Phases B, C, and E2 can be done independently (no cross-dependencies beyond Phase A).
D1 → D2 is strictly sequential.
E1 and F2 require D1+D2 to be useful.
F1 requires C (shared embedding search pattern).
