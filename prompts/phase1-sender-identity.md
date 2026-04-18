# Phase 1: Sender Identity + Contact Registry

## Goal
GHOST (an AI SMS representative) currently has no idea who is texting it. The sender's phone number is extracted in `sms_inbound()` but never passed to the AI dispatch pipeline. This means GHOST can't distinguish Isaac (the owner) from a stranger, and the AI disclosure rules in the system prompt are useless.

Fix this by:
1. Parsing a contact registry from env vars
2. Passing sender identity into the chat dispatcher
3. Injecting a sender context block into the system prompt so GHOST knows who it's talking to

## Current Architecture

### SMS inbound flow
File: `rust/crates/rusty-claude-cli/src/daemon.rs`

The `sms_inbound()` function (around line 994) handles incoming SMS:
1. Extracts `message` and `phone_from` from the webhook payload
2. Checks `phone_from` against the `GHOST_ALLOWED_NUMBERS` whitelist
3. Creates a job in the DB
4. Spawns a background task that calls `dispatch(&process_msg, &[], &job_id_bg, Some(&pool_arc))`
5. `dispatch()` returns the AI response, which is sent back via `sms::send_response(&phone_from, &text, &job_id_bg)`

The problem: step 4 passes ONLY the message text. No sender info reaches the AI.

### Chat dispatcher
File: `rust/crates/rusty-claude-cli/src/chat_dispatcher.rs`

The `dispatch()` function signature (line 31):
```rust
pub async fn dispatch(
    message: &str,
    history: &[serde_json::Value],
    _job_id: &str,
    pool: Option<&sqlx::PgPool>,
) -> Result<String, String>
```

It builds a system prompt from:
- `ghost-context.txt` (core personality)
- Memory notes (semantic search)
- Scholar DB context
- Response cache
- Web search results
- Bible context

Then sends to Claude Haiku with cascade to Sonnet/Opus.

### GHOST's system prompt (ghost-context.txt)
Contains this section that currently doesn't work because GHOST can't identify senders:
```
AI disclosure

You are an AI, not a human. When someone texts who is NOT Isaac, always identify yourself in your first reply: "Hey, this is GHOST -- Isaac's AI assistant." Be friendly and helpful, but never pretend to be Isaac. If someone asks to speak to Isaac directly, say you'll make sure he sees their message. If someone asks what you are, be honest: you're an AI built by Isaac that handles his texts when he's unavailable.

When talking to Isaac himself, you don't need to re-introduce yourself -- he knows who you are.
```

### Current env var format
`GHOST_ALLOWED_NUMBERS` is a comma-separated list of E.164 phone numbers:
```
+19706375614,+19709399543
```

Isaac's number is `+19706375614`. The other number (`+19709399543`) is a relative.

## What to Build

### Step 1: Contact registry parser

Create a new file: `rust/crates/rusty-claude-cli/src/contacts.rs`

This module parses `GHOST_CONTACTS` env var and provides sender identity lookup.

**Env var format for `GHOST_CONTACTS`:**
```
+19706375614:Isaac:owner,+19709399543:Mom:allowed
```

Format: `number:name:role` where role is `owner` or `allowed`. Comma-separated.

If `GHOST_CONTACTS` is not set, fall back to `GHOST_ALLOWED_NUMBERS` — all numbers get role `allowed` with name `Unknown`, EXCEPT use a separate env var `GHOST_OWNER_NUMBER` to identify which number is the owner (Isaac).

**Module contents:**
```rust
//! Contact registry — maps phone numbers to names and roles.
//!
//! Primary source: `GHOST_CONTACTS` env var.
//! Format: `+1XXXXXXXXXX:Name:role,+1XXXXXXXXXX:Name:role`
//! Roles: `owner` (Isaac), `allowed` (whitelisted contact)
//!
//! Fallback: `GHOST_ALLOWED_NUMBERS` (all get role `allowed`, name `Unknown`)
//! with `GHOST_OWNER_NUMBER` to identify the owner.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct Contact {
    pub number: String,  // normalized E.164
    pub name: String,
    pub role: ContactRole,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContactRole {
    Owner,    // Isaac — peer-to-peer mode, no AI disclosure
    Allowed,  // Whitelisted contact — representative mode, AI disclosure
}

static CONTACTS: OnceLock<HashMap<String, Contact>> = OnceLock::new();

/// Initialize or return the contact registry. Parsed once, cached forever.
pub fn registry() -> &'static HashMap<String, Contact> {
    CONTACTS.get_or_init(|| {
        if let Ok(raw) = std::env::var("GHOST_CONTACTS") {
            parse_contacts(&raw)
        } else {
            parse_fallback()
        }
    })
}

/// Look up a sender by phone number. Returns None if not in registry.
pub fn lookup(phone: &str) -> Option<&'static Contact> {
    let normalized = crate::daemon::normalize_phone(phone);
    registry().get(&normalized)
}

/// Returns true if the given number belongs to the owner (Isaac).
pub fn is_owner(phone: &str) -> bool {
    lookup(phone).map_or(false, |c| c.role == ContactRole::Owner)
}

/// Build a system prompt fragment describing who is texting.
/// This gets injected into the chat dispatcher's system prompt.
pub fn sender_context(phone: &str) -> String {
    match lookup(phone) {
        Some(contact) if contact.role == ContactRole::Owner => {
            format!(
                "## Who is texting you right now\n\
                 This message is from {} -- your owner. Talk to him peer-to-peer.\n\
                 Do NOT introduce yourself or identify as AI -- he knows who you are.",
                contact.name
            )
        }
        Some(contact) => {
            format!(
                "## Who is texting you right now\n\
                 This message is from {} (phone: {}). They are NOT Isaac.\n\
                 You MUST identify yourself as GHOST, Isaac's AI assistant, in your first reply.\n\
                 Be friendly and helpful, but never pretend to be Isaac.\n\
                 If they ask to speak to Isaac directly, say you'll make sure he sees their message.",
                contact.name, contact.number
            )
        }
        None => {
            format!(
                "## Who is texting you right now\n\
                 This message is from an unknown number: {}. They are NOT Isaac.\n\
                 You MUST identify yourself as GHOST, Isaac's AI assistant.\n\
                 Be helpful but cautious with unknown contacts.\n\
                 Do not share any personal information about Isaac.",
                phone
            )
        }
    }
}

fn parse_contacts(raw: &str) -> HashMap<String, Contact> {
    let mut map = HashMap::new();
    for entry in raw.split(',') {
        let parts: Vec<&str> = entry.trim().splitn(3, ':').collect();
        if parts.is_empty() || parts[0].is_empty() {
            continue;
        }
        let number = crate::daemon::normalize_phone(parts[0]);
        let name = parts.get(1).copied().unwrap_or("Unknown").to_string();
        let role = match parts.get(2).copied().unwrap_or("allowed") {
            "owner" => ContactRole::Owner,
            _ => ContactRole::Allowed,
        };
        map.insert(number.clone(), Contact { number, name, role });
    }
    map
}

fn parse_fallback() -> HashMap<String, Contact> {
    let mut map = HashMap::new();
    let owner_num = std::env::var("GHOST_OWNER_NUMBER")
        .ok()
        .map(|n| crate::daemon::normalize_phone(&n));

    if let Ok(allowed) = std::env::var("GHOST_ALLOWED_NUMBERS") {
        for entry in allowed.split(',') {
            let number = crate::daemon::normalize_phone(entry.trim());
            if number.is_empty() { continue; }
            let is_owner = owner_num.as_deref() == Some(&number);
            let role = if is_owner { ContactRole::Owner } else { ContactRole::Allowed };
            let name = if is_owner { "Isaac".to_string() } else { "Unknown".to_string() };
            map.insert(number.clone(), Contact { number, name, role });
        }
    }
    map
}
```

### Step 2: Make `normalize_phone` public

In `daemon.rs`, the `normalize_phone()` function is currently used for whitelist checks. It needs to be `pub(crate)` so `contacts.rs` can call it.

Find the function at line 1414 (search for `fn normalize_phone`) and change from `fn normalize_phone` to `pub(crate) fn normalize_phone`.

### Step 3: Register the module

In `rust/crates/rusty-claude-cli/src/main.rs`, add:
```rust
mod contacts;
```

alongside the other `mod` declarations.

### Step 4: Modify `dispatch()` to accept sender context

In `rust/crates/rusty-claude-cli/src/chat_dispatcher.rs`, change the `dispatch()` function signature to accept an optional sender phone number:

**Old signature (line 31):**
```rust
pub async fn dispatch(
    message: &str,
    history: &[serde_json::Value],
    _job_id: &str,
    pool: Option<&sqlx::PgPool>,
) -> Result<String, String> {
```

**New signature:**
```rust
pub async fn dispatch(
    message: &str,
    history: &[serde_json::Value],
    _job_id: &str,
    pool: Option<&sqlx::PgPool>,
    sender_phone: Option<&str>,
) -> Result<String, String> {
```

Then, after the line `let mut system = core_context;` (around line 89), inject the sender context:

```rust
    let mut system = core_context;

    // Inject sender identity so GHOST knows who it's talking to.
    if let Some(phone) = sender_phone {
        let sender_block = crate::contacts::sender_context(phone);
        system.push_str("\n\n");
        system.push_str(&sender_block);
    }
```

This should go BEFORE the Bible study mode block and BEFORE memory context — it's the most important context for determining GHOST's behavior.

### Step 5: Update all call sites of `dispatch()`

Search for all calls to `crate::chat_dispatcher::dispatch(` and `dispatch(` in the codebase. Add `None` or the appropriate sender phone to each call.

**In `daemon.rs` — SMS inbound background task (around line 1103):**
```rust
// OLD:
crate::chat_dispatcher::dispatch(&process_msg, &[], &job_id_bg, Some(&pool_arc)).await

// NEW:
crate::chat_dispatcher::dispatch(&process_msg, &[], &job_id_bg, Some(&pool_arc), Some(&phone_from)).await
```

**In `daemon.rs` — POST /chat handler (line 966):**
```rust
// OLD:
crate::chat_dispatcher::dispatch(&process_msg, &history, &job_id, Some(pool_ref)).await

// NEW:
crate::chat_dispatcher::dispatch(&process_msg, &history, &job_id, Some(pool_ref), None).await
```

These are the only two call sites in the codebase.

### Step 6: Update GHOST_ALLOWED_NUMBERS in Railway

The whitelist check in `sms_inbound()` (line 1054) still reads `GHOST_ALLOWED_NUMBERS` for the allow/deny decision. This doesn't need to change — it continues to work as-is. The new `GHOST_CONTACTS` env var is additive.

Set in Railway:
```
GHOST_CONTACTS=+19706375614:Isaac:owner,+19709399543:Mom:allowed
```

Also set `GHOST_OWNER_NUMBER=+19706375614` as a fallback in case GHOST_CONTACTS isn't set.

## Verification

1. `cargo fmt` from `rust/`
2. `cargo clippy -p rusty-claude-cli --bins -- -D warnings`
3. `cargo test -p rusty-claude-cli --bins`
4. Check that the build succeeds: `cargo build -p rusty-claude-cli`
5. After deploy: send a test SMS from a non-Isaac number and verify GHOST introduces itself as AI. Send from Isaac's number and verify it talks peer-to-peer.

## Files touched
- **NEW:** `rust/crates/rusty-claude-cli/src/contacts.rs`
- **EDIT:** `rust/crates/rusty-claude-cli/src/main.rs` (add `mod contacts;`)
- **EDIT:** `rust/crates/rusty-claude-cli/src/daemon.rs` (make `normalize_phone` pub(crate), update dispatch call)
- **EDIT:** `rust/crates/rusty-claude-cli/src/chat_dispatcher.rs` (add `sender_phone` param, inject sender context)
