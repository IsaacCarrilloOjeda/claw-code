//! SMS send helper.
//!
//! Primary path: Android SMS Gateway (`GHOST_SMS_GATEWAY_URL`).
//! Fallback: Twilio (`TWILIO_ACCOUNT_SID`, `TWILIO_AUTH_TOKEN`, `TWILIO_FROM_NUMBER`).
//!
//! Response length rule:
//!   - < 500 chars → send as-is
//!   - ≥ 500 chars → truncate to 497 chars + "..." + newline + job URL

use std::time::Duration;

const SMS_TIMEOUT_SECS: u64 = 10;
const RESPONSE_CHAR_LIMIT: usize = 500;
const TRUNCATE_AT: usize = 497;

/// Format + send a response SMS. Returns `Ok(())` on delivery, `Err(reason)`
/// if both Gateway and Twilio fail.
pub async fn send_response(to: &str, text: &str, job_id: &str) -> Result<(), String> {
    let cleaned = strip_markdown(text);
    let body = format_body(&cleaned, job_id);
    match send_via_gateway(to, &body).await {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("[ghost sms] gateway failed ({e}), trying Twilio");
            send_via_twilio(to, &body).await
        }
    }
}

fn format_body(text: &str, job_id: &str) -> String {
    if text.chars().count() < RESPONSE_CHAR_LIMIT {
        return text.to_string();
    }
    let truncated: String = text.chars().take(TRUNCATE_AT).collect();
    let base_url =
        std::env::var("GHOST_BASE_URL").unwrap_or_else(|_| "https://ghost.app".to_string());
    format!("{truncated}...\n{base_url}/read/{job_id}")
}

async fn send_via_gateway(to: &str, body: &str) -> Result<(), String> {
    let url = std::env::var("GHOST_SMS_GATEWAY_URL")
        .map_err(|_| "GHOST_SMS_GATEWAY_URL not set".to_string())?;

    let payload = serde_json::json!({
        "message": body,
        "phoneNumbers": [to],
    });

    let client = crate::http_client::shared_client();

    let mut req = client
        .post(&url)
        .timeout(Duration::from_secs(SMS_TIMEOUT_SECS))
        .json(&payload);

    // Cloud API requires basic auth (GHOST_SMS_GATEWAY_USER / GHOST_SMS_GATEWAY_PASS).
    if let (Ok(user), Ok(pass)) = (
        std::env::var("GHOST_SMS_GATEWAY_USER"),
        std::env::var("GHOST_SMS_GATEWAY_PASS"),
    ) {
        req = req.basic_auth(&user, Some(&pass));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Gateway request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("Gateway returned HTTP {}", resp.status()))
    }
}

async fn send_via_twilio(to: &str, body: &str) -> Result<(), String> {
    let sid = std::env::var("TWILIO_ACCOUNT_SID")
        .map_err(|_| "TWILIO_ACCOUNT_SID not set".to_string())?;
    let auth =
        std::env::var("TWILIO_AUTH_TOKEN").map_err(|_| "TWILIO_AUTH_TOKEN not set".to_string())?;
    let from = std::env::var("TWILIO_FROM_NUMBER")
        .map_err(|_| "TWILIO_FROM_NUMBER not set".to_string())?;

    let url = format!("https://api.twilio.com/2010-04-01/Accounts/{sid}/Messages.json");
    let encoded = format!(
        "To={}&From={}&Body={}",
        percent_encode(to),
        percent_encode(&from),
        percent_encode(body),
    );

    let client = crate::http_client::shared_client();

    let resp = client
        .post(&url)
        .timeout(Duration::from_secs(SMS_TIMEOUT_SECS))
        .basic_auth(&sid, Some(&auth))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(encoded)
        .send()
        .await
        .map_err(|e| format!("Twilio request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("Twilio error {status}: {text}"))
    }
}

/// Percent-encode a string for `application/x-www-form-urlencoded`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            b' ' => out.push('+'),
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Markdown stripping (SMS delivery only — DB keeps raw markdown)
// ---------------------------------------------------------------------------

/// Strip markdown formatting from text for clean SMS delivery.
///
/// Removes bold, italic, inline code, code fences, headers, links, images,
/// blockquotes, and horizontal rules. Preserves bullet points, numbered
/// lists, newlines, and paragraph structure.
fn strip_markdown(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut in_code_block = false;

    for line in text.lines() {
        let trimmed = line.trim_start();

        // Toggle code blocks — keep contents, drop fences
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            lines.push(line.to_string());
            continue;
        }

        let trimmed_full = line.trim();

        // Horizontal rules (---, ***, ___): skip entirely
        if is_horizontal_rule(trimmed_full) {
            continue;
        }

        // Headers: strip leading #s and the following space
        let processed = if trimmed_full.starts_with('#') {
            let no_hashes = trimmed_full.trim_start_matches('#');
            if let Some(heading) = no_hashes.strip_prefix(' ') {
                heading.to_string()
            } else {
                line.to_string()
            }
        }
        // Blockquotes: strip > prefix
        else if let Some(rest) = trimmed_full.strip_prefix("> ") {
            rest.to_string()
        } else if trimmed_full == ">" {
            String::new()
        } else {
            line.to_string()
        };

        lines.push(strip_inline(&processed));
    }

    let mut result = lines.join("\n");
    if text.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// True if the line is a horizontal rule: 3+ identical chars from {-, *, _},
/// optionally with spaces between them.
fn is_horizontal_rule(line: &str) -> bool {
    let stripped: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    stripped.len() >= 3
        && (stripped.chars().all(|c| c == '-')
            || stripped.chars().all(|c| c == '*')
            || stripped.chars().all(|c| c == '_'))
}

/// Strip inline markdown formatting from a single line.
fn strip_inline(s: &str) -> String {
    let mut out = s.to_string();

    // 1. Inline code: `code` → code (first, so code content isn't further stripped)
    out = strip_paired(&out, "`", "`");

    // 2. Images: ![alt](url) → alt
    out = strip_images(&out);

    // 3. Links: [text](url) → text url
    out = strip_links(&out);

    // 4. Bold: **text** → text (before italic * to avoid partial matches)
    out = strip_paired(&out, "**", "**");

    // 5. Bold: __text__ → text (before italic _ to avoid partial matches)
    out = strip_paired(&out, "__", "__");

    // 6. Italic: *text* → text (preserve * bullet points at line start)
    if out.starts_with("* ") {
        let rest = &out[2..];
        out = format!("* {}", strip_paired(rest, "*", "*"));
    } else {
        out = strip_paired(&out, "*", "*");
    }

    // 7. Italic: _text_ → text
    out = strip_paired(&out, "_", "_");

    out
}

/// Remove matched delimiter pairs, keeping the inner text.
fn strip_paired(s: &str, open: &str, close: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;

    while let Some(start) = rest.find(open) {
        result.push_str(&rest[..start]);
        let after_open = &rest[start + open.len()..];
        if let Some(end) = after_open.find(close) {
            let inner = &after_open[..end];
            if !inner.is_empty() {
                result.push_str(inner);
            }
            rest = &after_open[end + close.len()..];
        } else {
            // No closing delimiter — keep the opening as-is
            result.push_str(open);
            rest = after_open;
        }
    }
    result.push_str(rest);
    result
}

/// `![alt](url)` → `alt`
fn strip_images(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;

    while let Some(start) = rest.find("![") {
        result.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(bracket_end) = after.find("](") {
            let alt = &after[..bracket_end];
            let after_bracket = &after[bracket_end + 2..];
            if let Some(paren_end) = after_bracket.find(')') {
                result.push_str(alt);
                rest = &after_bracket[paren_end + 1..];
                continue;
            }
        }
        // Not a valid image — keep the `![`
        result.push_str("![");
        rest = after;
    }
    result.push_str(rest);
    result
}

/// `[text](url)` → `text url`
fn strip_links(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;

    while let Some(start) = rest.find('[') {
        // Skip if preceded by ! (image — already handled)
        if start > 0 && rest.as_bytes().get(start - 1) == Some(&b'!') {
            result.push_str(&rest[..=start]);
            rest = &rest[start + 1..];
            continue;
        }
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(bracket_end) = after.find("](") {
            let text = &after[..bracket_end];
            let after_bracket = &after[bracket_end + 2..];
            if let Some(paren_end) = after_bracket.find(')') {
                let url = &after_bracket[..paren_end];
                result.push_str(text);
                result.push(' ');
                result.push_str(url);
                rest = &after_bracket[paren_end + 1..];
                continue;
            }
        }
        // Not a valid link — keep the [
        result.push('[');
        rest = after;
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_bold() {
        assert_eq!(strip_markdown("**bold**"), "bold");
    }

    #[test]
    fn strip_underline_bold() {
        assert_eq!(strip_markdown("__bold__"), "bold");
    }

    #[test]
    fn strip_italic_star() {
        assert_eq!(strip_markdown("*italic*"), "italic");
    }

    #[test]
    fn strip_inline_code() {
        assert_eq!(strip_markdown("Check `this` out"), "Check this out");
    }

    #[test]
    fn strip_heading_and_formatting() {
        assert_eq!(
            strip_markdown("# Heading\n\nSome **bold** and *italic*."),
            "Heading\n\nSome bold and italic."
        );
    }

    #[test]
    fn plain_text_unchanged() {
        assert_eq!(strip_markdown("normal text"), "normal text");
    }

    #[test]
    fn bullet_points_survive() {
        assert_eq!(
            strip_markdown("- item one\n- item two"),
            "- item one\n- item two"
        );
    }

    #[test]
    fn star_bullet_points_survive() {
        assert_eq!(
            strip_markdown("* item one\n* item two"),
            "* item one\n* item two"
        );
    }

    #[test]
    fn numbered_list_survives() {
        assert_eq!(strip_markdown("1. first\n2. second"), "1. first\n2. second");
    }

    #[test]
    fn code_block_strips_fences() {
        assert_eq!(strip_markdown("```rust\nlet x = 1;\n```"), "let x = 1;");
    }

    #[test]
    fn link_strips_to_text_and_url() {
        assert_eq!(
            strip_markdown("[click here](https://example.com)"),
            "click here https://example.com"
        );
    }

    #[test]
    fn image_strips_to_alt() {
        assert_eq!(strip_markdown("![screenshot](image.png)"), "screenshot");
    }

    #[test]
    fn blockquote_strips_prefix() {
        assert_eq!(strip_markdown("> quoted text"), "quoted text");
    }

    #[test]
    fn horizontal_rule_removed() {
        assert_eq!(strip_markdown("above\n\n---\n\nbelow"), "above\n\n\nbelow");
    }

    #[test]
    fn mixed_formatting() {
        let input = "## Overview\n\n\
                      This is **important** and *noteworthy*.\n\n\
                      Check `npm install` for details.\n\n\
                      [Click here](https://example.com) to learn more.\n\n\
                      ---\n\n\
                      > A wise quote";
        let expected = "Overview\n\n\
                        This is important and noteworthy.\n\n\
                        Check npm install for details.\n\n\
                        Click here https://example.com to learn more.\n\n\n\
                        A wise quote";
        assert_eq!(strip_markdown(input), expected);
    }
}
