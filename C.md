# Worker C (Wave 5) — `POST /triggers` endpoint + approval→SMS wiring

One-line task: add the scheduler trigger-registration HTTP endpoint (`POST /triggers` + `GET /triggers` + `DELETE /triggers/:id`) to `daemon.rs`, AND wire the existing `infra::approval` module into the SMS inbound path so a reply of `y` / `yes` / `y <token>` actually resolves a pending job instead of being dispatched as chat.

You are the ONLY worker touching `daemon.rs` this wave. A stays out of it entirely. B only appends to `db.rs` — you do too, separate section, trivial merge.

---

## Read first

1. `CLAUDE.md` + `ARCHITECTURE.md`.
2. `rust/crates/rusty-claude-cli/src/infra/scheduler.rs` — understand the `ScheduledTrigger` shape, `due_triggers`, `mark_fired`, and that `next_fire_at` is computed via the `cron` crate. You'll compute `next_fire_at` the same way when inserting a new trigger.
3. `rust/crates/rusty-claude-cli/src/infra/approval.rs` — the full file. Read `is_approval_message`, `find_pending_for_contact`, `resolve_by_token`, `mark_job_approved`. You are composing these; do NOT modify `approval.rs`.
4. `rust/crates/rusty-claude-cli/src/daemon.rs` — specifically:
   - The `match (method.as_str(), path)` router around line 580–680 (add the three trigger endpoints there)
   - `sms_inbound` (starts around line 1188) — add the approval pre-check early, before the current intent classifier call (~line 1263)
   - Existing handler style: e.g., `sms_auto_reply_handler`, `schedule_create_handler` are the closest templates. Copy the auth/body-parse/JSON-response shape exactly.
5. `rust/crates/rusty-claude-cli/src/db.rs` — look at existing insert helpers (e.g., `insert_note`, `create_job`) for the style of the new `insert_scheduled_trigger`.
6. `rust/crates/rusty-claude-cli/src/sms.rs` — the outbound-SMS helper you'll call to tell Isaac "approved → running <job>". Match however other handlers invoke it.

---

## Scope — exactly these files

| Path | Action |
|---|---|
| `rust/crates/rusty-claude-cli/src/daemon.rs` | **Edit** — add 3 router arms + 3 handler fns + approval pre-check in `sms_inbound` |
| `rust/crates/rusty-claude-cli/src/db.rs` | **Edit (append only)** — `insert_scheduled_trigger` + `delete_scheduled_trigger` helpers |

**Do NOT touch:** `agents/*` (A owns intent/dispatcher; others are off-limits), `infra/*` (approval/scheduler/events/budget stay as-is), any migration, any dashboard file, any other rust file. Your edits are strictly additive.

---

## Design (already decided)

### Trigger-registration endpoints

All three require `Authorization: Bearer <GHOST_DAEMON_KEY>` via the existing `auth_matches(raw)` helper — same as `POST /sms/send` and other key-guarded endpoints. 401 on missing/wrong key.

| Method | Path | Body / Response |
|---|---|---|
| `GET` | `/triggers` | List all rows in `scheduled_triggers`, newest-first, limit 100. Returns `{"triggers":[ {...}, ... ]}`. |
| `POST` | `/triggers` | Body: `{"name":"morning_brief","cron_expr":"0 0 9 * * *","agent":"research","payload":"?what's new in rust async","enabled":true}`. Computes `next_fire_at` from `cron_expr` at insert time. Returns `{"id":"<uuid>","next_fire_at":"<rfc3339>"}`. |
| `DELETE` | `/triggers/:id` | Delete by UUID. Returns `{"deleted":true}` or 404 `{"error":"not found"}`. |

#### Handler shapes

```rust
async fn triggers_list(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) { ... }
async fn trigger_create(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) { ... }
async fn trigger_delete(cfg: &DaemonConfig, raw: &str, id: &str) -> (&'static str, String) { ... }
```

Router arms go next to the existing `/schedule` routes (around line 660 region — use the nearest schedule handler as a locator):

```rust
("GET", "/triggers") => triggers_list(cfg, raw).await,
("POST", "/triggers") => trigger_create(cfg, raw).await,
("DELETE", p) if p.starts_with("/triggers/") => {
    let id = &p["/triggers/".len()..];
    trigger_delete(cfg, raw, id).await
}
```

#### Validation (POST)

- `name`: non-empty, ≤128 chars.
- `cron_expr`: MUST parse via `cron::Schedule::from_str`. On error, return 400 `{"error":"invalid cron expression: <e>"}`. Document the 6-field format in the error string.
- `agent`: one of `"research" | "calendar" | "docs" | "chat_dispatcher" | "director" | "chief_of_staff" | "dreamer"` (allow-list). Reject unknown agent → 400. Note: chief_of_staff and dreamer may not be dispatcher-wired yet this wave — that's fine, Isaac is scheduling ahead for Wave 5.5.
- `payload`: non-empty, ≤4096 chars.
- `enabled`: optional, default `true`.

Compute `next_fire_at`:
```rust
let schedule = cron::Schedule::from_str(&cron_expr)
    .map_err(|e| format!("invalid cron expression: {e}"))?;
let next_fire_at = schedule.upcoming(chrono::Utc).next()
    .ok_or_else(|| "cron produced no upcoming fire time".to_string())?;
```

Hand the struct to a new `db::insert_scheduled_trigger` helper that returns the inserted `Uuid`.

### db.rs helpers (append only)

```rust
pub struct NewScheduledTrigger<'a> {
    pub name: &'a str,
    pub cron_expr: &'a str,
    pub agent: &'a str,
    pub payload: &'a str,
    pub enabled: bool,
    pub next_fire_at: DateTime<Utc>,
}

pub async fn insert_scheduled_trigger(
    pool: &PgPool,
    t: NewScheduledTrigger<'_>,
) -> Result<Uuid, sqlx::Error> { ... }

pub async fn delete_scheduled_trigger(
    pool: &PgPool,
    id: Uuid,
) -> Result<bool, sqlx::Error> {
    // Return true if a row was deleted, false if not found.
}
```

`triggers_list` can reuse B's Wave 4 `due_triggers` only if it's unbounded; more likely you need a thin new `list_scheduled_triggers(pool, limit)` — add that to db.rs too. Match the style of Wave 4's `due_triggers` / `mark_fired`.

### Approval → SMS wiring

Today in `sms_inbound`, after the allow-list check and BEFORE the existing `crate::agents::intent::classify(&message)` call, add this block (only runs when DB is configured and phone is non-empty):

```rust
// --- Approval intercept -------------------------------------------------
// A plain "y" / "yes" / "y <token>" resolves the sender's most recent
// pending job and does NOT go to any agent. Gated on DB being configured
// — without a pool we can't look up pending jobs anyway.
if let Some(pool) = cfg.db.as_ref() {
    if !phone_from.is_empty() {
        if let Some(kind) = crate::infra::approval::is_approval_message(&message) {
            match resolve_approval(pool, &phone_from, kind).await {
                ApprovalOutcome::Resolved { job_id } => {
                    let reply = format!("✓ approved — running job {job_id}");
                    let _ = crate::sms::send(&phone_from, &reply).await;
                    store_sms_reply(pool, &phone_from, &message, &reply).await;
                    // TODO(wave 6): actually kick the waiting job forward.
                    // For now just log; no agent re-dispatch path exists yet.
                    return ("200 OK", r#"{"status":"approved"}"#.to_owned());
                }
                ApprovalOutcome::NoMatch => {
                    // Token didn't match; fall through to normal dispatch.
                    // (Could also reply "no pending job found" — leave that
                    // UX choice for Isaac.)
                }
                ApprovalOutcome::DbError(e) => {
                    eprintln!("[ghost approval] lookup failed: {e}");
                    // Fall through — better to treat it as a chat message
                    // than to drop it on a flaky DB read.
                }
            }
        }
    }
}
```

Do NOT emoji in the reply if `sms.rs` can't handle unicode — match whatever other SMS paths do (if they use plain ASCII, drop the ✓ and use `"[ok] approved — running job {job_id}"`).

#### The `resolve_approval` helper

Lives in `daemon.rs` next to `sms_inbound`. Private. Wraps the `approval` module into a single call:

```rust
enum ApprovalOutcome {
    Resolved { job_id: uuid::Uuid },
    NoMatch,
    DbError(String),
}

async fn resolve_approval(
    pool: &sqlx::PgPool,
    phone: &str,
    kind: crate::infra::approval::ApprovalKind,
) -> ApprovalOutcome {
    use crate::infra::approval::{find_pending_for_contact, mark_job_approved, resolve_by_token, ApprovalKind};
    let pending = match kind {
        ApprovalKind::Plain => match find_pending_for_contact(pool, phone).await {
            Ok(p) => p,
            Err(e) => return ApprovalOutcome::DbError(e.to_string()),
        },
        ApprovalKind::Tokened(tok) => match resolve_by_token(pool, phone, &tok).await {
            Ok(p) => p,
            Err(e) => return ApprovalOutcome::DbError(e.to_string()),
        },
    };
    let Some(job) = pending else { return ApprovalOutcome::NoMatch; };
    match mark_job_approved(pool, job.id).await {
        Ok(()) => ApprovalOutcome::Resolved { job_id: job.id },
        Err(sqlx::Error::RowNotFound) => ApprovalOutcome::NoMatch,
        Err(e) => ApprovalOutcome::DbError(e.to_string()),
    }
}
```

#### `store_sms_reply`

If there's already a helper that writes an outbound reply into `sms_history`, reuse it. If not, keep the reply-logging inline — don't create a new helper this wave.

### Edge cases

- Approval message from a non-whitelisted number: already blocked upstream by the existing allow-list check — the approval block never runs, correct by construction.
- Approval message with no pending jobs: falls through to the existing dispatcher. Current behaviour ("y" becomes a chat to Haiku) is mildly weird but acceptable for this wave — fixing the UX is Wave 6.
- DB not configured: approval block skipped — falls through to existing dispatch path.
- Trigger `POST` with `next_fire_at < now()`: that's fine — the scheduler tick will pick it up on the next 30s boundary. Don't reject it.
- Trigger `DELETE /triggers/<bad-uuid>`: 400 `{"error":"invalid uuid"}` before the DB call.

### Unit tests (add where sensible)

`daemon.rs` already has integration-style tests in places; for THIS wave, inline tests are fine. At least:

1. `resolve_approval` with `ApprovalKind::Plain` and a non-existent phone returns `NoMatch` (mock the DB or skip the test if there's no existing in-memory DB harness — don't invent one).
2. `cron_expr` validation: feed `trigger_create` a bad expression and assert the response status string is `"400 Bad Request"`.

If the existing daemon tests require a running DB you don't have, it's acceptable to ship with only the cron-validation test + a unit test on any pure helper you extracted. Match the test style of the last daemon handler added.

---

## Constraints — do NOT

- Do NOT add new crate dependencies. `cron` (0.12) is already in Cargo.toml (Wave 4).
- Do NOT touch `infra/approval.rs` — you're consuming its public API.
- Do NOT touch `infra/scheduler.rs` — it already polls the table correctly.
- Do NOT modify `agents/intent.rs` or `agents/dispatcher.rs` — A owns both.
- Do NOT touch any other agent file.
- Do NOT add a new migration. The `scheduled_triggers` table (014) is enough.
- Do NOT add a "fire now" endpoint, manual dispatch, or admin-console route — trigger CRUD only.
- Do NOT attempt to actually resume a waiting job post-approval. Today the mark-approved is terminal — hooking it into agent continuation is Wave 6. The TODO comment makes that explicit.
- Do NOT loosen the auth on the trigger endpoints — they're bearer-gated like `/sms/send`.

---

## Implementation order

1. Append `db.rs` helpers: `NewScheduledTrigger` struct + `insert_scheduled_trigger` + `delete_scheduled_trigger` + `list_scheduled_triggers`.
2. Add three router arms + three handler fns in `daemon.rs`.
3. Add the approval pre-check block + `resolve_approval` helper + `ApprovalOutcome` enum in `daemon.rs`.
4. `cargo fmt && cargo clippy -p rusty-claude-cli --bins -- -D warnings && cargo test -p rusty-claude-cli --bins`.
5. All green → done.

---

## Verification

```bash
cd rust
cargo fmt
cargo clippy -p rusty-claude-cli --bins -- -D warnings
cargo test -p rusty-claude-cli --bins
```

All green. Pre-existing plugin-lifecycle failures don't count (see CLAUDE.md).

Manual smoke (optional, daemon + Postgres running):
```bash
curl -X POST http://127.0.0.1:7878/triggers \
  -H "Authorization: Bearer $GHOST_DAEMON_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name":"test","cron_expr":"0 * * * * *","agent":"chat_dispatcher","payload":"ping"}'
# Expected: 200 OK, body includes id + next_fire_at. Then within 60s+30s
# the scheduler should fire it and ghost_events should grow by one row.
```

Then from an SMS-allowed number, send `y`. If a pending job exists, you should see the confirmation reply.

---

## Done criteria

- `daemon.rs` has three new router arms (`GET/POST/DELETE /triggers`) and corresponding handlers.
- `daemon.rs` has an approval pre-check block in `sms_inbound` that intercepts `y` / `yes` before the agent dispatcher.
- `db.rs` has `NewScheduledTrigger` + `insert_scheduled_trigger` + `delete_scheduled_trigger` + `list_scheduled_triggers`.
- Scoped clippy green, scoped tests green.
- `POST /triggers` requires `Authorization: Bearer <GHOST_DAEMON_KEY>`.

---

## If you hit ambiguity

Stop and flag:
- If `crate::sms::send` has a different signature than `async fn send(to: &str, body: &str) -> Result<_, _>`, match reality.
- If `DaemonConfig.db` is `Option<Arc<PgPool>>` vs `Option<PgPool>`, dereference accordingly — look at how `schedule_create_handler` unwraps it.
- If `auth_matches` is named differently, use whatever the sibling handlers use.
- If any allow-listed agent name in your validation doesn't exist yet (e.g., `chief_of_staff` pre-A-merge), keep it in the allow-list anyway — rejecting valid future agent names would force Isaac to redeploy to add a trigger post-Wave-5.5.
- If B's `db.rs` append lands in the same hunk you're editing, standard git-merge append — both additions are pure new functions at end-of-file.
