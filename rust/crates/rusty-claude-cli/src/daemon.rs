//! Always-on HTTP daemon for GHOST.
//!
//! `claw daemon [--port N]` starts a lightweight HTTP/1.1 server that:
//!   - Survives across terminal sessions (intended for Railway / Task Scheduler / systemd)
//!   - Exposes health, status, session list, jobs, and director-config endpoints
//!   - Writes a PID file so external scripts can track the process
//!   - Serves as the backend for the dashboard UI
//!
//! Default port: 7878  (overridden by `PORT` env var on Railway)
//! PID file: `~/.claw/daemon.pid`
//!
//! ## Security model
//!
//! The daemon binds 127.0.0.1 by default. Localhost is **not** a security
//! boundary — a malicious webpage can hit it via fetch + DNS rebinding, and
//! any user-mode process on the box can trivially connect. So:
//!
//!   - `GET /health`, `/status`, `/sessions` are open on localhost.
//!   - `POST /prompt` ALWAYS requires `GHOST_DAEMON_KEY` AND the operator
//!     must pass `--allow-unsafe-prompt` at startup. Without both, the
//!     endpoint returns 403 Forbidden.
//!   - Every request's `Host` header is validated against the configured
//!     hostnames to block DNS rebinding attacks (bypassed when binding 0.0.0.0
//!     for cloud/Railway deployment where the platform handles TLS routing).
//!   - CORS uses an explicit allow-list (default <http://localhost:5173> for
//!     the dashboard dev server). Override with `GHOST_DAEMON_CORS_ORIGIN`.
use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::db;

const DEFAULT_DAEMON_PORT: u16 = 7878;
const PID_FILENAME: &str = "daemon.pid";
const LOG_PREFIX: &str = "[ghost daemon]";
const MAX_REQUEST_BYTES: usize = 1024 * 1024; // 1 MiB
const READ_CHUNK: usize = 8 * 1024;
/// Background health-check interval: reset circuit-breaker flags every 5 min.
const HEALTH_CHECK_INTERVAL_SECS: u64 = 300;
/// Confidence decay runs once every 24 hours.
const DECAY_INTERVAL_SECS: u64 = 86_400;
const DEFAULT_CORS_ORIGINS: &[&str] = &[
    "http://localhost:5173",
    "http://127.0.0.1:5173",
    "http://[::1]:5173",
];

pub fn default_daemon_port() -> u16 {
    DEFAULT_DAEMON_PORT
}

#[derive(Clone)]
struct DaemonConfig {
    host: String,
    port: u16,
    allow_unsafe_prompt: bool,
    /// Postgres connection pool. `None` when `DATABASE_URL` is not set.
    db: Option<Arc<PgPool>>,
}

/// Entry point called from `run()` in main.rs.
/// Blocks until the process is killed (SIGINT / Task Scheduler stop).
pub fn run_daemon(
    port: u16,
    host: &str,
    allow_unsafe_prompt: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let db = db::init_pool().await.map(Arc::new);
        let cfg = DaemonConfig {
            host: host.to_string(),
            port,
            allow_unsafe_prompt,
            db,
        };
        daemon_main(cfg).await
    })
}

async fn daemon_main(cfg: DaemonConfig) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(key) = std::env::var("GHOST_DAEMON_KEY") {
        if key.trim().is_empty() {
            return Err(
                "GHOST_DAEMON_KEY is set but empty — refusing to start. Either unset it or set a non-empty value.".into(),
            );
        }
    }

    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = TcpListener::bind(&addr).await.map_err(|e| {
        format!("ghost daemon: cannot bind {addr}: {e} — is another daemon already running?")
    })?;

    write_pid_file()?;

    // Spawn circuit-breaker health-reset task (resets primary/fallback_healthy every 5 min).
    if let Some(pool) = cfg.db.clone() {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS));
            interval.tick().await; // skip the immediate first tick
            loop {
                interval.tick().await;
                db::reset_health_flags(&pool).await;
                eprintln!("{LOG_PREFIX} circuit breaker: health flags reset");
            }
        });
    }

    // Spawn confidence decay task (runs once per day).
    // Reduces confidence by 5% on notes older than 30 days; expires notes below 0.1.
    if let Some(pool) = cfg.db.clone() {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(DECAY_INTERVAL_SECS));
            interval.tick().await; // skip the immediate first tick
            loop {
                interval.tick().await;
                db::decay_notes_confidence(&pool).await;
                eprintln!("{LOG_PREFIX} memory: confidence decay applied");
            }
        });
    }

    let start = Instant::now();
    eprintln!("{LOG_PREFIX} listening on http://{addr}");
    eprintln!("{LOG_PREFIX} PID {}", std::process::id());
    if cfg.allow_unsafe_prompt {
        if std::env::var("GHOST_DAEMON_KEY").is_ok() {
            eprintln!("{LOG_PREFIX} POST /prompt ENABLED (auth required)");
        } else {
            eprintln!(
                "{LOG_PREFIX} WARNING: --allow-unsafe-prompt set but GHOST_DAEMON_KEY is unset; /prompt will still refuse"
            );
        }
    } else {
        eprintln!("{LOG_PREFIX} POST /prompt disabled (pass --allow-unsafe-prompt to enable)");
    }
    if cfg.db.is_some() {
        eprintln!("{LOG_PREFIX} Postgres connected");
    } else {
        eprintln!("{LOG_PREFIX} Postgres not configured (DATABASE_URL unset) — /jobs and /director/* return 503");
    }
    eprintln!("{LOG_PREFIX} press Ctrl-C to stop");

    loop {
        let (mut stream, peer) = listener.accept().await?;
        let cfg = cfg.clone();
        let uptime = start.elapsed().as_secs();

        tokio::spawn(async move {
            let raw = match read_http_request(&mut stream).await {
                Ok(r) => r,
                Err(RequestError::TooLarge) => {
                    write_response(
                        &mut stream,
                        "413 Payload Too Large",
                        "{\"error\":\"request too large\"}",
                        None,
                    )
                    .await;
                    return;
                }
                Err(RequestError::Malformed | RequestError::Io) => {
                    write_response(
                        &mut stream,
                        "400 Bad Request",
                        "{\"error\":\"bad request\"}",
                        None,
                    )
                    .await;
                    return;
                }
            };

            let (status_line, body) = dispatch(&cfg, &raw, uptime).await;

            // Echo Origin only if it's in the allow-list
            let origin = header_value(&raw, "origin");
            let allowed_origin =
                origin.and_then(|o| if is_origin_allowed(&o) { Some(o) } else { None });

            write_response(&mut stream, status_line, &body, allowed_origin.as_deref()).await;
            eprintln!(
                "{LOG_PREFIX} {} {}",
                peer,
                status_line.split_once(' ').map_or("?", |(c, _)| c)
            );
        });
    }
}

// ---------------------------------------------------------------------------
// HTTP I/O
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum RequestError {
    TooLarge,
    Malformed,
    Io,
}

/// Read an HTTP/1.1 request: keep reading until we have headers AND, if a
/// `Content-Length` is declared, the full body. Cap total at `MAX_REQUEST_BYTES`.
async fn read_http_request(stream: &mut TcpStream) -> Result<String, RequestError> {
    let mut buf: Vec<u8> = Vec::with_capacity(READ_CHUNK);
    let mut chunk = [0u8; READ_CHUNK];

    // Read until we see header terminator
    let header_end = loop {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|_| RequestError::Io)?;
        if n == 0 {
            return Err(RequestError::Malformed);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_REQUEST_BYTES {
            return Err(RequestError::TooLarge);
        }
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        // Reasonable header size cap — stop pathological clients sending headers forever
        if buf.len() > 64 * 1024 {
            return Err(RequestError::Malformed);
        }
    };

    // Parse Content-Length from the headers we already have
    let header_block =
        std::str::from_utf8(&buf[..header_end]).map_err(|_| RequestError::Malformed)?;
    let content_length = header_block
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("content-length:") {
                line[15..].trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    if content_length > MAX_REQUEST_BYTES {
        return Err(RequestError::TooLarge);
    }

    // header_end points at the first byte of "\r\n\r\n"; body starts after the 4 bytes
    let body_start = header_end + 4;
    let target_total = body_start + content_length;

    while buf.len() < target_total {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|_| RequestError::Io)?;
        if n == 0 {
            break; // client hung up; let downstream JSON parse fail with a useful error
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_REQUEST_BYTES {
            return Err(RequestError::TooLarge);
        }
    }

    String::from_utf8(buf).map_err(|_| RequestError::Malformed)
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

async fn write_response(
    stream: &mut TcpStream,
    status_line: &str,
    body: &str,
    allowed_origin: Option<&str>,
) {
    let mut headers = format!(
        "HTTP/1.1 {status_line}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n",
        body.len()
    );
    if let Some(origin) = allowed_origin {
        use std::fmt::Write as _;
        let _ = write!(headers, "Access-Control-Allow-Origin: {origin}\r\n");
        headers.push_str("Vary: Origin\r\n");
        headers.push_str("Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n");
        headers
            .push_str("Access-Control-Allow-Headers: Content-Type, Authorization, X-Claw-Key\r\n");
        headers.push_str("Access-Control-Max-Age: 86400\r\n");
        // Chrome Private Network Access opt-in (localhost:5173 -> 127.0.0.1:7878)
        headers.push_str("Access-Control-Allow-Private-Network: true\r\n");
    }
    headers.push_str("Connection: close\r\n\r\n");
    headers.push_str(body);

    if let Err(e) = stream.write_all(headers.as_bytes()).await {
        eprintln!("{LOG_PREFIX} write error: {e}");
    }
    let _ = stream.shutdown().await;
}

// ---------------------------------------------------------------------------
// Header / origin helpers
// ---------------------------------------------------------------------------

fn header_value(raw: &str, name_lower: &str) -> Option<String> {
    let prefix = format!("{name_lower}:");
    for line in raw.lines() {
        if line.is_empty() || line.starts_with(' ') {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix(&prefix) {
            // Use the offset into the original (case-preserved) line
            let offset = prefix.len();
            if line.len() >= offset {
                return Some(line[offset..].trim().to_string());
            }
            // Fall back to the lowercased value if offsets misalign (shouldn't with ASCII)
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn allowed_cors_origins() -> Vec<String> {
    let mut origins: Vec<String> = DEFAULT_CORS_ORIGINS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    if let Ok(extra) = std::env::var("GHOST_DAEMON_CORS_ORIGIN") {
        for o in extra.split(',') {
            let trimmed = o.trim();
            if !trimmed.is_empty() {
                origins.push(trimmed.to_string());
            }
        }
    }
    origins
}

fn is_origin_allowed(origin: &str) -> bool {
    allowed_cors_origins().iter().any(|o| o == origin)
}

/// Reject Host headers that don't match the daemon's bind address. Blocks
/// DNS rebinding: a malicious webpage that resolves evil.com → 127.0.0.1
/// will still send `Host: evil.com`, which we drop.
///
/// When bound to 0.0.0.0 (Railway / cloud deployment behind a reverse proxy),
/// any Host value is accepted — DNS rebinding is not a threat model in that
/// environment because the platform controls inbound TLS routing.
fn host_allowed(cfg: &DaemonConfig, raw: &str) -> bool {
    // 0.0.0.0: intentional public bind (Railway etc.) — accept all hosts.
    if cfg.host == "0.0.0.0" {
        return true;
    }

    let Some(host_header) = header_value(raw, "host") else {
        return false;
    };
    // Strip optional :port
    let host_only = host_header.split(':').next().unwrap_or("");

    // If the operator bound a specific non-loopback host, match it exactly.
    if cfg.host != "127.0.0.1" {
        return host_only == cfg.host;
    }

    // Default localhost bind: only accept loopback names.
    matches!(host_only, "127.0.0.1" | "localhost" | "[::1]" | "::1")
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// Returns `Some(key)` if a `GHOST_DAEMON_KEY` is configured (non-empty).
fn configured_key() -> Option<String> {
    let key = std::env::var("GHOST_DAEMON_KEY").ok()?;
    let trimmed = key.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn extract_bearer_or_claw_key(raw: &str) -> Option<String> {
    for line in raw.lines() {
        if let Some(val) = strip_prefix_ci(line, "authorization:") {
            let trimmed = val.trim();
            let token = trimmed.strip_prefix("Bearer ").unwrap_or(trimmed);
            return Some(token.trim().to_string());
        }
        if let Some(val) = strip_prefix_ci(line, "x-claw-key:") {
            return Some(val.trim().to_string());
        }
    }
    None
}

fn strip_prefix_ci<'a>(line: &'a str, prefix_lower: &str) -> Option<&'a str> {
    if line.len() < prefix_lower.len() {
        return None;
    }
    let (head, tail) = line.split_at(prefix_lower.len());
    if head.eq_ignore_ascii_case(prefix_lower) {
        Some(tail)
    } else {
        None
    }
}

/// Constant-time string comparison.
fn ct_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// True iff a key is configured AND the request presents the matching key.
fn auth_matches(raw: &str) -> bool {
    let Some(required) = configured_key() else {
        return false;
    };
    let Some(presented) = extract_bearer_or_claw_key(raw) else {
        return false;
    };
    ct_eq(&presented, &required)
}

// ---------------------------------------------------------------------------
// Request routing
// ---------------------------------------------------------------------------

async fn dispatch(cfg: &DaemonConfig, raw: &str, uptime_secs: u64) -> (&'static str, String) {
    let first_line = raw.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();

    let (method, path) = match parts.as_slice() {
        [m, p, ..] => (*m, *p),
        _ => return ("400 Bad Request", r#"{"error":"bad request"}"#.to_owned()),
    };

    let path_clean = path.split('?').next().unwrap_or(path);

    // Reject DNS-rebinding attempts
    if !host_allowed(cfg, raw) {
        return (
            "421 Misdirected Request",
            r#"{"error":"host header not allowed"}"#.to_owned(),
        );
    }

    match (method, path_clean) {
        ("GET", "/health") => health(uptime_secs, cfg.db.is_some()),
        ("GET", "/status") => status(uptime_secs, cfg.db.as_deref()),
        ("GET", "/sessions") => sessions(),
        ("GET", "/jobs") => jobs_list(cfg.db.as_deref()).await,
        ("GET", p) if p.starts_with("/jobs/") => {
            let id = &p["/jobs/".len()..];
            job_get(cfg.db.as_deref(), id).await
        }
        ("GET", "/director/config") => director_config_get(cfg.db.as_deref()).await,
        ("POST", "/director/config") => {
            if !auth_matches(raw) {
                return (
                    "401 Unauthorized",
                    r#"{"error":"missing or invalid API key"}"#.to_owned(),
                );
            }
            director_config_update(cfg.db.as_deref(), raw).await
        }
        ("POST", "/prompt") => {
            if !cfg.allow_unsafe_prompt {
                return (
                    "403 Forbidden",
                    r#"{"error":"prompt endpoint disabled; start daemon with --allow-unsafe-prompt"}"#.to_owned(),
                );
            }
            if !auth_matches(raw) {
                return (
                    "401 Unauthorized",
                    r#"{"error":"missing or invalid API key"}"#.to_owned(),
                );
            }
            run_prompt(raw).await
        }
        ("GET", "/memories") => memories_list(cfg.db.as_deref()).await,
        ("DELETE", p) if p.starts_with("/memories/") => {
            let id = &p["/memories/".len()..];
            memory_delete(cfg.db.as_deref(), raw, id).await
        }
        ("POST", "/chat") => chat_handler(cfg, raw).await,
        ("POST", "/sms/inbound") => sms_inbound(cfg, raw).await,
        ("POST", "/sms/send") => sms_send_handler(raw).await,
        ("OPTIONS", _) => options_preflight(),
        _ => ("404 Not Found", r#"{"error":"not found"}"#.to_owned()),
    }
}

fn health(uptime_secs: u64, db_connected: bool) -> (&'static str, String) {
    let body = json!({
        "status": "ok",
        "uptime_secs": uptime_secs,
        "pid": std::process::id(),
        "db_connected": db_connected,
    })
    .to_string();
    ("200 OK", body)
}

fn status(uptime_secs: u64, db: Option<&PgPool>) -> (&'static str, String) {
    let cwd =
        std::env::current_dir().map_or_else(|_| "unknown".to_string(), |p| p.display().to_string());

    let config_home = daemon_config_home();

    let body = json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "uptime_secs": uptime_secs,
        "cwd": cwd,
        "config_home": config_home.display().to_string(),
        "session_count": count_sessions(&config_home),
        "db_connected": db.is_some(),
    })
    .to_string();
    ("200 OK", body)
}

// ---------------------------------------------------------------------------
// Job endpoints
// ---------------------------------------------------------------------------

async fn jobs_list(db: Option<&PgPool>) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let jobs = db::list_jobs(pool, 100).await;
    let body = json!({ "jobs": jobs }).to_string();
    ("200 OK", body)
}

async fn job_get(db: Option<&PgPool>, id: &str) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    match db::get_job(pool, id).await {
        Some(job) => {
            let body = json!(job).to_string();
            ("200 OK", body)
        }
        None => ("404 Not Found", r#"{"error":"job not found"}"#.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// Director config endpoints
// ---------------------------------------------------------------------------

async fn director_config_get(db: Option<&PgPool>) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    match db::get_director_config(pool).await {
        Some(cfg) => {
            let body = json!(cfg).to_string();
            ("200 OK", body)
        }
        None => (
            "500 Internal Server Error",
            r#"{"error":"director config missing"}"#.to_owned(),
        ),
    }
}

async fn director_config_update(db: Option<&PgPool>, raw: &str) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    let body_str = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .map_or("", |(_, b)| b);

    let Ok(body) = serde_json::from_str::<serde_json::Value>(body_str) else {
        return (
            "400 Bad Request",
            r#"{"error":"body must be JSON"}"#.to_owned(),
        );
    };

    let primary = body["primary_model"].as_str();
    let fallback = body["fallback_model"].as_str();

    if primary.is_none() && fallback.is_none() {
        return (
            "400 Bad Request",
            r#"{"error":"provide primary_model or fallback_model"}"#.to_owned(),
        );
    }

    match db::update_director_config(pool, primary, fallback).await {
        Ok(cfg) => {
            let body = json!(cfg).to_string();
            ("200 OK", body)
        }
        Err(e) => {
            let body = json!({ "error": e }).to_string();
            ("400 Bad Request", body)
        }
    }
}

fn sessions() -> (&'static str, String) {
    let config_home = daemon_config_home();
    let list = collect_sessions(&config_home);
    let body = json!({ "sessions": list }).to_string();
    ("200 OK", body)
}

async fn run_prompt(raw: &str) -> (&'static str, String) {
    // Extract JSON body — tolerate either CRLF or LF terminators
    let body_str = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .map_or("", |(_, b)| b);

    let Ok(body) = serde_json::from_str::<serde_json::Value>(body_str) else {
        return (
            "400 Bad Request",
            r#"{"error":"body must be JSON: {\"prompt\":\"...\",\"model\":\"...\"}"}"#.to_owned(),
        );
    };

    let Some(prompt) = body["prompt"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: prompt"}"#.to_owned(),
        );
    };

    // Reject prompts that try to inject CLI flags via the leading position.
    if prompt.trim_start().starts_with("--") {
        return (
            "400 Bad Request",
            r#"{"error":"prompt may not start with --"}"#.to_owned(),
        );
    }

    let model = body["model"].as_str().unwrap_or("claude-sonnet-4-6");
    if !is_safe_model_name(model) {
        return (
            "400 Bad Request",
            r#"{"error":"invalid model name"}"#.to_owned(),
        );
    }

    let claw_bin = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("claw"));

    // tokio::process::Command so we don't block the runtime worker
    let mut cmd = tokio::process::Command::new(&claw_bin);
    cmd.args([
        "--model",
        model,
        "--dangerously-skip-permissions",
        "--allow-broad-cwd",
        "--output-format",
        "json",
        "prompt",
        prompt,
    ]);

    // Forward auth-related env vars (including OpenRouter base URL for non-Claude models).
    for var in &[
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "CLAW_CONFIG_HOME",
    ] {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    if std::env::var("ANTHROPIC_API_KEY").is_err() && std::env::var("ANTHROPIC_AUTH_TOKEN").is_err()
    {
        if let Some(key) = read_api_key_from_settings() {
            cmd.env("ANTHROPIC_API_KEY", key);
        }
    }

    match cmd.output().await {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let response_body = json!({
                "ok": out.status.success(),
                "model": model,
                "output": redact_secrets(stdout.trim()),
                "stderr": redact_secrets(stderr.trim()),
                "exit_code": out.status.code(),
            })
            .to_string();
            ("200 OK", response_body)
        }
        Err(e) => {
            let body = json!({ "error": redact_secrets(&e.to_string()) }).to_string();
            ("500 Internal Server Error", body)
        }
    }
}

fn is_safe_model_name(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 128
        && !model.starts_with('-')
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
}

/// Replace any live secret values in `s` with `***redacted***`.
fn redact_secrets(s: &str) -> String {
    let mut out = s.to_string();
    let secret_sources = [
        std::env::var("ANTHROPIC_API_KEY").ok(),
        std::env::var("ANTHROPIC_AUTH_TOKEN").ok(),
        std::env::var("OPENAI_API_KEY").ok(),
        read_api_key_from_settings(),
    ];
    for secret in secret_sources.into_iter().flatten() {
        if secret.len() >= 8 && out.contains(&secret) {
            out = out.replace(&secret, "***redacted***");
        }
    }
    out
}

// ---------------------------------------------------------------------------
// SMS endpoints
// ---------------------------------------------------------------------------

/// `POST /sms/inbound` — receives a webhook from Android SMS Gateway.
///
/// Synchronous chat endpoint for the dashboard.
///
/// POST /chat  `{"message": "..."}`
/// Returns `{"response": "...", "job_id": "..."}` after AI call completes.
/// Requires bearer auth when `GHOST_DAEMON_KEY` is set.
/// Same routing logic as SMS: `!` prefix → Director, else → chat dispatcher.
async fn chat_handler(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if configured_key().is_some() && !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }

    let body_str = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .map_or("", |(_, b)| b);

    let Ok(body) = serde_json::from_str::<serde_json::Value>(body_str) else {
        return (
            "400 Bad Request",
            r#"{"error":"body must be JSON"}"#.to_owned(),
        );
    };

    let message = body["message"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();

    if message.is_empty() {
        return (
            "400 Bad Request",
            r#"{"error":"missing message"}"#.to_owned(),
        );
    }

    // Parse optional conversation history — at most 6 entries (3 exchanges), each
    // validated to have a known role and non-empty content under 8 KiB.
    let history: Vec<serde_json::Value> = body["history"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|m| {
                    let role = m["role"].as_str().unwrap_or("");
                    let content = m["content"].as_str().unwrap_or("");
                    (role == "user" || role == "assistant")
                        && !content.is_empty()
                        && content.len() <= 8192
                })
                .take(6)
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let Some(pool_ref) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    let (agent_name, process_msg) = if let Some(stripped) = message.strip_prefix('!') {
        ("director", stripped.trim().to_string())
    } else {
        ("chat_dispatcher", message.clone())
    };

    let Some(job_id) =
        db::create_job(pool_ref, &message, agent_name, "dashboard", None).await
    else {
        return (
            "500 Internal Server Error",
            r#"{"error":"failed to create job"}"#.to_owned(),
        );
    };

    let result = if agent_name == "director" {
        crate::director::handle(&process_msg, &job_id).await
    } else {
        crate::chat_dispatcher::dispatch(&process_msg, &history, &job_id, Some(pool_ref)).await
    };

    match result {
        Ok(text) => {
            db::update_job_done(pool_ref, &job_id, &text).await;
            let resp = serde_json::json!({"response": text, "job_id": job_id}).to_string();
            ("200 OK", resp)
        }
        Err(e) => {
            db::update_job_failed(pool_ref, &job_id, &e).await;
            let resp = serde_json::json!({"error": e, "job_id": job_id}).to_string();
            ("502 Bad Gateway", resp)
        }
    }
}

/// Accepts inbound SMS from two sources:
///   - Android SMS Gateway: JSON body with `message`/`phoneNumber` fields
///   - Twilio inbound webhook: `application/x-www-form-urlencoded` with `Body`/`From` fields
///
/// Creates a job, spawns a background task to process it, returns 200 immediately.
///
/// Routing:
///   - starts with `.` → ignore (reserved), return 200
///   - starts with `!` → Director stub (strip `!`)
///   - everything else → Chat dispatcher
async fn sms_inbound(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    let (headers_part, body_str) = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .map_or(("", ""), |(h, b)| (h, b));

    // Twilio sends application/x-www-form-urlencoded; Gateway sends JSON.
    let is_form = headers_part
        .lines()
        .any(|l| l.to_ascii_lowercase().contains("application/x-www-form-urlencoded"));

    let (message, phone_from) = if is_form {
        let fields = parse_urlencoded(body_str);
        let msg = fields.get("Body").map_or("", String::as_str).trim().to_string();
        let from = fields.get("From").cloned().unwrap_or_default();
        (msg, from)
    } else {
        let Ok(body) = serde_json::from_str::<serde_json::Value>(body_str) else {
            return (
                "400 Bad Request",
                r#"{"error":"body must be JSON or form-encoded"}"#.to_owned(),
            );
        };
        // Android SMS Gateway may use either field name.
        let msg = body["message"]
            .as_str()
            .or_else(|| body["text"].as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let from = body["phoneNumber"]
            .as_str()
            .or_else(|| body["from"].as_str())
            .unwrap_or("")
            .to_string();
        (msg, from)
    };

    if message.is_empty() {
        return (
            "400 Bad Request",
            r#"{"error":"missing or empty message field"}"#.to_owned(),
        );
    }

    // Whitelist check — only process messages from allowed numbers.
    if let Ok(allowed) = std::env::var("GHOST_ALLOWED_NUMBERS") {
        let allowed_list: Vec<&str> = allowed.split(',').map(str::trim).collect();
        if !phone_from.is_empty() && !allowed_list.iter().any(|n| *n == phone_from) {
            return ("200 OK", r#"{"status":"ignored"}"#.to_owned());
        }
    }

    // `.` prefix → reserved/ignored for now.
    if message.starts_with('.') {
        return ("200 OK", r#"{"status":"ignored"}"#.to_owned());
    }

    // Determine route and strip prefix if needed.
    let (agent_name, process_msg) = if let Some(stripped) = message.strip_prefix('!') {
        ("director", stripped.trim().to_string())
    } else {
        ("chat_dispatcher", message.clone())
    };

    let Some(pool_ref) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    let Some(job_id) =
        db::create_job(pool_ref, &message, agent_name, "sms", Some(&phone_from)).await
    else {
        return (
            "500 Internal Server Error",
            r#"{"error":"failed to create job"}"#.to_owned(),
        );
    };

    // Clone what the background task needs — respond 200 before AI call.
    let pool_arc = Arc::clone(cfg.db.as_ref().unwrap()); // safe: just checked above
    let job_id_bg = job_id.clone();
    let use_director = agent_name == "director";

    tokio::spawn(async move {
        let result = if use_director {
            crate::director::handle(&process_msg, &job_id_bg).await
        } else {
            // SMS is single-turn — no conversation history.
            crate::chat_dispatcher::dispatch(&process_msg, &[], &job_id_bg, Some(&pool_arc)).await
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

    let resp = serde_json::json!({"status": "accepted", "job_id": job_id}).to_string();
    ("200 OK", resp)
}

/// `POST /sms/send` — internal helper to deliver an outbound SMS.
///
/// Body: `{"to": "+1234567890", "body": "Hello"}`
async fn sms_send_handler(raw: &str) -> (&'static str, String) {
    let body_str = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .map_or("", |(_, b)| b);

    let Ok(body) = serde_json::from_str::<serde_json::Value>(body_str) else {
        return (
            "400 Bad Request",
            r#"{"error":"body must be JSON"}"#.to_owned(),
        );
    };

    let Some(to) = body["to"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: to"}"#.to_owned(),
        );
    };

    let Some(msg_body) = body["body"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: body"}"#.to_owned(),
        );
    };

    match crate::sms::send_response(to, msg_body, "manual").await {
        Ok(()) => ("200 OK", r#"{"status":"sent"}"#.to_owned()),
        Err(e) => {
            let resp = serde_json::json!({"error": e}).to_string();
            ("502 Bad Gateway", resp)
        }
    }
}

fn options_preflight() -> (&'static str, String) {
    ("204 No Content", String::new())
}

/// `GET /memories` — list up to 200 non-expired memory notes.
async fn memories_list(db: Option<&PgPool>) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let notes = db::list_notes(pool, 200).await;
    ("200 OK", serde_json::json!({ "notes": notes }).to_string())
}

/// `DELETE /memories/:id` — delete a single memory note by UUID.
/// Requires bearer auth (same key as `/chat`).
async fn memory_delete(db: Option<&PgPool>, raw: &str, id: &str) -> (&'static str, String) {
    if configured_key().is_some() && !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    if db::delete_note(pool, id).await {
        ("200 OK", r#"{"status":"deleted"}"#.to_owned())
    } else {
        ("404 Not Found", r#"{"error":"not found"}"#.to_owned())
    }
}

// ---------------------------------------------------------------------------
// Form body helpers (Twilio inbound webhook)
// ---------------------------------------------------------------------------

/// Parse `application/x-www-form-urlencoded` into key→value pairs.
fn parse_urlencoded(s: &str) -> std::collections::HashMap<String, String> {
    s.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (url_decode(k), url_decode(v)))
        .collect()
}

/// Decode a percent-encoded string (`%XX` and `+` as space). UTF-8 safe.
fn url_decode(s: &str) -> String {
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    let input = s.as_bytes();
    let mut i = 0;
    while i < input.len() {
        match input[i] {
            b'+' => {
                bytes.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < input.len() => {
                if let Ok(hex_str) = std::str::from_utf8(&input[i + 1..i + 3]) {
                    if let Ok(b) = u8::from_str_radix(hex_str, 16) {
                        bytes.push(b);
                        i += 3;
                        continue;
                    }
                }
                bytes.push(b'%');
                i += 1;
            }
            b => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---------------------------------------------------------------------------
// Session helpers
// ---------------------------------------------------------------------------

fn collect_sessions(config_home: &std::path::Path) -> Vec<serde_json::Value> {
    let sessions_dir = config_home.join("sessions");
    let mut out = Vec::new();

    let Ok(workspace_dirs) = std::fs::read_dir(&sessions_dir) else {
        return out;
    };

    for ws_entry in workspace_dirs.flatten() {
        let ws_path = ws_entry.path();
        if !ws_path.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&ws_path) else {
            continue;
        };
        for file_entry in files.flatten() {
            let p = file_entry.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "jsonl" | "json") {
                continue;
            }
            let modified = p
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|d| d.as_secs())
                });
            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
            out.push(json!({
                "file": p.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                "workspace": ws_path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                "modified_unix": modified,
                "bytes": size,
            }));
        }
    }

    out.sort_by(|a, b| {
        let ta = a["modified_unix"].as_u64().unwrap_or(0);
        let tb = b["modified_unix"].as_u64().unwrap_or(0);
        tb.cmp(&ta)
    });

    out
}

fn count_sessions(config_home: &std::path::Path) -> usize {
    collect_sessions(config_home).len()
}

// ---------------------------------------------------------------------------
// PID file
// ---------------------------------------------------------------------------

fn write_pid_file() -> Result<(), Box<dyn std::error::Error>> {
    let home = daemon_config_home();
    std::fs::create_dir_all(&home)?;
    let pid_path = home.join(PID_FILENAME);
    std::fs::write(&pid_path, std::process::id().to_string())?;
    eprintln!("{LOG_PREFIX} PID file: {}", pid_path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Config home — mirrors runtime::default_config_home without a crate dep
// ---------------------------------------------------------------------------

/// Public re-export so other modules (gerald.rs) can reuse this without
/// duplicating the env-var resolution logic.
pub fn daemon_config_home_pub() -> std::path::PathBuf {
    daemon_config_home()
}

fn daemon_config_home() -> std::path::PathBuf {
    std::env::var_os("CLAW_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".claw")))
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|h| std::path::PathBuf::from(h).join(".claw"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from(".claw"))
}

/// Try to read `ANTHROPIC_API_KEY` from `~/.claw/settings.json`.
fn read_api_key_from_settings() -> Option<String> {
    let settings = daemon_config_home().join("settings.json");
    let raw = std::fs::read_to_string(settings).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v["anthropicApiKey"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_localhost() -> DaemonConfig {
        DaemonConfig {
            host: "127.0.0.1".into(),
            port: 7878,
            allow_unsafe_prompt: true,
            db: None,
        }
    }

    #[test]
    fn ct_eq_basics() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(ct_eq("", ""));
    }

    #[test]
    fn strip_prefix_ci_handles_case() {
        assert_eq!(
            strip_prefix_ci("Authorization: Bearer x", "authorization:"),
            Some(" Bearer x")
        );
        assert_eq!(strip_prefix_ci("X-CLAW-KEY: y", "x-claw-key:"), Some(" y"));
        assert_eq!(strip_prefix_ci("Other: 1", "authorization:"), None);
        assert_eq!(strip_prefix_ci("Auth", "authorization:"), None);
    }

    #[test]
    fn extract_bearer_token() {
        let raw = "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer secret\r\n\r\n";
        assert_eq!(extract_bearer_or_claw_key(raw), Some("secret".to_string()));
    }

    #[test]
    fn extract_claw_key_header() {
        let raw = "GET / HTTP/1.1\r\nX-Claw-Key: tok\r\n\r\n";
        assert_eq!(extract_bearer_or_claw_key(raw), Some("tok".to_string()));
    }

    #[test]
    fn host_allowed_accepts_loopback() {
        let cfg = cfg_localhost();
        assert!(host_allowed(
            &cfg,
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:7878\r\n\r\n"
        ));
        assert!(host_allowed(
            &cfg,
            "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n"
        ));
    }

    #[test]
    fn host_allowed_rejects_dns_rebinding() {
        let cfg = cfg_localhost();
        assert!(!host_allowed(
            &cfg,
            "GET / HTTP/1.1\r\nHost: evil.example.com\r\n\r\n"
        ));
        assert!(!host_allowed(&cfg, "GET / HTTP/1.1\r\n\r\n"));
    }

    #[test]
    fn safe_model_name_validation() {
        assert!(is_safe_model_name("claude-opus-4-6"));
        assert!(is_safe_model_name("claude-haiku-4-5-20251001"));
        assert!(is_safe_model_name("gpt-4o-mini"));
        assert!(!is_safe_model_name(""));
        assert!(!is_safe_model_name("--evil"));
        assert!(!is_safe_model_name("model with space"));
        assert!(!is_safe_model_name("model;rm -rf /"));
    }

    #[test]
    fn find_header_end_finds_terminator() {
        let buf = b"GET / HTTP/1.1\r\nHost: x\r\n\r\nbody";
        let pos = find_header_end(buf).unwrap();
        assert_eq!(&buf[pos..pos + 4], b"\r\n\r\n");
    }

    #[test]
    fn redact_secrets_no_secret_no_change() {
        // Without env vars set, output should pass through unchanged
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::remove_var("ANTHROPIC_AUTH_TOKEN");
        std::env::remove_var("OPENAI_API_KEY");
        assert_eq!(redact_secrets("hello world"), "hello world");
    }

    #[test]
    fn is_origin_allowed_default_dashboard() {
        std::env::remove_var("GHOST_DAEMON_CORS_ORIGIN");
        assert!(is_origin_allowed("http://localhost:5173"));
        assert!(is_origin_allowed("http://127.0.0.1:5173"));
        assert!(is_origin_allowed("http://[::1]:5173"));
        assert!(!is_origin_allowed("http://evil.example"));
    }
}
