//! Gerald Brain integration for `CheetahClaws`.
//!
//! Auto-loads memories at REPL startup by calling the MCP HTTP server,
//! and saves session insights at the end. The server URL is read from
//! `~/.claw/settings.json` (`gerald-brain` mcpServer entry).
//!
//! All network calls have a 4-second timeout so a cold Render start
//! doesn't block the REPL from launching.
use serde_json::{json, Value};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(4);
const DEFAULT_GERALD_URL: &str = "https://gerald-core-1.onrender.com/messages";
const GERALD_SERVER_KEY: &str = "gerald-brain";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Fetch a Gerald Brain overview and return it as a system-prompt block.
/// Returns `None` silently on any network/parse failure — the REPL still
/// starts, just without memory context.
pub fn load_context() -> Option<String> {
    let url = resolve_url();
    let client = build_client()?;

    let session_id = initialize(&client, &url)?;
    let overview = call_tool(&client, &url, &session_id, "get_overview", &json!({}))?;

    let text = extract_text(&overview)?;
    if text.trim().is_empty() {
        return None;
    }

    Some(format!(
        "## Gerald Brain — loaded memories\n\n{text}\n\n\
         (Use `mcp__gerald-brain__store_memory` to save new insights before this session ends.)"
    ))
}

/// Save a brief session summary to Gerald Brain.
/// Called by the REPL when the user exits (`/exit` or Ctrl-C).
/// Failures are silently ignored — this is best-effort.
pub fn save_session(working_dir: &str, session_id: &str, note: &str) {
    let url = resolve_url();
    let Some(client) = build_client() else { return };
    let Some(sid) = initialize(&client, &url) else {
        return;
    };

    let content = format!("Session ended in {working_dir} (id: {session_id}). {note}");

    let _ = call_tool(
        &client,
        &url,
        &sid,
        "store_memory",
        &json!({
            "content": content,
            "tags": ["session", "auto-save"],
        }),
    );
}

// ---------------------------------------------------------------------------
// MCP HTTP helpers
// ---------------------------------------------------------------------------

fn resolve_url() -> String {
    // Read from ~/.claw/settings.json if possible; fall back to default.
    if let Some(url) = read_url_from_settings() {
        return url;
    }
    DEFAULT_GERALD_URL.to_string()
}

fn read_url_from_settings() -> Option<String> {
    let config_home = crate::daemon::daemon_config_home_pub();
    let settings_path = config_home.join("settings.json");
    let raw = std::fs::read_to_string(settings_path).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    v["mcpServers"][GERALD_SERVER_KEY]["url"]
        .as_str()
        .map(str::to_string)
}

fn build_client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .ok()
}

/// MCP initialize handshake — returns a session id for subsequent calls.
/// The MCP HTTP transport may return a `Mcp-Session-Id` header after init.
fn initialize(client: &reqwest::blocking::Client, url: &str) -> Option<String> {
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "claw", "version": env!("CARGO_PKG_VERSION") }
        }
    });

    let resp = client.post(url).json(&init_req).send().ok()?;

    // Capture session id header if the server sends one
    let session_id = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body: Value = resp.json().ok()?;
    // Server must respond with a result (not an error) for init to succeed
    if body.get("error").is_some() {
        return None;
    }

    // Send initialized notification (fire-and-forget, ignore response)
    let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    let mut req = client.post(url).json(&notif);
    if !session_id.is_empty() {
        req = req.header("mcp-session-id", &session_id);
    }
    let _ = req.send();

    Some(session_id)
}

fn call_tool(
    client: &reqwest::blocking::Client,
    url: &str,
    session_id: &str,
    tool: &str,
    args: &Value,
) -> Option<Value> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": tool, "arguments": args }
    });

    let mut req = client.post(url).json(&body);
    if !session_id.is_empty() {
        req = req.header("mcp-session-id", session_id);
    }

    let resp: Value = req.send().ok()?.json().ok()?;

    if resp.get("error").is_some() {
        return None;
    }
    Some(resp["result"].clone())
}

fn extract_text(result: &Value) -> Option<String> {
    // MCP tool result: { content: [ { type: "text", text: "..." } ] }
    if let Some(arr) = result["content"].as_array() {
        let parts: Vec<&str> = arr
            .iter()
            .filter_map(|item| {
                if item["type"].as_str() == Some("text") {
                    item["text"].as_str()
                } else {
                    None
                }
            })
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    // Some servers return text directly
    result.as_str().map(str::to_string)
}
