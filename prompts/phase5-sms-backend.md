# Phase 5: SMS Backend -- History API, Auto-Reply Controls, Schedule Storage

## Goal

Add the daemon endpoints and DB tables that the dashboard SMS tab (Phase 6) will consume:

1. **SMS history endpoints** -- list contacts, load conversation history, send outbound SMS from dashboard
2. **Auto-reply toggle** -- per-contact on/off switch stored in DB, checked before processing inbound SMS
3. **Schedule/commitments storage** -- daily schedule boxes + persistent commitments, injected into GHOST's context for SMS replies

---

## Prerequisite

Phase 4 (markdown stripping + `/read/` endpoint) should be completed first, but this phase doesn't depend on its code -- just on the same codebase being stable.

---

## Context: Existing SMS infrastructure

### Database tables
- `sms_history` (migration 006): `id UUID`, `phone TEXT`, `role TEXT`, `content TEXT`, `created_at TIMESTAMPTZ`, `loadbearing BOOLEAN`
- Index on `(phone, created_at DESC)` for fast per-contact history

### Key files
- `rust/crates/rusty-claude-cli/src/daemon.rs` -- HTTP server, route matching at ~line 540-610, SMS inbound handler at ~line 882
- `rust/crates/rusty-claude-cli/src/db.rs` -- all Postgres queries, SMS functions at ~line 1280+
- `rust/crates/rusty-claude-cli/src/sms.rs` -- SMS sending (gateway + Twilio)
- `rust/crates/rusty-claude-cli/src/contacts.rs` -- contact registry from `GHOST_CONTACTS` env var
- `rust/crates/rusty-claude-cli/src/chat_dispatcher.rs` -- AI dispatch with memory injection

### How inbound SMS works (daemon.rs ~line 1050-1167)
1. Webhook arrives at `POST /sms/inbound`
2. Phone number extracted, normalized, checked against `GHOST_ALLOWED_NUMBERS`
3. Job created in `director_jobs` table
4. Background task spawned:
   - Stores inbound message in `sms_history`
   - Loads last 10 messages + loadbearing messages as conversation history
   - Calls `chat_dispatcher::dispatch()` with history + sender phone
   - Runs guard check on response
   - Sends SMS via `sms::send_response()`
   - Stores outbound reply in `sms_history`

### How contacts work (contacts.rs)
- `GHOST_CONTACTS` env var: `+1XXXXXXXXXX:Name:role,...`
- Roles: `owner` (Isaac), `allowed` (others)
- `sender_context(phone)` generates system prompt fragment describing who is texting
- Owner gets peer-to-peer mode; others get AI disclosure mode

---

## Task 1: New migration -- `008_sms_contacts.sql`

Create `rust/migrations/008_sms_contacts.sql`:

```sql
-- Per-contact SMS settings: auto-reply toggle, display name override.
CREATE TABLE IF NOT EXISTS sms_contacts (
    phone        TEXT PRIMARY KEY,
    display_name TEXT,
    auto_reply   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Schedule / commitments context injected into GHOST's SMS system prompt.
-- Two types:
--   'daily' -- applies to a specific day (e.g., "Math test 3rd period")
--   'persistent' -- always active until deleted (e.g., "Honors English Mon-Fri 8:00-9:30 AM")
CREATE TABLE IF NOT EXISTS sms_schedule (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    kind        TEXT NOT NULL CHECK (kind IN ('daily', 'persistent')),
    day_date    DATE,             -- required for 'daily', NULL for 'persistent'
    content     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sms_schedule_kind_date
    ON sms_schedule (kind, day_date);
```

**Design notes:**
- `sms_contacts.auto_reply` defaults to `FALSE` (off). User must explicitly enable per contact from the dashboard.
- `sms_contacts.display_name` overrides the name from `GHOST_CONTACTS` env var. NULL = use env var name.
- `sms_schedule.day_date` is a `DATE` (not timestamp). For `daily` entries, this is the specific calendar date. For `persistent`, it's NULL.
- `sms_schedule.content` is free-form text. Examples:
  - Daily: `"Math test 3rd period"` (day_date = 2026-04-18)
  - Persistent: `"Honors English Mon-Fri 8:00-9:30 AM, room 204"`

---

## Task 2: DB functions in `db.rs`

Add these functions to `rust/crates/rusty-claude-cli/src/db.rs`:

### SMS contacts

```rust
/// List all distinct phone numbers from sms_history, joined with sms_contacts
/// for display_name and auto_reply status. Returns contacts sorted by most
/// recent message.
pub async fn list_sms_contacts(pool: &PgPool) -> Vec<SmsContact> { ... }

/// Get or create a contact record. If the phone doesn't exist in sms_contacts,
/// inserts a default row (auto_reply=false, display_name=NULL).
pub async fn get_sms_contact(pool: &PgPool, phone: &str) -> Option<SmsContact> { ... }

/// Update auto_reply flag for a contact.
pub async fn set_auto_reply(pool: &PgPool, phone: &str, enabled: bool) { ... }

/// Update display_name for a contact.
pub async fn set_contact_name(pool: &PgPool, phone: &str, name: &str) { ... }

/// Check if auto_reply is enabled for a phone number.
/// Returns false if the phone is not in sms_contacts (default off).
pub async fn is_auto_reply_enabled(pool: &PgPool, phone: &str) -> bool { ... }
```

The `SmsContact` struct:
```rust
pub struct SmsContact {
    pub phone: String,
    pub display_name: Option<String>,
    pub auto_reply: bool,
    pub last_message_at: Option<chrono::DateTime<chrono::Utc>>,
    pub message_count: i64,
}
```

The `list_sms_contacts` query should be:
```sql
SELECT
    h.phone,
    c.display_name,
    COALESCE(c.auto_reply, FALSE) as auto_reply,
    MAX(h.created_at) as last_message_at,
    COUNT(*) as message_count
FROM sms_history h
LEFT JOIN sms_contacts c ON c.phone = h.phone
GROUP BY h.phone, c.display_name, c.auto_reply
ORDER BY MAX(h.created_at) DESC
```

### SMS history (paginated)

```rust
/// Load SMS history for a phone number with cursor-based pagination.
/// Returns messages in chronological order (oldest first within the page).
/// `before_id` is the UUID of the oldest message from the previous page -- load
/// messages older than this one. NULL = load most recent.
pub async fn load_sms_history_page(
    pool: &PgPool,
    phone: &str,
    limit: i64,
    before_id: Option<&str>,
) -> Vec<SmsMessage> { ... }
```

The `SmsMessage` struct:
```rust
pub struct SmsMessage {
    pub id: String,  // UUID as string
    pub phone: String,
    pub role: String,
    pub content: String,
    pub loadbearing: bool,
    pub created_at: String,  // ISO 8601
}
```

The paginated query:
```sql
-- Most recent page (no cursor):
SELECT id, phone, role, content, loadbearing, created_at
FROM sms_history
WHERE phone = $1
ORDER BY created_at DESC
LIMIT $2

-- Older page (with cursor):
SELECT id, phone, role, content, loadbearing, created_at
FROM sms_history
WHERE phone = $1 AND created_at < (SELECT created_at FROM sms_history WHERE id = $3::uuid)
ORDER BY created_at DESC
LIMIT $2
```

Reverse the results before returning so they're chronological (oldest first).

### Schedule / commitments

```rust
/// Load today's schedule context: all 'persistent' entries + 'daily' entries
/// for today's date. Returns a formatted string for system prompt injection.
pub async fn load_schedule_context(pool: &PgPool) -> String { ... }

/// List all schedule entries (for dashboard display).
/// Returns persistent entries first, then daily entries sorted by date desc.
pub async fn list_schedule_entries(pool: &PgPool) -> Vec<ScheduleEntry> { ... }

/// Add a schedule entry.
pub async fn insert_schedule(pool: &PgPool, kind: &str, day_date: Option<&str>, content: &str) -> Option<String> { ... }

/// Delete a schedule entry by ID.
pub async fn delete_schedule(pool: &PgPool, id: &str) -> bool { ... }
```

The `ScheduleEntry` struct:
```rust
pub struct ScheduleEntry {
    pub id: String,
    pub kind: String,        // "daily" or "persistent"
    pub day_date: Option<String>,  // "2026-04-18" or null
    pub content: String,
    pub created_at: String,
}
```

The `load_schedule_context` function should return a formatted string like:
```
## Isaac's current schedule
### Always (recurring)
- Honors English Mon-Fri 8:00-9:30 AM, room 204
- Baseball practice Tue/Thu 3:30-5:30 PM

### Today (Friday, April 18)
- Math test 3rd period
- Dentist appointment 4:00 PM
```

If there are no schedule entries, return an empty string (don't inject anything).

---

## Task 3: Daemon endpoints

Add these routes to the `match` block in `daemon.rs`:

### `GET /sms/contacts`
Returns the contact list with auto-reply status and message counts.

```json
{
  "contacts": [
    {
      "phone": "+11234567890",
      "display_name": "Mom",
      "auto_reply": true,
      "last_message_at": "2026-04-18T10:30:00Z",
      "message_count": 47
    }
  ]
}
```

Auth: **bearer** (protected -- only dashboard should access this).
503 if DB not configured.

### `GET /sms/history/{phone}?limit=30&before={id}`
Returns paginated message history for a contact.

The `{phone}` segment is URL-encoded (e.g., `%2B11234567890` for `+11234567890`). Decode it and normalize before querying.

```json
{
  "messages": [
    {
      "id": "uuid-here",
      "role": "user",
      "content": "hey what's up",
      "loadbearing": false,
      "created_at": "2026-04-18T10:28:00Z"
    },
    {
      "id": "uuid-here",
      "role": "assistant",
      "content": "Not much! What can I help you with?",
      "loadbearing": false,
      "created_at": "2026-04-18T10:28:05Z"
    }
  ],
  "has_more": true
}
```

`has_more` is `true` if the query returned exactly `limit` results (there may be older messages).

Auth: **bearer**.
Default limit: 30, max: 100.

### `POST /sms/contacts/{phone}/auto-reply`
Toggle auto-reply for a contact.

```json
// Request body:
{ "enabled": true }
```

If the contact doesn't exist in `sms_contacts`, create it first (upsert).

Auth: **bearer**.

### `PUT /sms/contacts/{phone}/name`
Update display name for a contact.

```json
// Request body:
{ "name": "Mom" }
```

Auth: **bearer**.

### `GET /schedule`
List all schedule entries.

```json
{
  "entries": [
    { "id": "uuid", "kind": "persistent", "day_date": null, "content": "Honors English Mon-Fri 8:00-9:30 AM" },
    { "id": "uuid", "kind": "daily", "day_date": "2026-04-18", "content": "Math test 3rd period" }
  ]
}
```

Auth: **bearer**.

### `POST /schedule`
Add a new schedule entry.

```json
// Request body:
{ "kind": "daily", "day_date": "2026-04-18", "content": "Math test 3rd period" }
// or:
{ "kind": "persistent", "content": "Honors English Mon-Fri 8:00-9:30 AM" }
```

Validate: `kind` must be `"daily"` or `"persistent"`. If `daily`, `day_date` is required.

Auth: **bearer**.
Returns: `{ "id": "new-uuid" }`

### `DELETE /schedule/{id}`
Delete a schedule entry.

Auth: **bearer**.

### `POST /sms/send` (update existing)
The existing `/sms/send` endpoint in `daemon.rs` (~line 1169) accepts `{"to":"+1...","body":"..."}`. Update it to:
1. **Require bearer auth** (it's currently open -- this is a security fix since it can send SMS as GHOST).
2. After sending, **store the outbound message** in `sms_history` with role `"assistant"`.
3. Return the message ID so the dashboard can append it to the thread.

Updated response:
```json
{ "status": "sent", "message_id": "uuid-of-stored-message" }
```

---

## Task 4: Auto-reply gate in SMS inbound handler

In `daemon.rs`, inside the `sms_inbound` handler (around line 1099, inside the `tokio::spawn` block), add an auto-reply check **after** storing the inbound message but **before** calling dispatch:

```rust
// Store inbound message in conversation history.
db::insert_sms_history(&pool_arc, &phone_from, "user", &process_msg).await;

// Check auto-reply setting. If disabled, store the message but don't respond.
if !db::is_auto_reply_enabled(&pool_arc, &phone_from).await {
    eprintln!("[ghost sms] auto-reply disabled for {phone_from}, storing message only");
    db::update_job_done(&pool_arc, &job_id_bg, "(auto-reply disabled)").await;
    return;  // exit the spawned task -- message is stored, no reply sent
}

// Load recent conversation history...
```

This means:
- The inbound message is **always stored** in `sms_history` (you can see it in the dashboard).
- GHOST only **replies** if `auto_reply = TRUE` for that contact.
- Default is `FALSE` (off), so new/unknown contacts get no auto-reply.
- The job is still created and marked done, so it's visible in the jobs list.

**Important**: The `GHOST_ALLOWED_NUMBERS` whitelist check (line ~1055) still runs first. If a number isn't in the allowed list, the message is rejected entirely (never stored). The auto-reply check is a second gate for numbers that ARE allowed but shouldn't get automatic responses.

---

## Task 5: Inject schedule context into SMS system prompt

In `chat_dispatcher.rs`, inside the `dispatch` function, after loading Bible context and before building the final system prompt, load and inject the schedule context:

```rust
// Schedule context: inject Isaac's current schedule so GHOST can tell people
// where he is and when he'll be available.
let schedule_context = if let Some(p) = pool {
    crate::db::load_schedule_context(p).await
} else {
    String::new()
};

// ... later, when building the system prompt:
if !schedule_context.is_empty() {
    system.push_str("\n\n");
    system.push_str(&schedule_context);
}
```

Place this injection **after** the sender identity block (`sender_context`) and **before** the memory notes. The schedule is factual context that should inform GHOST's replies about Isaac's availability.

The schedule context only matters for the SMS path (when `sender_phone` is `Some`), but injecting it unconditionally is fine -- it's just extra context that the dashboard chat can also use.

---

## Verification

1. `cargo fmt` from `rust/`
2. `cargo clippy -p rusty-claude-cli --bins -- -D warnings`
3. `cargo test -p rusty-claude-cli --bins`
4. Migration runs cleanly: check that `008_sms_contacts.sql` creates both tables without errors on a fresh DB.
5. Test the endpoints manually:
   - `GET /sms/contacts` returns contact list
   - `GET /sms/history/{phone}` returns paginated history
   - `POST /sms/contacts/{phone}/auto-reply` toggles the flag
   - `GET /schedule` / `POST /schedule` / `DELETE /schedule/{id}` CRUD works
6. Test the auto-reply gate: set auto_reply=false for a number, send an SMS, confirm GHOST stores it but doesn't reply.

---

## Files touched

- `rust/migrations/008_sms_contacts.sql` -- **new file**
- `rust/crates/rusty-claude-cli/src/db.rs` -- new structs + query functions
- `rust/crates/rusty-claude-cli/src/daemon.rs` -- new routes, auto-reply gate in `sms_inbound`, secure `/sms/send`
- `rust/crates/rusty-claude-cli/src/chat_dispatcher.rs` -- inject schedule context

## CLAUDE.md updates

After completing this phase, add to the endpoint table in CLAUDE.md:

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/sms/contacts` | bearer | List contacts with auto-reply status + message counts |
| GET | `/sms/history/{phone}` | bearer | Paginated message history for a contact |
| POST | `/sms/contacts/{phone}/auto-reply` | bearer | Toggle auto-reply. Body: `{"enabled": bool}` |
| PUT | `/sms/contacts/{phone}/name` | bearer | Set display name. Body: `{"name": "..."}` |
| GET | `/schedule` | bearer | List all schedule entries |
| POST | `/schedule` | bearer | Add entry. Body: `{"kind":"daily/persistent","day_date":"...","content":"..."}` |
| DELETE | `/schedule/{id}` | bearer | Delete a schedule entry |
