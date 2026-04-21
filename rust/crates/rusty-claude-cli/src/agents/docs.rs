//! Docs agent — Google Docs over raw HTTP.
//!
//! Wave 4 scope: Create + Read + Append work end-to-end; `insert_at_heading`,
//! `replace_text`, `delete_section` return clear "Wave 5" messages. This file
//! intentionally does NOT register itself with the dispatcher — that's a
//! one-liner in Wave 4.5 after all Wave 4 branches merge.
//!
//! Auth: `oauth::valid_access_token("google_docs", <account>)`. Reuses the
//! same `oauth_tokens` table as Calendar. Isaac completes the OAuth consent
//! flow out-of-band and stores the `refresh_token` via `oauth::save_refresh`
//! before the agent can do anything useful.

use async_trait::async_trait;
use serde_json::json;
use sqlx::PgPool;

use super::oauth;
use super::{Agent, AgentRequest, AgentResponse, ModelTier, Usage};
use crate::http_client::shared_client;

const DOCS_BASE: &str = "https://docs.googleapis.com/v1";
const READ_MAX_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocsIntent {
    Create,
    Read,
    Append,
    InsertAtHeading,
    ReplaceText,
    Delete,
    Unknown,
}

/// Deterministic keyword classifier. Dumb on purpose — mirrors Calendar's
/// approach. Order matters: more-specific / higher-risk verbs first so
/// approval-gated intents can't be shadowed by cheaper ones.
pub fn classify_intent(msg: &str) -> DocsIntent {
    let m = msg.to_lowercase();
    if m.contains("replace") || m.contains("find and replace") {
        return DocsIntent::ReplaceText;
    }
    if m.contains("delete") || m.contains("remove") {
        return DocsIntent::Delete;
    }
    if m.contains("insert") && m.contains("heading") {
        return DocsIntent::InsertAtHeading;
    }
    if m.contains("append") || m.contains("add to") {
        return DocsIntent::Append;
    }
    if m.contains("read") || m.contains("show me") || m.contains("what's in") {
        return DocsIntent::Read;
    }
    if m.contains("create") || m.contains("new doc") || m.contains("write a doc") {
        return DocsIntent::Create;
    }
    DocsIntent::Unknown
}

pub struct Docs {
    account: String,
}

impl Docs {
    pub fn new() -> Self {
        Self {
            account: "primary".to_string(),
        }
    }

    pub fn with_account(account: impl Into<String>) -> Self {
        Self {
            account: account.into(),
        }
    }
}

impl Default for Docs {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Agent for Docs {
    fn name(&self) -> &'static str {
        "docs"
    }

    fn declared_tier(&self) -> ModelTier {
        ModelTier::Fast
    }

    fn requires_approval(&self, req: &AgentRequest) -> bool {
        matches!(
            classify_intent(&req.message),
            DocsIntent::Delete | DocsIntent::ReplaceText
        )
    }

    async fn handle(&self, req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String> {
        let token = oauth::valid_access_token(pool, "google_docs", &self.account).await?;
        let intent = classify_intent(&req.message);
        let text = match intent {
            DocsIntent::Create => create_doc(&token, &req.message).await?,
            DocsIntent::Read => read_doc(&token, &req.message).await?,
            DocsIntent::Append => append_to_doc(&token, &req.message).await?,
            DocsIntent::InsertAtHeading => insert_at_heading_stub(),
            DocsIntent::ReplaceText => replace_text_stub(),
            DocsIntent::Delete => delete_stub(),
            DocsIntent::Unknown => {
                return Err("could not understand docs request".into());
            }
        };
        // Docs ops are plain API calls, not LLM — zero token usage.
        Ok(AgentResponse {
            text,
            usage: Usage::default(),
            tier: ModelTier::Fast,
        })
    }
}

// ---------------------------------------------------------------------------
// Shared parsing helpers
// ---------------------------------------------------------------------------

fn extract_quoted(msg: &str) -> Option<String> {
    let first = msg.find('"')?;
    let rest = &msg[first + 1..];
    let close = rest.find('"')?;
    Some(rest[..close].to_string())
}

/// Pull a Google Docs document ID out of `msg`. Accepts either a full
/// `/document/d/<id>` URL or a bare 20+ char `[A-Za-z0-9_-]` token.
pub(crate) fn extract_doc_id(msg: &str) -> Option<String> {
    const MARKER: &str = "/document/d/";
    if let Some(idx) = msg.find(MARKER) {
        let rest = &msg[idx + MARKER.len()..];
        let end = rest
            .char_indices()
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
            .map_or(rest.len(), |(i, _)| i);
        if end >= 20 {
            return Some(rest[..end].to_string());
        }
    }
    for token in msg.split_whitespace() {
        let clean =
            token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
        if clean.len() >= 20
            && clean
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Some(clean.to_string());
        }
    }
    None
}

/// Most-specific title-triggers first so "create a new doc about X" returns
/// "X" rather than "a new doc about X".
const CREATE_TITLE_TRIGGERS: &[&str] = &[
    "create a new doc about ",
    "create a new doc called ",
    "create a new doc titled ",
    "create a new doc ",
    "write a doc about ",
    "write a doc called ",
    "write a doc titled ",
    "write a doc ",
    "new doc about ",
    "new doc called ",
    "new doc titled ",
    "new doc ",
    "create a doc about ",
    "create a doc called ",
    "create a doc titled ",
    "create a doc ",
    "create doc about ",
    "create doc ",
    "create ",
];

/// Title for a new doc. Prefers a quoted string; otherwise strips a common
/// trigger prefix and uses the remainder; falls back to a timestamped label.
pub(crate) fn parse_create_title(msg: &str) -> String {
    if let Some(q) = extract_quoted(msg) {
        let trimmed = q.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    let lower = msg.to_lowercase();
    for t in CREATE_TITLE_TRIGGERS {
        if let Some(idx) = lower.find(t) {
            let after = msg[idx + t.len()..].trim();
            let after = after.trim_matches('"').trim();
            if !after.is_empty() {
                return after.to_string();
            }
        }
    }

    format!(
        "Untitled — {}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    )
}

/// Everything after `doc_id` in `msg`, past any URL path / query that
/// immediately followed the ID. Trimmed of whitespace.
pub(crate) fn extract_append_content(msg: &str, doc_id: &str) -> String {
    let Some(pos) = msg.find(doc_id) else {
        return String::new();
    };
    let after = &msg[pos + doc_id.len()..];
    // Skip URL continuation characters (/edit, ?tab=t.0, #bookmark, etc.)
    // up to the first whitespace — everything past that is real content.
    let mut split = 0usize;
    for c in after.chars() {
        if c.is_whitespace() {
            break;
        }
        split += c.len_utf8();
    }
    after[split..].trim().to_string()
}

// ---------------------------------------------------------------------------
// CREATE — POST /documents
// ---------------------------------------------------------------------------

async fn create_doc(access_token: &str, msg: &str) -> Result<String, String> {
    let title = parse_create_title(msg);
    let body = json!({ "title": title });
    let url = format!("{DOCS_BASE}/documents");

    let resp = shared_client()
        .post(&url)
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("docs create request failed: {e}"))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("read docs create body: {e}"))?;

    if !status.is_success() {
        return Err(format!("docs create returned {status}: {text}"));
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse docs create response: {e}"))?;
    let id = parsed
        .get("documentId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "docs create response missing documentId".to_string())?;

    Ok(format!(
        "Created doc: {title}\nhttps://docs.google.com/document/d/{id}/edit"
    ))
}

// ---------------------------------------------------------------------------
// READ — GET /documents/{documentId}
// ---------------------------------------------------------------------------

/// Walk the Docs `body.content[]` tree and concatenate every `textRun.content`
/// into a single plaintext string. Defensive against missing/shaped-other
/// structural elements (sectionBreak, table, tableOfContents).
pub(crate) fn extract_body_plaintext(doc: &serde_json::Value) -> String {
    let Some(content) = doc
        .get("body")
        .and_then(|b| b.get("content"))
        .and_then(|c| c.as_array())
    else {
        return String::new();
    };

    let mut out = String::new();
    for item in content {
        let Some(paragraph) = item.get("paragraph") else {
            continue;
        };
        let Some(elements) = paragraph.get("elements").and_then(|e| e.as_array()) else {
            continue;
        };
        for el in elements {
            if let Some(text) = el
                .get("textRun")
                .and_then(|tr| tr.get("content"))
                .and_then(|c| c.as_str())
            {
                out.push_str(text);
            }
        }
    }
    out
}

async fn read_doc(access_token: &str, msg: &str) -> Result<String, String> {
    let doc_id = extract_doc_id(msg).ok_or_else(|| {
        "no document ID found in message — paste a Docs URL or the bare ID".to_string()
    })?;

    let url = format!("{DOCS_BASE}/documents/{doc_id}");
    let resp = shared_client()
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("docs read request failed: {e}"))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("read docs get body: {e}"))?;

    if !status.is_success() {
        return Err(format!("docs read returned {status}: {text}"));
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse docs get response: {e}"))?;

    let plaintext = extract_body_plaintext(&parsed);
    if plaintext.is_empty() {
        return Ok(format!("(doc {doc_id} is empty)"));
    }

    if plaintext.chars().count() > READ_MAX_CHARS {
        let truncated: String = plaintext.chars().take(READ_MAX_CHARS).collect();
        Ok(format!("{truncated}\n… (truncated)"))
    } else {
        Ok(plaintext)
    }
}

// ---------------------------------------------------------------------------
// APPEND — POST /documents/{documentId}:batchUpdate
// ---------------------------------------------------------------------------

/// Build the batchUpdate body for an append. Split out for unit testing.
pub(crate) fn build_append_body(content: &str) -> serde_json::Value {
    json!({
        "requests": [
            {
                "insertText": {
                    "text": format!("\n{content}"),
                    "endOfSegmentLocation": {}
                }
            }
        ]
    })
}

async fn append_to_doc(access_token: &str, msg: &str) -> Result<String, String> {
    let doc_id = extract_doc_id(msg).ok_or_else(|| {
        "no document ID found in message — paste a Docs URL or the bare ID".to_string()
    })?;
    let content = extract_append_content(msg, &doc_id);
    if content.is_empty() {
        return Err("nothing to append — put the text to add after the doc ID".into());
    }

    let url = format!("{DOCS_BASE}/documents/{doc_id}:batchUpdate");
    let body = build_append_body(&content);
    let resp = shared_client()
        .post(&url)
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("docs append request failed: {e}"))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("read docs append body: {e}"))?;

    if !status.is_success() {
        return Err(format!("docs append returned {status}: {text}"));
    }

    Ok(format!("Appended {} chars to {doc_id}", content.len()))
}

// ---------------------------------------------------------------------------
// Wave 5 stubs
// ---------------------------------------------------------------------------

fn insert_at_heading_stub() -> String {
    "insert_at_heading lands in Wave 5".to_string()
}

fn replace_text_stub() -> String {
    "replace_text lands in Wave 5".to_string()
}

fn delete_stub() -> String {
    "doc deletion lands in Wave 5 — will require approval".to_string()
}

// ---------------------------------------------------------------------------
// Tests — pure pieces only. Real Google calls need a sandbox.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_create_variants() {
        for msg in [
            "create a new doc about rust lifetimes",
            "create a doc titled Team Notes",
            "new doc called weekly",
            "write a doc about the migration",
        ] {
            assert_eq!(classify_intent(msg), DocsIntent::Create, "msg={msg}");
        }
    }

    #[test]
    fn classify_read_variants() {
        for msg in [
            "read doc 1AbCdEfGhIjKlMnOpQrStUvWxYz",
            "show me doc 1AbCdEfGhIjKlMnOpQrStUvWxYz",
            "what's in doc 1AbCdEfGhIjKlMnOpQrStUvWxYz",
        ] {
            assert_eq!(classify_intent(msg), DocsIntent::Read, "msg={msg}");
        }
    }

    #[test]
    fn classify_append_variants() {
        for msg in [
            "append a new section to doc 1AbCdEf",
            "add to doc 1AbCdEf: closing thoughts",
        ] {
            assert_eq!(classify_intent(msg), DocsIntent::Append, "msg={msg}");
        }
    }

    #[test]
    fn classify_replace_and_delete_and_insert_heading() {
        assert_eq!(
            classify_intent("replace 'foo' with 'bar' in doc 1AbCdEf"),
            DocsIntent::ReplaceText
        );
        assert_eq!(
            classify_intent("delete the intro section of doc 1AbCdEf"),
            DocsIntent::Delete
        );
        assert_eq!(
            classify_intent("insert a new heading at the top"),
            DocsIntent::InsertAtHeading
        );
    }

    #[test]
    fn classify_unknown_falls_through() {
        assert_eq!(classify_intent("hello"), DocsIntent::Unknown);
        assert_eq!(classify_intent("tell me a joke"), DocsIntent::Unknown);
    }

    #[test]
    fn requires_approval_only_for_delete_and_replace() {
        use crate::agents::Source;
        let agent = Docs::new();
        let make = |msg: &str| AgentRequest {
            message: msg.to_string(),
            history: Vec::new(),
            source: Source::Dashboard,
            job_id: "t".to_string(),
            sender_phone: None,
        };
        assert!(agent.requires_approval(&make("delete doc 1AbCdEf")));
        assert!(agent.requires_approval(&make("replace 'x' with 'y' in doc 1AbCdEf")));
        assert!(!agent.requires_approval(&make("read doc 1AbCdEf")));
        assert!(!agent.requires_approval(&make("create a new doc about rust")));
        assert!(!agent.requires_approval(&make("append notes to doc 1AbCdEf")));
        assert!(!agent.requires_approval(&make("insert a heading at the top of doc 1AbCdEf")));
    }

    #[test]
    fn extract_doc_id_from_full_url() {
        let msg =
            "read https://docs.google.com/document/d/1aBcDeFgHiJkLmNoPqRsTuVwXyZ01234567/edit hi";
        assert_eq!(
            extract_doc_id(msg).as_deref(),
            Some("1aBcDeFgHiJkLmNoPqRsTuVwXyZ01234567")
        );
    }

    #[test]
    fn extract_doc_id_bare_token() {
        let msg = "read doc 1aBcDeFgHiJkLmNoPqRsTuVwXyZ01234567 please";
        assert_eq!(
            extract_doc_id(msg).as_deref(),
            Some("1aBcDeFgHiJkLmNoPqRsTuVwXyZ01234567")
        );
    }

    #[test]
    fn extract_doc_id_rejects_short_tokens() {
        assert!(extract_doc_id("read doc ABC123").is_none());
        assert!(extract_doc_id("what's on my calendar").is_none());
    }

    #[test]
    fn parse_create_title_prefers_quoted() {
        let title = parse_create_title(r#"create a doc titled "Team Sync Notes""#);
        assert_eq!(title, "Team Sync Notes");
    }

    #[test]
    fn parse_create_title_strips_specific_trigger() {
        // "create a new doc about " wins over bare "create " so we don't
        // return "a new doc about rust lifetimes".
        assert_eq!(
            parse_create_title("create a new doc about rust lifetimes"),
            "rust lifetimes"
        );
    }

    #[test]
    fn parse_create_title_falls_back_to_untitled() {
        let title = parse_create_title("");
        assert!(
            title.starts_with("Untitled"),
            "expected Untitled fallback, got: {title}"
        );
    }

    #[test]
    fn extract_append_content_past_url_suffix() {
        let msg = "append https://docs.google.com/document/d/1aBcDeFgHiJkLmNoPqRsTuVwXyZ01234567/edit closing thoughts here";
        let id = extract_doc_id(msg).expect("doc id");
        let content = extract_append_content(msg, &id);
        assert_eq!(content, "closing thoughts here");
    }

    #[test]
    fn extract_append_content_after_bare_id() {
        let msg = "add to doc 1aBcDeFgHiJkLmNoPqRsTuVwXyZ01234567 a quick update";
        let id = extract_doc_id(msg).expect("doc id");
        let content = extract_append_content(msg, &id);
        assert_eq!(content, "a quick update");
    }

    #[test]
    fn build_append_body_shape() {
        let body = build_append_body("hello world");
        assert_eq!(body["requests"][0]["insertText"]["text"], "\nhello world");
        assert!(
            body["requests"][0]["insertText"]["endOfSegmentLocation"].is_object(),
            "endOfSegmentLocation must be an empty object"
        );
    }

    #[test]
    fn extract_body_plaintext_concatenates_runs() {
        let doc = json!({
            "body": {
                "content": [
                    { "sectionBreak": {} },
                    {
                        "paragraph": {
                            "elements": [
                                { "textRun": { "content": "Hello " } },
                                { "textRun": { "content": "world.\n" } }
                            ]
                        }
                    },
                    {
                        "paragraph": {
                            "elements": [
                                { "textRun": { "content": "Second line.\n" } }
                            ]
                        }
                    },
                    { "table": {} }
                ]
            }
        });
        let text = extract_body_plaintext(&doc);
        assert_eq!(text, "Hello world.\nSecond line.\n");
    }

    #[test]
    fn extract_body_plaintext_handles_missing_body() {
        let doc = json!({ "documentId": "abc" });
        assert_eq!(extract_body_plaintext(&doc), "");
    }
}
