# GHOST — Handoff Document

> Written 2026-04-22 by Claude (Opus 4.7) for Isaac before his API key runs out.
> Read this first. It consolidates everything a new Claude needs to pick up work on GHOST
> without asking Isaac 30 questions. Pair it with `VISION.md`, `ARCHITECTURE.md`,
> `PIPELINE.md`, and `CLAUDE.md` in this repo.

---

## 0. Who Isaac is (read before you say anything)

- **Isaac Carrillo Ojeda**, 15, dual-enrollment student in Greeley, CO.
- Founder of **KYNE Systems LLC**. Inventor of the **LATCH haptic actuator** (provisional patent filed).
- Freelancing a website for **RC Concrete** (his father's business).
- Has **aphantasia** — no mental imagery. Thinks in "feltspace" / physical intuition. Catches on fast.
- Legendary rank in CODM. Plays flute in marching band.
- Email: `isaaccarrilloojeda@gmail.com`. Works in Git Bash on Windows 11.

### How to talk to him
- **Short responses.** No bullet walls. One clear action at a time.
- **Copy-paste ready commands always.** Never "try something like..." — give the exact line.
- Explain setup/tooling simply (like he's 8). Treat **architecture and system design** as peer conversation.
- **Don't repeat yourself.** Don't summarize what was just done.
- **Your role here is brainstormer and realist system designer, NOT code writer.** Isaac builds the code. You help design the architecture, surface tradeoffs, think through logistics.
- **Clarify before coding on non-trivial features.** When scope/semantics/data-model have more than ~2 open design questions, ask a tight numbered list (<5) with a "my gut" default for each, wait for answers, restate the plan in a few lines, get explicit go-ahead before writing code. Isaac has called this out as the right default — guess-and-refactor wastes his time.
- **Why the self-hosted fallback matters:** Isaac's Claude Max 5x subscription may get deactivated due to age. His primary Anthropic API key is expiring soon (this is why this document exists). GHOST needs to survive any single account issue — API key separation, provider routing, OpenRouter fallback are not optional niceties.

---

## 1. What GHOST is

A **personal AI operating system**. SMS as primary mobile interface, web dashboard as full control panel. Always on, always his.

- Text GHOST → a real AI system executes it. Code, email, calendar, research, chat, law.
- All server-side. Nothing depends on Isaac's local machine.
- Survives account issues via API key separation + provider routing (Anthropic ↔ OpenRouter ↔ DeepSeek ↔ etc.).

**Core pattern:**
- **Director AI** — persistent brain. Routes requests. Maintains memory. Default Claude Sonnet 4.6, fallback GPT-4o, hard error beyond that.
- **Specialist Agents** — ephemeral workers spawned by the Director. Each handles one domain. Report back to Director, then terminate.
- **Chat Dispatcher** — lightweight path for no-prefix messages. Skips full Director overhead. Injects core context file + semantic search from categorized `director_notes`. Fast, cheap.

---

## 2. Where GHOST actually lives (files + infra)

### Code
- Repo root: `c:/claw-code/claw-code/` (nested clone — left as-is, not worth fixing).
- Runtime: Rust binary `claw` in `rust/crates/rusty-claude-cli/src/`.
- Dashboard: React 18 + Vite in `dashboard/` (primarily `dashboard/src/App.jsx` + panels under `dashboard/src/panels/`).
- Docs: `VISION.md`, `ARCHITECTURE.md`, `ROADMAP.md`, `PIPELINE.md`, `PHILOSOPHY.md`, `CLAUDE.md`, `README.md`, `USAGE.md`, `WORKER-PROMPT.md`, `PARITY.md`.
- Scripts: `scripts/` (PowerShell helpers).
- Migrations: `rust/migrations/` — numbered SQL files, run automatically via `sqlx::migrate!()` on daemon startup.

### Deployments
- **Daemon (production):** Railway → `https://brave-cat-production-dd8e.up.railway.app`
- **Dashboard (production):** Vercel → `claw-code-gules.vercel.app` (plus two preview URLs)
- **DB:** Railway-managed Postgres with **pgvector** (1536-dim embeddings, enabled in migration `001_initial.sql`)
- **GitHub:** `https://github.com/IsaacCarrilloOjeda/claw-code`

### SMS stack
- **Android SMS Gateway** v1.20.0 on Isaac's S25+.
- Local API via Cloudflare tunnel: `https://sms.kynesystems.com/api/v1/message` → `http://192.168.0.132:8080` (S25+ LAN IP — update tunnel if it changes).
- Local creds: user `sms`, password stored in Railway env.
- Cloud API creds (webhook management only): user `PYNBEN`, pw `gjjdqxqareezv5`.
- Registered webhook ID: `3YHH7fWMWX5mW6BMzlNVT` (re-registered 2026-04-17 to clear backoff). Points to `https://brave-cat-production-dd8e.up.railway.app/sms/inbound`.
- **Twilio (backup):** account `isaac@kynesystems.com`, `TWILIO_FROM_NUMBER=+18336283910` (trial), `TWILIO_ACCOUNT_SID=AC0527a48d95141bed85c8ad05043e2e50`. Toll-free verification SID `HH3e12d2c7ce79a53529bbc36dac0a0dc9` pending. Trial can only SMS verified caller IDs.

### Railway env vars (required)
| Var | Purpose |
|-----|---------|
| `ANTHROPIC_API_KEY` | All Anthropic calls (chat, extraction, memory embedding classification) |
| `OPENAI_API_KEY` | **OpenRouter key** (NOT OpenAI) — used for provider routing. OpenRouter doesn't support embeddings — do NOT use it for embeddings. |
| `VOYAGE_API_KEY` | Voyage AI embeddings (`voyage-3`, 1536 dims). Required for semantic memory search. |
| `GHOST_DAEMON_KEY` | Bearer auth for `/chat`, `/prompt`, `/director/config`, `DELETE /memories/:id`, most SMS endpoints |
| `HOST` | `0.0.0.0` (Railway auto-injects) |
| `PORT` | Auto-injected by Railway |
| `DATABASE_URL` | Auto-injected by Railway Postgres add-on |
| `GHOST_CORE_CONTEXT_PATH` | Path to GHOST's personality/system context file (`/app/ghost-context.txt` in container) |
| `GHOST_ALLOWED_NUMBERS` | Comma-separated E.164 numbers allowed to SMS in |
| `GHOST_SMS_GATEWAY_URL` | `https://sms.kynesystems.com/api/v1/message` |
| `GHOST_BASE_URL` | Public URL of the daemon |

---

## 3. Status by phase

### ✅ Phase 0 — Foundation (COMPLETE)
Railway deploy, Postgres + pgvector, jobs/director_config/director_notes tables, circuit breaker reset task.

### ✅ Phase 1 — SMS Loop (COMPLETE)
SMS in/out via Android Gateway + Twilio, chat dispatcher (Haiku), Director stub (Sonnet), core context injection, prefix routing (`.` ignore, `!` force-director, `?` research stub, `>name` scheduled, none = chat dispatcher).

### ✅ Phase 2 — Memory + Context (COMPLETE)
`director_notes` with 1536-dim embeddings, Voyage AI primary / OpenAI fallback, fire-and-forget note extraction after every chat response, semantic top-5 injection on every inbound, confidence decay every 24h (5% daily, soft-delete below 0.1), memory panel in dashboard.

### 🟡 Phase 3 — Specialist Agents (IN PROGRESS — current phase)
- **Done:** Brave Search wired into `chat_dispatcher` (`search.rs`); Dreamer prefix (`~`) registered; chief_of_staff agent with Sonnet self-correction; agent dispatcher foundation; SMS phases 4–6 (shareable facts box, interactive replies, quick replies, unread indicators, contact notes, conversation summary, SMS identity + outbound guard + auto-ack, event-driven availability slots A/B/C + sleep mode).
- **Done (infra waves):** Phase A cost infra — provider router, `settings_kv` table, `coder_spend` ledger, token stream SSE, kill switch. Phase B+C — agents, tools, file index, templates, coder endpoints. Dashboard: pinned-chat bar, Code nav tab + CoderPanel skeleton, diff review + Apply/Reject wiring, TokenMeter, Settings page rebuild, coder chat-kind badges.
- **Not done (designed, not built):** Email Agent (Gmail OAuth), Calendar Agent (Google Cal), dedicated Research Agent tab (search is wired to chat dispatcher, not a standalone tab), Citations `[1]` formatting, ntfy.sh push notifications on `#notify`.

### 🔜 Phase 4 — Code Agent + IT Guide + Law Agent
Code Agent scaffolding is partially landed via the A/B/C/D coder plan (see §6). IT Guide and Law Agent not started.

### Phase 5 — (reserved / dashboard polish)

### Phase 6 — Automations
Cron job system, morning brief, evening close, proactive monitoring (email + calendar urgency scoring 0–1), Distill.io web monitoring.

### Phase 7 — Voice + Style
Android wake word (Porcupine on-device + Whisper), shadow mode (incoming message triage → AI drafts → `y` to send), voice toggle (reactive vs voice mode), writing-style training from sent emails/messages.

### Phase 8 — Advanced
Swappable/transferrable Director with full memory export/import, self-training Director (every approve/reject/edit = labeled training data), fine-tune Mistral 7B / Llama 3.1 8B via Together AI, local Director on Hetzner VPS, GitHub repo watcher, semantic Director auto-selection.

---

## 4. Locked design decisions — do not re-debate

From `CLAUDE.md` and `VISION.md` (locked 2026-04-20 and earlier):

- **Agents live inline** at `rust/crates/rusty-claude-cli/src/agents/`. Not a sibling crate.
- **`director.rs` is a thin facade** over `agents/dispatcher.rs`. The `!` prefix contract stays; real routing lives in the dispatcher.
- **Prompt caching ships before any new agent work** — `cache_control` markers on the stable prefix (core context + personality + Gerald overview) in `chat_dispatcher.rs` and `director.rs`.
- **Prefix parsing consolidates into `agents/intent.rs`** on the first agent PR — not scattered across `daemon.rs` and `chat_dispatcher.rs`.
- **Budget caps are per-agent per-day.** Hard cap, not soft. Blown budget → fall back or refuse.
- **Dreamer runs on cron** (MVP). No always-on loops anywhere.
- **`ghost_events` writes to Postgres every turn, mirrors weekly-condensed to Gerald.**
- **Transports: SMS + dashboard only.** Telegram (if ever added) sits alongside SMS, not a replacement — SMS-specific state (contacts, auto-reply, loadbearing) stays on the SMS path.
- **Memory architecture:** one `director_notes` table, `agent_tags` array column per note. Notes tagged by agent relevance (e.g. `['law', 'research']`). Master context (name, age, timezone) lives in the core context file, not notes. Semantic search filters by agent tag + `general` on read.
- **Director fallback:** #1 Sonnet → #2 GPT-4o → hard error. No chains beyond two.
- **Verification:** lowercase `y` only. No complex confirm flows.
- **Email send / calendar delete / phone interaction** always require `y`. **Calendar create/edit, code push** do not.
- **"." prefix ignored** — GHOST returns 200 but sends no reply (useful for testing webhook delivery without triggering AI).
- **No tool-based routing.** Director spawns agents, agents are ephemeral.
- **Citations** are `[1]` inline + numbered sources list. Triggered by the word "source" anywhere in prompt OR by Law Agent (always cites). Dashboard only — too verbose for SMS.
- **Notifications:** `#notify` anywhere in a prompt flags the task → push via **ntfy.sh** (not SMS — Twilio unreliable). SMS fallback only if ntfy fails.
- **Brainstorm is a mode modifier, not an agent.** If Isaac says "brainstorm" anywhere in a message, active agents receive brainstorm context (rabbit-hole thinking, tangents, deep dives).
- **Multi-agent:** if Echo + specialist(s) are both toggled, Echo acts as Director, a Judge agent merges specialist outputs into one combined reply.
- **Default agent when nothing is selected:** Echo.
- **Every approve/reject/edit in the UI is training data infrastructure** for the Phase 8 self-training Director. Design data capture from day one.

---

## 5. Cost-Intelligence Pipeline (PIPELINE.md, designed 2026-04-16)

Isaac has no local GPU; API cost is the primary constraint. The system must get cheaper over time, not stay flat. Phases A–F, built in order. Each phase compounds on the previous.

| Phase | What | Why first / why here |
|-------|------|----------------------|
| **A** | **Cascade routing** (Haiku → Sonnet → Opus, heuristic confidence) + filler stripping | Highest immediate savings (~60–70% on simple queries), zero new infra, simplest. |
| **B** | **Intake model** — cheap model asks clarifying Qs before expensive calls | Prevents wasted expensive calls on ambiguous requests. |
| **C** | **Scholar DB** — cache solutions, track failed attempts, compound savings over time | Repeated tasks get near-free. |
| **D** | **Orchestrator-worker dispatch** — Opus plans, mini executes in parallel, checkpoints verify | Big tasks get fan-out; planning is expensive but happens once. |
| **E** | **Code macros + prompt compression** | Shrinks the per-call token footprint across everything. |
| **F** | **Speculative parallel execution + token recycling** | Latency and cost wins on branches that would have retried. |

**Worker prompt for Phase A** is in `WORKER-PROMPT.md`. Design doc: `PIPELINE.md` in repo root.

Any work on GHOST's chat dispatch path MUST be aware of this pipeline — don't build anything that conflicts with the cascade or orchestrator-worker pattern.

---

## 6. Coding Agent plan (A/B/C/D) — decided 2026-04-21

Four standalone prompt files on disk at `C:\Users\carro\.claude\plans\`:

- `apply-all-the-things-abundant-russell.md` — master plan with context + REUSE MAP
- `prompt-A.md` — cost infra (provider router, settings_kv, budget ledger, live token SSE, summarize-as-you-go, kill switch)
- `prompt-B.md` — three agents (brainstorm / coder / orchestrator) + tool system + daemon endpoints
- `prompt-C.md` — semantic file index + 6 templates + git hook / watcher / manual refresh
- `prompt-D.md` — dashboard Code tab + Settings tab + pinned-chat bar + diff review + live token meter

Order: **A first, B and C in parallel, D last.** Each prompt is meant for a fresh Claude Code chat.

### Why this plan exists
Isaac's Claude Max subscription may be revoked → API token cost becomes the bottleneck for building GHOST. The coding agent is cheap-by-default (DeepSeek via OpenRouter) with Anthropic as an opt-in premium lane. Simultaneously, it accelerates GHOST development itself — the brainstorm → orchestrator → coder pipeline "launders" Isaac's rambling specs through cheap models into clean prompts before the expensive coder runs.

### Locked decisions (do NOT re-debate unless Isaac re-opens them)
- Provider toggle is **per-agent** (global default + per-agent override), stored in `settings_kv` Postgres table.
- **Diff-apply only; no full-file writes.** Approve-always default with auto-apply toggle in Settings.
- **Semantic file search** (whole-file + signature summary embeddings), NOT RAG-over-chunks.
- Templates first for boilerplate (6 scaffolds), RAG-over-past-code is a Phase-2 optional.
- **Orchestrator-worker for big tasks:** orchestrator fragments spec → ≤5 tasks → parallel coder workers via JoinSet, fresh conversations each, no shared context.
- **Fallback chain:** DeepSeek → Haiku → Sonnet → MiMo → refuse. **Budget exhaustion is NOT a fallback trigger.**
- **$2/day hard cap** on coder, configurable from Settings. In-flight tasks finish; next task blocks.
- **Kill switch:** `GHOST_CODING_AGENT=off` env var. Surfaced at unauthenticated `/code/health`.
- **Tool scope:** read / grep / list_dir / diff / cargo_check / cargo_test / cargo_fmt. No arbitrary bash, no network tools, no writes outside canonicalized `repo_root`.
- **Dashboard:** Code tab next to SMS; pinned-chat bar holds one main + one code chat for instant switching; Settings tab rebuilt from stub into full page.

### Reuse map (~70% of scaffolding already exists)
Agent trait, dispatcher, budget cap entry `"code": 500` (needs change → 200 = $2/day per Prompt A), events infra, `infra/cache.rs`, embeddings in `memory.rs`, daemon routing, `SmsPanel.jsx` as panel template.

### Status of coder build (from recent commits)
- `14251cc` — Phase A cost infra landed.
- `dd5f498` — Phase B+C landed (agents, tools, file index, templates, endpoints).
- `947061b` → `5765721` — Phase D dashboard landed (pinned-chat, Code tab, CoderPanel skeleton, `/code/files/search`, diff review, Apply/Reject, TokenMeter, Settings rebuild, chat-kind badges).

**→ The coder agent plan is largely LANDED as of 2026-04-22.** Verify current state before recommending next steps.

---

## 7. SMS system — the most production-tested piece

Phases 1–6 are live. Detailed in `CLAUDE.md`. Key features to preserve:

- **Conversation history cap:** `POST /chat` accepts optional `history: [{role, content}]`. Daemon caps at 10 entries (5 exchanges). Each content field capped at 8192 chars. SMS path loads last 10 messages + any `loadbearing=TRUE` messages.
- **Loadbearing messages:** messages producing memory notes during extraction are auto-flagged in `sms_history`. Always included in SMS context regardless of recency. Migration `007_sms_loadbearing.sql`.
- **SMS auto-reply gate:** `sms_contacts.auto_reply` defaults `FALSE`. Inbound SMS from allowed numbers always stored, but GHOST only replies if `auto_reply=TRUE` for that contact. Migration `008_sms_contacts.sql`.
- **Schedule context injection:** `sms_schedule` stores daily + persistent entries. `load_schedule_context()` formats them into the system prompt (after sender identity, before memory notes) so GHOST knows Isaac's availability.
- **Availability slots A/B/C (migration `017_sms_schedules.sql`):** each contact assigned to one slot or none. Slot has 0..N windows (weekly-recurring weekday bitmask, or one-off date). While "now" in America/Denver is inside any window of a contact's slot → `auto_reply=TRUE`. Outside → `FALSE`. Evaluated every 60s in `db::tick_schedule`.
- **Sleep mode:** manual toggle, "SLEEP NOW" button with awake-by HH:MM in Mountain Time. While active, every contact in `sms_sleep_contacts` gets `auto_reply=TRUE` regardless of slot. Auto-clears when `now() >= awake_by`. **Sleep always wins over slot scheduling.**
- **Manual override semantics:** toggling a contact's auto-reply via UI clears `schedule_slot=NULL` — manual toggles opt the contact OUT of the scheduler until reassigned. No fight between human intent and tick task.
- **Weekday bitmask matches Postgres `EXTRACT(DOW)`:** Sun=bit0 … Sat=bit6. Mon–Fri = `0b0111110 = 62`.
- **All timezone math in Postgres** (`AT TIME ZONE 'America/Denver'`). No `chrono-tz` dependency. DST handled correctly.

---

## 8. Bible Agent (landed, live)

Files: `bible.rs`, `bible_ingest.rs`, Bible queries in `db.rs`. Migration `002_bible.sql`.

- Classification: `Reference` / `WordStudy` / `Topical` / `NotBible`.
- Trigger: prefix `bible:` forces Bible mode. Otherwise auto-classified.
- CLI: `claw bible-ingest [--data-dir path]` reads verse-aligned JSON/TSV from `.ghost/bible-data/`, batch-embeds via Voyage, bulk-inserts. Required: `DATABASE_URL`, `VOYAGE_API_KEY` (or `OPENAI_API_KEY`).
- Tables: `bible_verses` (embeddings + Strong's + morphology), `bible_pericopes`, `bible_cross_refs`, `bible_lexicon`.
- Endpoints: `/bible/stats`, `/bible/verse/:book/:ch/:v`, `/bible/range/...`, `/bible/search?q=...`, `/bible/strongs/:id`, `/bible/crossrefs/...`.

---

## 9. Memory system (Phase 2 — COMPLETE)

Fire-and-forget `extract_and_store` via `tokio::spawn` on every chat response. Haiku extracts 0–3 factual notes in `category|content` format → embedded → stored in `director_notes`.

On every inbound chat: message embedded → top-5 notes retrieved (`embedding <=> $1::vector`) → injected into system prompt under `## What you remember about Isaac`.

- Embedding providers checked in order: **Voyage** (`voyage-3`, 1536 dims via `output_dimension`) → **OpenAI** (`text-embedding-3-small`, 1536 dims) → NULL (stored, visible in panel, no semantic ranking).
- Categories: `personal`, `social`, `code`, `projects`, `style`, `calendar`.
- **Confidence decay:** notes > 30 days old → `confidence * 0.95` daily. Below 0.1 → expired (soft-delete via `expires_at`).
- Background task: `db::decay_notes_confidence` every 24h.

---

## 10. Dashboard

React 18 + Vite. Main component `dashboard/src/App.jsx`, panels under `dashboard/src/panels/`.

```
cd dashboard && npm run dev   # http://localhost:5173
```

Dev server proxies nothing — daemon must already be running on 7878.

- Auth: `localStorage['ghost-daemon-key']`, sent as `Authorization: Bearer <key>` on every request.
- Env: `VITE_DAEMON_URL` (falls back to `http://127.0.0.1:7878`).
- All agent responses stored as jobs in Postgres. Dashboard reads `GET /jobs`, `GET /jobs/:id`.

**UI structure (locked, partly built):**
- Top bar: green/red dot, `GHOST`, uptime, `|`, then browser-style chat tabs (green=open, blue=active, red=unread).
- Left sidebar: collapsible. Top scrollable — project/chat tree. Bottom fixed — Settings, Statistics, About.
- Per-chat top tabs: **Chat / Preview / Context / Thinking**.
- **Preview by agent:** Echo blank · Research formatted-results · Email draft-cards · Calendar timeline · Code live-terminal · IT Guide step-map · Law citations.
- **Context tab:** tools the active agent has (used=blue, planned=orange), core context card at top, injected memories/files below.
- **Agent toggles above input:** Echo · Research · Email · Calendar · Code · IT Guide · Law. Brainstorm is a mode modifier, not a toggle.
- **Job status banner:** below top bar when job running, dismissible per-job, toggle-off-globally in Settings.

---

## 11. Daemon endpoints quick reference

Full table in `CLAUDE.md`. Highlights:

- Public-ish: `/health`, `/status`, `/sessions`, `/jobs`, `/jobs/:id`, `/director/config` (GET), `/memories`, `/bible/*`.
- Bearer required: `/chat`, `/prompt` (+ flag + key), `/director/config` (POST), `/sms/send`, `/sms/contacts*`, `/sms/history/*`, `/schedule*`, `/sms/availability/*`, `/sms/sleep*`, `DELETE /memories/:id`.
- Open (for SMS gateways/Twilio): `/sms/inbound`.

### `/prompt` security model (fails closed)
`/prompt` shells out with `--dangerously-skip-permissions`. Hardened three ways:
1. Daemon started with `--allow-unsafe-prompt` or `/prompt` → 403.
2. `GHOST_DAEMON_KEY` must be non-empty at startup (empty = refuse to start).
3. Each request needs `Authorization: Bearer <key>` or `X-Claw-Key: <key>` (constant-time compare). Missing/wrong → 401.

Plus: `Host` header validated (DNS-rebinding defense → 421, bypassed on `0.0.0.0`). Body cap 1 MiB. `model` allow-list charset. Prompts starting with `--` rejected. stdout/stderr piped through `redact_secrets` (API keys → `***redacted***`).

### Background tasks at daemon startup
1. Circuit-breaker reset — every 5 min, restores `primary_healthy` / `fallback_healthy`.
2. Confidence decay — every 24h.
3. **SMS schedule tick — every 60s.** Evaluates slot windows + sleep mode against America/Denver, flips `sms_contacts.auto_reply`. Auto-clears sleep when `now() >= awake_by`.

### Phone / network access (one-liner)
```bash
GHOST_DAEMON_KEY=<token> claw daemon --host 0.0.0.0 --port 7878 --allow-unsafe-prompt
```
Clients send `Authorization: Bearer <token>` or `X-Claw-Key: <token>`.

### Task Scheduler (Windows, one-time, as admin)
```powershell
.\scripts\daemon-install.ps1
```
Creates `ClawDaemon` task: starts at boot, runs as current user, restarts on failure (3× / 1 min).

---

## 12. Prefix command language

| Prefix | Behavior |
|--------|----------|
| (none) | Chat dispatcher — fast, semantic context, no agent spawning |
| `!` | Force Director + agents (override simple-task logic) |
| `?` | Research only, no write actions |
| `>name` | Run a named scheduled task immediately |
| `.` | **Ignored** — 200 OK, no reply (testing webhook delivery without triggering AI) |
| `~` | Dreamer (registered 2026-04-21) |
| `->` | IT Guide (Phase 4) |

---

## 13. Known gotchas (do not relitigate)

- **API key env:** `ANTHROPIC_API_KEY` must be set in the terminal that starts the daemon. Subprocess inherits from there, NOT Task Scheduler or admin shells.
- **Git Bash vs PowerShell:** Isaac uses Git Bash (`/c/` paths). PowerShell scripts need `C:\`. Never mix them in instructions.
- **`claw.exe` file lock:** kill running `claw.exe` before rebuilding (`Stop-Process -Name claw -Force`) or cargo fails to replace the binary.
- **CORS for POST:** `Allow-Origin: *` alone isn't enough — browsers also require `Allow-Methods` and `Allow-Headers`. Daemon uses an explicit allow-list (not wildcard) + sends all three.
- **Chrome Private Network Access:** Chrome 104+ blocks `http://localhost:5173` → `http://127.0.0.1:7878` unless server echoes `Access-Control-Allow-Private-Network: true`. Symptom: polls succeed at TCP, fetch throws `TypeError: Failed to fetch`, UI shows "daemon unreachable". Fix is in `write_response` in `daemon.rs`.
- **CORS allow-list covers all loopback forms:** `http://localhost:5173`, `http://127.0.0.1:5173`, `http://[::1]:5173` — all three. Extra origins via `GHOST_DAEMON_CORS_ORIGIN` (comma-separated, renamed from `CLAW_DAEMON_CORS_ORIGIN` in Phase 0).
- **Em-dashes in PowerShell scripts:** write `.ps1` files ASCII-only; `—` corrupts on write and causes parse errors.
- **Inline comments on backtick continuations (PS):** backtick must be the very last character on the line.
- **Gerald Brain cold starts:** Render.com server sleeps; 4s timeout is the right call. Don't raise it.
- **Task Scheduler env:** scheduled tasks don't inherit user shell env vars; store `ANTHROPIC_API_KEY` in `~/.claw/settings.json` under `anthropicApiKey` for headless operation.
- **sqlx offline mode:** `SQLX_OFFLINE=true` in the Dockerfile — build doesn't need a DB. Migrations run at daemon startup via embedded `sqlx::migrate!()`. No `.sqlx/` cache dir needed.
- **pgvector on Railway:** `CREATE EXTENSION IF NOT EXISTS vector;` in migration 001. Railway's managed Postgres has pgvector available.
- **`OPENAI_API_KEY` is OpenRouter here.** OpenRouter does NOT support the embeddings endpoint. Use `VOYAGE_API_KEY` instead.
- **Embeddings fallback:** Voyage → OpenAI → NULL. NULL notes appear in memory panel but don't rank in semantic search.
- **Gerald Brain search bug:** was iterating a dict instead of `dict["results"]`. Fixed.
- **CheetahClaws Gerald MCP:** `'str' object has no attribute 'get'` — fixed via `_ok()`/`_err()` helpers returning `list[TextContent]` and `/search` unwrapping.
- **Claw Code MCP transport:** Rust MCP manager only supports **stdio** (no SSE, no HTTP). CheetahClaws Python has SSE. That's why CheetahClaws is the agent layer, not Claw Code.
- **Nested clone** at `/c/claw-code/claw-code`: left as-is, not worth fixing.
- **Sandbox warning:** Linux-only, never applies on Windows. Ignore forever.
- **Three pre-existing Rust test failures** (`build_runtime_plugin_state_discovers_mcp_tools`, `build_runtime_runs_plugin_lifecycle_init_and_shutdown`, `parses_direct_agents_mcp_and_skills_slash_commands`) — separate cleanup, don't let them block unrelated work.
- **Full-workspace clippy blocked on Windows** by Unix-only code in `runtime` crate (`std::os::unix::fs::PermissionsExt`, `set_mode(0o755)` without `#[cfg(unix)]` gates in `mcp_stdio.rs`, `mcp_tool_bridge.rs`, `file_ops.rs`, `tests/mock_parity_harness.rs`). Scope clippy/tests per CLAUDE.md instead: `cargo clippy -p rusty-claude-cli -p plugins --bins -- -D warnings` + `cargo test -p rusty-claude-cli --bins`.

---

## 14. Provider routing (existing, opt-in)

`rust/crates/rusty-claude-cli/src/routing.rs`. Opt-in via `CLAW_ROUTING=1`. One-shot `claw prompt "..."` only (not REPL).

| Tier | Model | Trigger |
|------|-------|---------|
| fast | `gpt-4o-mini` | ≤20 words, no code/arch signals, `OPENAI_API_KEY` set |
| code | `deepseek-chat` | code signals present, no arch signals, `OPENAI_API_KEY` set |
| mid | `claude-sonnet-4-6` | arch/design/review signals, or fallback |
| full | `claude-opus-4-6` | default |

`OPENAI_API_KEY` points at OpenRouter, not real OpenAI. DeepSeek via OpenAI-compatible API. If `OPENAI_API_KEY` absent, fast/code tiers fall through to mid/full.

---

## 15. Gerald Brain (optional enrichment)

`rust/crates/rusty-claude-cli/src/gerald.rs`. URL from `~/.claw/settings.json` → `mcpServers.gerald-brain.url`. Fallback `https://gerald-core-1.onrender.com/messages`. **4-second timeout** (Render cold starts are slow; failures are silent).

- Session start: `load_context()` calls `get_overview` via MCP HTTP, injects as last system-prompt section. Skipped silently if unreachable.
- Session end: `save_session()` fires on `/exit` or Ctrl-C. Stores workdir, session ID, turn count, model used.
- Protocol: `initialize` → `notifications/initialized` → `tools/call`. `Mcp-Session-Id` header passed when returned.

---

## 16. Verification checklist (per CLAUDE.md)

From `rust/`:
```bash
cargo fmt
cargo clippy -p rusty-claude-cli -p plugins --bins -- -D warnings
cargo test -p rusty-claude-cli --bins
```

`src/` and `tests/` are both present; update both surfaces together when behavior changes.

---

## 17. Estimated monthly costs (for intuition)

Target: **~$25/mo normal use, ~$53/mo heavy.** Railway $5–10, Postgres included→$5, Sonnet Director $3–15, Haiku specialists $1–6, DeepSeek code $0.50–3, GPT-4o fallback rare $0.50–3, E2B sandboxes $0–3, Whisper voice $0.25–1.50, Brave/Tavily search $0–3. SMS via Android Gateway: **$0**.

One-time: domain $10–12/yr, Together AI fine-tune run (Phase 8) $30–50.

---

## 18. North-star features (design with these in mind)

- **Morning brief** (calendar + email + tasks + overnight flags → 5-line SMS).
- **Evening close** (what got done, what didn't, what to think about).
- **Swappable Director** — full memory export/import between models, portable across providers.
- **Self-training Director** — every approve/reject/edit = labeled training data → eventually fine-tuned local model on Hetzner VPS, zero per-token routing cost.
- **Shadow mode** — incoming messages AI-triaged (spam skipped, human senders → draft), Isaac sends with `y`.
- **Two-sided AI style** — reactive mode (AI adapts to Isaac) vs voice mode (AI sounds like Isaac), voice used for drafting only.

---

## 19. What to do first in a new Claude session

1. **Read this file top-to-bottom.** Then skim `VISION.md` + `ARCHITECTURE.md` + `CLAUDE.md`.
2. **Check current state before recommending** — memories in this doc are a point-in-time snapshot. Run `git log --oneline -20` and `git status`. Grep for any file/function this doc names before asserting it exists.
3. **Match Isaac's style:** short, peer on architecture, explain tooling like he's 8, copy-paste commands, no summaries of what was just done.
4. **Clarify before building.** For any non-trivial feature: tight numbered Q list with your "gut" default per Q, wait for answers, restate the plan, get explicit go-ahead, then build.
5. **Don't propose tool-based routing.** Pattern is Director → specialist agents.
6. **Don't dismiss Phase 8 self-training Director.** It's intentional and high-priority long-term.
7. **Flag high-risk, high-reward ideas clearly but propose them.** Isaac is 15, runs a real LLC, thinks at a systems level. He can handle tradeoffs — don't water them down.

---

## 20. Where Isaac wants to take GHOST (direction, not tasks)

- **GHOST becomes Isaac's daily OS.** SMS + dashboard covers everything he currently context-switches between (email, calendar, code, research, law, IT troubleshooting, KYNE business tasks, RC Concrete freelance work).
- **Cost-intelligent by construction.** The A–F pipeline isn't a nice-to-have — it's the only way GHOST survives if his Claude Max is revoked. Every new feature should default to the cheapest provider that works and escalate only on confidence failure.
- **Local-eventual.** Phase 8 moves the Director off paid APIs entirely — Hetzner VPS + fine-tuned Mistral/Llama, funded by the accumulated approve/reject decision data. GHOST's UI should already be capturing training data today.
- **SMS-first, dashboard-deep.** SMS is the "always available from anywhere" lane. Dashboard is the "full feature depth, all the knobs" lane. They're never allowed to diverge in features that matter — only in presentation (SMS short/no citations, dashboard verbose/cited).
- **Survives Isaac being 15.** If an account gets deactivated, GHOST keeps working via a fallback provider. This is why API key separation and provider routing are structural, not cosmetic.
- **Every approve/reject is training data.** The whole UI is secretly a labeling system for a future local Director.

---

## 21. One-line reminders for the next Claude

- You're a **brainstormer and realist system designer**, not a code writer. Isaac builds.
- **Short. Tight. Peer on architecture. Childproof on setup. No repetition. No summaries.**
- **Ask clarifying Qs before non-trivial builds.** Numbered list, <5 at a time, "my gut" per Q, wait, restate, confirm, then build.
- **Check current code before citing file:line from memory.** Files rename, functions move, assumptions rot.
- **Don't re-debate locked decisions** (§4, §6).
- **The Rust daemon is the chassis everything attaches to.** Don't propose rebuilding from scratch.
