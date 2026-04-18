# Phase 3: Auto-Acknowledgment + Per-Contact Conversation History

## Goal
Two features that make GHOST feel like a real conversation partner instead of a slow, amnesiac bot:

1. **Auto-ack**: Send an immediate "One sec..." SMS when a message arrives, before the AI processes. Eliminates the 4-7 second dead silence.
2. **Conversation history**: Store recent SMS exchanges per phone number and load them as context, so GHOST remembers what was said earlier in the thread.

## Prerequisites
- Phase 1 (sender identity) must be complete — we use contact info to decide whether to ack.
- Phase 2 (outbound guard) must be complete — the guard sits between dispatch and send.

## Current Architecture

### SMS background task (daemon.rs)
After Phase 2, the background task in `sms_inbound()` looks roughly like:
```rust
tokio::spawn(async move {
    let result = crate::chat_dispatcher::dispatch(
        &process_msg, &[], &job_id_bg, Some(&pool_arc), Some(&phone_from)
    ).await;

    match result {
        Ok(text) => {
            let verdict = crate::guard::check(&process_msg, &text, &phone_from).await;
            // ... guard logic, then send_response ...
        }
        Err(e) => { /* ... */ }
    }
});
```

Key: the `&[]` empty slice is the conversation history. Currently always empty for SMS.

### Chat dispatcher (chat_dispatcher.rs)
`dispatch()` already accepts `history: &[serde_json::Value]` — a slice of `{role, content}` messages. It prepends them to the messages array before the current user message. This was built for the `/chat` HTTP endpoint which supports multi-turn. SMS just never uses it.

### SMS send helper (sms.rs)
`sms::send_response(to, text, job_id)` handles formatting and delivery. The auto-ack can use the same function with a short message, or call `send_via_gateway` directly to avoid the truncation logic.

### Database (db.rs)
Contains all DB functions. New table functions go here.

## What to Build

### Part A: Auto-Acknowledgment

#### Step A1: Add an ack helper in sms.rs

In `rust/crates/rusty-claude-cli/src/sms.rs`, add a simple function:

```rust
/// Send a brief acknowledgment SMS. Best-effort — failures are logged but
/// don't affect the main pipeline.
pub async fn send_ack(to: &str) {
    let body = "Got it -- one sec.";
    if let Err(e) = send_via_gateway(to, body).await {
        // Try Twilio as fallback
        if let Err(e2) = send_via_twilio(to, body).await {
            eprintln!("[ghost sms] ack failed (gateway: {e}, twilio: {e2})");
        }
    }
}
```

#### Step A2: Wire ack into SMS background task

In `daemon.rs`, at the TOP of the spawned async block (before dispatch), add the ack — but only for non-owner contacts:

```rust
tokio::spawn(async move {
    // Send immediate ack to non-owner contacts so they know the message was received.
    if !crate::contacts::is_owner(&phone_from) {
        crate::sms::send_ack(&phone_from).await;
    }

    // Load conversation history for this contact.
    // ... (Part B below)

    let result = if use_director {
        crate::director::handle(&process_msg, &job_id_bg, Some(&pool_arc)).await
    } else {
        crate::chat_dispatcher::dispatch(/* ... */).await
    };
    // ... rest of pipeline (guard, send) ...
});
```

**Why skip ack for owner (Isaac)?** Isaac knows GHOST is processing. Getting "Got it -- one sec." every time he texts himself would be annoying. Only strangers/contacts need the reassurance.

### Part B: Per-Contact Conversation History

#### Step B1: Database migration

Create a new migration file: `rust/migrations/006_sms_history.sql`

Migrations live at `rust/migrations/` (workspace root, not per-crate). Existing migrations: 001 through 005.

```sql
-- Per-contact SMS conversation history for multi-turn context.
CREATE TABLE IF NOT EXISTS sms_history (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    phone       TEXT NOT NULL,           -- E.164 normalized phone number
    role        TEXT NOT NULL,           -- 'user' (inbound) or 'assistant' (outbound)
    content     TEXT NOT NULL,           -- message text
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Index for fast lookups by phone number, ordered by time.
CREATE INDEX IF NOT EXISTS idx_sms_history_phone_time
    ON sms_history (phone, created_at DESC);

-- Auto-cleanup: keep only last 20 messages per number.
-- (Handled in application code, not a DB trigger, for simplicity.)
```

The migration macro in the daemon uses `sqlx::migrate!()` which reads from `rust/migrations/`.

#### Step B2: Database functions in db.rs

In `rust/crates/rusty-claude-cli/src/db.rs`, add these functions:

```rust
/// Store an SMS message in the conversation history.
pub async fn insert_sms_history(
    pool: &sqlx::PgPool,
    phone: &str,
    role: &str,  // "user" or "assistant"
    content: &str,
) {
    let normalized = crate::daemon::normalize_phone(phone);
    let result = sqlx::query(
        "INSERT INTO sms_history (phone, role, content) VALUES ($1, $2, $3)"
    )
    .bind(&normalized)
    .bind(role)
    .bind(content)
    .execute(pool)
    .await;

    if let Err(e) = result {
        eprintln!("[ghost db] failed to insert sms_history: {e}");
    }

    // Prune old messages — keep only the last 20 per phone number.
    let prune = sqlx::query(
        "DELETE FROM sms_history WHERE phone = $1 AND id NOT IN (
            SELECT id FROM sms_history WHERE phone = $1 ORDER BY created_at DESC LIMIT 20
        )"
    )
    .bind(&normalized)
    .execute(pool)
    .await;

    if let Err(e) = prune {
        eprintln!("[ghost db] failed to prune sms_history: {e}");
    }
}

/// Load recent SMS history for a phone number.
/// Returns messages in chronological order (oldest first) as {role, content} JSON values
/// suitable for passing to `dispatch()` as the history parameter.
/// Returns at most `limit` messages (default 6 = 3 exchanges).
pub async fn load_sms_history(
    pool: &sqlx::PgPool,
    phone: &str,
    limit: i64,
) -> Vec<serde_json::Value> {
    let normalized = crate::daemon::normalize_phone(phone);

    // Query newest N messages, then reverse to get chronological order.
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT role, content FROM sms_history
         WHERE phone = $1
         ORDER BY created_at DESC
         LIMIT $2"
    )
    .bind(&normalized)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.into_iter()
        .rev()  // reverse: oldest first
        .map(|(role, content)| serde_json::json!({"role": role, "content": content}))
        .collect()
}
```

#### Step B3: Wire history into SMS background task

In `daemon.rs`, modify the background task to load and store conversation history:

```rust
tokio::spawn(async move {
    // Send immediate ack to non-owner contacts.
    if !crate::contacts::is_owner(&phone_from) {
        crate::sms::send_ack(&phone_from).await;
    }

    // Store inbound message in conversation history.
    db::insert_sms_history(&pool_arc, &phone_from, "user", &process_msg).await;

    // Load recent conversation history for this contact (last 3 exchanges = 6 messages).
    let history = db::load_sms_history(&pool_arc, &phone_from, 6).await;

    let result = if use_director {
        crate::director::handle(&process_msg, &job_id_bg, Some(&pool_arc)).await
    } else {
        crate::chat_dispatcher::dispatch(
            &process_msg, &history, &job_id_bg, Some(&pool_arc), Some(&phone_from)
        ).await
    };

    match result {
        Ok(text) => {
            let verdict = crate::guard::check(&process_msg, &text, &phone_from).await;
            let (send_text, was_blocked) = match verdict {
                crate::guard::GuardVerdict::Allow => (text.clone(), false),
                crate::guard::GuardVerdict::Block(reason) => {
                    eprintln!(
                        "[ghost guard] blocked reply for job {job_id_bg}: {reason}\n\
                         [ghost guard] original reply was: {text}"
                    );
                    (crate::guard::BLOCKED_FALLBACK.to_string(), true)
                }
            };

            match crate::sms::send_response(&phone_from, &send_text, &job_id_bg).await {
                Ok(()) => {
                    // Store outbound reply in conversation history.
                    db::insert_sms_history(&pool_arc, &phone_from, "assistant", &send_text).await;

                    if was_blocked {
                        let blocked_note = format!(
                            "[BLOCKED BY GUARD]\nOriginal reply: {text}\nSent instead: {send_text}"
                        );
                        db::update_job_done(&pool_arc, &job_id_bg, &blocked_note).await;
                    } else {
                        db::update_job_done(&pool_arc, &job_id_bg, &send_text).await;
                    }
                }
                Err(e) => {
                    eprintln!("[ghost sms] send failed for job {job_id_bg}: {e}");
                    db::update_job_failed(&pool_arc, &job_id_bg, &e).await;
                }
            }
        }
        Err(e) => {
            eprintln!("[ghost] processing failed for job {job_id_bg}: {e}");
            db::update_job_failed(&pool_arc, &job_id_bg, &e).await;
        }
    }
});
```

Key changes from Phase 2:
1. `send_ack()` at the top (only for non-owner)
2. `insert_sms_history("user", &process_msg)` before dispatch
3. `load_sms_history()` → passed as `&history` to dispatch (was `&[]`)
4. `insert_sms_history("assistant", &send_text)` after successful send

### Part C: Update ghost-context.txt (optional but recommended)

Add a note about conversation history to `ghost-context.txt` so GHOST knows it has context:

After the "How to respond" section, add:
```
Conversation context

You may see prior messages in this thread. Use them for context -- don't repeat yourself or re-introduce yourself if you already did. If someone references something from earlier in the conversation, you should be able to follow along.
```

## Verification

1. `cargo fmt` from `rust/`
2. `cargo clippy -p rusty-claude-cli --bins -- -D warnings`
3. `cargo test -p rusty-claude-cli --bins`
4. `cargo build -p rusty-claude-cli`
5. After deploy:
   - Have a non-Isaac contact send a message → they should get "Got it -- one sec." followed by the real reply
   - Isaac sends a message → no ack, just the real reply
   - Send 3 messages in a row from the same number → GHOST should reference earlier messages in its replies
   - Check `/jobs` for history-aware responses
6. Verify the migration ran: check Railway logs for sqlx migration output at startup.

## Latency Impact
- Auto-ack: adds ~0.3-0.5s at the start (gateway send), but runs before dispatch so it's parallel from the user's perspective. The sender gets the ack immediately.
- History load: adds ~10-50ms (simple indexed query). Negligible.
- History store: two inserts per exchange, ~10ms each. Negligible and async.

## Files touched
- **EDIT:** `rust/crates/rusty-claude-cli/src/sms.rs` (add `send_ack()`)
- **NEW:** `rust/migrations/006_sms_history.sql`
- **EDIT:** `rust/crates/rusty-claude-cli/src/db.rs` (add `insert_sms_history`, `load_sms_history`)
- **EDIT:** `rust/crates/rusty-claude-cli/src/daemon.rs` (wire ack + history into background task)
- **EDIT:** `ghost-context.txt` (add conversation context note)
