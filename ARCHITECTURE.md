# GHOST — Architecture Map

> The file tree, annotated. Every file that exists or will exist, what it does,
> and how it connects to neighbours. Read this before writing any new code so
> you drop work into the right place instead of inventing a parallel spot.
>
> **This is a living map, not a spec.** Update entries as files are created,
> renamed, or deleted. If a file exists on disk but isn't here, add it.
> If it's here and marked `[NEW]` but already written, promote it to `[LIVE]`.

---

## Legend

| Marker | Meaning |
|---|---|
| `[LIVE]` | Exists and is in the hot path today |
| `[STUB]` | Exists but is skeletal — schema/plumbing present, logic missing |
| `[IDLE]` | Exists but is not wired into the live path (dead code or standalone) |
| `[NEW]` | Planned — does not exist yet |
| `[UI]` | Dashboard-side file |
| `[SQL]` | Postgres migration |

Arrow conventions in annotations:
- `→ X` this file calls into X
- `← X` this file is called by X
- `↔ X` bidirectional / shared state

---

## Layer overview

```
┌─────────────────────────────────────────────────────────────────┐
│  TRANSPORTS    SMS (Android Gateway, Twilio), Dashboard          │
│                (future: Telegram? Voice? — open)                 │
├─────────────────────────────────────────────────────────────────┤
│  INGRESS       daemon.rs — single HTTP entry for all transports  │
├─────────────────────────────────────────────────────────────────┤
│  ROUTING       chat_dispatcher (default) · director (!) ·        │
│                [NEW] intent classifier → agent dispatcher        │
├─────────────────────────────────────────────────────────────────┤
│  AGENTS        chief_of_staff · calendar · docs · research ·     │
│                dreamer · [NEW] alarm · code · law · it_guide     │
├─────────────────────────────────────────────────────────────────┤
│  INFRA         [NEW] budget · approval · scheduler · events      │
├─────────────────────────────────────────────────────────────────┤
│  CONTEXT       memory (pgvector) · core_context · gerald ·       │
│                [NEW] interest_graph                              │
├─────────────────────────────────────────────────────────────────┤
│  STORAGE       Postgres (jobs, notes, sms_*, schedule, bible_*)  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Rust workspace — `rust/crates/rusty-claude-cli/src/`

```
rusty-claude-cli/src/
│
├── main.rs                      [LIVE]  CLI entry (claw prompt, claw daemon, claw bible-ingest, …)
│
├── daemon.rs                    [LIVE]  The ingress. HTTP server on :7878. CORS, auth, host-header
│                                        validation, background tasks (circuit-breaker reset, confidence
│                                        decay). All endpoints live here today — this file is ~2.3k LoC
│                                        and WILL need to be split as the agent layer lands.
│                                        → chat_dispatcher · director · sms · db · memory · bible
│                                        ← every transport
│
├── constants.rs                 [LIVE]  Model IDs (HAIKU_MODEL, SONNET_MODEL, OPUS_MODEL) + Anthropic URL
│
├── http_client.rs               [LIVE]  Shared reqwest client (connection pooling)
│
│ ── Routing paths (current) ──────────────────────────────────────
│
├── chat_dispatcher.rs           [LIVE]  Default path (no prefix). Haiku. Loads core context + top-5
│                                        pgvector memories + schedule. Fire-and-forget memory extraction.
│                                        → memory · db · gerald (indirectly via core context) · bible
│                                        ← daemon (/sms/inbound no-prefix, /chat)
│
├── director.rs                  [LIVE]  `!` prefix path. Now a facade: `handle()` calls
│                                        `Dispatcher::dispatch`. The actual Sonnet call lives in
│                                        `pub(crate) sonnet_reply()` which the dispatcher invokes
│                                        for `Intent::Director`.
│                                        → agents::dispatcher · memory · db
│                                        ← daemon (/sms/inbound, /chat)
│
├── routing.rs                   [IDLE]  Opt-in per-prompt model classifier. Only touches one-shot
│                                        `claw prompt` CLI. Not used by daemon. Keep as reference for
│                                        agent-tier routing design.
│
├── orchestrator.rs              [IDLE]  Plan/worker decomposition engine (~950 LoC). Smart model plans,
│                                        Haiku workers execute. Not wired. Possible reuse for Research
│                                        Agent deep-mode or Docs Agent multi-section writes.
│
│ ── Transports ────────────────────────────────────────────────────
│
├── sms.rs                       [LIVE]  Outbound SMS delivery (Android Gateway + Twilio). Inbound parse
│                                        lives in daemon.rs (split candidate).
│                                        → daemon (re-enters via internal call? — no, daemon calls into sms)
│                                        ← daemon
│
├── contacts.rs                  [LIVE]  sms_contacts table helpers (auto-reply toggle, name, counts)
│
│ ── Memory & context ─────────────────────────────────────────────
│
├── memory.rs                    [LIVE]  Haiku-based note extraction (category|content), embedding via
│                                        Voyage or OpenAI. Fire-and-forget after every response.
│                                        → db · http_client
│
├── gerald.rs                    [LIVE]  Gerald Brain MCP client. Today: session-start get_overview,
│                                        session-end save_session only. NOT called per-turn.
│                                        ← main (CLI), daemon (session bookends)
│
├── db.rs                        [LIVE]  All SQL. ~1.8k LoC. Jobs, director_notes, sms_*, schedule,
│                                        bible_*, director_config, scholar_solutions, response_cache.
│                                        Split candidate: one file per table group.
│
├── search.rs                    [LIVE]  pgvector similarity query helpers (used by memory + bible)
│
│ ── Specialists that exist ────────────────────────────────────────
│
├── bible.rs                     [LIVE]  Bible query classification + context loading. Wired as context
│                                        injection into chat_dispatcher — not a spawned agent.
│
├── bible_ingest.rs              [LIVE]  `claw bible-ingest` CLI. Reads .ghost/bible-data/*, batch-embeds,
│                                        bulk-inserts. One-shot, not runtime.
│
│ ── Support / misc ────────────────────────────────────────────────
│
├── compress.rs                  [LIVE]  Prompt compression utilities (for PIPELINE.md work)
├── render.rs                    [LIVE]  Output formatting (terminal/markdown)
├── input.rs                     [LIVE]  CLI input handling (REPL)
├── init.rs                      [LIVE]  `claw init` — bootstrap project CLAUDE.md
├── guard.rs                     [LIVE]  Secret redaction, command safety checks
├── macros.rs                    [LIVE]  Internal macros (logging, etc.)
│
│ ══════════════════════════════════════════════════════════════════
│    BELOW THIS LINE: PLANNED. NOTHING BELOW EXISTS YET.
│ ══════════════════════════════════════════════════════════════════
│
│ ── Agent dispatcher layer ───────────────────────────────────────
│
├── agents/                      [LIVE]  Inline module tree. Every specialist lives here.
│   │
│   ├── mod.rs                   [LIVE]  Agent trait + AgentRequest / AgentResponse / Source /
│   │                                    ModelTier types. Foundation for every specialist.
│   │
│   ├── dispatcher.rs            [LIVE]  The router. Calls `intent::classify`, dispatches to
│   │                                    chat_dispatcher (Chat) or director::sonnet_reply
│   │                                    (Director). Per-call budget check → agent call →
│   │                                    event record → token debit. Stubs Research /
│   │                                    Scheduled / Calendar with clear errors.
│   │                                    ← director.rs · daemon.rs
│   │                                    → chat_dispatcher · director::sonnet_reply ·
│   │                                      infra::budget · infra::events
│   │
│   ├── intent.rs                [LIVE]  `classify(raw) -> (Intent, stripped_message)`. Handles
│   │                                    `!` (Director) / `?` (Research) / `>` (Scheduled) /
│   │                                    `@` (Calendar) / `#` (ChiefOfStaff) / `&` (Docs) /
│   │                                    `.` (Ignore). Single source of truth for routing —
│   │                                    called by dispatcher + daemon jobs label.
│   │
│   ├── chief_of_staff.rs        [LIVE]  Sonnet-driven orchestrator. Two-pass flow: build_plan
│   │                                    (Sonnet → JSON plan of sub-agent legs) → execute each
│   │                                    leg by re-entering Dispatcher::dispatch (so budget +
│   │                                    events record per leg) → compose_reply (Sonnet composes
│   │                                    a single answer). Mid tier. `#` prefix (Wave 5). Never
│   │                                    emits a self-referential leg.
│   │                                    → agents/dispatcher · constants::SONNET_MODEL
│   │
│   ├── calendar.rs              [LIVE]  Google Calendar agent. List + Create fully wired;
│   │                                    Update / Delete / Suggest stubbed for Wave 4 (delete
│   │                                    pending approval gate). Raw reqwest — no Google SDK.
│   │                                    Dispatcher-registered via `@` prefix (Wave 3.5).
│   │                                    → oauth · http_client
│   │
│   ├── oauth.rs                 [LIVE]  Google OAuth refresh-token helper. `access_token_for()`
│   │                                    reads `oauth_tokens`, refreshes against Google when
│   │                                    needed, writes back. Shared by calendar + (later) docs,
│   │                                    email.
│   │                                    → db (oauth_tokens)
│   │
│   ├── docs.rs                  [LIVE]  Google Docs agent. Create + Read + Append wired;
│   │                                    insert_at_heading / replace_text / delete stubbed for
│   │                                    Wave 6 (delete + replace will gate on approval).
│   │                                    Shares `oauth_tokens` (provider = "google_docs").
│   │                                    Dispatcher-registered via `&` prefix (Wave 5).
│   │                                    → oauth · http_client
│   │
│   ├── alarm.rs                 [NEW]   POST to iOS Shortcut webhook URL. Contract documented in
│   │                                    docs/alarm-webhook.md [NEW]. Isaac builds the Shortcut.
│   │
│   ├── research.rs              [LIVE]  Brave Search → top-3 page fetch (5s timeout, 8KiB cap,
│   │                                    hand-rolled HTML strip) → Haiku 3–5 bullet summary with
│   │                                    inline URL cites. `?` prefix dispatch-registered.
│   │                                    Deep mode (Sonnet + orchestrator.rs) deferred to Wave 6.
│   │                                    → http_client · constants::HAIKU_MODEL
│   │
│   ├── dreamer.rs               [LIVE]  Reflection agent. Reads `dreamer_window` (recent
│   │                                    ghost_events + director_notes, ~8 KiB oldest-truncated),
│   │                                    asks Sonnet for 3–8 recurring-theme topics, embeds each
│   │                                    (voyage-3 / 1024-dim), upserts into `interest_nodes`.
│   │                                    Intended to fire from a `scheduled_triggers` row; NOT
│   │                                    yet dispatcher-registered (prefix pending — Wave 5.5).
│   │                                    → db (interest_nodes, ghost_events) · memory::embed
│   │                                    ← infra/scheduler (once a prefix is wired)
│   │
│   ├── email.rs                 [NEW]   Gmail OAuth. Draft + send with approval. (VISION Phase 3.)
│   ├── code.rs                  [NEW]   DeepSeek + E2B sandbox. (VISION Phase 4.)
│   ├── it_guide.rs              [NEW]   `->` prefix + screenshot. (VISION Phase 4.)
│   ├── law.rs                   [NEW]   US legal retrieval, always-cites. (VISION Phase 4.)
│   │
│   └── coder/                   [LIVE]  Phase C — semantic file index + template scaffolds for
│       │                                the coder agent. `agent.rs` (Prompt B) wires in the
│       │                                DeepSeek / Anthropic fallback chain; everything below
│       │                                is the retrieval + boilerplate-stamping layer.
│       │
│       ├── mod.rs               [LIVE]  `repo_root(pool)` resolver. Cascade:
│       │                                `GHOST_CODER_REPO_ROOT` env > `coder.repo_root` setting
│       │                                > `std::env::current_dir()`. One source of truth for the
│       │                                indexer, watcher, and `/code/index/*` endpoints.
│       │
│       ├── index.rs             [LIVE]  Per-file signature embeddings in `coder_file_index`
│       │                                (migration 023, VECTOR(1024)). `index_file` / `index_repo`
│       │                                / `search_files` / `remove_path`. Walks via `ignore`
│       │                                (gitignore-aware), skips target/node_modules/.git/dist/
│       │                                .ghost, 100 KiB cap, binary-byte sniff. Embed via
│       │                                `memory::embed`; NULL embedding row stored on failure so
│       │                                `search_files` still finds it via the ILIKE fallback.
│       │                                → db (coder_file_index) · memory::embed · ignore crate
│       │
│       └── templates/           [LIVE]  Six baked-in scaffolds (`.tmpl` + `.meta.json`) for
│                                        new_migration / new_daemon_endpoint / new_agent /
│                                        new_dashboard_panel / new_tool / new_db_helper. Bundled
│                                        via `include_str!`. `stamp()` renders `{{placeholder}}`
│                                        markers; special server-side placeholders
│                                        `{{next_migration_number}}` and `{{today_date}}` are
│                                        computed only when referenced. Does NOT touch disk —
│                                        callers feed the output through the normal diff queue.
│
│ ── Infra layer (agent-wide concerns) ────────────────────────────
│
├── infra/                       [LIVE]  Cross-cutting. Seeded Wave 1 with caching; grows wave by wave.
│   │
│   ├── mod.rs                   [LIVE]  Re-exports. Grows as modules land.
│   │
│   ├── cache.rs                 [LIVE]  `build_cached_system(stable, dynamic) -> Value`. Returns
│   │                                    JSON array with `cache_control: ephemeral` on the stable
│   │                                    block. Used by chat_dispatcher.rs + director.rs.
│   │
│   ├── budget.rs                [LIVE]  Per-agent daily caps. `check()` reads today's spend,
│   │                                    `debit()` adds tokens + cost. Hardcoded caps map
│   │                                    (DEFAULT_CAP_CENTS = 100). Tier prices via
│   │                                    `cost_cents(tier, tokens_in, tokens_out)`. Blown budget
│   │                                    → dispatcher returns refusal. Backed by `agent_spend`.
│   │
│   ├── approval.rs              [LIVE]  `is_approval_message()` detects `y` / `yes [token]`.
│   │                                    `find_pending_for_contact()` / `resolve_by_token()` look
│   │                                    up waiting jobs. `mark_job_approved()` flips status.
│   │                                    Wired into SMS inbound in Wave 5 — a bare `y`/`yes` from
│   │                                    a whitelisted phone intercepts BEFORE the intent
│   │                                    classifier and resolves the sender's most recent pending
│   │                                    job. Job continuation post-approval is Wave 6.
│   │
│   ├── scheduler.rs             [LIVE]  Single tokio task on a 30s ticker. Pulls due rows from
│   │                                    `scheduled_triggers`, stamps `next_fire_at` + `last_fired_at`
│   │                                    before dispatch (no double-fires), creates a `jobs` row per
│   │                                    fire, calls `Dispatcher::dispatch(..., Source::Scheduled)`.
│   │                                    Cron format is 6-field (sec min hr dom mon dow) — `cron`
│   │                                    crate. Spawned from `daemon.rs` startup.
│   │                                    → agents/dispatcher · db (jobs) · cron
│   │
│   └── events.rs                [LIVE]  Per-turn event log. `record()` / `recent()` / `for_agent()`
│                                        / `attach_correction()`. Backed by `ghost_events` table.
│                                        Outcome enum: Success / Fallback / Refused / Error /
│                                        Escalated. Written on every dispatch by agents/dispatcher.
│                                        Surfaced via `GET /events`. Weekly Gerald mirror: Wave 4+.
│
│ ── Context layer additions ──────────────────────────────────────
│
└── context/                     [NEW]
    │
    ├── mod.rs                   [NEW]
    │
    ├── core.rs                  [NEW]   Wraps today's loose core-context loading
    │                                    (GHOST_CORE_CONTEXT_PATH reads) into one module with
    │                                    cache-friendly stable ordering.
    │
    └── interest_graph.rs        [NEW]   Dreamer's output substrate. NEW table `interest_nodes`
                                         (topic, last_touched, depth_explored, related_topics[],
                                         embedding). Read by dreamer to pick next curiosity
                                         thread, written when dreamer finishes a loop.
```

---

## HTTP endpoints — ownership map

Already in daemon.rs today (see CLAUDE.md for the full table). As the agent layer lands, split
daemon.rs into topic routers: `daemon/sms.rs`, `daemon/memory.rs`, `daemon/agents.rs`, etc.

New endpoints the expansion implies:

| Method | Path | Status | Purpose |
|---|---|---|---|
| `GET`  | `/events` | **LIVE (Wave 3)** | Paginated event log for dashboard audit view |
| `GET`  | `/agents/budget` | **LIVE (Wave 3)** | Today's spend per agent |
| `GET`  | `/agents` | **LIVE (Wave 3)** | Static agent registry (hardcoded list, Wave 4+ = dispatcher query) |
| `POST` | `/agents/approve` | [NEW] | HTTP approval endpoint (Wave 5 wired approvals via SMS `y`/`yes` instead; HTTP path still open) |
| `GET`  | `/triggers` | **LIVE (Wave 5)** | List scheduled_triggers, newest-first, limit 100 |
| `POST` | `/triggers` | **LIVE (Wave 5)** | Register a scheduled trigger (validates cron_expr + agent allow-list, computes next_fire_at) |
| `DELETE` | `/triggers/:id` | **LIVE (Wave 5)** | Delete a scheduled trigger by UUID |
| `POST` | `/dreamer/dispatch` | [NEW] | Manual kick of the morning dispatch (pending Wave 5.5 dispatcher-registration) |

---

## Database — `rust/migrations/`

```
rust/migrations/
├── 001_initial.sql              [SQL]  jobs, director_config, director_notes, pgvector ext
├── 002_shrink_embedding_dim.sql [SQL]  1536-dim fix
├── 003_scholar_solutions.sql    [SQL]  PIPELINE.md cache — solved-problem lookups
├── 004_response_cache.sql       [SQL]  PIPELINE.md response cache
├── 005_bible_schema.sql         [SQL]  bible_verses, pericopes, cross_refs, lexicon
├── 006_sms_history.sql          [SQL]  sms_history
├── 007_sms_loadbearing.sql      [SQL]  sms_history.loadbearing flag
├── 008_sms_contacts.sql         [SQL]  sms_contacts + auto_reply
├── 009_sms_unread.sql           [SQL]  unread tracking
├── 010_contact_notes.sql        [SQL]  per-contact notes
│
│ ── Planned (none exist) ──────────────────────────────────────────
│
├── 011_ghost_events.sql         [SQL]  Per-turn event log (landed Wave 2)
├── 012_agent_spend.sql          [SQL]  Daily per-agent token + cost counters (landed Wave 3)
├── 013_oauth_tokens.sql         [SQL]  Google OAuth refresh tokens (landed Wave 3)
├── 014_scheduled_triggers.sql   [SQL]  Cron + agent + payload (landed Wave 4)
├── 015_interest_nodes.sql       [SQL]  Dreamer's interest graph — topic, summary, weight,
│                                       embedding VECTOR(1024), source_refs jsonb (landed Wave 5)

Note: `sms_schedule` is NOT missing — it's created inside `008_sms_contacts.sql:14`
(bundled with sms_contacts). No new migration needed for it.
```

---

## Dashboard — `dashboard/src/`

Wave 1 split the old 2.8k-line `App.jsx` into one-panel-per-file. Names match the real UI
regions (not the earlier aspirational list). `App.jsx` is now a 268-line composition root
holding auth, status polling, project/tab state, and the send-message flow — everything
else lives in a panel.

```
dashboard/src/
├── main.jsx                     [UI LIVE]  React root + ErrorBoundary
├── App.jsx                      [UI LIVE]  Composition root. Auth gate, status polling, send flow.
├── App.css                      [UI LIVE]
├── index.css                    [UI LIVE]
│
├── lib/
│   └── api.js                   [UI LIVE]  Shared API helper — apiFetch, STORAGE_KEY, AGENTS,
│                                           QUICK_REPLIES, timeAgo, project persistence, etc.
│
└── panels/
    ├── AuthScreen.jsx           [UI LIVE]  Centered key input, wrong-key banner, persist on success.
    ├── TopBar.jsx               [UI LIVE]  Alive dot · GHOST · uptime · open chat tabs.
    ├── JobBanner.jsx            [UI LIVE]  Running-job banner below the top bar (dismissible).
    ├── Sidebar.jsx              [UI LIVE]  Project/chat tree + sidebar nav (Settings/Stats/About).
    │                                       Includes ProjectTree, ProjectItem, ChatItem, SidebarNav.
    ├── ChatArea.jsx             [UI LIVE]  Main chat body: Chat / Preview / Context / Thinking
    │                                       inner tabs + agent toggles above the input bar.
    ├── SmsPanel.jsx             [UI LIVE]  Contacts list, conversation view, auto-reply toggle,
    │                                       schedule overlay. Includes SmsAddForm, SmsContactRow,
    │                                       SmsConversation, SmsSchedulePanel.
    ├── SettingsPanel.jsx        [UI LIVE]
    ├── StatisticsPanel.jsx      [UI LIVE]
    ├── AboutPanel.jsx           [UI LIVE]
    ├── NoChatSelected.jsx       [UI LIVE]  Empty-state for the main area when no chat is open.
    ├── EventsPanel.jsx          [UI LIVE]  Audit view of ghost_events. Table w/ filter by agent,
    │                                       Load More pagination, coloured outcome pills.
    ├── BudgetPanel.jsx          [UI LIVE]  Today's spend per agent. Horizontal bars (green <50,
    │                                       yellow 50–90, red ≥90), per-agent calls / cost.
    └── AgentsPanel.jsx          [UI LIVE]  Static agent registry grid (name, tier badge,
                                            trigger, implemented vs planned dot). Reference
                                            display — replaced by dispatcher-query in Wave 4+.
│
│ ── Planned (none exist yet) ─────────────────────────────────────
│
    └── FileTreePanel.jsx        [UI NEW]   Visual version of THIS file. Deferred — open question
                                            on whether source of truth is this markdown or a
                                            generated architecture.json.
```

---

## Other trees (unchanged, noted for completeness)

```
trading/                         [LIVE]   Python sidecar — alt-data nowcast scaffold. Separate from
                                          Ghost. Do not entangle with agents/.

prompts/                         [LIVE]   Phase-specific SMS prompts (1–6). Reference material for
                                          agent system prompts.

scripts/                         [LIVE]   daemon-install.ps1, download-bible-data.ps1

rust/crates/                              The non-daemon crates — not GHOST-pipeline relevant but
  ├── api/                                listed so nobody assumes they're dead code.
  ├── runtime/                            Pre-existing Anthropic parity scaffolding. Not on hot path.
  ├── commands/
  ├── plugins/
  ├── telemetry/
  ├── tools/
  ├── compat-harness/
  └── mock-anthropic-service/             Mock Anthropic for testing runtime parity.
```

---

## Decisions (locked 2026-04-20)

1. **`agents/` is inline** under `rusty-claude-cli/src/agents/`. Promote to sibling crate only
   if build times hurt.
2. **`director.rs` becomes a thin facade** — a ~20-line function that calls `agents/dispatcher.rs`.
   The `!` prefix contract stays intact. Dispatcher is the real router.
3. **Prompt caching ships first**, before any agent work. `cache_control` markers on the stable
   prefix in `chat_dispatcher.rs` + `director.rs`. Saves tokens on every downstream call.
4. **Intent classification consolidates into `agents/intent.rs`** on the first agent PR.
   Today's prefix parsing (scattered in `daemon.rs` and `chat_dispatcher.rs`) moves here.
5. **Budget granularity: per-agent per-day.** Simple hard cap. Roll-up views in dashboard.
6. **Dreamer runs on cron** for MVP (e.g. 6am daily). Idle-gap triggering comes later once we
   have a signal for what "idle" means.
7. **`ghost_events` writes to Postgres always**, mirrors a condensed weekly summary to Gerald.
   Postgres = fast/local/structured; Gerald = long-horizon semantic queries.
8. **SMS-only for transports** (decision 2026-04-20). If Telegram is ever added it sits
   alongside SMS, not replacing it — contact / auto-reply / loadbearing semantics stay on SMS.

## Still open

1. **FileTree UI source of truth** — render from this markdown, or generate an
   `architecture.json` that both the doc and UI read? Deferred until the UI work starts.

---

## How to use this map when writing code

1. Find the layer your change touches.
2. If the file is `[LIVE]`, read it before editing.
3. If the file is `[NEW]`, check the open-questions list — your PR may need a decision first.
4. If the file doesn't appear here at all, either you're wrong about needing it, or the map is
   stale. Fix the map in the same PR as the code.
5. Arrows are the contract. If you change what a file depends on, update the arrows.
