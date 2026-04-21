//! Calendar agent — Google Calendar over raw HTTP.
//!
//! Wave 3 scope: List + Create work end-to-end; Update / Delete / Suggest
//! return clear "Wave 4" messages. This file intentionally does NOT register
//! itself with the dispatcher — that's a one-liner in Wave 3.5 after all
//! Wave 3 branches merge.
//!
//! Auth: `oauth::valid_access_token("google_calendar", <account>)`. Isaac
//! completes the OAuth consent flow out-of-band; the `refresh_token` is
//! stored via `oauth::save_refresh` before the agent can do anything useful.

use std::fmt::Write as _;

use async_trait::async_trait;
use serde_json::json;
use sqlx::PgPool;

use super::oauth;
use super::{Agent, AgentRequest, AgentResponse, ModelTier, Usage};
use crate::http_client::shared_client;

const CAL_BASE: &str = "https://www.googleapis.com/calendar/v3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarIntent {
    List,
    Create,
    Update,
    Delete,
    Suggest,
    Unknown,
}

/// Deterministic keyword classifier. Dumb on purpose — Wave 4 replaces this
/// with a Haiku-driven version if the misclassification rate warrants it.
pub fn classify_intent(msg: &str) -> CalendarIntent {
    let m = msg.to_lowercase();
    if m.contains("delete") || m.contains("cancel") || m.contains("remove") {
        return CalendarIntent::Delete;
    }
    if m.contains("suggest") || m.contains("when are you free") || m.contains("find time") {
        return CalendarIntent::Suggest;
    }
    if m.contains("update")
        || m.contains("change")
        || m.contains("move")
        || m.contains("reschedule")
    {
        return CalendarIntent::Update;
    }
    if m.contains("create")
        || m.contains("schedule")
        || m.contains("book")
        || m.contains("add to calendar")
    {
        return CalendarIntent::Create;
    }
    if m.contains("what")
        || m.contains("list")
        || m.contains("today")
        || m.contains("tomorrow")
        || m.contains("this week")
    {
        return CalendarIntent::List;
    }
    CalendarIntent::Unknown
}

pub struct Calendar {
    account: String,
}

impl Calendar {
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

impl Default for Calendar {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Agent for Calendar {
    fn name(&self) -> &'static str {
        "calendar"
    }

    fn declared_tier(&self) -> ModelTier {
        ModelTier::Fast
    }

    fn requires_approval(&self, req: &AgentRequest) -> bool {
        classify_intent(&req.message) == CalendarIntent::Delete
    }

    async fn handle(&self, req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String> {
        let token = oauth::valid_access_token(pool, "google_calendar", &self.account).await?;
        let intent = classify_intent(&req.message);
        let text = match intent {
            CalendarIntent::List => list_events(&token).await?,
            CalendarIntent::Create => create_event(&token, &req.message).await?,
            CalendarIntent::Update => update_event_stub(),
            CalendarIntent::Delete => delete_event_stub(),
            CalendarIntent::Suggest => suggest_time_stub(),
            CalendarIntent::Unknown => {
                return Err("could not understand calendar request".into());
            }
        };
        // Calendar ops are plain API calls, not LLM — zero token usage.
        Ok(AgentResponse {
            text,
            usage: Usage::default(),
            tier: ModelTier::Fast,
        })
    }
}

// ---------------------------------------------------------------------------
// LIST — GET /calendars/primary/events
// ---------------------------------------------------------------------------

async fn list_events(access_token: &str) -> Result<String, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let url = format!("{CAL_BASE}/calendars/primary/events");

    let resp = shared_client()
        .get(&url)
        .bearer_auth(access_token)
        .query(&[
            ("timeMin", now.as_str()),
            ("maxResults", "20"),
            ("singleEvents", "true"),
            ("orderBy", "startTime"),
        ])
        .send()
        .await
        .map_err(|e| format!("calendar list request failed: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("read calendar list body: {e}"))?;

    if !status.is_success() {
        return Err(format!("calendar list returned {status}: {body}"));
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("parse calendar list json: {e}"))?;
    let items = parsed
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if items.is_empty() {
        return Ok("No upcoming events.".to_string());
    }

    let mut out = String::from("Upcoming events:\n");
    for ev in items.iter().take(20) {
        let summary = ev
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("(no title)");
        let start = ev
            .get("start")
            .and_then(|s| {
                s.get("dateTime")
                    .and_then(|v| v.as_str())
                    .or_else(|| s.get("date").and_then(|v| v.as_str()))
            })
            .unwrap_or("?");
        let _ = writeln!(out, "- {start}: {summary}");
    }
    Ok(out.trim_end().to_string())
}

// ---------------------------------------------------------------------------
// CREATE — POST /calendars/primary/events
// ---------------------------------------------------------------------------

/// Build a POST body for `create_event` from a parsed (summary, start, end,
/// tz). Split out so we can unit-test the shape without any network.
pub(crate) fn build_create_body(
    summary: &str,
    start_iso: &str,
    end_iso: &str,
    tz: &str,
) -> serde_json::Value {
    json!({
        "summary": summary,
        "start": { "dateTime": start_iso, "timeZone": tz },
        "end":   { "dateTime": end_iso,   "timeZone": tz },
    })
}

/// Extremely limited NL parser. Accepts messages shaped like:
///
///   create "Team sync" at 2026-04-21T15:00:00-05:00 for 30m
///
/// If the shape doesn't match, returns an error string telling the caller
/// exactly what format is required. LLM-driven parsing lands in Wave 4.
fn parse_create_message(
    msg: &str,
) -> Result<(String, chrono::DateTime<chrono::FixedOffset>, i64), String> {
    let summary = extract_quoted(msg).ok_or_else(|| {
        "calendar create needs a quoted summary, e.g. create \"Team sync\" at <ISO datetime> for <duration>"
            .to_string()
    })?;

    let at_idx = msg.find(" at ").ok_or_else(|| {
        "calendar create needs: create \"<summary>\" at <ISO datetime> for <duration> (e.g. 30m, 1h)"
            .to_string()
    })?;
    let after_at = &msg[at_idx + 4..];

    let for_idx = after_at.find(" for ").ok_or_else(|| {
        "calendar create needs: create \"<summary>\" at <ISO datetime> for <duration> (e.g. 30m, 1h)"
            .to_string()
    })?;
    let start_raw = after_at[..for_idx].trim();
    let duration_raw = after_at[for_idx + 5..].trim();

    let start = chrono::DateTime::parse_from_rfc3339(start_raw)
        .map_err(|e| format!("could not parse start datetime '{start_raw}' as RFC 3339: {e}"))?;
    let duration_minutes = parse_duration_to_minutes(duration_raw)?;

    Ok((summary, start, duration_minutes))
}

fn extract_quoted(msg: &str) -> Option<String> {
    let first = msg.find('"')?;
    let rest = &msg[first + 1..];
    let close = rest.find('"')?;
    Some(rest[..close].to_string())
}

fn parse_duration_to_minutes(s: &str) -> Result<i64, String> {
    let s = s.trim();
    // Strip a trailing unit char if present.
    let (num_part, unit) = match s.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => (&s[..s.len() - 1], c),
        _ => (s, 'm'),
    };
    let n: i64 = num_part
        .trim()
        .parse()
        .map_err(|_| format!("duration '{s}' must look like '30m' or '1h'"))?;
    match unit.to_ascii_lowercase() {
        'm' => Ok(n),
        'h' => Ok(n * 60),
        other => Err(format!("unknown duration unit '{other}' — use 'm' or 'h'")),
    }
}

async fn create_event(access_token: &str, msg: &str) -> Result<String, String> {
    let (summary, start, minutes) = parse_create_message(msg)?;
    let end = start + chrono::Duration::minutes(minutes);
    let tz = "UTC"; // Local-offset preserved in the ISO string; UTC is safe as the label.
    let body = build_create_body(&summary, &start.to_rfc3339(), &end.to_rfc3339(), tz);

    let url = format!("{CAL_BASE}/calendars/primary/events");
    let resp = shared_client()
        .post(&url)
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("calendar create request failed: {e}"))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("read calendar create body: {e}"))?;

    if !status.is_success() {
        return Err(format!("calendar create returned {status}: {text}"));
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse create response: {e}"))?;
    let link = parsed
        .get("htmlLink")
        .and_then(|v| v.as_str())
        .unwrap_or("(no link)");
    Ok(format!(
        "Created '{summary}' at {} ({} min). {link}",
        start.to_rfc3339(),
        minutes
    ))
}

// ---------------------------------------------------------------------------
// Wave 4 stubs
// ---------------------------------------------------------------------------

fn update_event_stub() -> String {
    "calendar update is not yet implemented — Wave 4.".to_string()
}

fn delete_event_stub() -> String {
    "calendar delete is not yet implemented — Wave 4 wires approvals.".to_string()
}

fn suggest_time_stub() -> String {
    "calendar suggest-time is not yet implemented — Wave 4.".to_string()
}

// ---------------------------------------------------------------------------
// Tests — pure pieces only. Real Google calls need a sandbox.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_delete_variants() {
        for msg in ["delete that meeting", "cancel the 3pm", "remove my 10am"] {
            assert_eq!(classify_intent(msg), CalendarIntent::Delete, "msg={msg}");
        }
    }

    #[test]
    fn classify_suggest_variants() {
        for msg in [
            "suggest a time",
            "when are you free tomorrow",
            "find time for a sync",
        ] {
            assert_eq!(classify_intent(msg), CalendarIntent::Suggest, "msg={msg}");
        }
    }

    #[test]
    fn classify_update_variants() {
        for msg in [
            "update my 3pm",
            "change the meeting title",
            "move the 4pm to 5pm",
            "reschedule lunch",
        ] {
            assert_eq!(classify_intent(msg), CalendarIntent::Update, "msg={msg}");
        }
    }

    #[test]
    fn classify_create_variants() {
        for msg in [
            "create a meeting",
            "schedule lunch",
            "book the 3pm slot",
            "add to calendar: team sync",
        ] {
            assert_eq!(classify_intent(msg), CalendarIntent::Create, "msg={msg}");
        }
    }

    #[test]
    fn classify_list_variants() {
        for msg in [
            "what is on my calendar",
            "list events",
            "what's today",
            "anything tomorrow",
            "this week's meetings",
        ] {
            assert_eq!(classify_intent(msg), CalendarIntent::List, "msg={msg}");
        }
    }

    #[test]
    fn classify_unknown_falls_through() {
        assert_eq!(classify_intent("hello there"), CalendarIntent::Unknown);
        assert_eq!(classify_intent("tell me a joke"), CalendarIntent::Unknown);
    }

    #[test]
    fn classify_delete_beats_other_verbs() {
        // "delete" wins even if "schedule" also appears — Delete is the only
        // verb that triggers approval, so any ambiguity must resolve to Delete.
        assert_eq!(
            classify_intent("delete the scheduled sync"),
            CalendarIntent::Delete
        );
    }

    #[test]
    fn build_create_body_shape() {
        let body = build_create_body(
            "Team sync",
            "2026-04-21T15:00:00-05:00",
            "2026-04-21T15:30:00-05:00",
            "UTC",
        );
        assert_eq!(body["summary"], "Team sync");
        assert_eq!(body["start"]["dateTime"], "2026-04-21T15:00:00-05:00");
        assert_eq!(body["start"]["timeZone"], "UTC");
        assert_eq!(body["end"]["dateTime"], "2026-04-21T15:30:00-05:00");
    }

    #[test]
    fn parse_duration_minutes() {
        assert_eq!(parse_duration_to_minutes("30m").unwrap(), 30);
        assert_eq!(parse_duration_to_minutes("1h").unwrap(), 60);
        assert_eq!(parse_duration_to_minutes("2h").unwrap(), 120);
        assert!(parse_duration_to_minutes("weird").is_err());
        assert!(parse_duration_to_minutes("5x").is_err());
    }

    #[test]
    fn parse_create_message_ok() {
        let msg = r#"create "Team sync" at 2026-04-21T15:00:00-05:00 for 30m"#;
        let (summary, start, minutes) = parse_create_message(msg).expect("parse");
        assert_eq!(summary, "Team sync");
        assert_eq!(start.to_rfc3339(), "2026-04-21T15:00:00-05:00");
        assert_eq!(minutes, 30);
    }

    #[test]
    fn parse_create_message_missing_quotes_fails_clearly() {
        let msg = "create Team sync at 2026-04-21T15:00:00-05:00 for 30m";
        let err = parse_create_message(msg).expect_err("should fail");
        assert!(err.contains("quoted summary"), "got: {err}");
    }

    #[test]
    fn parse_create_message_missing_duration_fails_clearly() {
        let msg = r#"create "X" at 2026-04-21T15:00:00-05:00"#;
        let err = parse_create_message(msg).expect_err("should fail");
        assert!(
            err.contains("duration") || err.contains("for"),
            "got: {err}"
        );
    }

    #[test]
    fn requires_approval_only_for_delete() {
        use crate::agents::Source;
        let agent = Calendar::new();
        let make = |msg: &str| AgentRequest {
            message: msg.to_string(),
            history: Vec::new(),
            source: Source::Dashboard,
            job_id: "t".to_string(),
            sender_phone: None,
        };
        assert!(agent.requires_approval(&make("delete the 3pm")));
        assert!(!agent.requires_approval(&make("what's on my calendar")));
        assert!(!agent.requires_approval(&make("create a meeting")));
    }
}
