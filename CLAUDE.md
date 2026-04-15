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
| GET | `/sessions` | open on localhost | Workspace + file basenames (no absolute paths — info-disclosure fix). |
| GET | `/jobs` | open | List recent 100 jobs from Postgres. 503 if DB not configured. |
| GET | `/jobs/:id` | open | Single job detail by UUID. |
| GET | `/director/config` | open | Current primary/fallback model + health flags. |
| POST | `/director/config` | **bearer** | Swap primary or fallback model. Body: `{"primary_model":"...","fallback_model":"..."}`. |
| POST | `/prompt` | **flag + key + bearer** | Runs a one-shot prompt via `claw prompt` subprocess. Body: `{"prompt":"...","model":"..."}` |

### `/prompt` security model (mandatory — fails closed)
`POST /prompt` shells out to a subprocess with `--dangerously-skip-permissions`, so it is hardened three ways:
1. Daemon must be started with `--allow-unsafe-prompt` — otherwise `/prompt` returns `403`.
2. `GHOST_DAEMON_KEY` env var must be set to a **non-empty** value when the daemon starts; empty = refuse to start.
3. Each request must carry `Authorization: Bearer <key>` or `X-Claw-Key: <key>` matching `GHOST_DAEMON_KEY` (constant-time compare). Missing/wrong → `401`.

Additional hardening in the same path:
- `Host` header is validated against the bind address (DNS-rebinding defense → `421`). Bypassed when binding `0.0.0.0` (Railway/cloud).
- Request body is capped at 1 MiB (→ `413`) and read in a bounded loop, not a single 8 KiB read.
- `model` field is validated against an allow-list charset; prompts starting with `--` are rejected (CLI-flag injection).
- Subprocess stdout/stderr is piped through `redact_secrets` so `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` / `OPENAI_API_KEY` / settings-file key values are replaced with `***redacted***` before returning to the client.

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

DeepSeek is called via OpenAI-compatible API using `OPENAI_API_KEY`.  
If `OPENAI_API_KEY` is absent, fast/code tiers fall through to mid/full.

---

## GHOST — Current Project

This repo is being built into **GHOST**, a personal AI operating system.
Full vision, architecture, and roadmap: **`VISION.md`** (read this first for any non-trivial work).

**Current phase: Phase 0 — Foundation**
Goal: make the daemon cloud-deployable on Railway, add Postgres job model, wire Director config with fallback ranking. No new user-facing features yet.

### Phase 0 task list (in order)
1. Add `#[cfg(unix)]` gates to all `PermissionsExt` / `set_mode(0o755)` calls in `runtime` crate so the workspace compiles on Linux (required for Docker build).
2. Make daemon bind host/port from `HOST` + `PORT` env vars (Railway injects these). Currently hardcoded to `127.0.0.1:7878`.
3. Add Postgres connection via `DATABASE_URL` env var (Railway injects this). Use `sqlx` with compile-time checked queries.
4. Create migrations for three tables: `jobs`, `director_config`, `director_notes`. Schema is in `VISION.md` Phase 0 section. Enable `pgvector` extension in migration.
5. Seed `director_config` singleton row on startup if absent: `primary_model = 'claude-sonnet-4-6'`, `fallback_model = 'gpt-4o'`, both healthy.
6. Add circuit breaker logic: on Director call failure (429/402/500/timeout), flip `primary_healthy = false`, try fallback. Both fail → hard error. Background task resets health flags every 5 min.
7. Add new endpoints: `GET /jobs`, `GET /jobs/:id`, `GET /director/config`, `POST /director/config`.
8. Write `Dockerfile` (multi-stage Rust build → debian-slim, expose `$PORT`).
9. Update dashboard `DAEMON_URL` to read from an env var instead of hardcoded `http://127.0.0.1:7878`. Add Railway URL to CORS allow-list.
10. Verify: `POST /prompt` works end-to-end via the Railway URL.

### Railway deploy notes
- Add Postgres add-on in Railway dashboard → `DATABASE_URL` is auto-injected.
- Run `CREATE EXTENSION IF NOT EXISTS vector;` in first migration (pgvector for Phase 2 semantic search).
- Env vars needed at deploy: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GHOST_DAEMON_KEY`, `HOST=0.0.0.0`. `PORT` and `DATABASE_URL` are injected by Railway automatically.
- The daemon auth key env var is `GHOST_DAEMON_KEY` going forward (was `CLAW_DAEMON_KEY` — rename during Phase 0).

---

## Dashboard (`dashboard/`)

**Stack:** React 18 + Vite. Single-file component: `dashboard/src/App.jsx`.  
**Dev server:**
```bash
cd dashboard && npm run dev
# http://localhost:5173
```
**Daemon dependency:** expects daemon at `http://127.0.0.1:7878`. Shows "offline" gracefully if unreachable.

### Features
- Status bar — alive indicator, uptime, session count, version. Auto-polls every 10s with 5s per-request timeout.
- **KEY field** — header input persists to `localStorage['claw-daemon-key']` and is sent as `Authorization: Bearer <key>` on every daemon request. Must match the daemon's `CLAW_DAEMON_KEY` exactly for `/prompt` to succeed.
- Prompt tab — textarea → POST `/prompt` → output panel. Ctrl+Enter to send. In-flight requests are aborted on unmount / new-send via `AbortController`.
- Memory tab — placeholder for future Gerald Brain search (`GET /memories?q=...`).
- Session list — right sidebar, sorted by most recent, shows workspace hash + time ago + size. Stable keys (no `key={i}`).
- `ErrorBoundary` wraps `<App/>` in `main.jsx` — render-time crashes show a reload button instead of a blank screen.
