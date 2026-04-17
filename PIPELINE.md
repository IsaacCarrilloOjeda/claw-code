# GHOST Cost-Intelligence Pipeline — Design Document

> Brainstormed by Isaac + Claude on 2026-04-16.
> This is the source of truth for the orchestrator-worker pipeline, prompt compression,
> macro system, scholar database, and all cost-optimization patterns designed for GHOST.
> Read this before implementing any of these systems.

---

## Problem Statement

API calls are expensive. Isaac has no local GPU. GHOST needs to be powerful but cheap.
The insight: most of what an expensive model does on any given task is stuff a cheap model
could handle, or stuff that's already been solved before. Pay the smart model to think.
Pay the dumb model to type. Pay nothing for solved problems.

---

## Architecture Overview

```
Isaac types (messy, polite, vague)
    |
    v
[1. INTAKE MODEL] — GPT-4o-mini
    Strips filler/manners, asks 5-10 clarifying questions,
    compresses final spec. You write English, it produces specs.
    |
    v
[2. ORCHESTRATOR] — Opus or Sonnet (one call)
    Reads the spec. Produces:
      - 3-5 CRITICAL items it handles itself (architecture, design, glue logic)
      - N DELEGATE items as hyper-specific worker prompts
    |
    ├── CRITICAL items: Orchestrator writes these directly
    |
    ├── DELEGATE items (parallel):
    |     |
    |     ├── [3a. SCHOLAR DB CHECK] — is this solved already?
    |     |     Hit → skip API call, use cached solution ($0.00)
    |     |     Miss → continue to worker
    |     |
    |     ├── [3b. MACRO EXPANSION CHECK] — can the worker use shorthand?
    |     |     Inject macro dictionary into worker prompt
    |     |
    |     └── [3c. WORKER MODEL] — GPT-4o-mini, <1000 tokens output
    |           Hyper-specific prompt, nearly impossible to misunderstand
    |           |
    |           v
    |     [4. CHECKPOINT] — compile/lint/parse check (local, free)
    |           Pass → accept
    |           Fail → one retry with error context
    |           Fail again → ESCALATE to orchestrator
    |
    v
[5. ASSEMBLY + REVIEW]
    All outputs collected. Cheap reviewer (mini) checks:
      - Missing imports, duplicate names, type mismatches
      - Cross-file integration errors
    Issues found → Orchestrator micro-fixes (small, targeted calls)
    |
    v
[6. POST-PROCESSING]
    - Macro shorthand expanded to real code
    - Solutions stored in Scholar DB for next time
    - Compression dictionary updated if new patterns detected
    |
    v
Final output written to files
```

---

## Component 1: Intake Model (Prompt Polisher)

**Purpose:** Isaac writes casually. The intake model turns that into a precise spec
before anything expensive happens.

**Model:** GPT-4o-mini (~$0.0003 per intake)

**What it does:**
1. STRIPS: filler words, manners, greetings, sign-offs, hedging ("sorry if", "i know this is dumb"), restating the same idea multiple times
2. ASKS: 5-10 targeted questions to resolve ambiguity. Returns these to Isaac.
3. COMPRESSES: Isaac's answers + original intent → tight spec for the orchestrator.

**Estimated savings:** Prevents ~2 misunderstanding-and-retry cycles per complex task.
At Opus prices, that's $0.30-0.50 saved per task for a $0.0003 investment.

**Filler word strip list (starter — grows over time):**
```
hey, hi, hello, so, like, just, please, thanks, thank you, sorry,
lol, haha, um, uhh, hmm, basically, actually, honestly, literally,
i think, i guess, i know this is, would you mind, can you please,
if that makes sense, does that make sense, no worries if not,
sorry if this is a lot, i hope this isn't too much
```

**Pattern learning:** After 50 conversations, log recurring prompt structures.
~40% of prompts match ~15 structural patterns. These get compressed via lookup
table, no model call needed.

---

## Component 2: Orchestrator (Smart Planner)

**Purpose:** The expensive model's job is to PLAN, not to TYPE.
One call. Produces a plan + worker prompts. Never writes bulk code itself.

**Model:** Opus for complex tasks, Sonnet for medium tasks.
(Routing decision: if intake spec is <100 tokens and no architecture signals, use Sonnet.)

**Output format:**
```
PLAN:
1. [DO] Design the schema — embedding strategy needs to handle...
2. [DELEGATE] Write SQL migration: "CREATE TABLE bible_verses (id UUID PRIMARY KEY, ...)"
3. [DELEGATE] Write parser function: "pub fn parse_line(line: &str) -> ..."
4. [DO] Write the search ranking algorithm — needs cosine similarity with...
5. [DELEGATE] Write endpoint handler: "pub async fn search_bible(query: ...) -> ..."
...
```

**Worker prompt quality is everything.** Each DELEGATE prompt must include:
- Exact function signature
- Exact behavior (numbered steps)
- Which crates/libraries to use
- What NOT to add (no extra error handling, no comments, no logging)
- Expected output length hint

**Bad:** "Write the auth module"
**Good:** "Write a function `pub async fn validate_bearer(header: &str, expected: &str) -> bool` that: 1) strips 'Bearer ' prefix case-insensitively, 2) compares remainder to expected using constant-time comparison via the `subtle` crate's ConstantTimeEq trait, 3) returns bool. Do not add error handling, logging, or comments."

---

## Component 3: Scholar Database (Institutional Memory)

**Purpose:** Never re-solve a solved problem. Never repeat a known-bad approach.

**Schema:**
```sql
scholar_solutions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    problem_sig     TEXT NOT NULL,        -- semantic description of the problem
    problem_embed   VECTOR(1536),         -- for semantic search
    failed_attempts TEXT[],               -- what DIDN'T work (prevents loops)
    solution        TEXT NOT NULL,         -- what DID work
    solution_lang   TEXT,                 -- rust, sql, javascript, etc.
    context_file    TEXT,                 -- which file this was for
    success_count   INT DEFAULT 1,        -- incremented each time reused successfully
    created_at      TIMESTAMPTZ DEFAULT now(),
    last_used_at    TIMESTAMPTZ DEFAULT now()
)
```

**Flow:**
```
Worker prompt arrives
  → embed the problem description
  → cosine search against scholar_solutions.problem_embed
  → if match with success_count >= 3 AND cosine_sim > 0.92:
      → return cached solution directly, skip API call ($0.00)
  → if match with failed_attempts:
      → inject "DO NOT TRY: [list]" into worker prompt
  → no match:
      → call worker model normally
      → if checkpoint passes → store new solution
      → if checkpoint fails → add to failed_attempts for closest match
```

**The failed_attempts array is critical.** This is what prevents the infinite loop
where the AI keeps trying the same broken approach. After one failure, that approach
is blacklisted for that problem signature forever.

**Warmup period:** First month, almost no cache hits. By month 3, ~20-30% of worker
tasks skip the API entirely. By month 6, ~40-50%. The system gets cheaper over time.

---

## Component 4: Code Macro System

**Purpose:** The AI outputs shorthand references instead of writing common code patterns
line by line. A post-processor expands macros into real code.

**How macros are built:**
1. One-time scan of the repo for repeated patterns (3+ occurrences)
2. Extract as named macros with parameter slots
3. Store in `.ghost/macros.toml` or similar

**Example macros (GHOST-specific):**
```toml
[CORS.preflight]
params = ["allowed_origins"]
expansion = """
fn write_cors_headers(response: &mut Response, origin: &str) {
    response.headers_mut().insert("Access-Control-Allow-Origin", origin.parse().unwrap());
    response.headers_mut().insert("Access-Control-Allow-Methods", "GET, POST, DELETE, OPTIONS".parse().unwrap());
    response.headers_mut().insert("Access-Control-Allow-Headers", "Content-Type, Authorization, X-Claw-Key".parse().unwrap());
    response.headers_mut().insert("Access-Control-Allow-Private-Network", "true".parse().unwrap());
}
"""

[AUTH.bearer]
params = ["key_var"]
expansion = """
let expected = std::env::var("{key_var}").unwrap_or_default();
if !validate_bearer(&auth_header, &expected) {
    return write_response(stream, 401, r#"{{"error":"unauthorized"}}"#, origin).await;
}
"""

[DB.query.by_embedding]
params = ["table", "embed_col", "limit"]
expansion = """
sqlx::query_as!(
    NoteRow,
    "SELECT * FROM {table} WHERE {embed_col} IS NOT NULL ORDER BY {embed_col} <=> $1::vector LIMIT {limit}",
    &embedding as _
)
.fetch_all(pool)
.await?
"""
```

**AI outputs:** `AUTH.bearer("GHOST_DAEMON_KEY")`
**Post-processor expands** to the full code block with the parameter filled in.

**Output token savings:** 60-80% for code-heavy tasks. The AI writes ~5 tokens
where it would have written ~80.

**Macro evolution:** When the AI writes the same pattern 3+ times without using a
macro, the system flags it: "New macro candidate detected. Auto-registering."
The library grows organically.

---

## Component 5: Cascade with Confidence Check

**Purpose:** For simple tasks that don't need the full orchestrator pipeline —
try the cheapest model first, escalate only on failure.

**Flow (no AI routing — pure heuristic):**
```
Step 1: Send to GPT-4o-mini ($0.15/M input tokens)
Step 2: Check response:
        - Shorter than 50 chars? → probably failed, escalate
        - Contains "I can't" / "I'm not sure" / "I don't know"? → escalate
        - Asked for code and response doesn't parse? → escalate
        - All checks pass → return it, done
Step 3: Only if checks fail → send to Sonnet ($3/M input tokens)
Step 4: Only if Sonnet also fails → send to Opus ($15/M input tokens)
```

~80% of requests resolve at Step 1. Dramatic cost savings for routine tasks.

---

## Component 6: Prompt Compression (Background, Passive)

**Purpose:** Reduce input token cost without Isaac changing how he writes.

**Three layers:**

### Layer A: Filler stripper (regex, free)
Removes manners, greetings, filler words before any API call.
Isaac's messages are ~30-40% noise tokens. This layer is immediate savings.

### Layer B: Project-aware abbreviation (lookup table, free)
Maps Isaac's natural language to project terms:
```
"the daemon file" → daemon.rs
"the dashboard" → dashboard/src/App.jsx
"the memory stuff" → director_notes + memory.rs
"the auth thing" → validate_bearer in daemon.rs
```
Table grows as GHOST learns Isaac's vocabulary.

### Layer C: Compression dictionary (evolves over time)
Frequently repeated phrases get short codes:
```
"look at the daemon and fix" → "daemon.fix:"
"add a new endpoint that" → "ep+:"
"search the notes for" → "notes?:"
```
Injected as a preamble so the model understands the codes.
Net token change: small preamble cost vs. large per-message savings.
Breaks even after ~5 messages using any given code.

---

## Component 7: Speculative Parallel Execution

**Purpose:** When uncertain, explore multiple paths cheaply in parallel instead
of one expensive path that might be wrong.

```
AI is unsure whether bug is in auth or routing

Fork:
  ├── Universe A: mini investigates auth path ($0.0002)
  └── Universe B: mini investigates routing path ($0.0002)

Whichever finds the bug → wins
Other → killed

Cost: $0.0004 total
vs. Opus guessing: $0.02-0.05 (and might guess wrong)
```

Use for: debugging, research queries, any task where the approach is ambiguous
but the verification is cheap.

---

## Component 8: Token Recycling (Response Cache)

**Purpose:** When the AI generates a good response, cache it. Similar future
queries get the cached response injected as context — the AI edits instead of
re-deriving from scratch.

```
Query: "How do I add a new endpoint to the daemon?"
  → hash query + embed
  → search response cache
  → found similar (cosine > 0.95): previous answer about adding /memories endpoint
  → inject: "You previously answered a similar question: [cached]. Adapt for this case."
  → AI edits existing answer instead of generating from scratch
  → ~60% fewer output tokens
```

---

## Cost Model: Full Pipeline vs. Baseline

### Hard project (~50 items, e.g. "Build Bible search module")

| Stage | Baseline (Opus only) | Full Pipeline |
|-------|---------------------|---------------|
| Prompt refinement | $0.47 (2 misunderstandings + retries) | $0.0004 (intake model) |
| Planning | $0.25 | $0.31 (orchestrator, one call) |
| Implementation | $0.90 (Opus writes all code) | $0.011 (27 workers @ mini) |
| Scholar DB hits | — | $0.002 (8 items free from cache) |
| Bug fixing | $0.45 (3 bugs, retries) | $0.001 (checkpoints catch 2 early) |
| Escalation | — | $0.07 (1 item sent back to Opus) |
| Review | — | $0.03 (reviewer + micro-fixes) |
| Wasted work | $0.30 (dead-end approach) | $0.00 |
| **TOTAL** | **$1.00** (normalized) | **$0.43** |

### Same project at month 6 (warm scholar DB)
| | Cost |
|---|---|
| Month 1 | $0.43 |
| Month 3 | $0.30 |
| Month 6 | $0.20 |
| Month 12 | $0.12-0.15 |

**The system gets cheaper the more you use it.**

---

## Accuracy Improvements

| Failure mode | Baseline | Pipeline |
|---|---|---|
| Misunderstood requirements | Common | Rare (intake asks questions) |
| Wrong approach, full redo | ~20% chance | ~5% (plan reviewed first) |
| Bugs in generated code | ~3 per project | ~1 (checkpoints catch early) |
| Repeated mistake loop | Possible | Blocked (scholar DB failed_attempts) |
| Style drift in long context | Common | Rare (workers share macro library) |

---

## Build Order (recommended)

### Phase A: Cascade routing (highest immediate savings, simplest)
- Implement try-cheap-first-then-escalate in chat_dispatcher.rs
- No new infrastructure needed
- Immediate ~60-70% cost reduction on simple queries

### Phase B: Intake model (prevents wasted orchestrator calls)
- Add intake step before chat_dispatcher
- Strip filler, ask questions, compress
- Prevents the most expensive failure mode: misunderstandings

### Phase C: Scholar database (compounds over time)
- New Postgres table + embedding pipeline
- Wire into worker dispatch
- Starts saving money after ~2 weeks of use

### Phase D: Orchestrator-worker dispatch
- The big architectural change
- Opus plans, mini executes, checkpoints verify
- Requires A, B, C to be in place for full effect

### Phase E: Code macros + prompt compression
- Polish layer — meaningful but not urgent
- Best built organically as patterns emerge from real usage

### Phase F: Speculative execution + token recycling
- Advanced optimizations
- Build when the core pipeline is stable and proven

---

## Integration with Existing GHOST Architecture

All of this lives inside the existing daemon. No new services needed.

- **Intake** → new function in `chat_dispatcher.rs`, called before `dispatch()`
- **Orchestrator** → new module `orchestrator.rs`, called for complex tasks
- **Scholar DB** → new table in Postgres, new functions in `db.rs`
- **Workers** → `tokio::spawn` parallel calls to mini, new module `workers.rs`
- **Checkpoints** → worker output → `cargo check` / syntax parse → pass/fail
- **Macros** → `.ghost/macros.toml` → expander in post-processing
- **Compression** → preprocessor in `chat_dispatcher.rs`

The daemon already has: Postgres, pgvector, embedding pipeline (Voyage AI),
async runtime (tokio), multi-model support. The pipeline is an extension of
what's already there, not a rewrite.

---

## Related Future Projects (discussed, not yet designed)

- **Bible semantic search** — embed KJV by verse, cosine search, cross-reference via similarity. Same pgvector pattern as director_notes. New table `bible_verses`.
- **Lawyer AI** — embed US Code / CFR / case law, search tool in agent harness, system prompt constrains to scope/citations only (no legal advice). New table `legal_sources`.
- **Code-only agent** — strip tools to file ops + shell + code execution, point at DeepSeek-Coder via Ollama or API. Uses orchestrator-worker pattern with code-specialized macros.

All three are variations of the same embed-store-search pattern. The pipeline
infrastructure built here serves all of them.
