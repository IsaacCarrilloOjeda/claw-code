# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Detected stack
- Languages: Rust (CLI/runtime), JavaScript/React (dashboard).
- Frameworks: Vite + React for `dashboard/`.

## Verification
- Run Rust verification from `rust/`: `cargo fmt`, then scoped clippy/tests: `cargo clippy -p rusty-claude-cli -p plugins --bins -- -D warnings` and `cargo test -p rusty-claude-cli --bins`.
- Full-workspace clippy is currently blocked on Windows by pre-existing Unix-only code in the `runtime` crate (`std::os::unix::fs::PermissionsExt`, `set_mode(0o755)` without `#[cfg(unix)]` gates in `mcp_stdio.rs`, `mcp_tool_bridge.rs`, `file_ops.rs`, `tests/mock_parity_harness.rs`). Known follow-up — do not let it block unrelated work.
- Three pre-existing test failures in `build_runtime_plugin_state_discovers_mcp_tools`, `build_runtime_runs_plugin_lifecycle_init_and_shutdown`, `parses_direct_agents_mcp_and_skills_slash_commands` — separate cleanup.
- `src/` and `tests/` are both present; update both surfaces together when behavior changes.

## Repository shape
- `rust/` — Rust workspace, active CLI/runtime. Main binary: `claw` (`rusty-claude-cli`).
- `dashboard/` — React/Vite web UI. Talks to daemon at `http://127.0.0.1:7878`.
- `scripts/` — PowerShell helpers (Task Scheduler setup, etc.).
- `src/` and `tests/` — stay consistent with generated guidance and tests.

## Working agreement
- Prefer small, reviewable changes and keep generated bootstrap files aligned with actual repo workflows.
- Keep shared defaults in `.claude.json`; reserve `.claude/settings.local.json` for machine-local overrides.
- Do not overwrite existing `CLAUDE.md` content automatically; update it intentionally when repo workflows change.

---

## Lessons & gotchas

- **API key env**: `ANTHROPIC_API_KEY` must be set in the terminal that starts the daemon — subprocess inherits it from there, not from Task Scheduler or admin shells.
- **Git Bash vs PowerShell**: user works in Git Bash (`/c/` paths); PowerShell scripts need `C:\` paths; never mix them in instructions.
- **claw.exe file lock**: kill running `claw.exe` before rebuilding (`Stop-Process -Name claw -Force`) or cargo will fail to replace the binary.
- **CORS for POST**: `Allow-Origin: *` alone isn't enough — browsers also require `Allow-Methods` and `Allow-Headers` on preflight or POST fails with "Failed to fetch". The daemon now uses an explicit allow-list (not wildcard) and sends all three.
- **Chrome Private Network Access**: Chrome 104+ blocks requests from a regular origin (`http://localhost:5173`) to a "private" loopback (`http://127.0.0.1:7878`) unless the server echoes `Access-Control-Allow-Private-Network: true`. Symptom: dashboard polls successfully at TCP level (TIME_WAITs visible) but fetch throws `TypeError: Failed to fetch`, so the UI shows "daemon unreachable". Fix lives in `write_response` in `daemon.rs`.
- **CORS allow-list must cover all loopback forms**: `http://localhost:5173`, `http://127.0.0.1:5173`, and `http://[::1]:5173` are all legitimate dashboard origins depending on how the user navigated. Allow all three by default; extra origins via `GHOST_DAEMON_CORS_ORIGIN` (comma-separated, renamed from `CLAW_DAEMON_CORS_ORIGIN` in Phase 0).
- **Em-dashes in PowerShell scripts**: write `.ps1` files with ASCII-only characters; em-dashes (`—`) corrupt on write and cause parse errors.
- **Inline comments on backtick continuations**: PowerShell line-continuation backtick (`` ` ``) must be the very last character on the line — a trailing `# comment` breaks the parser.
- **Gerald Brain cold starts**: the Render.com server sleeps; 4s timeout is the right call — don't raise it or REPL startup feels broken.
- **Task Scheduler env**: scheduled tasks don't inherit user shell env vars; store `ANTHROPIC_API_KEY` in `~/.claw/settings.json` under `anthropicApiKey` for headless operation.
- **Dashboard dev server**: run `npm run dev` from `dashboard/`; it proxies nothing — the daemon must already be running on port 7878.
- **Railway HOST/PORT**: Railway injects `HOST=0.0.0.0` and `PORT=<dynamic>` — daemon reads these automatically; no flags needed in Railway CMD. Set `HOST=0.0.0.0` in Railway env vars if it's not injected.
- **GHOST_DAEMON_KEY** (renamed from `CLAW_DAEMON_KEY` in Phase 0): set this in Railway environment variables before deploy or `/prompt` will always refuse.
- **sqlx offline mode**: `SQLX_OFFLINE=true` is set in the Dockerfile so the build doesn't need a DB. Migrations run at daemon startup via embedded `sqlx::migrate!()`. No `.sqlx/` cache dir needed.
- **pgvector on Railway**: enable via `CREATE EXTENSION IF NOT EXISTS vector;` in first migration. Requires Railway's Postgres to have the pgvector extension available — it is on Railway's managed Postgres.
- **Dashboard DAEMON_URL**: set `VITE_DAEMON_URL=https://<your-service>.railway.app` in the dashboard's Vite env (`.env.production` or Railway build env) to point at the deployed daemon.
- **OPENAI_API_KEY is OpenRouter here**: the user routes non-Anthropic models through OpenRouter using `OPENAI_API_KEY`. OpenRouter does NOT support the embeddings endpoint — do not use it for embeddings. Use `VOYAGE_API_KEY` instead.
- **Embeddings provider**: `VOYAGE_API_KEY` → Voyage AI `voyage-3` with `output_dimension=1536` (matches DB schema). Falls back to `OPENAI_API_KEY` → OpenAI `text-embedding-3-small` (1536 dims). If neither key is set, notes store with `NULL` embedding and still appear in the memory panel — semantic injection just stays empty.
- **Conversation history cap**: `POST /chat` accepts optional `history: [{role, content}]`. Daemon validates and caps at 6 entries (3 exchanges). Each content field capped at 8192 chars. SMS path always sends empty history (single-turn by nature).

---

## Daemon (`claw daemon`)

**File:** `rust/crates/rusty-claude-cli/src/daemon.rs`  
**Default port:** 7878 — binds `127.0.0.1` by default.  
**PID file:** `~/.claw/daemon.pid`  
**Log file:** `~/.claw/daemon.log` (written by Task Scheduler launcher)

### Endpoints
All return JSON. CORS header on every response.

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/health` | open | Uptime + pid. Always public. |
| GET | `/status` | open on localhost | Version, cwd, session count, config home, uptime, db_connected. |
| GET | `/sessions` | open on localhost | Workspace + file basenames (no absolute paths). |
| GET | `/jobs` | open | List recent 100 jobs from Postgres. 503 if DB not configured. |
| GET | `/jobs/:id` | open | Single job detail by UUID. |
| GET | `/director/config` | open | Current primary/fallback model + health flags. |
| POST | `/director/config` | **bearer** | Swap primary or fallback model. Body: `{"primary_model":"...","fallback_model":"..."}`. |
| POST | `/prompt` | **flag + key + bearer** | Runs a one-shot prompt via `claw prompt` subprocess. Body: `{"prompt":"...","model":"..."}` |
| POST | `/chat` | **bearer** (when key set) | Synchronous chat via `chat_dispatcher`. Body: `{"message":"...","history":[{role,content}]}`. Returns `{"response":"...","job_id":"..."}`. 503 if DB not configured. |
| POST | `/sms/inbound` | open | Accept SMS from Android Gateway (JSON) or Twilio (form-encoded). Spawns background task, returns 200 immediately. |
| POST | `/sms/send` | open | Internal: deliver outbound SMS. Body: `{"to":"+1...","body":"..."}`. |
| GET | `/memories` | open | List up to 200 non-expired memory notes. 503 if DB not configured. |
| DELETE | `/memories/:id` | **bearer** | Delete a single note by UUID. |
| GET | `/bible/stats` | open | Row counts for all Bible tables. 503 if DB not configured. |
| GET | `/bible/verse/:book/:ch/:v` | open | Single verse lookup. URL-encode book names with spaces (`1%20John`). |
| GET | `/bible/range/:book/:sCh/:sV/:eCh/:eV` | open | Verse range (e.g., `Romans/8/28/8/30`). |
| GET | `/bible/search?q=...` | open | Semantic verse search via embedding. Returns top 20 matches with distances. 503 if no embedding provider. |
| GET | `/bible/strongs/:id` | open | Lexicon entry + verses containing the Strong's number. |
| GET | `/bible/crossrefs/:book/:ch/:v` | open | Cross-references from and to a verse. |

### `/prompt` security model (mandatory — fails closed)
`POST /prompt` shells out to a subprocess with `--dangerously-skip-permissions`, so it is hardened three ways:
1. Daemon must be started with `--allow-unsafe-prompt` — otherwise `/prompt` returns `403`.
2. `GHOST_DAEMON_KEY` env var must be set to a **non-empty** value when the daemon starts; empty = refuse to start.
3. Each request must carry `Authorization: Bearer <key>` or `X-Claw-Key: <key>` matching `GHOST_DAEMON_KEY` (constant-time compare). Missing/wrong → `401`.

Additional hardening:
- `Host` header validated against bind address (DNS-rebinding defense → `421`). Bypassed on `0.0.0.0`.
- Request body capped at 1 MiB (→ `413`), read in bounded loop.
- `model` field validated against allow-list charset; prompts starting with `--` rejected.
- stdout/stderr piped through `redact_secrets` — replaces API keys with `***redacted***`.

### Background tasks (daemon startup)
Two tasks are spawned alongside the HTTP server:
1. **Circuit-breaker reset** — calls `db::reset_health_flags` every 5 minutes. Restores `primary_healthy` / `fallback_healthy` after failures.
2. **Confidence decay** — calls `db::decay_notes_confidence` every 24 hours. Reduces confidence 5% on notes older than 30 days; expires notes below 0.1.

### Phone / network access
```bash
GHOST_DAEMON_KEY=<token> claw daemon --host 0.0.0.0 --port 7878 --allow-unsafe-prompt
```
Clients send `Authorization: Bearer <token>` or `X-Claw-Key: <token>`.

### Task Scheduler (Windows, run once as admin)
```powershell
.\scripts\daemon-install.ps1
```
Creates `ClawDaemon` task: starts at boot, runs as current user, restarts on failure (3x / 1 min).

---

## Memory system (Phase 2 — complete)

**Files:**
- `rust/crates/rusty-claude-cli/src/memory.rs` — embedding + note extraction
- `rust/crates/rusty-claude-cli/src/db.rs` — `insert_note`, `search_notes`, `list_notes`, `delete_note`, `decay_notes_confidence`
- `rust/crates/rusty-claude-cli/src/chat_dispatcher.rs` — dispatch with history + memory injection

### How it works
1. **On every chat response**: `extract_and_store` runs fire-and-forget via `tokio::spawn`. Calls Haiku with a prompt that extracts 0–3 factual notes in `category|content` format. Each note is embedded and stored in `director_notes`.
2. **On every incoming chat message**: the message is embedded, top-5 notes retrieved by cosine similarity (`embedding <=> $1::vector`), injected into the system prompt under `## What you remember about Isaac`.
3. **Conversation history**: `dispatch(message, history, job_id, pool)` accepts up to 6 prior messages (3 exchanges) and sends them as a messages array to Anthropic so GHOST maintains context.
4. **Confidence decay**: notes older than 30 days get `confidence * 0.95` daily. Notes below 0.1 are expired (soft-deleted via `expires_at`).

### Embedding providers (checked in order)
| Priority | Env var | Provider | Model | Dims |
|----------|---------|----------|-------|------|
| 1st | `VOYAGE_API_KEY` | Voyage AI | `voyage-3` | 1536 (via `output_dimension`) |
| 2nd | `OPENAI_API_KEY` | OpenAI | `text-embedding-3-small` | 1536 |
| fallback | — | none | NULL embedding stored | — |

Notes with NULL embedding appear in the memory panel but don't rank in semantic search.

### Note categories
`personal` · `social` · `code` · `projects` · `style` · `calendar`

### DB schema (migration `001_initial.sql`)
```sql
director_notes (
  id           UUID PRIMARY KEY,
  category     TEXT NOT NULL,
  content      TEXT NOT NULL,
  embedding    VECTOR(1536),       -- NULL if no embedding key set
  confidence   FLOAT DEFAULT 1.0,
  expires_at   TIMESTAMPTZ,        -- set when confidence < 0.1
  created_at   TIMESTAMPTZ DEFAULT now()
)
```

---

## Chat dispatcher (`chat_dispatcher.rs`)

**Signature:** `dispatch(message, history, job_id, pool) -> Result<String, String>`

- Loads core context from `GHOST_CORE_CONTEXT_PATH` (falls back to minimal default).
- Embeds `message`, pulls top-5 memory notes, injects as `## What you remember about Isaac` block.
- Sends `[...history, {role: "user", content: message}]` to Claude Haiku (`claude-haiku-4-5-20251001`), max 1024 tokens.
- After response: `tokio::spawn(extract_and_store(...))` — fire-and-forget, never blocks latency path.

---

## Bible Agent (`bible.rs` + `bible_ingest.rs`)

**Files:**
- `rust/crates/rusty-claude-cli/src/bible.rs` — query classification, context assembly
- `rust/crates/rusty-claude-cli/src/bible_ingest.rs` — data ingestion pipeline
- `rust/crates/rusty-claude-cli/src/db.rs` — Bible table queries (search, insert, stats)

### How it works
1. **Classification:** `classify_query()` categorizes incoming messages as `Reference` (e.g., "John 3:16"), `WordStudy` (e.g., "what does agape mean"), `Topical` (e.g., "what does the bible say about patience"), or `NotBible`.
2. **Context loading:** `load_bible_context()` retrieves relevant verses, cross-refs, lexicon entries, and pericopes from Postgres, formatted as a context block injected into the system prompt.
3. **Trigger:** Prefix messages with `bible:` to force Bible study mode. Otherwise, classification runs automatically.
4. **Integration:** Chat dispatcher calls `load_bible_context()` after web context, before building the messages array. Bible study mode adds a preamble instructing the model to respond as a Bible scholar.

### CLI: `claw bible-ingest`
```bash
claw bible-ingest [--data-dir path/to/bible-data]
```
Reads verse-aligned JSON/TSV from `.ghost/bible-data/` (or `--data-dir`), batch-embeds via Voyage AI, and bulk-inserts into the four Bible tables. Requires `DATABASE_URL` and `VOYAGE_API_KEY` (or `OPENAI_API_KEY` for embeddings).

### Data files (in `.ghost/bible-data/`)
| File | Required | Format |
|------|----------|--------|
| `kjv.json` | **yes** | `[{book, chapter, verse, text}, ...]` |
| `web.json` | no | Same format as KJV |
| `hebrew-wlc.json` | no | `[{book, chapter, verse, text, strongs[], morphology{}}]` |
| `greek-ugnt.json` | no | Same as Hebrew |
| `strongs-hebrew.json` | no | `[{strongs_id, original_word, transliteration, definition, root, semantic_range[]}]` |
| `strongs-greek.json` | no | Same as Hebrew lexicon |
| `cross-refs.tsv` | no | TSV: `source_book\tsource_chapter\tsource_verse\ttarget_book\ttarget_chapter\ttarget_verse\trel_type` |
| `pericopes.json` | no | `[{title, start_book, start_chapter, start_verse, end_book, end_chapter, end_verse, genre}]` |

### DB schema (migration `002_bible.sql`)
```
bible_verses    — 66 books, verse-level text + embeddings + Strong's + morphology
bible_pericopes — thematic section boundaries with optional embeddings
bible_cross_refs — verse-to-verse relationships (TSK, etc.)
bible_lexicon   — Strong's concordance entries with semantic ranges
```

---

## Gerald Brain (`gerald.rs`)

**File:** `rust/crates/rusty-claude-cli/src/gerald.rs`  
**Server URL:** read from `~/.claw/settings.json` → `mcpServers.gerald-brain.url`  
**Fallback URL:** `https://gerald-core-1.onrender.com/messages`  
**Timeout:** 4 seconds (cold Render starts are slow — failures are silent)

### Behaviour
- **Session start:** `load_context()` calls `get_overview` via MCP HTTP and injects the result as the last system prompt section. Skipped silently if unreachable.
- **Session end:** `save_session()` fires on `/exit` or Ctrl-C. Stores working dir, session ID, turn count, and model used.

### MCP protocol used
Initialize → `notifications/initialized` → `tools/call`. Passes `Mcp-Session-Id` header when the server returns one.

---

## Provider routing (`routing.rs`)

**File:** `rust/crates/rusty-claude-cli/src/routing.rs`  
**Opt-in:** set `CLAW_ROUTING=1` (off by default).  
**Scope:** one-shot `claw prompt "..."` only (not REPL — prompt isn't known upfront).

### Routing table
| Tier | Model | Trigger |
|------|-------|---------|
| fast | `gpt-4o-mini` | ≤ 20 words, no code/arch signals, `OPENAI_API_KEY` set |
| code | `deepseek-chat` | code signals present, no arch signals, `OPENAI_API_KEY` set |
| mid | `claude-sonnet-4-6` | arch/design/review signals, or fallback from above tiers |
| full | `claude-opus-4-6` | default |

`OPENAI_API_KEY` here points to OpenRouter (not real OpenAI). DeepSeek is called via OpenAI-compatible API.  
If `OPENAI_API_KEY` is absent, fast/code tiers fall through to mid/full.

---

## GHOST — Current Project

This repo is being built into **GHOST**, a personal AI operating system.
Full vision, architecture, and roadmap: **`VISION.md`** (read this first for any non-trivial work).

**Phase 0 (Foundation) — COMPLETE**  
**Phase 2 (Memory + Context) — COMPLETE**  
**Current: Phase 3 — Specialist Agents + Research Agent (web search / internet access)**

### Railway env vars (all required for full function)
| Var | Purpose |
|-----|---------|
| `ANTHROPIC_API_KEY` | All Anthropic calls (chat, extraction) |
| `OPENAI_API_KEY` | OpenRouter key — used for provider routing (not embeddings) |
| `VOYAGE_API_KEY` | Voyage AI embeddings — enables semantic memory search |
| `GHOST_DAEMON_KEY` | Bearer auth for `/chat`, `/prompt`, `/director/config`, `DELETE /memories/:id` |
| `HOST` | Set to `0.0.0.0` (Railway injects this automatically) |
| `PORT` | Injected by Railway automatically |
| `DATABASE_URL` | Injected by Railway Postgres add-on |
| `GHOST_CORE_CONTEXT_PATH` | Path to GHOST's personality/system context file |
| `GHOST_ALLOWED_NUMBERS` | Comma-separated E.164 numbers allowed to SMS in |

### Railway deploy notes
- Add Postgres add-on in Railway dashboard → `DATABASE_URL` is auto-injected.
- Migrations run automatically at daemon startup (`sqlx::migrate!`). No manual DB setup needed.
- pgvector (`CREATE EXTENSION IF NOT EXISTS vector`) is in migration `001_initial.sql`.

---

## Dashboard (`dashboard/`)

**Stack:** React 18 + Vite. Single-file component: `dashboard/src/App.jsx`.  
**Dev server:**
```bash
cd dashboard && npm run dev
# http://localhost:5173
```
**Daemon dependency:** expects daemon at `http://127.0.0.1:7878` locally, or `VITE_DAEMON_URL` in production.

### Features
- **Status bar** — alive indicator, uptime, session count, version, pid. Auto-polls every 10s.
- **KEY field** — password input persists to `localStorage['ghost-daemon-key']`. Sent as `Authorization: Bearer <key>` on every request.
- **Chat tab** — Claude-style conversation UI: messages grow upward, input bar pinned at bottom. Enter sends, Shift+Enter for newlines. Sends last 3 exchanges as `history` for context. Auto-scrolls to latest message. Clear button resets thread.
- **Memory tab** — lists all non-expired notes with category color badges. Filter input + refresh button. Delete (×) per note.
- **Sessions sidebar** — right panel, sorted by most recent, workspace hash + time ago + size.
- `ErrorBoundary` wraps `<App/>` in `main.jsx` — render crashes show a reload button.
