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
| Agent | Integration | Model |
|---|---|---|
| Email Agent | Gmail API (OAuth) | Sonnet or Haiku |
| Calendar Agent | Google Calendar API | Haiku |
| Code Agent | E2B sandbox | DeepSeek (cost) or Sonnet |
| Research Agent | Brave Search / Tavily | Haiku |

### Director Memory System
Stored in Postgres. Model-agnostic — portable when Director is swapped.
Categories: `personal`, `social`, `code`, `projects`, `style`, `calendar`.
Notes have confidence decay — old notes get down-weighted, Director can refresh or expire them.
On "." prefix: core context file always loaded + semantic search pulls 3–5 most relevant notes.

### Input Interfaces
| Interface | Notes |
|---|---|
| SMS (Android SMS Gateway) | Real S25+ number, free, open-source app |
| SMS (Twilio backup) | Auto-fallback if Gateway unreachable |
| Web dashboard | More features than SMS, always functional |
| Wake word app (Phase 7) | Porcupine (on-device) + Whisper, Android |

### Prefix Command Language (SMS + Dashboard)
| Prefix | Behavior |
|---|---|
| `.` | Chat dispatcher — fast, semantic context, no agent spawning |
| `!` | Force Director + agents (override simple-task logic) |
| `?` | Research only, no write actions |
| `>` | Run a named scheduled task immediately |

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

### Phase 1 — SMS Loop
- [ ] Android SMS Gateway integration (receive + send via real number)
- [ ] SMS → Director → SMS response loop
- [ ] "." prefix chat dispatcher with core context file + semantic search
- [ ] `y` verification flow for flagged actions
- [ ] Twilio backup with auto-fallback

### Phase 2 — Memory + Context
- [ ] Director memory store with categories
- [ ] Semantic search on "." prefix queries
- [ ] Core context file (always injected on "." as safety net)
- [ ] Confidence decay on notes
- [ ] Memory panel in dashboard (view + edit)

### Phase 3 — Specialist Agents
- [ ] Email Agent (Gmail OAuth, read/draft/send)
- [ ] Calendar Agent (Google Calendar API)
- [ ] Research Agent (Brave/Tavily)
- [ ] Director → spawn specialist → synthesize result → SMS/dashboard flow

### Phase 4 — Code Execution
- [ ] Code Agent with E2B sandbox
- [ ] Live terminal in dashboard (SSE stream)
- [ ] Job status SMS notifications (start + done)
- [ ] File delivery via GitHub push or email

### Phase 5 — Dashboard Command Center
- [ ] Live task terminal
- [ ] Scheduler panel (plain English input → cron conversion)
- [ ] Agent panel (active specialists + run history)
- [ ] Director model toggle (swap #1 and #2 manually before any model configured)
- [ ] Memory panel

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
| Twilio backup number + messages | $2 | $2 | $2 |
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
| Twilio backup | +$2 |
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
