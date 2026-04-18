# Phase 4: SMS Polish -- Markdown Stripping + HTML Full-Message Endpoint

## Goal

Two independent fixes for the SMS experience:

1. **Strip markdown formatting** from GHOST's replies before sending via SMS (only on the SMS path -- the dashboard and DB should keep raw markdown).
2. **Serve a mobile-friendly HTML page** at `GET /read/{job_id}` so that when a long response is truncated, the "read more" link opens a clean, readable page instead of raw JSON.

---

## Context: How SMS sending works today

- File: `rust/crates/rusty-claude-cli/src/sms.rs`
- `send_response(to, text, job_id)` calls `format_body(text, job_id)` which:
  - If `text.chars().count() < 500` -> sends as-is
  - Otherwise -> truncates to 497 chars + `"...\n{GHOST_BASE_URL}/jobs/{job_id}"`
- The `/jobs/{id}` endpoint (`daemon.rs`, around line 557-560) returns **raw JSON** -- not human-readable.
- The response text comes from `chat_dispatcher::dispatch()` or `director::handle()`, both of which return Claude's raw output including markdown (`**bold**`, `# headers`, `` `code` ``, etc.)

## Context: How the daemon serves responses

- File: `rust/crates/rusty-claude-cli/src/daemon.rs`
- The daemon is a raw TCP listener (not using a framework like Axum/Actix). Responses are written via a `write_response` helper that takes a status line, content-type, and body.
- Route matching happens in a big `match (method, path)` block around line 540-610.
- Existing endpoints return JSON with `Content-Type: application/json`.

---

## Task 1: Strip markdown from SMS responses

### What to strip (formatting characters only, NOT structural elements)

Remove these patterns from the text before it goes out as SMS:

| Pattern | Replacement | Example |
|---------|-------------|---------|
| `**text**` or `__text__` | `text` (just the inner text) | `**important**` -> `important` |
| `*text*` or `_text_` (italic) | `text` | `*note*` -> `note` |
| `` `code` `` (inline code) | `code` | `` `npm install` `` -> `npm install` |
| ```` ```lang ... ``` ```` (code blocks) | contents only, no fences | Keep the code, remove the ``` lines |
| `# Heading` through `###### Heading` | `Heading` (strip leading `#`s and space) | `## Overview` -> `Overview` |
| `[text](url)` (links) | `text (url)` or just `text url` | `[click here](https://...)` -> `click here https://...` |
| `![alt](url)` (images) | `alt` | Strip the image entirely, keep alt text |
| `> blockquote` | strip the `> ` prefix per line | |
| `---` or `***` (horizontal rules) | remove entirely | |

**Do NOT strip:**
- Bullet points (`- item` or `* item` at line start) -- these are readable in SMS
- Numbered lists (`1. item`) -- also fine
- Newlines / paragraph breaks -- preserve structure
- Em-dashes, ellipses, or other punctuation

### Where to add it

Create a function `strip_markdown(text: &str) -> String` in `sms.rs`. Call it inside `send_response()` on the text **before** passing it to `format_body()`. This way:
- The DB stores the raw markdown (for dashboard rendering)
- `sms_history` stores the raw markdown (for dashboard rendering)
- Only the actual SMS delivery gets the stripped version

Signature change for `send_response`:
```rust
// Before:
pub async fn send_response(to: &str, text: &str, job_id: &str) -> Result<(), String> {
    let body = format_body(text, job_id);
    // ...
}

// After:
pub async fn send_response(to: &str, text: &str, job_id: &str) -> Result<(), String> {
    let cleaned = strip_markdown(text);
    let body = format_body(&cleaned, job_id);
    // ...
}
```

**Important**: The `insert_sms_history` call in `daemon.rs:1139` stores `&send_text` -- this is the text *after* guard check but *before* SMS send. That should keep storing the original (possibly markdown) text. The stripping happens only inside `send_response` right before the actual network call. Review `daemon.rs:1136` to confirm `send_text` is passed to `send_response` and that the history insert at line 1139 happens independently. Currently both use `&send_text`, so the markdown will be stripped only in `send_response` and the history will keep the original -- correct behavior.

Wait -- actually look more carefully. `daemon.rs:1136` calls `sms::send_response(&phone_from, &send_text, &job_id_bg)` and then `daemon.rs:1139` calls `db::insert_sms_history(&pool_arc, &phone_from, "assistant", &send_text)`. Both use `&send_text`. Since `strip_markdown` happens inside `send_response`, the DB gets the original markdown. This is correct.

### Tests

Add a `#[cfg(test)] mod tests` block in `sms.rs` with cases:
- `strip_markdown("**bold**")` == `"bold"`
- `strip_markdown("normal text")` == `"normal text"`
- `strip_markdown("# Heading\n\nSome **bold** and *italic*.")` == `"Heading\n\nSome bold and italic."`
- `strip_markdown("Check `this` out")` == `"Check this out"`
- Mixed: a paragraph with headers, bold, code, links
- Bullet points survive: `"- item one\n- item two"` stays unchanged

---

## Task 2: HTML full-message endpoint

### New endpoint: `GET /read/{job_id}`

Add a new route in the `match` block in `daemon.rs`:

```rust
("GET", p) if p.starts_with("/read/") => {
    let id = &p["/read/".len()..];
    read_page(cfg.db.as_deref(), id).await
}
```

### The `read_page` handler

1. Look up the job by UUID from `director_jobs` table (same query as `job_get`).
2. If not found, return a 404 HTML page ("Message not found").
3. If found, extract the `result` field (which contains the full response text).
4. Return an HTML page with `Content-Type: text/html; charset=utf-8`.

### HTML template requirements

The HTML page must be:

- **Self-contained**: all CSS inline in a `<style>` tag, no external dependencies.
- **Mobile-first**: `<meta name="viewport" content="width=device-width, initial-scale=1">` so it's zoomable and scrollable on phones.
- **Dark theme by default**, with a toggle button at the top-right corner to switch to light theme.
- **No branding**: no "GHOST" header, no logos. Just the message text.
- The toggle should use a small sun/moon icon or just the text "light" / "dark". Persist the choice in `localStorage` so repeat visits remember.
- The message text should be rendered with basic markdown-to-HTML conversion (since the DB stores markdown). You can do a simple server-side conversion:
  - `**text**` -> `<strong>text</strong>`
  - `*text*` -> `<em>text</em>`
  - `` `code` `` -> `<code>code</code>`
  - `# Heading` -> `<h2>Heading</h2>` (use h2-h4, not h1)
  - Newlines -> `<br>` or `<p>` tags
  - Or: just wrap the text in `<pre style="white-space: pre-wrap;">` for simplicity. The pre-wrap approach is actually fine since it preserves formatting without needing a full markdown parser.

**Recommended approach**: Use `<pre style="white-space: pre-wrap; word-break: break-word;">` for the message body. This preserves the text structure without needing a markdown-to-HTML converter in Rust. The markdown will be somewhat readable even without rendering (headers show as `## Header`, bold as `**bold**`). This is the pragmatic path.

If you want to go further, write a simple `markdown_to_html(text: &str) -> String` function that handles the basics listed above. Put it in `sms.rs` or a new `render.rs` file.

### HTML template (dark theme)

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Message</title>
<style>
  :root { --bg: #0a0a0a; --text: #e0e0e0; --muted: #888; --border: #222; }
  .light { --bg: #fafafa; --text: #1a1a1a; --muted: #666; --border: #ddd; }
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    background: var(--bg); color: var(--text);
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    font-size: 15px; line-height: 1.7;
    padding: 20px; min-height: 100vh;
    transition: background 0.2s, color 0.2s;
  }
  .toggle {
    position: fixed; top: 12px; right: 12px;
    background: var(--border); border: none; color: var(--muted);
    padding: 6px 12px; border-radius: 6px; font-size: 12px;
    cursor: pointer; z-index: 10;
  }
  .toggle:hover { color: var(--text); }
  .content {
    max-width: 640px; margin: 40px auto 60px;
    white-space: pre-wrap; word-break: break-word;
    font-size: 15px; line-height: 1.7;
  }
  .meta {
    color: var(--muted); font-size: 11px;
    margin-top: 40px; padding-top: 12px;
    border-top: 1px solid var(--border);
  }
</style>
</head>
<body>
  <button class="toggle" onclick="toggleTheme()">light</button>
  <div class="content">{{MESSAGE_BODY}}</div>
  <div class="meta">Sent via GHOST</div>
  <script>
    const saved = localStorage.getItem('ghost-read-theme');
    if (saved === 'light') { document.body.classList.add('light'); document.querySelector('.toggle').textContent = 'dark'; }
    function toggleTheme() {
      const isLight = document.body.classList.toggle('light');
      document.querySelector('.toggle').textContent = isLight ? 'dark' : 'light';
      localStorage.setItem('ghost-read-theme', isLight ? 'light' : 'dark');
    }
  </script>
</body>
</html>
```

### Update the truncation URL

In `sms.rs`, change `format_body` to use `/read/` instead of `/jobs/`:

```rust
fn format_body(text: &str, job_id: &str) -> String {
    if text.chars().count() < RESPONSE_CHAR_LIMIT {
        return text.to_string();
    }
    let truncated: String = text.chars().take(TRUNCATE_AT).collect();
    let base_url =
        std::env::var("GHOST_BASE_URL").unwrap_or_else(|_| "https://ghost.app".to_string());
    // Changed from /jobs/ to /read/ -- serves HTML instead of JSON
    format!("{truncated}...\n{base_url}/read/{job_id}")
}
```

### Security note

The `/read/{id}` endpoint is **public** (no auth) -- this is intentional because SMS recipients need to open it without logging in. The job ID is a UUID, so it's unguessable. Do NOT add auth to this endpoint.

### HTML escaping

**Critical**: The message body is user-generated content (or AI-generated). Before inserting it into HTML, escape `<`, `>`, `&`, `"` to prevent XSS. Write a simple `html_escape(text: &str) -> String` function:

```rust
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
```

Call this on the message body before inserting into the template.

---

## Verification

1. `cargo fmt` from `rust/`
2. `cargo clippy -p rusty-claude-cli --bins -- -D warnings`
3. `cargo test -p rusty-claude-cli --bins` (existing tests + new `strip_markdown` tests)
4. Manual test: send a long message (>500 chars) via SMS, confirm the link goes to `/read/{id}` and renders a clean HTML page.
5. Manual test: send a short message with `**bold**` and `*italic*`, confirm the SMS arrives without formatting characters.

---

## Files touched

- `rust/crates/rusty-claude-cli/src/sms.rs` -- `strip_markdown()`, update `format_body` URL, update `send_response` to strip before sending
- `rust/crates/rusty-claude-cli/src/daemon.rs` -- add `/read/{id}` route + `read_page` handler + `html_escape` helper
- No migrations needed
- No dashboard changes
