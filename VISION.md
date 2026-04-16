# GHOST — System Vision & Roadmap

> This file is the source of truth for what we are building and why.
> All Claudes working in this repo should read this before making suggestions.
> Claude's role here is brainstormer and realist system designer. Isaac builds the code.

---

## What We're Building

A personal AI operating system with SMS as the primary mobile interface.
Send a text → a real AI system executes it. Code, email, calendar, research, chat.
All from your phone or the web dashboard. Always on. Always yours.

---

## Confirmed Architecture

### Core Pattern
- **Director AI** — persistent brain. Routes all requests. Maintains your memory.
  Default model: Claude Sonnet 4.6.
  Fallback ranking: #1 Sonnet → #2 GPT-4o → hard error (manual switch, no infinite chains).
  Director is swappable before any model is configured — model is the engine, memory is yours.
- **Specialist Agents** — ephemeral workers spawned by the Director. Each handles one domain.
  Each can run a different model. They report back to the Director, then terminate.
- **Chat Dispatcher** — lightweight path for "." prefix messages.
  Skips full Director overhead. Always injects a core context file (safety net) + semantic search
  results from categorized Director notes. Fast, cheap, context-aware.

### Specialist Agents
| Agent | Trigger | Integration | Model |
|---|---|---|---|
| Research Agent | `?` prefix or Director routing | Brave Search + page reader | Haiku (standard) / Sonnet (deep) |
| Email Agent | natural language or Director routing | Gmail API (OAuth) | Sonnet or Haiku |
| Calendar Agent | natural language or Director routing | Google Calendar API | Haiku |
| Code Agent | natural language or Director routing | E2B sandbox + GitHub API | DeepSeek (cost) or Sonnet |
| IT Guide Agent | `->` prefix or strong contextual clues | Brave Search + screenshot reader | Sonnet |
| Law Agent | natural language or Director routing | Cornell Law + public legal DBs | Sonnet |

### Specialist Agent Design (locked decisions)

**Memory**
- One `director_notes` table, all agents share it.
- Every note has an `agent_tags` array column (e.g. `['research', 'general']`).
- On memory write: Haiku classifies which agent tags apply. A note about a court case gets `['law', 'research']`. A personal preference gets `['general']`.
- On memory read: each agent queries filtered by its own tag + `general`. Shared notes surface naturally via semantic search without duplication.
- Master context (Isaac's name, age, timezone, core facts) is injected into every agent always — never stored as notes, lives in the core context file.

**Citations**
- Format: inline `[1]` with a numbered sources list at the bottom of the response.
- Triggered when: (a) Isaac includes the word "source" anywhere in the prompt, or (b) it's the Law Agent (always cites).
- Citations only appear in the dashboard, never in SMS (too verbose).
- All agents support citations when triggered — Research, Law, IT Guide especially.

**Approval / confirmation**
- Email send: always requires `y` (dashboard or SMS both count).
- Calendar delete: always requires `y`.
- Calendar create/edit: no approval needed.
- Code push to GitHub: no approval needed (Isaac can revert via git).
- Deep research expansion: always asks before going deeper ("results are thin — want me to dig deeper?").
- Email drafts: show all found drafts simultaneously with brief summaries, approve each individually with `y [number]`.

**Notifications**
- `#notify` anywhere in a prompt (start or mid-conversation) flags the task.
- Delivery: push notification via ntfy.sh (not SMS — Twilio unreliable).
- Notification fires when the agent marks the job done.
- SMS fallback only if ntfy.sh fails.

**Agent thinking / reasoning display**
- Toggleable per-agent in Settings.
- When enabled: the agent's preview panel streams its reasoning ("searching for X... 3 results, but sources are thin... trying deeper search...").
- This is narrated reasoning, not raw chain-of-thought — agent writes thinking steps as it goes.
- When disabled: preview panel shows only the final output.

**Dashboard output tabs**
- One input bar shared across all agents.
- Each agent gets its own output tab with a specialized preview:
  - **Research**: formatted results with source links
  - **Email**: draft cards with approve/reject per draft
  - **Calendar**: timeline view of affected events
  - **Code**: live terminal streaming execution output
  - **IT Guide**: step map — full path laid out visually, current step highlighted
  - **Law**: formatted legal citations with statute/case refs
- Tab auto-switch on agent response: configurable in Settings (auto / manual / notify-only).
- `#notify` tasks send a push notification when done regardless of tab setting.

**About tab**
- Static page in dashboard. Contains:
  - Instructions for Isaac on how to use each agent
  - Base system prompts for every agent (readable, not editable from UI)
  - AI disclaimer (buried here, not on every response)
  - Current env var / integration status (which APIs are connected)

### Research Agent detail
- Standard path: Brave Search → summarize top results → return with sources if triggered.
- Deep path (user-requested or agent-initiated with approval): follows links, reads page content, synthesizes across sources.
- Deep path uses Sonnet instead of Haiku (more expensive, only when needed).
- Research memories tagged `['research', 'general']` for broadly useful facts, `['research']` only for domain-specific findings.

### Email Agent detail
- Gmail account: `isaac@kynesystems.com` by default. Toggleable to personal in Settings.
- Draft mode: triggered by "draft an email for X" or "find emails worth drafting to". Always waits for `y`.
- Send mode: triggered by "send an email to X about Y". Shows draft first, waits for `y`.
- Proactive drafting (Phase 6): scans inbox for emails worth replying to, surfaces summaries, drafts on request.
- OAuth token stored server-side in Railway env / secrets.

### Calendar Agent detail
- Read: always available, no approval.
- Create/edit: no approval (low stakes, easily undone).
- Delete: always requires `y`.
- Morning brief pulls from calendar only when Isaac asks — not automatic until Phase 6.

### Code Agent detail
- Writes code via DeepSeek, executes in E2B sandbox, returns live output to dashboard terminal.
- If execution fails: agent iterates (rewrites + reruns) up to 3 times before surfacing the error to Isaac.
- Finished scripts: pushed to a GitHub repo Isaac creates (`ghost-output` or similar), folder structure `/scripts/YYYY-MM-DD-task-name/`.
- Scripts also accessible from dashboard (link to GitHub + inline preview).
- No approval needed for push — Isaac can revert via git.

### IT Guide Agent detail
- Trigger: `->` prefix, or Director routes automatically if Isaac expresses frustration or confusion navigating something.
- Input: text description + optional screenshot upload (dashboard only until Twilio MMS is enabled).
- Preview panel: step map — full path to goal laid out at once, current step highlighted as Isaac progresses.
- Agent researches the specific site/service Isaac is navigating before generating steps.
- Screenshot reading: agent analyzes uploaded image to identify where Isaac is in the flow.
- One step at a time confirmation not required — full map shown upfront, Isaac works through it.

### Law Agent detail
- Scope: US law only.
- Sources: Cornell Law LII, court opinion databases, public statute repositories.
- Always cites — every legal claim gets `[1]` inline + source at bottom. No exceptions.
- Confidence: agent flags uncertainty explicitly ("this interpretation is contested" / "consult an attorney for binding advice") without a blanket disclaimer on every message.
- Disclaimer lives in the About tab only.
- Law memories tagged `['law']` — not shared with other agents unless the note is factual/general enough to warrant `['law', 'general']`.

### Director Memory System
Stored in Postgres. Model-agnostic — portable when Director is swapped.
Categories: `personal`, `social`, `code`, `projects`, `style`, `calendar`.
Notes have confidence decay — old notes get down-weighted, Director can refresh or expire them.
On "." prefix: core context file always loaded + semantic search pulls 3–5 most relevant notes.

### Input Interfaces
| Interface | Notes |
|---|---|
| SMS (Android SMS Gateway) | Real S25+ number, free, open-source app |
| SMS (backup provider) | Twilio deferred — age restriction. Will replace with TextBelt, email-to-SMS, or ntfy.sh. |
| Web dashboard | More features than SMS, always functional |
| Wake word app (Phase 7) | Porcupine (on-device) + Whisper, Android |

### Dashboard UI Design (locked — built in Phase 3)

**Stack:** React 18 + Vite. Replaces `dashboard/src/App.jsx` entirely.

#### Startup / Auth
- Centered key input on blank screen before anything loads
- Wrong key → inline "wrong key" text + red error banner
- Correct key → persisted to `localStorage['ghost-daemon-key']`, enter main UI

#### Overall Layout
```
┌─────────────────────────────────────────────────────────┐
│ ● GHOST  14d 3h uptime  │  [Chat tab] [Preview] [...]   │
├──────────┬──────────────────────────────────────────────┤
│          │                                              │
│ Sidebar  │              Main chat area                  │
│ (tree)   │                                              │
│          │                                              │
│──────────│                                              │
│ Settings │    [input bar with agent toggles above]      │
│ Stats    │                                              │
│ etc      │                                              │
└──────────┴──────────────────────────────────────────────┘
```

#### Top Bar (always visible, never hidden)
- Far left: green/red dot → `GHOST` → uptime
- Then a `|` divider
- Then open chat tabs (each is a browser-style tab): green=open, blue=active, red=unread message
- Tabs stay until closed with X

#### Left Sidebar
- Collapsible with a `<` / `>` button on its right edge
- **Top section (scrollable):** project/chat tree
  - Projects = full-width rectangles with editable names
  - Click project → uncollapse → reveals chats indented ~4 spaces below it
  - Click chat → opens in main area + spawns a top-bar tab (green)
  - Add/delete for both projects and chats. Delete = 2-click confirm.
- **Bottom section (fixed, ~1/4 height, draggable top border):**
  - Settings, Statistics, About, and future nav items
  - Always visible above the project tree as you scroll
  - Draggable border lets Isaac resize it (1/10 to 1/3 of sidebar)

#### Per-Chat: Top Tabs
Each open chat has four tabs across the top:
| Tab | Content |
|---|---|
| Chat | Main input/output thread |
| Preview | Agent-specific view (see below) |
| Context | Tools panel + injected memories/files |
| Thinking | Narrated reasoning stream, then final response |

**Preview tab by agent:**
- Echo / Brainstorm: blank / plain text
- Research: formatted results with source links
- Email: draft cards, approve/reject per draft
- Calendar: timeline view of affected events
- Code: live terminal streaming execution output
- IT Guide: step map — full path, current step highlighted
- Law: formatted citations with statute/case refs

**Context tab:**
- Shows all tools the active agent has available
- Used tools highlighted blue, planned-to-use highlighted orange
- Core context file always shown at top as a collapsed card
- Injected memories/files as expandable cards below

#### Agent Toggles (above input bar, collapsible row)
Agents:
- **Echo** — general chat, default if nothing selected. Routes via Director if multiple selected.
- **Research** — Brave search, `?` prefix or explicit toggle
- **Email** — Gmail, explicit toggle
- **Calendar** — Google Cal, explicit toggle
- **Code** — DeepSeek + E2B, explicit toggle
- **IT Guide** — `->` prefix or explicit toggle
- **Law** — explicit toggle

**Brainstorm** is not a toggle — it's a mode modifier. If Isaac says "brainstorm" anywhere in a message, the active agent(s) receive brainstorm context injected (rabbit hole thinking, explores tangents, goes deep). No separate agent.

**Multi-agent behavior:** If Echo + any specialist are both toggled, Echo acts as Director (picks which specialists to spawn). A Judge agent combines all specialist responses into one clean reply. Single chat thread, one combined response.

**Nothing selected:** defaults to Echo.

#### Job Status Banner
- Appears below top bar when a job is running
- Dismissible with X per-job
- Can be toggled off globally in Settings
- Shows: job type, agent, elapsed time, status

#### Notes for future Claudes building the dashboard
- `VITE_DAEMON_URL` env var → falls back to `http://127.0.0.1:7878`
- `localStorage['ghost-daemon-key']` for auth token
- All agent responses stored as jobs in Postgres — dashboard reads from `GET /jobs` and `GET /jobs/:id`
- SSE stream endpoint needed for live terminal (Code agent) and Thinking tab — design for it in Phase 4

### Prefix Command Language (SMS + Dashboard)
| Prefix | Behavior |
|---|---|
| (none) | Chat dispatcher — fast, semantic context, no agent spawning |
| `!` | Force Director + agents (override simple-task logic) |
| `?` | Research only, no write actions |
| `>` | Run a named scheduled task immediately |
| `.` | Ignored — GHOST returns 200 but sends no reply (useful for testing webhook delivery) |

### Verification Rules
- **Requires `y`:** email send, calendar create/edit, code push, any phone interaction
- **No verification:** chat, research, read-only tasks (summarize, "what's on my cal")
- **Shadow mode:** AI drafts response to incoming messages, spam triage first, sends only on `y`

---

## Roadmap (Priority Order)

### Phase 0 — Foundation (CURRENT PHASE)

**Goal:** Make the existing Rust daemon cloud-deployable, add a Postgres-backed job model,
and wire up Director config with fallback ranking. Nothing user-facing yet — this is the skeleton
everything else attaches to.

#### Platform Decision: Railway
- Auto-deploys from GitHub push. Zero manual ops.
- Built-in managed Postgres (enable pgvector extension — needed for Phase 2 semantic search).
- Logs visible in dashboard immediately. Predictable $5-7/month.
- No SSL config, no restart policies, no server maintenance.
- Alternatives considered: Fly.io (more complex, cold starts hurt SMS UX), Hetzner VPS (right for Phase 8 local models, wrong for Phase 0).

#### What Needs to Change in the Existing Daemon
The daemon (`rust/crates/rusty-claude-cli/src/daemon.rs`) was built for local Windows use.
Before Railway deploy:
- [ ] Add Postgres connection (use `DATABASE_URL` env var — Railway injects this automatically)
- [ ] Replace hardcoded `127.0.0.1:7878` with `HOST` + `PORT` env vars
- [ ] Write a `Dockerfile` (Rust multi-stage build → slim Debian image, single binary)
- [ ] Confirm CORS allows Railway-hosted dashboard origin (add to allow-list alongside localhost)
- [ ] Remove or gate any Windows-only code paths that block Linux compile

#### Database Schema (lock in now, extend later)

**jobs** — every request becomes a trackable job
```
id               UUID PRIMARY KEY
status           TEXT  -- pending | running | waiting_confirmation | done | failed | cancelled
input            TEXT  -- raw prompt or trigger description
output           TEXT  -- result (null until done)
agent            TEXT  -- 'director' | 'chat_dispatcher' | 'email' | 'code' | 'calendar' | 'research'
source           TEXT  -- 'sms' | 'dashboard' | 'scheduled' | 'proactive'
phone_from       TEXT  -- sender number if source=sms, null otherwise
requires_confirmation  BOOLEAN DEFAULT false
confirmation_token     TEXT    -- short token matched when user replies 'y'
created_at       TIMESTAMPTZ DEFAULT now()
updated_at       TIMESTAMPTZ DEFAULT now()
completed_at     TIMESTAMPTZ
```

**director_config** — singleton row, controls which models run
```
id               INTEGER PRIMARY KEY DEFAULT 1  -- always 1, singleton
primary_model    TEXT DEFAULT 'claude-sonnet-4-6'
fallback_model   TEXT DEFAULT 'gpt-4o'
primary_healthy  BOOLEAN DEFAULT true   -- circuit breaker flag
fallback_healthy BOOLEAN DEFAULT true   -- circuit breaker flag
last_health_check TIMESTAMPTZ
updated_at       TIMESTAMPTZ DEFAULT now()
```

**director_notes** — Phase 2 memory, but schema locked now to avoid painful migration
```
id               UUID PRIMARY KEY
category         TEXT  -- personal | social | code | projects | style | calendar
content          TEXT
embedding        VECTOR(1536)  -- pgvector; null until embedded after write
confidence       FLOAT DEFAULT 1.0
created_at       TIMESTAMPTZ DEFAULT now()
last_accessed_at TIMESTAMPTZ DEFAULT now()
expires_at       TIMESTAMPTZ  -- null = never expires
```

#### Director Fallback: Circuit Breaker Pattern
Not a retry loop — a state machine stored in `director_config`:

```
Incoming request
    → primary_healthy = true?  → call Sonnet
        success → return result
        fail (429 / 402 / 500 / timeout) → set primary_healthy = false → call GPT-4o
            success → return result
            fail → hard error → SMS user: "GHOST is down — switch models in dashboard"

Background health check (every 5 min):
    → ping primary model with minimal token request
        responds → set primary_healthy = true
    → same for fallback
```

This means you never hammer a dead model and you never silently degrade without knowing.

#### Environment Variables Required at Deploy
```
ANTHROPIC_API_KEY       Claude API (Director primary)
OPENAI_API_KEY          OpenAI API (Director fallback GPT-4o)
DATABASE_URL            Injected automatically by Railway Postgres add-on
GHOST_DAEMON_KEY        Bearer token for /prompt auth (you set this)
HOST                    0.0.0.0 (bind all interfaces on Railway)
PORT                    Injected automatically by Railway
```

#### Dockerfile (multi-stage Rust build)
```dockerfile
FROM rust:1.75-slim AS builder
WORKDIR /app
COPY rust/ ./rust/
WORKDIR /app/rust
RUN cargo build --release -p rusty-claude-cli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/rust/target/release/claw /usr/local/bin/claw
EXPOSE 8080
CMD ["claw", "daemon", "--host", "0.0.0.0", "--allow-unsafe-prompt"]
```

#### API Endpoints Added in Phase 0

| Method | Path | Description |
|---|---|---|
| GET | `/health` | Already exists — verify it works in cloud |
| GET | `/status` | Already exists — extend to include DB connection status |
| GET | `/jobs` | List recent jobs with status |
| GET | `/jobs/:id` | Single job detail |
| GET | `/director/config` | Returns current primary + fallback model, health flags |
| POST | `/director/config` | Swap primary or fallback model (dashboard toggle) |

#### Phase 0 Success Criteria
Everything below must be true before Phase 1 starts:
- [x] `claw daemon` builds and runs inside Docker on Linux — Dockerfile written, Linux compile fixed (`#[cfg(unix)]` gates added)
- [ ] Railway deployment is live at a public HTTPS URL
- [ ] `GET /health` returns 200 from that URL
- [ ] Postgres connected, all three tables created with correct schema
- [ ] pgvector extension enabled on Postgres
- [ ] `director_config` row seeded: primary = `claude-sonnet-4-6`, fallback = `gpt-4o`
- [ ] `POST /prompt` works end-to-end with bearer auth (proves full stack)
- [x] Dashboard `DAEMON_URL` is configurable — reads `VITE_DAEMON_URL` env var, falls back to `http://127.0.0.1:7878`
- [ ] Dashboard connects to Railway URL and shows GHOST online
- [x] Circuit breaker logic exists — `db.rs` + background 5-min health reset task in `daemon.rs`

**Remaining: push to GitHub → Railway setup → verify deploy (steps below)**

#### Railway Deploy Steps (manual, one-time)
1. `git push` — Railway detects the push and starts building the Dockerfile automatically
2. In Railway dashboard: click **New** → **Database** → **Add PostgreSQL** (links to your project, injects `DATABASE_URL`)
3. Set these env vars in Railway project settings:
   - `ANTHROPIC_API_KEY` — your Anthropic key
   - `OPENAI_API_KEY` — your OpenAI key (GPT-4o fallback)
   - `GHOST_DAEMON_KEY` — pick any strong token (this is your bearer auth)
   - `HOST` — `0.0.0.0`
4. Wait for deploy to go green (first build takes ~3-5 min, Rust compile is slow)
5. Verify:
   ```bash
   curl https://<your-service>.up.railway.app/health
   curl https://<your-service>.up.railway.app/director/config
   ```
   Second call confirms pgvector ran and `director_config` was seeded.
6. Set `VITE_DAEMON_URL=https://<your-service>.up.railway.app` in Railway dashboard build env (or `dashboard/.env.production`) so the dashboard points at the live server.

### Phase 1 — SMS Loop (CURRENT PHASE)

**Goal:** Wire the full SMS in/out loop. Text GHOST from your S25+, get a response back.
No specialists yet — Director responds with text only. Phase 1 is about proving the pipe.

#### Prefix routing (locked — matches code)
| Input | Route | Model |
|---|---|---|
| No leading punctuation | Chat dispatcher | Claude Haiku |
| `!` | Director stub | Sonnet 4.6 |
| `.` | Ignored — 200 OK, no reply sent | — |
| `?` | Research only (Phase 3, not yet wired) | — |
| `>name` | Named scheduled task (Phase 6, not yet wired) | — |

`?` and `>` with no specialist wired → Director responds with plain text stub. Never silent-fail.

#### Long response handling
- Responses under 500 chars → send as-is
- Responses 500+ chars → truncate at 497 chars + append dashboard link (3 chars for `...` + link on next segment)
- Dashboard link format: `ghost.app/jobs/<job-id>` (or Railway URL until custom domain exists)
- Every Director response (regardless of length) appends the job ID so you can always pull it up

#### Android SMS Gateway setup
- Install **Android SMS Gateway** app on S25+ (open source, F-Droid or GitHub)
- App exposes a local HTTP API — GHOST sends outbound SMS by POSTing to it
- Inbound: app forwards received texts to a webhook URL (your Railway server `POST /sms/inbound`)
- App must stay running — add to battery optimization whitelist on Samsung

#### New endpoints (Phase 1)
| Method | Path | Description |
|---|---|---|
| POST | `/sms/inbound` | Receives webhook from Android SMS Gateway. Creates job, routes to chat dispatcher or Director. |
| POST | `/sms/send` | Internal — daemon calls SMS Gateway to send outbound message |

#### SMS backup + dedicated GHOST number (deferred, Phase 1 follow-up)
- Twilio requires 18+. Deferred.
- `sms.rs` fallback path is stubbed — will wire to TextBelt or similar when age-appropriate option is picked.
- **Dedicated inbound number:** Isaac needs a second number to text *to* GHOST from his S25+.
  Current setup (Gateway on S25+) requires another device to trigger GHOST.
  Solution: inbound SMS provider with webhook support → assign GHOST a real number → Isaac texts it, GHOST replies.
  Options to evaluate: TextBelt (outbound only, no inbound), Bandwidth.com, Telnyx (no age gate confirmed?), or a pre-paid SIM in a cheap second phone.
- For now: test via another device. Dedicated number is Phase 1 follow-up before Phase 2.

#### Chat dispatcher flow (no-prefix messages)
```
Inbound SMS → strip whitespace → no leading punctuation?
    → create job (source=sms, agent=chat_dispatcher)
    → load core context file (hardcoded path, always injected)
    → call Haiku with [core context + message]
    → get response
    → length check: under 500? send direct. over 500? truncate + job link
    → POST to SMS Gateway → delivered to your number
    → mark job done
```

#### Director flow (! prefix, Phase 1 stub)
```
Inbound SMS → starts with !
    → strip ! → create job (source=sms, agent=director)
    → call Sonnet Director with [message]
    → Director responds (no specialists yet — plain text only)
    → same length check + send
    → mark job done
```

#### Core context file
A plain text file on the server (path in env var `GHOST_CORE_CONTEXT_PATH`) that always gets
injected into chat dispatcher calls. This is your safety net when semantic search (Phase 2)
doesn't return the right notes. Start with basics: your name, timezone, current projects,
preferred response style. You edit it directly — no UI needed until Phase 5.

#### Phase 1 Success Criteria
- [x] Android SMS Gateway v1.20.0 installed, webhook registered at `https://brave-cat-production-dd8e.up.railway.app/sms/inbound` (webhook ID: `Ky_981sReNwNF7SOEuq-n`)
- [x] `/sms/inbound`, `/sms/send`, `chat_dispatcher.rs`, `director.rs`, `sms.rs` — all built
- [x] `ghost-context.txt` written with identity, style, projects, current date
- [x] Push Phase 1 code to GitHub → Railway redeploys
- [x] Add `COPY ghost-context.txt /app/ghost-context.txt` to Dockerfile
- [x] Set Railway env vars: `GHOST_BASE_URL`, `GHOST_SMS_GATEWAY_URL`, `GHOST_CORE_CONTEXT_PATH`, `GHOST_ALLOWED_NUMBERS`
- [x] Railway build green — GET /health returns 200, db_connected: true
- [x] Job created in Postgres for every inbound message
- [x] Chat dispatcher (Haiku) responds — confirmed via curl simulation, job status: done in ~1.5s
- [x] `GET /jobs` shows conversation history
- [x] Twilio account created (isaac@kynesystems.com), number +18336283910 assigned
- [x] Twilio env vars set in Railway: TWILIO_ACCOUNT_SID, TWILIO_AUTH_TOKEN, TWILIO_FROM_NUMBER
- [ ] Real SMS test — text S25+ from another device, confirm reply arrives
- [ ] Text GHOST with `!` → Sonnet Director responds via SMS
- [ ] Response over 500 chars → truncated with job link appended
- [ ] Wire Twilio inbound webhook → update sms_inbound to parse Twilio form-encoded payload so Isaac can text +18336283910 from his S25+
- [ ] Add GHOST_ALLOWED_NUMBERS entry for +18336283910 inbound (or skip whitelist for Twilio path)

### Phase 2 — Memory + Context
- [ ] Director memory store with categories
- [ ] Semantic search on "." prefix queries
- [ ] Core context file (always injected on "." as safety net)
- [ ] Confidence decay on notes
- [ ] Memory panel in dashboard (view + edit)

### Phase 3 — Specialist Agents + Dashboard Tabs

**Goal:** Wire Research, Email, Calendar agents. Introduce per-agent output tabs in dashboard.
Director routes to specialists based on prefix + intent.

- [ ] Add `agent_tags` array column to `director_notes` (migration)
- [ ] Memory tagging: Haiku classifies agent tags on every note write
- [ ] Research Agent — Brave search + summarize, `?` prefix, standard/deep modes
- [ ] Email Agent — Gmail OAuth, isaac@kynesystems.com, draft+send with `y` approval
- [ ] Calendar Agent — read + create/edit (delete requires `y`)
- [ ] Director routing — intent classification → spawn correct specialist
- [ ] Dashboard: per-agent output tabs (Research, Email, Calendar, Chat)
- [ ] Dashboard: Settings menu (tab auto-switch, agent thinking toggle, Gmail account toggle)
- [ ] Dashboard: About tab (base prompts, instructions, disclaimer, API status)
- [ ] Citations: `[1]` format triggered by "source" keyword or Law Agent
- [ ] Push notifications via ntfy.sh on `#notify` tasks

### Phase 4 — Code Agent + IT Guide + Law Agent

- [ ] Code Agent — DeepSeek writes, E2B executes, live terminal streams to dashboard
- [ ] Code Agent — iterate on failure (up to 3 rewrites), surface error if still failing
- [ ] Code Agent — push finished scripts to GitHub (`/scripts/YYYY-MM-DD-task-name/`)
- [ ] Code Agent — dashboard inline preview + GitHub link
- [ ] IT Guide Agent — `->` prefix, screenshot upload, step map preview panel
- [ ] IT Guide Agent — Director auto-routes on frustration/navigation context clues
- [ ] Law Agent — Cornell Law + public legal DBs, always cites, US only
- [ ] Law Agent — confidence flagging ("this interpretation is contested")
- [ ] Agent thinking mode — toggleable per-agent in Settings, streams narrated reasoning
- [ ] Dashboard: Code terminal tab (live E2B output)
- [ ] Dashboard: IT Guide tab (step map visual)
- [ ] Dashboard: Law tab (formatted citations)

### Phase 6 — Automations
- [ ] Cron job system (configured from dashboard, not hardcoded)
- [ ] Morning brief (calendar + email + tasks + overnight flags → 5-line SMS)
- [ ] Evening close (what got done, what didn't, what to think about)
- [ ] Proactive monitoring: email + calendar background agent with urgency scoring (0–1 threshold, user-adjustable)
- [ ] Web monitoring via Distill.io webhooks + AI interpretation before notifying

### Phase 7 — Voice + Style
- [ ] Android wake word app (Porcupine on-device detection + Whisper transcription)
- [ ] Shadow mode (incoming message triage → AI drafts → `y` to send)
- [ ] Voice toggle: reactive mode (AI adapts to you) vs voice mode (AI sounds like you)
- [ ] Writing style training from emails + messages sent

### Phase 8 — Advanced (Later)
- [ ] Swappable/transferrable Director (full memory export/import between models)
- [ ] Self-training Director pipeline: every approve/reject/edit = labeled training data
- [ ] Fine-tune Mistral 7B or Llama 3.1 8B via Together AI on accumulated decision data
- [ ] Local Director model on Hetzner VPS (zero per-token routing cost)
- [ ] GitHub repo watcher (webhook-based, enabled per-repo from dashboard)
- [ ] Semantic Director auto-selection (rabbit hole — post-v1, needs benchmarking)

---

## Considering / Expanding (Not Yet Locked)

All of these are green-lit in spirit — need design work before implementation:

| # | Feature | Status |
|---|---|---|
| 31 | Proactive AI with urgency scoring | Expanding — threshold model needed |
| 32 | Photo/image input via SMS or dashboard | Go |
| 33 | Phone call input (transcribe → prompt) | Go |
| 34 | AI-maintained todo list with follow-ups | Go |
| 35 | Web monitoring + AI interpretation | Go — use Distill.io webhooks |
| 36 | Shadow mode for incoming messages (spam triage first) | Go — expand triage logic |
| 37 | Ambient wake word on S25+ | Go — Porcupine + Whisper, Phase 7 |
| 38 | GitHub repo watcher | Go — only when enabled per-repo |
| 39 | Two-sided AI style: reactive mode + voice mode | Go — toggle, voice used for drafting only |
| 40 | Morning brief + evening close | Go — Phase 6 |

---

## Estimated Monthly Costs

| Component | Low Use | Normal | Heavy |
|---|---|---|---|
| Railway server | $5 | $7 | $10 |
| Postgres (Railway) | incl. | incl. | $5 |
| Domain (~$1/mo amortized) | $1 | $1 | $1 |
| Claude Sonnet 4.6 (Director) | $3 | $7 | $15 |
| Claude Haiku (simple specialists) | $1 | $3 | $6 |
| DeepSeek (code tasks) | $0.50 | $1 | $3 |
| GPT-4o (fallback Director, rare) | $0.50 | $1 | $3 |
| E2B code sandboxes | $0 | $1 | $3 |
| Android SMS Gateway | $0 | $0 | $0 |
| SMS backup (TextBelt/email-to-SMS — deferred) | $0 | $0–1 | $1–2 |
| Whisper (voice transcription) | $0.25 | $0.50 | $1.50 |
| Brave/Tavily search | $0 | $1 | $3 |
| **Total** | **~$13** | **~$25** | **~$53** |

### One-Time Costs
| Item | Cost |
|---|---|
| Domain name | $10–12/year |
| Together AI fine-tune run (Phase 8) | $30–50 one-time |

### Per-Feature Cost Contribution
| Feature | Monthly Add |
|---|---|
| Android SMS Gateway | $0 |
| SMS backup (deferred) | +$0–2 |
| Director (Sonnet) | ~$5–10 |
| Director fallback (GPT-4o) | ~$0.50–2 |
| Email Agent | $0 (Gmail API free) |
| Calendar Agent | $0 (Google Cal API free) |
| Research Agent | $0–3 (free tier covers personal use) |
| Code Agent + E2B | $0–3 |
| Wake word (Whisper only) | ~$0.25–1.50 |
| Web monitoring (Distill.io) | $0 (25 monitors free) |
| Proactive monitoring agent | ~$1–3 (background Director calls) |
| Self-training fine-tune | $30–50 one-time when data is ready |
| Local Director on Hetzner VPS | +$4.20/mo, saves ~$5–10/mo on tokens |

---

## Key Design Decisions (Locked)

- Director memory is model-agnostic (Postgres schema, not model-specific format)
- "." prefix bypasses Director entirely — chat dispatcher only
- Core context file always injected on "." as safety net if semantic search misses
- Verification: lowercase `y` only. No complex confirm flows.
- No tool-based routing. Director spawns agents, agents are ephemeral.
- Dashboard ≥ SMS in feature depth. Both always functional.
- Everything server-side. Nothing depends on Isaac's local machine.
- Director fallback: #1 → #2 → hard error. No chains beyond two.
- Shadow mode triage: spam/automated = skip. Human senders = draft.
- Voice mode is a toggle. Default is reactive (AI adapts to Isaac). Voice (sounds like Isaac) only when explicitly needed.
- Every approve/reject/edit in the UI is training data — design data capture from day one.

---

## Notes for Future Claudes

- Claude's role is brainstormer and realist system designer. Isaac builds the code.
- Do not suggest tool-based routing. The pattern is Director → specialist agents.
- The self-training Director (Phase 8) is intentional and high-priority long-term. Don't dismiss it.
- Morning brief and evening close are north star features — design everything with them in mind.
- Every approve/reject UI element is training data infrastructure, not just UX.
- Surface hard design questions rather than making assumptions.
- High-risk, high-reward ideas are welcome and encouraged. Flag them clearly but propose them.
- Isaac is 15, runs KYNE Systems, thinks at a systems level. Treat him as a peer on architecture.
