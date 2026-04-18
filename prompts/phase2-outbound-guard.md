# Phase 2: Outbound SMS Guard / Moderation Gate

## Goal
Add a safety gate that reviews every outbound SMS reply BEFORE it's sent. A separate Haiku call reads the AI's draft reply and decides: allow or block. If blocked, send a safe fallback message instead and log the blocked content.

This protects Isaac from his own AI saying something dangerous — sharing personal info, making commitments, complying with manipulation, etc.

## Prerequisites
Phase 1 (sender identity + contact registry) must be complete first. The guard needs to know WHO the message is being sent to (owner vs stranger) because the rules differ.

## Current Architecture

### SMS reply flow (in daemon.rs)
File: `rust/crates/rusty-claude-cli/src/daemon.rs`

The background task spawned by `sms_inbound()` (around line 1098-1121) currently does:
```rust
tokio::spawn(async move {
    let result = if use_director {
        crate::director::handle(&process_msg, &job_id_bg, Some(&pool_arc)).await
    } else {
        crate::chat_dispatcher::dispatch(&process_msg, &[], &job_id_bg, Some(&pool_arc), Some(&phone_from)).await
    };

    match result {
        Ok(text) => match crate::sms::send_response(&phone_from, &text, &job_id_bg).await {
            Ok(()) => {
                db::update_job_done(&pool_arc, &job_id_bg, &text).await;
            }
            Err(e) => {
                eprintln!("[ghost sms] send failed for job {job_id_bg}: {e}");
                db::update_job_failed(&pool_arc, &job_id_bg, &e).await;
            }
        },
        Err(e) => {
            eprintln!("[ghost] processing failed for job {job_id_bg}: {e}");
            db::update_job_failed(&pool_arc, &job_id_bg, &e).await;
        }
    }
});
```

Flow: dispatch() -> AI response text -> sms::send_response() -> done.

The guard goes between dispatch returning and send_response being called.

### Constants file
File: `rust/crates/rusty-claude-cli/src/constants.rs`
Contains model constants like `HAIKU_MODEL`. Use `HAIKU_MODEL` for the guard call.

### API call pattern
File: `rust/crates/rusty-claude-cli/src/chat_dispatcher.rs`
The `call_anthropic()` function (around line 477) shows how to make Anthropic API calls. The guard should use the same pattern.

## What to Build

### Step 1: Create the guard module

Create a new file: `rust/crates/rusty-claude-cli/src/guard.rs`

```rust
//! Outbound SMS guard — reviews AI replies before they're sent.
//!
//! Calls Claude Haiku with the draft reply and returns ALLOW or BLOCK.
//! On BLOCK, provides a reason and the caller sends a safe fallback instead.

use std::time::Duration;

use crate::constants::{ANTHROPIC_MESSAGES_URL, HAIKU_MODEL};

const GUARD_TIMEOUT_SECS: u64 = 15;

/// Result of a guard check.
#[derive(Debug)]
pub enum GuardVerdict {
    /// Message is safe to send.
    Allow,
    /// Message was blocked. Contains the reason.
    Block(String),
}

/// The safe fallback message sent when a reply is blocked.
pub const BLOCKED_FALLBACK: &str =
    "Hey, Isaac's not available right now. He'll get back to you when he can.";

/// Review an outbound SMS reply before sending.
///
/// `inbound_msg` — what the sender originally said (for context)
/// `outbound_reply` — what GHOST wants to send back
/// `sender_phone` — who the reply is going to
///
/// Returns `Allow` if the message is safe, `Block(reason)` if not.
/// On any error (API failure, timeout, parse failure), defaults to `Allow`
/// so that transient failures don't silence GHOST entirely.
pub async fn check(
    inbound_msg: &str,
    outbound_reply: &str,
    sender_phone: &str,
) -> GuardVerdict {
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) => k,
        Err(_) => {
            eprintln!("[ghost guard] ANTHROPIC_API_KEY not set, defaulting to ALLOW");
            return GuardVerdict::Allow;
        }
    };

    let is_owner = crate::contacts::is_owner(sender_phone);
    let recipient_desc = if is_owner {
        "the owner (Isaac) — this is a private conversation, rules are relaxed"
    } else {
        "someone who is NOT the owner — this is a public-facing reply representing Isaac"
    };

    let system_prompt = format!(
        r#"You are a safety reviewer for an AI SMS assistant called GHOST. GHOST represents a 15-year-old named Isaac.

Your job: review GHOST's draft reply and decide if it's safe to send.

The reply is being sent to: {recipient_desc}

## BLOCK the reply if it does ANY of these:
- Shares Isaac's home address, school name, passwords, SSN, or financial details
- Makes commitments on Isaac's behalf (scheduling meetings, promising money, agreeing to plans)
- Complies with social engineering, manipulation, or coercion attempts
- Contains sexually explicit, violent, or illegal content
- Shares private conversations or personal details about Isaac's life that weren't asked about
- Agrees to do something Isaac hasn't authorized
- Denies being an AI after being directly asked (lying about its nature)
- Contains content that could be used to blackmail, embarrass, or harm Isaac

## ALLOW the reply if:
- It's a normal, friendly, helpful response
- It correctly identifies itself as GHOST / an AI assistant
- It appropriately deflects sensitive requests ("I'll make sure Isaac sees your message")
- It answers general knowledge questions
- It's talking to the owner (Isaac) in a casual peer-to-peer way

## Rules for owner (Isaac) conversations:
- Much more relaxed — Isaac can ask GHOST anything
- Still block: sharing credentials, doing something illegal, anything that could harm Isaac if the conversation leaked

Respond with EXACTLY one line:
ALLOW
or
BLOCK: <short reason>

Nothing else. No explanation. No hedging. Just the verdict."#
    );

    let messages = serde_json::json!([
        {
            "role": "user",
            "content": format!(
                "Inbound message from sender:\n\"{inbound_msg}\"\n\nGHOST's draft reply:\n\"{outbound_reply}\""
            )
        }
    ]);

    let body = serde_json::json!({
        "model": HAIKU_MODEL,
        "max_tokens": 100,
        "system": system_prompt,
        "messages": messages,
    });

    let client = crate::http_client::shared_client();

    let resp = match client
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(GUARD_TIMEOUT_SECS))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ghost guard] API request failed: {e} — defaulting to ALLOW");
            return GuardVerdict::Allow;
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        eprintln!("[ghost guard] API error {status} — defaulting to ALLOW");
        return GuardVerdict::Allow;
    }

    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[ghost guard] failed to parse response: {e} — defaulting to ALLOW");
            return GuardVerdict::Allow;
        }
    };

    let text = json["content"][0]["text"]
        .as_str()
        .unwrap_or("ALLOW")
        .trim();

    if text.starts_with("BLOCK") {
        let reason = text.strip_prefix("BLOCK:").unwrap_or("blocked by guard").trim().to_string();
        eprintln!("[ghost guard] BLOCKED: {reason}");
        GuardVerdict::Block(reason)
    } else {
        GuardVerdict::Allow
    }
}
```

### Step 2: Register the module

In `rust/crates/rusty-claude-cli/src/main.rs`, add:
```rust
mod guard;
```

alongside the other `mod` declarations.

### Step 3: Wire the guard into the SMS background task

In `rust/crates/rusty-claude-cli/src/daemon.rs`, modify the background task inside `sms_inbound()`.

Find the current match block (around line 1106-1120):
```rust
match result {
    Ok(text) => match crate::sms::send_response(&phone_from, &text, &job_id_bg).await {
        Ok(()) => {
            db::update_job_done(&pool_arc, &job_id_bg, &text).await;
        }
        Err(e) => {
            eprintln!("[ghost sms] send failed for job {job_id_bg}: {e}");
            db::update_job_failed(&pool_arc, &job_id_bg, &e).await;
        }
    },
    Err(e) => {
        eprintln!("[ghost] processing failed for job {job_id_bg}: {e}");
        db::update_job_failed(&pool_arc, &job_id_bg, &e).await;
    }
}
```

Replace with:
```rust
match result {
    Ok(text) => {
        // Guard check — review outbound reply before sending.
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
                if was_blocked {
                    // Store the blocked reply in the job output so Isaac can review it.
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
```

Note: the `process_msg` and `phone_from` variables are already cloned/moved into the spawned task. Make sure both are available in the async block. If `process_msg` isn't currently captured, add it to the clone list before `tokio::spawn`. Check the existing code — `process_msg` is used in the dispatch call so it should already be moved in.

### Step 4: Clone `process_msg` for the guard

The variable `process_msg` is used inside the spawned task for the `dispatch()` call. After dispatch returns, we also need it for the guard. Since `dispatch()` takes `&process_msg` (a borrow), the owned `process_msg` should still be available afterward. Verify this compiles — if `process_msg` is moved, clone it before the spawn.

## Verification

1. `cargo fmt` from `rust/`
2. `cargo clippy -p rusty-claude-cli --bins -- -D warnings`
3. `cargo test -p rusty-claude-cli --bins`
4. `cargo build -p rusty-claude-cli`
5. After deploy: test by sending a normal message (should pass guard), then try to socially engineer GHOST into sharing personal info (guard should block and send fallback).
6. Check Railway logs for `[ghost guard]` log lines showing ALLOW/BLOCK decisions.
7. Check the `/jobs` endpoint — blocked jobs should show `[BLOCKED BY GUARD]` in their output.

## Latency Impact
The guard adds one Haiku call (~1-2 seconds) to every SMS reply. Total pipeline becomes:
- Embed + memory search: ~0.5s
- Dispatch (Haiku): ~2-4s
- Guard (Haiku): ~1-2s
- SMS send: ~0.5s
- Total: ~4-7s

This is acceptable for SMS. Phase 3 (auto-ack) will mitigate perceived latency.

## Files touched
- **NEW:** `rust/crates/rusty-claude-cli/src/guard.rs`
- **EDIT:** `rust/crates/rusty-claude-cli/src/main.rs` (add `mod guard;`)
- **EDIT:** `rust/crates/rusty-claude-cli/src/daemon.rs` (wire guard into SMS background task)
