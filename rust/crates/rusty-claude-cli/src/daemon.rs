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
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::db;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use tokio::sync::Mutex as AsyncMutex;

const DEFAULT_DAEMON_PORT: u16 = 7878;
const PID_FILENAME: &str = "daemon.pid";
const LOG_PREFIX: &str = "[ghost daemon]";
const MAX_REQUEST_BYTES: usize = 1024 * 1024; // 1 MiB
const READ_CHUNK: usize = 8 * 1024;
/// Background health-check interval: reset circuit-breaker flags every 5 min.
const HEALTH_CHECK_INTERVAL_SECS: u64 = 300;
/// Confidence decay runs once every 24 hours.
const DECAY_INTERVAL_SECS: u64 = 86_400;
/// Maximum POST requests per IP per 60s window.
const RATE_LIMIT_MAX: u32 = 30;
/// Rate limit window in seconds.
const RATE_LIMIT_WINDOW_SECS: u64 = 60;

/// Simple in-memory token-bucket rate limiter keyed by IP address.
struct RateLimiter {
    /// Map of IP → (remaining tokens, window start time).
    buckets: Mutex<HashMap<String, (u32, Instant)>>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `true` if the request is allowed, `false` if rate-limited.
    fn check(&self, ip: &str) -> bool {
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        let entry = buckets
            .entry(ip.to_string())
            .or_insert((RATE_LIMIT_MAX, now));

        // Reset window if expired
        if now.duration_since(entry.1).as_secs() >= RATE_LIMIT_WINDOW_SECS {
            entry.0 = RATE_LIMIT_MAX;
            entry.1 = now;
        }

        if entry.0 > 0 {
            entry.0 -= 1;
            true
        } else {
            false
        }
    }
}

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
    /// In-memory rate limiter for POST endpoints.
    rate_limiter: Arc<RateLimiter>,
    /// Serializes `POST /code/index/rebuild` — only one full reindex runs at
    /// a time; contention returns 409 via `try_lock`.
    coder_rebuild_lock: Arc<AsyncMutex<()>>,
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
            rate_limiter: Arc::new(RateLimiter::new()),
            coder_rebuild_lock: Arc::new(AsyncMutex::new(())),
        };
        daemon_main(cfg).await
    })
}

#[allow(clippy::too_many_lines)]
async fn daemon_main(cfg: DaemonConfig) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(key) = std::env::var("GHOST_DAEMON_KEY") {
        if key.trim().is_empty() {
            return Err(
                "GHOST_DAEMON_KEY is set but empty — refusing to start. Either unset it or set a non-empty value.".into(),
            );
        }
    }

    // Public bind requires auth key — prevent accidental open deployment.
    if cfg.host == "0.0.0.0" && configured_key().is_none() {
        return Err(
            "GHOST_DAEMON_KEY must be set when binding 0.0.0.0 (public deployment). \
             Set the key or bind 127.0.0.1 for local dev."
                .into(),
        );
    }

    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = TcpListener::bind(&addr).await.map_err(|e| {
        format!("ghost daemon: cannot bind {addr}: {e} — is another daemon already running?")
    })?;

    write_pid_file()?;

    // Clean up stale jobs at startup, then spawn circuit-breaker health-reset task.
    if let Some(pool) = cfg.db.clone() {
        db::cleanup_stale_jobs(&pool).await;
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS));
            interval.tick().await; // skip the immediate first tick
            loop {
                interval.tick().await;
                db::reset_health_flags(&pool).await;
                db::cleanup_stale_jobs(&pool).await;
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
                db::cleanup_response_cache(&pool).await;
                eprintln!("{LOG_PREFIX} memory: confidence decay + cache cleanup applied");
            }
        });
    }

    // Spawn scheduler task (polls `scheduled_triggers` every 30s, fires due
    // rows through the dispatcher). See `infra/scheduler.rs`.
    if let Some(pool) = cfg.db.clone() {
        let _scheduler_handle = crate::infra::scheduler::spawn((*pool).clone());
    }

    // Initialize provider-router config cache + 60s refresh task. No-ops when
    // DB is absent; the cache defaults to OpenRouter + no overrides.
    if let Some(pool) = cfg.db.clone() {
        crate::infra::provider::init(pool);
    }

    // Spawn availability-scheduler tick: every 60s, evaluates A/B/C windows and
    // sleep mode in America/Denver, writes `auto_reply` for each assigned
    // contact. See `db::tick_schedule` for the precedence rules.
    if let Some(pool) = cfg.db.clone() {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                db::tick_schedule(&pool).await;
            }
        });
    }

    // Spawn coder file-index filesystem watcher. Opt-in via the
    // `coder.index_watcher_enabled` setting (default on). Silently skips
    // when the resolved repo_root doesn't exist on disk (e.g. Railway), so
    // cloud deploys don't crash on startup.
    if let Some(pool) = cfg.db.clone() {
        tokio::spawn(async move {
            let enabled = db::get_setting::<bool>(&pool, "coder.index_watcher_enabled")
                .await
                .unwrap_or(true);
            if !enabled {
                eprintln!(
                    "{LOG_PREFIX} coder index watcher: disabled via coder.index_watcher_enabled"
                );
                return;
            }
            let root = crate::agents::coder::repo_root(&pool).await;
            if !root.exists() || !root.is_dir() {
                eprintln!(
                    "{LOG_PREFIX} coder index watcher: repo_root {} missing, skipping",
                    root.display()
                );
                return;
            }
            if let Err(e) = spawn_coder_watcher(pool, root.clone()) {
                eprintln!("{LOG_PREFIX} coder index watcher failed to start: {e}");
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
                        "application/json",
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
                        "application/json",
                    )
                    .await;
                    return;
                }
            };

            let peer_ip = peer.ip().to_string();

            // SSE routes hold the socket open for the lifetime of the
            // subscription, so they bypass the one-shot dispatch/write_response
            // flow. Parse the request line here and hand off to the streaming
            // handler; anything else falls through to `dispatch`.
            if let Some(job_id) = parse_stream_tokens_path(&raw) {
                let origin = header_value(&raw, "origin");
                let allowed_origin =
                    origin.and_then(|o| if is_origin_allowed(&o) { Some(o) } else { None });
                stream_tokens_handler(&mut stream, &raw, &job_id, allowed_origin.as_deref()).await;
                return;
            }

            let (status_line, body) = dispatch(&cfg, &raw, uptime, &peer_ip).await;

            // Echo Origin only if it's in the allow-list
            let origin = header_value(&raw, "origin");
            let allowed_origin =
                origin.and_then(|o| if is_origin_allowed(&o) { Some(o) } else { None });

            let ct = if body.starts_with("<!DOCTYPE") {
                "text/html; charset=utf-8"
            } else {
                "application/json"
            };
            write_response(
                &mut stream,
                status_line,
                &body,
                allowed_origin.as_deref(),
                ct,
            )
            .await;
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
    let content_length: usize = match header_block
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
    {
        Some(line) => line[15..]
            .trim()
            .parse()
            .map_err(|_| RequestError::Malformed)?,
        None => 0, // No Content-Length header is fine (GET requests)
    };

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
    content_type: &str,
) {
    let csp = if content_type.starts_with("text/html") {
        "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'"
    } else {
        "default-src 'none'"
    };
    let mut headers = format!(
        "HTTP/1.1 {status_line}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         X-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\n\
         Content-Security-Policy: {csp}\r\n\
         Cache-Control: no-store\r\n",
        body.len()
    );
    if let Some(origin) = allowed_origin {
        let _ = write!(headers, "Access-Control-Allow-Origin: {origin}\r\n");
        headers.push_str("Vary: Origin\r\n");
        headers.push_str("Access-Control-Allow-Methods: GET, POST, PUT, DELETE, OPTIONS\r\n");
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

#[allow(clippy::too_many_lines)]
async fn dispatch(
    cfg: &DaemonConfig,
    raw: &str,
    uptime_secs: u64,
    peer_ip: &str,
) -> (&'static str, String) {
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

    // Rate-limit POST endpoints (GET / OPTIONS / preflight are exempt).
    if method == "POST" && !cfg.rate_limiter.check(peer_ip) {
        return (
            "429 Too Many Requests",
            r#"{"error":"rate limited"}"#.to_owned(),
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
        ("GET", p) if p.starts_with("/read/") => {
            let id = &p["/read/".len()..];
            read_page(cfg.db.as_deref(), id).await
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

        // --- Observability endpoints (Wave 3) ---
        ("GET", "/events") => {
            let qs = path.split_once('?').map_or("", |(_, q)| q);
            events_list(cfg, raw, qs).await
        }
        ("GET", "/agents/budget") => agents_budget(cfg, raw).await,
        ("GET", "/agents") => agents_list(raw),

        ("POST", "/chat") => chat_handler(cfg, raw).await,
        ("POST", "/sms/inbound") => sms_inbound(cfg, raw).await,
        ("POST", "/sms/send") => sms_send_handler(raw, cfg).await,

        // --- SMS contacts + history endpoints ---
        ("GET", "/sms/contacts") => sms_contacts_list(cfg, raw).await,
        ("GET", p) if p.starts_with("/sms/history/") => {
            let phone_encoded = &p["/sms/history/".len()..];
            // Strip query string from phone segment
            let phone_seg = phone_encoded.split('?').next().unwrap_or(phone_encoded);
            let phone = url_decode(phone_seg);
            // Parse query params
            let qs = p.split_once('?').map_or("", |(_, q)| q);
            sms_history_handler(cfg, raw, &phone, qs).await
        }
        ("POST", p) if p.starts_with("/sms/contacts/") && p.ends_with("/auto-reply") => {
            let mid = &p["/sms/contacts/".len()..p.len() - "/auto-reply".len()];
            let phone = url_decode(mid);
            sms_auto_reply_handler(cfg, raw, &phone).await
        }
        ("PUT", p) if p.starts_with("/sms/contacts/") && p.ends_with("/name") => {
            let mid = &p["/sms/contacts/".len()..p.len() - "/name".len()];
            let phone = url_decode(mid);
            sms_contact_name_handler(cfg, raw, &phone).await
        }
        ("POST", p) if p.starts_with("/sms/contacts/") && p.ends_with("/read") => {
            let mid = &p["/sms/contacts/".len()..p.len() - "/read".len()];
            let phone = url_decode(mid);
            sms_mark_read_handler(cfg, raw, &phone).await
        }
        ("PUT", p) if p.starts_with("/sms/contacts/") && p.ends_with("/notes") => {
            let mid = &p["/sms/contacts/".len()..p.len() - "/notes".len()];
            let phone = url_decode(mid);
            sms_contact_notes_handler(cfg, raw, &phone).await
        }
        ("GET", p) if p.starts_with("/sms/contacts/") && p.ends_with("/summary") => {
            let mid = &p["/sms/contacts/".len()..p.len() - "/summary".len()];
            let phone = url_decode(mid);
            sms_summary_handler(cfg, raw, &phone).await
        }

        // --- Schedule endpoints ---
        ("GET", "/schedule") => schedule_list(cfg, raw).await,
        ("POST", "/schedule") => schedule_create(cfg, raw).await,
        ("DELETE", p) if p.starts_with("/schedule/") => {
            let id = &p["/schedule/".len()..];
            schedule_delete(cfg, raw, id).await
        }

        // --- Facts box (SMS-shareable singleton blob) ---
        ("GET", "/facts") => facts_get(cfg, raw).await,
        ("PUT", "/facts") => facts_put(cfg, raw).await,

        // --- Settings KV (Phase A) ---
        ("GET", "/settings") => settings_list(cfg, raw).await,
        ("PUT", p) if p.starts_with("/settings/") => {
            let key = &p["/settings/".len()..];
            settings_put(cfg, raw, key).await
        }

        // --- Coder health (Phase A) — unauthenticated on purpose ---
        ("GET", "/code/health") => code_health(cfg).await,

        // --- Coder file index + templates (Phase C) ---
        ("POST", "/code/index/rebuild") => code_index_rebuild(cfg, raw).await,
        ("POST", "/code/index/file") => code_index_file(cfg, raw).await,
        ("GET", "/code/index/stats") => code_index_stats(cfg, raw).await,
        ("GET", "/code/templates") => code_templates_list(raw),
        ("POST", "/code/templates/stamp") => code_templates_stamp(cfg, raw).await,

        // --- Coder chat + brainstorm + orchestrate (Phase B.6) ---
        ("POST", "/code/chat") => code_chat_handler(cfg, raw).await,
        ("POST", "/code/brainstorm") => code_brainstorm_handler(cfg, raw).await,
        ("POST", "/code/orchestrate") => code_orchestrate_handler(cfg, raw).await,
        ("POST", p) if p.starts_with("/code/orchestrate/") && p.ends_with("/run") => {
            let id = &p["/code/orchestrate/".len()..p.len() - "/run".len()];
            code_orchestrate_run_handler(cfg, raw, id).await
        }
        ("GET", p) if p.starts_with("/code/orchestrate/") => {
            let id = &p["/code/orchestrate/".len()..];
            let id = id.split('?').next().unwrap_or(id);
            code_orchestrate_get_handler(cfg, raw, id).await
        }
        ("GET", "/code/pending_diffs") => code_pending_diffs_handler(cfg, raw).await,
        ("POST", p) if p.starts_with("/code/diffs/") && p.ends_with("/apply") => {
            let id = &p["/code/diffs/".len()..p.len() - "/apply".len()];
            code_diff_apply_handler(cfg, raw, id).await
        }
        ("POST", p) if p.starts_with("/code/diffs/") && p.ends_with("/reject") => {
            let id = &p["/code/diffs/".len()..p.len() - "/reject".len()];
            code_diff_reject_handler(cfg, raw, id).await
        }
        ("GET", "/code/spend") => code_spend_handler(cfg, raw).await,

        // --- Availability schedules (slots A/B/C) + sleep mode ---
        ("GET", "/sms/availability") => availability_list(cfg, raw).await,
        ("PUT", p) if p.starts_with("/sms/availability/slots/") => {
            let slot = &p["/sms/availability/slots/".len()..];
            availability_rename_slot(cfg, raw, slot).await
        }
        ("POST", "/sms/availability/windows") => availability_window_create(cfg, raw).await,
        ("DELETE", p) if p.starts_with("/sms/availability/windows/") => {
            let id = &p["/sms/availability/windows/".len()..];
            availability_window_delete(cfg, raw, id).await
        }
        ("PUT", p) if p.starts_with("/sms/contacts/") && p.ends_with("/schedule-slot") => {
            let mid = &p["/sms/contacts/".len()..p.len() - "/schedule-slot".len()];
            let phone = url_decode(mid);
            availability_contact_slot(cfg, raw, &phone).await
        }
        ("GET", "/sms/sleep") => sleep_get(cfg, raw).await,
        ("POST", "/sms/sleep/start") => sleep_start(cfg, raw).await,
        ("POST", "/sms/sleep/end") => sleep_end(cfg, raw).await,
        ("GET", "/sms/sleep/contacts") => sleep_contacts_list(cfg, raw).await,
        ("POST", "/sms/sleep/contacts") => sleep_contact_add(cfg, raw).await,
        ("DELETE", p) if p.starts_with("/sms/sleep/contacts/") => {
            let phone = url_decode(&p["/sms/sleep/contacts/".len()..]);
            sleep_contact_remove(cfg, raw, &phone).await
        }

        // --- Trigger CRUD (Wave 5) ---
        ("GET", "/triggers") => triggers_list(cfg, raw).await,
        ("POST", "/triggers") => trigger_create(cfg, raw).await,
        ("DELETE", p) if p.starts_with("/triggers/") => {
            let id = &p["/triggers/".len()..];
            trigger_delete(cfg, raw, id).await
        }

        // --- Bible endpoints ---
        ("GET", "/bible/stats") => bible_stats_handler(cfg.db.as_deref()).await,
        ("GET", p) if p.starts_with("/bible/verse/") => {
            bible_verse_handler(cfg.db.as_deref(), &p["/bible/verse/".len()..]).await
        }
        ("GET", p) if p.starts_with("/bible/range/") => {
            bible_range_handler(cfg.db.as_deref(), &p["/bible/range/".len()..]).await
        }
        ("GET", p) if p.starts_with("/bible/search") => {
            bible_search_handler(cfg.db.as_deref(), p).await
        }
        ("GET", p) if p.starts_with("/bible/strongs/") => {
            bible_strongs_handler(cfg.db.as_deref(), &p["/bible/strongs/".len()..]).await
        }
        ("GET", p) if p.starts_with("/bible/crossrefs/") => {
            bible_crossrefs_handler(cfg.db.as_deref(), &p["/bible/crossrefs/".len()..]).await
        }

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
// Read page — HTML view for SMS "read more" links
// ---------------------------------------------------------------------------

const READ_PAGE_TEMPLATE: &str = r#"<!DOCTYPE html>
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
</html>"#;

/// Escape HTML special characters to prevent XSS.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Build a minimal HTML error page (dark theme, centered message).
fn html_error_page(message: &str) -> String {
    let escaped = html_escape(message);
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>Error</title>\
         <style>body{{background:#0a0a0a;color:#e0e0e0;\
         font-family:-apple-system,sans-serif;\
         display:flex;align-items:center;justify-content:center;\
         min-height:100vh;margin:0}}p{{font-size:16px;color:#888}}</style>\
         </head><body><p>{escaped}</p></body></html>"
    )
}

/// `GET /read/:id` — serves a mobile-friendly HTML page with the full
/// job response. Public (no auth) because SMS recipients need to open it
/// without logging in; the UUID is unguessable.
async fn read_page(db: Option<&PgPool>, id: &str) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            html_error_page("Service temporarily unavailable."),
        );
    };
    match db::get_job(pool, id).await {
        Some(job) => {
            let content = job.output.as_deref().unwrap_or("(no content)");
            let html = READ_PAGE_TEMPLATE.replace("{{MESSAGE_BODY}}", &html_escape(content));
            ("200 OK", html)
        }
        None => ("404 Not Found", html_error_page("Message not found.")),
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
        "GHOST_CONFIG_HOME",
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
        std::env::var("GHOST_DAEMON_KEY").ok(),
        std::env::var("VOYAGE_API_KEY").ok(),
        std::env::var("BRAVE_API_KEY").ok(),
        std::env::var("TWILIO_AUTH_TOKEN").ok(),
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
#[allow(clippy::too_many_lines)]
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

    let message = body["message"].as_str().unwrap_or("").trim().to_string();

    if message.is_empty() {
        return (
            "400 Bad Request",
            r#"{"error":"missing message"}"#.to_owned(),
        );
    }

    if message.len() > 16_384 {
        return (
            "400 Bad Request",
            r#"{"error":"message too long (max 16384 bytes)"}"#.to_owned(),
        );
    }

    // Parse optional conversation history — at most 10 entries (5 exchanges), each
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
                .take(10)
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

    // Routing lives in agents::dispatcher. `agent_name` is only for the jobs
    // table — classify just to get a label.
    let (intent_label, _) = crate::agents::intent::classify(&message);
    let agent_name = match intent_label {
        crate::agents::intent::Intent::Director => "director",
        crate::agents::intent::Intent::Research => "research",
        crate::agents::intent::Intent::Scheduled => "scheduled",
        crate::agents::intent::Intent::Calendar => "calendar",
        crate::agents::intent::Intent::ChiefOfStaff => "chief_of_staff",
        crate::agents::intent::Intent::Docs => "docs",
        crate::agents::intent::Intent::Dreamer => "dreamer",
        crate::agents::intent::Intent::Coder => "coder",
        crate::agents::intent::Intent::Brainstorm => "brainstorm",
        crate::agents::intent::Intent::Orchestrator => "orchestrator",
        crate::agents::intent::Intent::Ignore => "ignored",
        crate::agents::intent::Intent::Chat => "chat_dispatcher",
    };

    let Some(job_id) = db::create_job(pool_ref, &message, agent_name, "dashboard", None).await
    else {
        return (
            "500 Internal Server Error",
            r#"{"error":"failed to create job"}"#.to_owned(),
        );
    };

    let dispatcher = crate::agents::dispatcher::Dispatcher::new();
    let req = crate::agents::AgentRequest {
        message: message.clone(),
        history,
        source: crate::agents::Source::Dashboard,
        job_id: job_id.clone(),
        sender_phone: None,
    };
    let result = dispatcher.dispatch(req, pool_ref).await;

    match result {
        Ok(resp) => {
            db::update_job_done(pool_ref, &job_id, &resp.text).await;
            let cost = crate::infra::budget::cost_cents(
                resp.tier,
                i64::from(resp.usage.tokens_in),
                i64::from(resp.usage.tokens_out),
            );
            let body = serde_json::json!({
                "response": resp.text,
                "job_id": job_id,
                "agent": agent_name,
                "tokens": {
                    "input": resp.usage.tokens_in,
                    "output": resp.usage.tokens_out,
                    "cache_read": 0,
                    "cost_cents": cost,
                },
            })
            .to_string();
            ("200 OK", body)
        }
        Err(e) => {
            db::update_job_failed(pool_ref, &job_id, &e).await;
            let resp = serde_json::json!({
                "error": e,
                "job_id": job_id,
                "agent": agent_name,
            })
            .to_string();
            ("502 Bad Gateway", resp)
        }
    }
}

/// Outcome of an SMS-driven approval lookup. `NoMatch` means the message
/// looked like an approval but didn't resolve any pending job — caller should
/// fall through to normal chat dispatch rather than swallow it.
enum ApprovalOutcome {
    Resolved { job_id: uuid::Uuid },
    NoMatch,
    DbError(String),
}

/// Resolve an SMS approval message ("y" / "yes" / "y <token>") against the
/// caller's pending jobs. Composes the public API in `infra::approval` so
/// `sms_inbound` only has to make one call.
async fn resolve_approval(
    pool: &sqlx::PgPool,
    phone: &str,
    kind: crate::infra::approval::ApprovalKind,
) -> ApprovalOutcome {
    use crate::infra::approval::{
        find_pending_for_contact, mark_job_approved, resolve_by_token, ApprovalKind,
    };
    let pending = match kind {
        ApprovalKind::Plain => match find_pending_for_contact(pool, phone).await {
            Ok(p) => p,
            Err(e) => return ApprovalOutcome::DbError(e.to_string()),
        },
        ApprovalKind::Tokened(tok) => match resolve_by_token(pool, phone, &tok).await {
            Ok(p) => p,
            Err(e) => return ApprovalOutcome::DbError(e.to_string()),
        },
    };
    let Some(job) = pending else {
        return ApprovalOutcome::NoMatch;
    };
    match mark_job_approved(pool, job.id).await {
        Ok(()) => ApprovalOutcome::Resolved { job_id: job.id },
        Err(sqlx::Error::RowNotFound) => ApprovalOutcome::NoMatch,
        Err(e) => ApprovalOutcome::DbError(e.to_string()),
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
#[allow(clippy::too_many_lines)]
async fn sms_inbound(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    let (headers_part, body_str) = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .map_or(("", ""), |(h, b)| (h, b));

    // Twilio sends application/x-www-form-urlencoded; Gateway sends JSON.
    let is_form = headers_part.lines().any(|l| {
        l.to_ascii_lowercase()
            .contains("application/x-www-form-urlencoded")
    });

    let (message, phone_from) = if is_form {
        let fields = parse_urlencoded(body_str);
        let msg = fields
            .get("Body")
            .map_or("", String::as_str)
            .trim()
            .to_string();
        let from = fields.get("From").cloned().unwrap_or_default();
        (msg, from)
    } else {
        let Ok(body) = serde_json::from_str::<serde_json::Value>(body_str) else {
            return (
                "400 Bad Request",
                r#"{"error":"body must be JSON or form-encoded"}"#.to_owned(),
            );
        };
        // Android SMS Gateway wraps data in a `payload` object:
        //   { "payload": { "message": "...", "phoneNumber": "..." }, "event": "sms:received", ... }
        // Also support flat format for direct API calls:
        //   { "message": "...", "phoneNumber": "..." }
        let payload = &body["payload"];
        let msg = payload["message"]
            .as_str()
            .or_else(|| body["message"].as_str())
            .or_else(|| body["text"].as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let from = payload["phoneNumber"]
            .as_str()
            .or_else(|| payload["sender"].as_str())
            .or_else(|| body["phoneNumber"].as_str())
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
    // Numbers are normalized (stripped to digits + leading +) before comparison
    // so "+1-234-567-8900" matches "+12345678900".
    if let Ok(allowed) = std::env::var("GHOST_ALLOWED_NUMBERS") {
        let normalized_from = normalize_phone(&phone_from);
        let allowed_list: Vec<String> = allowed
            .split(',')
            .map(|n| normalize_phone(n.trim()))
            .collect();
        if !normalized_from.is_empty() && !allowed_list.contains(&normalized_from) {
            return ("200 OK", r#"{"status":"ignored"}"#.to_owned());
        }
    }

    // --- Approval intercept (Wave 5) -------------------------------------
    // A plain "y" / "yes" / "y <token>" resolves the sender's most recent
    // pending job and does NOT go to any agent. Gated on DB being configured
    // — without a pool we can't look up pending jobs anyway.
    if let Some(pool) = cfg.db.as_deref() {
        if !phone_from.is_empty() {
            if let Some(kind) = crate::infra::approval::is_approval_message(&message) {
                match resolve_approval(pool, &phone_from, kind).await {
                    ApprovalOutcome::Resolved { job_id } => {
                        let reply = format!("[ok] approved -- running job {job_id}");
                        if let Err(e) =
                            crate::sms::send_response(&phone_from, &reply, &job_id.to_string())
                                .await
                        {
                            eprintln!("[ghost approval] reply send failed: {e}");
                        }
                        db::insert_sms_history(pool, &phone_from, "user", &message).await;
                        db::insert_sms_history(pool, &phone_from, "assistant", &reply).await;
                        // TODO(wave 6): actually kick the waiting job forward.
                        // mark_job_approved is terminal today — no agent
                        // continuation path exists yet.
                        return ("200 OK", r#"{"status":"approved"}"#.to_owned());
                    }
                    ApprovalOutcome::NoMatch => {
                        // Token didn't match an awaiting job; fall through to
                        // the normal dispatcher. Wave 6 may surface a "no
                        // pending job found" UX hint here.
                    }
                    ApprovalOutcome::DbError(e) => {
                        eprintln!("[ghost approval] lookup failed: {e}");
                        // Better to treat it as a chat message than to drop
                        // it on a flaky DB read.
                    }
                }
            }
        }
    }

    // Routing + prefix stripping now lives in agents::intent / agents::dispatcher.
    // We still classify here once to label the `jobs.agent` column and to store
    // a prefix-stripped copy in `sms_history`. The dispatcher re-classifies
    // internally — it's the single source of truth for the routing decision.
    let (intent_label, process_msg) = crate::agents::intent::classify(&message);
    let agent_name = match intent_label {
        crate::agents::intent::Intent::Director => "director",
        crate::agents::intent::Intent::Research => "research",
        crate::agents::intent::Intent::Scheduled => "scheduled",
        crate::agents::intent::Intent::Calendar => "calendar",
        crate::agents::intent::Intent::ChiefOfStaff => "chief_of_staff",
        crate::agents::intent::Intent::Docs => "docs",
        crate::agents::intent::Intent::Dreamer => "dreamer",
        crate::agents::intent::Intent::Coder => "coder",
        crate::agents::intent::Intent::Brainstorm => "brainstorm",
        crate::agents::intent::Intent::Orchestrator => "orchestrator",
        crate::agents::intent::Intent::Ignore => "ignored",
        crate::agents::intent::Intent::Chat => "chat_dispatcher",
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
    let message_for_bg = message.clone();

    tokio::spawn(async move {
        // Store inbound message in conversation history.
        db::insert_sms_history(&pool_arc, &phone_from, "user", &process_msg).await;

        // Check auto-reply setting. If disabled, store the message but don't respond.
        if !db::is_auto_reply_enabled(&pool_arc, &phone_from).await {
            eprintln!("[ghost sms] auto-reply disabled for {phone_from}, storing message only");
            db::update_job_done(&pool_arc, &job_id_bg, "(auto-reply disabled)").await;
            return;
        }

        // Load recent conversation history (last 10 messages + any loadbearing).
        let mut history = db::load_sms_history(&pool_arc, &phone_from, 10).await;
        let loadbearing = db::load_loadbearing_history(&pool_arc, &phone_from).await;
        merge_loadbearing(&mut history, loadbearing);

        let dispatcher = crate::agents::dispatcher::Dispatcher::new();
        let req = crate::agents::AgentRequest {
            message: message_for_bg,
            history,
            source: crate::agents::Source::Sms,
            job_id: job_id_bg.clone(),
            sender_phone: Some(phone_from.clone()),
        };
        let result = dispatcher.dispatch(req, &pool_arc).await.map(|r| r.text);

        match result {
            Ok(text) if text.is_empty() => {
                // Ignore intent (e.g. `.` prefix): do not send an SMS reply.
                eprintln!("[ghost sms] ignored message for job {job_id_bg}, no reply sent");
                db::update_job_done(&pool_arc, &job_id_bg, "(ignored)").await;
            }
            Ok(text) => {
                // Guard check — review outbound reply before sending.
                // Pass shareable context so the guard approves info Isaac has
                // marked public (facts box + schedule) rather than blocking it.
                let shareable = db::load_shareable_context(&pool_arc).await;
                let verdict =
                    crate::guard::check(&process_msg, &text, &phone_from, &shareable).await;
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
                        db::insert_sms_history(&pool_arc, &phone_from, "assistant", &send_text)
                            .await;

                        if was_blocked {
                            let blocked_note = format!(
                                "[BLOCKED BY GUARD]\nOriginal reply: {text}\n\
                                 Sent instead: {send_text}"
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

    let resp = serde_json::json!({"status": "accepted", "job_id": job_id}).to_string();
    ("200 OK", resp)
}

/// `POST /sms/send` — deliver an outbound SMS and store in history.
///
/// Body: `{"to": "+1234567890", "body": "Hello"}`
/// Requires bearer auth. Stores the outbound message in `sms_history`.
async fn sms_send_handler(raw: &str, cfg: &DaemonConfig) -> (&'static str, String) {
    if !auth_matches(raw) {
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
        Ok(()) => {
            // Store outbound message in sms_history.
            let message_id = if let Some(pool) = cfg.db.as_deref() {
                db::insert_sms_history_returning_id(pool, to, "assistant", msg_body).await
            } else {
                None
            };
            let resp = json!({"status": "sent", "message_id": message_id}).to_string();
            ("200 OK", resp)
        }
        Err(e) => {
            let resp = serde_json::json!({"error": e}).to_string();
            ("502 Bad Gateway", resp)
        }
    }
}

// ---------------------------------------------------------------------------
// SMS contacts + history handlers (Phase 5)
// ---------------------------------------------------------------------------

/// `GET /sms/contacts` -- list contacts with auto-reply status + message counts.
async fn sms_contacts_list(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let contacts = db::list_sms_contacts(pool).await;
    ("200 OK", json!({ "contacts": contacts }).to_string())
}

/// `GET /sms/history/{phone}?limit=30&before={id}` -- paginated message history.
async fn sms_history_handler(
    cfg: &DaemonConfig,
    raw: &str,
    phone: &str,
    query_string: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    // Parse query params.
    let params = parse_urlencoded(query_string);
    let limit: i64 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
        .min(100);
    let before_id = params.get("before").map(String::as_str);

    let messages = db::load_sms_history_page(pool, phone, limit, before_id).await;
    let has_more = usize::try_from(limit)
        .map(|l| messages.len() == l)
        .unwrap_or(false);

    (
        "200 OK",
        json!({ "messages": messages, "has_more": has_more }).to_string(),
    )
}

/// `POST /sms/contacts/{phone}/auto-reply` -- toggle auto-reply.
async fn sms_auto_reply_handler(
    cfg: &DaemonConfig,
    raw: &str,
    phone: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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

    let Some(enabled) = body["enabled"].as_bool() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: enabled (bool)"}"#.to_owned(),
        );
    };

    db::set_auto_reply(pool, phone, enabled).await;
    (
        "200 OK",
        json!({"status": "ok", "auto_reply": enabled}).to_string(),
    )
}

/// `PUT /sms/contacts/{phone}/name` -- update display name.
async fn sms_contact_name_handler(
    cfg: &DaemonConfig,
    raw: &str,
    phone: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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

    let Some(name) = body["name"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: name"}"#.to_owned(),
        );
    };

    db::set_contact_name(pool, phone, name).await;
    ("200 OK", json!({"status": "ok"}).to_string())
}

/// `POST /sms/contacts/{phone}/read` -- mark conversation as read.
async fn sms_mark_read_handler(
    cfg: &DaemonConfig,
    raw: &str,
    phone: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    db::mark_contact_read(pool, phone).await;
    ("200 OK", json!({"status": "ok"}).to_string())
}

/// `PUT /sms/contacts/{phone}/notes` -- set contact notes.
async fn sms_contact_notes_handler(
    cfg: &DaemonConfig,
    raw: &str,
    phone: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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

    let notes = body["notes"].as_str().unwrap_or("");
    db::set_contact_notes(pool, phone, notes).await;
    ("200 OK", json!({"status": "ok"}).to_string())
}

/// `GET /sms/contacts/{phone}/summary` -- AI-generated conversation summary.
async fn sms_summary_handler(cfg: &DaemonConfig, raw: &str, phone: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    let history = db::load_sms_history(pool, phone, 20).await;
    if history.is_empty() {
        return ("200 OK", json!({"summary": "No messages yet."}).to_string());
    }

    // Format history into a readable block for the summarizer.
    let mut transcript = String::new();
    for msg in &history {
        if let (Some(role), Some(content)) = (
            msg.get("role").and_then(|v| v.as_str()),
            msg.get("content").and_then(|v| v.as_str()),
        ) {
            let label = if role == "user" { "Them" } else { "GHOST" };
            let _ = writeln!(transcript, "{label}: {content}");
        }
    }

    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            return (
                "503 Service Unavailable",
                r#"{"error":"ANTHROPIC_API_KEY not set"}"#.to_owned(),
            );
        }
    };

    let body = json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 256,
        "system": "Summarize this SMS conversation in 2-3 sentences. Focus on key topics, decisions, and any outstanding items. Be concise.",
        "messages": [{"role": "user", "content": transcript}]
    });

    let client = reqwest::Client::new();
    match client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
    {
        Ok(resp) => match resp.text().await {
            Ok(text) => {
                let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or(json!({}));
                let summary = parsed["content"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|b| b["text"].as_str())
                    .unwrap_or("Could not generate summary.");
                ("200 OK", json!({"summary": summary}).to_string())
            }
            Err(e) => (
                "500 Internal Server Error",
                json!({"error": format!("failed to read response: {e}")}).to_string(),
            ),
        },
        Err(e) => (
            "500 Internal Server Error",
            json!({"error": format!("API call failed: {e}")}).to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Schedule handlers (Phase 5)
// ---------------------------------------------------------------------------

/// `GET /schedule` -- list all schedule entries.
async fn schedule_list(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let entries = db::list_schedule_entries(pool).await;
    ("200 OK", json!({ "entries": entries }).to_string())
}

/// `POST /schedule` -- add a new schedule entry.
async fn schedule_create(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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

    let Some(kind) = body["kind"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: kind"}"#.to_owned(),
        );
    };

    if kind != "daily" && kind != "persistent" {
        return (
            "400 Bad Request",
            r#"{"error":"kind must be 'daily' or 'persistent'"}"#.to_owned(),
        );
    }

    let day_date = body["day_date"].as_str();
    if kind == "daily" && day_date.is_none() {
        return (
            "400 Bad Request",
            r#"{"error":"day_date is required for daily entries"}"#.to_owned(),
        );
    }

    let Some(content) = body["content"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: content"}"#.to_owned(),
        );
    };

    match db::insert_schedule(pool, kind, day_date, content).await {
        Some(id) => ("200 OK", json!({"id": id}).to_string()),
        None => (
            "500 Internal Server Error",
            r#"{"error":"failed to create schedule entry"}"#.to_owned(),
        ),
    }
}

/// `DELETE /schedule/{id}` -- delete a schedule entry.
async fn schedule_delete(cfg: &DaemonConfig, raw: &str, id: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    if db::delete_schedule(pool, id).await {
        ("200 OK", r#"{"status":"deleted"}"#.to_owned())
    } else {
        ("404 Not Found", r#"{"error":"not found"}"#.to_owned())
    }
}

// ---------------------------------------------------------------------------
// Availability schedules + sleep mode (migration 017)
// ---------------------------------------------------------------------------

fn parse_hhmm(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 5
        && bytes[2] == b':'
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
        && s[..2].parse::<u8>().is_ok_and(|h| h < 24)
        && s[3..].parse::<u8>().is_ok_and(|m| m < 60)
}

fn parse_iso_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && s[..4].parse::<u16>().is_ok()
        && s[5..7].parse::<u8>().is_ok_and(|m| (1..=12).contains(&m))
        && s[8..].parse::<u8>().is_ok_and(|d| (1..=31).contains(&d))
}

async fn availability_list(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let slots = db::list_schedule_slots(pool).await;
    let sleep = db::get_sleep_state(pool).await;
    (
        "200 OK",
        json!({ "slots": slots, "sleep": sleep }).to_string(),
    )
}

async fn availability_rename_slot(
    cfg: &DaemonConfig,
    raw: &str,
    slot: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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
    let name = body["name"].as_str().unwrap_or("");
    if name.len() > 64 {
        return (
            "400 Bad Request",
            r#"{"error":"name must be 64 chars or fewer"}"#.to_owned(),
        );
    }
    if db::rename_schedule_slot(pool, slot, name).await {
        ("200 OK", r#"{"status":"ok"}"#.to_owned())
    } else {
        (
            "400 Bad Request",
            r#"{"error":"invalid slot (must be A/B/C)"}"#.to_owned(),
        )
    }
}

#[allow(clippy::too_many_lines)]
async fn availability_window_create(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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

    let Some(slot) = body["slot"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: slot"}"#.to_owned(),
        );
    };
    let Some(kind) = body["kind"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: kind"}"#.to_owned(),
        );
    };
    let Some(start_time) = body["start_time"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: start_time (HH:MM)"}"#.to_owned(),
        );
    };
    let Some(end_time) = body["end_time"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: end_time (HH:MM)"}"#.to_owned(),
        );
    };
    if !parse_hhmm(start_time) || !parse_hhmm(end_time) || start_time >= end_time {
        return (
            "400 Bad Request",
            r#"{"error":"start_time/end_time must be HH:MM and start < end"}"#.to_owned(),
        );
    }

    let result = match kind {
        "weekly" => {
            let Some(mask) = body["weekday_mask"].as_i64() else {
                return (
                    "400 Bad Request",
                    r#"{"error":"weekly windows require weekday_mask (int, bits 0..6 = Sun..Sat)"}"#
                        .to_owned(),
                );
            };
            if !(1..=127).contains(&mask) {
                return (
                    "400 Bad Request",
                    r#"{"error":"weekday_mask must be between 1 and 127"}"#.to_owned(),
                );
            }
            let m: i32 = match mask.try_into() {
                Ok(v) => v,
                Err(_) => {
                    return (
                        "400 Bad Request",
                        r#"{"error":"weekday_mask out of range"}"#.to_owned(),
                    );
                }
            };
            db::insert_weekly_window(pool, slot, m, start_time, end_time).await
        }
        "oneoff" => {
            let Some(day_date) = body["day_date"].as_str() else {
                return (
                    "400 Bad Request",
                    r#"{"error":"oneoff windows require day_date (YYYY-MM-DD)"}"#.to_owned(),
                );
            };
            if !parse_iso_date(day_date) {
                return (
                    "400 Bad Request",
                    r#"{"error":"day_date must be YYYY-MM-DD"}"#.to_owned(),
                );
            }
            db::insert_oneoff_window(pool, slot, day_date, start_time, end_time).await
        }
        _ => {
            return (
                "400 Bad Request",
                r#"{"error":"kind must be 'weekly' or 'oneoff'"}"#.to_owned(),
            );
        }
    };

    match result {
        Some(id) => ("200 OK", json!({ "id": id }).to_string()),
        None => (
            "500 Internal Server Error",
            r#"{"error":"failed to insert window"}"#.to_owned(),
        ),
    }
}

async fn availability_window_delete(
    cfg: &DaemonConfig,
    raw: &str,
    id: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    if db::delete_schedule_window(pool, id).await {
        ("200 OK", r#"{"status":"deleted"}"#.to_owned())
    } else {
        ("404 Not Found", r#"{"error":"not found"}"#.to_owned())
    }
}

async fn availability_contact_slot(
    cfg: &DaemonConfig,
    raw: &str,
    phone: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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
    let slot = body["slot"].as_str();
    // null or absent → clear assignment.
    let slot_opt = if body["slot"].is_null() { None } else { slot };
    if db::set_contact_schedule_slot(pool, phone, slot_opt).await {
        // Immediately re-evaluate so the UI sees the correct state without waiting 60s.
        db::tick_schedule(pool).await;
        ("200 OK", json!({ "slot": slot_opt }).to_string())
    } else {
        (
            "400 Bad Request",
            r#"{"error":"invalid slot (must be A/B/C or null)"}"#.to_owned(),
        )
    }
}

async fn sleep_get(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let state = db::get_sleep_state(pool).await;
    (
        "200 OK",
        serde_json::to_string(&state).unwrap_or_else(|_| "{}".into()),
    )
}

async fn sleep_start(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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
    let Some(awake_by) = body["awake_by_local"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: awake_by_local (HH:MM in America/Denver)"}"#.to_owned(),
        );
    };
    if !parse_hhmm(awake_by) {
        return (
            "400 Bad Request",
            r#"{"error":"awake_by_local must be HH:MM"}"#.to_owned(),
        );
    }
    match db::start_sleep(pool, awake_by).await {
        Ok(state) => {
            db::tick_schedule(pool).await;
            (
                "200 OK",
                serde_json::to_string(&state).unwrap_or_else(|_| "{}".into()),
            )
        }
        Err(e) => (
            "500 Internal Server Error",
            json!({ "error": format!("failed to start sleep: {e}") }).to_string(),
        ),
    }
}

async fn sleep_end(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    match db::end_sleep(pool).await {
        Ok(()) => {
            db::tick_schedule(pool).await;
            ("200 OK", r#"{"status":"ok"}"#.to_owned())
        }
        Err(e) => (
            "500 Internal Server Error",
            json!({ "error": format!("failed to end sleep: {e}") }).to_string(),
        ),
    }
}

async fn sleep_contacts_list(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let phones = db::list_sleep_contacts(pool).await;
    ("200 OK", json!({ "phones": phones }).to_string())
}

async fn sleep_contact_add(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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
    let Some(phone) = body["phone"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: phone"}"#.to_owned(),
        );
    };
    db::add_sleep_contact(pool, phone).await;
    db::tick_schedule(pool).await;
    ("200 OK", r#"{"status":"ok"}"#.to_owned())
}

async fn sleep_contact_remove(
    cfg: &DaemonConfig,
    raw: &str,
    phone: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let removed = db::remove_sleep_contact(pool, phone).await;
    db::tick_schedule(pool).await;
    if removed {
        ("200 OK", r#"{"status":"removed"}"#.to_owned())
    } else {
        (
            "404 Not Found",
            r#"{"error":"not in sleep list"}"#.to_owned(),
        )
    }
}

/// `GET /facts` -- return the shareable-facts blob.
async fn facts_get(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let content = db::get_facts(pool).await;
    ("200 OK", json!({ "content": content }).to_string())
}

/// `PUT /facts` -- overwrite the shareable-facts blob.
async fn facts_put(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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

    let Some(content) = body["content"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: content"}"#.to_owned(),
        );
    };

    // Cap at 16 KiB so a runaway paste can't bloat the system prompt on
    // every SMS turn.
    if content.len() > 16 * 1024 {
        return (
            "413 Payload Too Large",
            r#"{"error":"facts content exceeds 16 KiB"}"#.to_owned(),
        );
    }

    match db::set_facts(pool, content).await {
        Ok(()) => ("200 OK", r#"{"status":"saved"}"#.to_owned()),
        Err(e) => {
            eprintln!("[ghost facts] set_facts failed: {e}");
            (
                "500 Internal Server Error",
                r#"{"error":"failed to save facts"}"#.to_owned(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// /code/health — unauthenticated coder status dot (Phase A.6)
//
// Returns the kill-switch state (env var or settings flag), the remaining
// coder budget for today, and a daemon-alive flag. Kept unauthenticated so
// the dashboard status dot works before Isaac types his key.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Coder file index + templates (Phase C)
//
// `/code/index/*` endpoints operate on the `coder_file_index` table — a
// pgvector store of per-file signature embeddings the coder agent uses to
// find relevant files without reading the whole repo. Rebuilds are
// serialized via `cfg.coder_rebuild_lock` (try_lock → 409 on contention).
// Single-file indexing and stats are unserialized by design.
//
// `/code/templates` serves bundled template metadata; `/code/templates/stamp`
// renders a named template + placeholders and returns `{path, content}`
// WITHOUT writing to disk — the caller feeds the content through the normal
// diff queue if it wants to apply it.
// ---------------------------------------------------------------------------

async fn code_index_rebuild(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    let Ok(_guard) = cfg.coder_rebuild_lock.try_lock() else {
        return (
            "409 Conflict",
            r#"{"error":"rebuild already in progress"}"#.to_owned(),
        );
    };

    let root = crate::agents::coder::repo_root(pool).await;
    if !root.exists() || !root.is_dir() {
        return (
            "400 Bad Request",
            json!({ "error": "repo_root missing", "resolved": root.display().to_string() })
                .to_string(),
        );
    }

    match crate::agents::coder::index::index_repo(pool, &root).await {
        Ok(stats) => (
            "200 OK",
            json!({
                "files_scanned": stats.files_scanned,
                "files_embedded": stats.files_embedded,
                "duration_ms": stats.duration_ms,
                "repo_root": root.display().to_string(),
            })
            .to_string(),
        ),
        Err(e) => {
            eprintln!("{LOG_PREFIX} /code/index/rebuild failed: {e}");
            (
                "500 Internal Server Error",
                json!({ "error": "rebuild failed", "detail": e.to_string() }).to_string(),
            )
        }
    }
}

async fn code_index_file(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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
    let Some(path_str) = body.get("path").and_then(|v| v.as_str()) else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: path"}"#.to_owned(),
        );
    };

    let root = crate::agents::coder::repo_root(pool).await;
    let rel = PathBuf::from(path_str);
    match crate::agents::coder::index::index_file(pool, &root, &rel).await {
        Ok(outcome) => (
            "200 OK",
            json!({
                "path": path_str,
                "skipped_unchanged": outcome.skipped_unchanged,
                "embedded": outcome.embedded,
            })
            .to_string(),
        ),
        Err(e) => (
            "500 Internal Server Error",
            json!({ "error": "index_file failed", "detail": e.to_string() }).to_string(),
        ),
    }
}

async fn code_index_stats(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    match crate::agents::coder::index::stored_stats(pool).await {
        Ok(s) => (
            "200 OK",
            serde_json::to_string(&s).unwrap_or_else(|_| "{}".to_string()),
        ),
        Err(e) => (
            "500 Internal Server Error",
            json!({ "error": "stats failed", "detail": e.to_string() }).to_string(),
        ),
    }
}

fn code_templates_list(raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let body = serde_json::to_string(crate::agents::coder::templates::all())
        .unwrap_or_else(|_| "[]".to_string());
    ("200 OK", body)
}

async fn code_templates_stamp(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
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

    let Some(name) = body.get("template_name").and_then(|v| v.as_str()) else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: template_name"}"#.to_owned(),
        );
    };
    let placeholders = body
        .get("placeholders")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let Some(tmpl) = crate::agents::coder::templates::by_name(name) else {
        return (
            "404 Not Found",
            json!({ "error": "unknown template", "template_name": name }).to_string(),
        );
    };

    // Resolve migrations dir relative to the coder's repo_root so
    // `{{next_migration_number}}` picks up existing files even when the
    // daemon's cwd isn't the repo root. DB is guaranteed by the endpoints
    // that need it, but template stamping itself doesn't require one — we
    // fall back to cwd when the pool is absent.
    let migrations_dir = match cfg.db.as_deref() {
        Some(pool) => {
            let root = crate::agents::coder::repo_root(pool).await;
            Some(root.join("rust").join("migrations"))
        }
        None => None,
    };

    match crate::agents::coder::templates::stamp(tmpl, &placeholders, migrations_dir.as_deref()) {
        Ok(out) => (
            "200 OK",
            json!({ "path": out.path, "content": out.content }).to_string(),
        ),
        Err(e) => (
            "400 Bad Request",
            json!({ "error": "stamp failed", "detail": e.to_string() }).to_string(),
        ),
    }
}

// Filesystem watcher: feeds notify events through a tokio channel, debounces
// a 3s idle window, then re-indexes changed paths and deletes removed ones.
fn spawn_coder_watcher(pool: Arc<PgPool>, root: PathBuf) -> Result<(), String> {
    use notify::{RecommendedWatcher, RecursiveMode, Watcher};

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<notify::Event>();
    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            if let Ok(evt) = res {
                let _ = tx.send(evt);
            }
        },
        notify::Config::default(),
    )
    .map_err(|e| format!("watcher init: {e}"))?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| format!("watch {}: {e}", root.display()))?;

    eprintln!(
        "{LOG_PREFIX} coder index watcher: watching {}",
        root.display()
    );

    tokio::spawn(async move {
        // Move the watcher into the task so it stays alive for the duration.
        let _watcher = watcher;

        let debounce = std::time::Duration::from_secs(3);
        let mut changed: HashSet<PathBuf> = HashSet::new();
        let mut removed: HashSet<PathBuf> = HashSet::new();
        let mut deadline: Option<tokio::time::Instant> = None;

        loop {
            let sleep_until = deadline.unwrap_or_else(|| {
                tokio::time::Instant::now() + std::time::Duration::from_secs(3600)
            });
            tokio::select! {
                maybe_evt = rx.recv() => {
                    let Some(evt) = maybe_evt else { break; };
                    if !record_event(&evt, &root, &mut changed, &mut removed) {
                        continue;
                    }
                    deadline = Some(tokio::time::Instant::now() + debounce);
                }
                () = tokio::time::sleep_until(sleep_until) => {
                    if deadline.is_none() {
                        continue;
                    }
                    deadline = None;
                    let to_remove: Vec<PathBuf> = removed.drain().collect();
                    let to_index: Vec<PathBuf> = changed.drain().collect();
                    for rel in &to_remove {
                        if let Err(e) =
                            crate::agents::coder::index::remove_path(&pool, rel).await
                        {
                            eprintln!("{LOG_PREFIX} watcher remove {} failed: {e}", rel.display());
                        }
                    }
                    for rel in &to_index {
                        if let Err(e) =
                            crate::agents::coder::index::index_file(&pool, &root, rel).await
                        {
                            eprintln!("{LOG_PREFIX} watcher index {} failed: {e}", rel.display());
                        }
                    }
                }
            }
        }
    });

    Ok(())
}

/// Classify a notify event into the debounce sets. Returns `true` when at
/// least one included path landed in `changed`/`removed`, which gates the
/// deadline bump.
fn record_event(
    evt: &notify::Event,
    root: &std::path::Path,
    changed: &mut HashSet<PathBuf>,
    removed: &mut HashSet<PathBuf>,
) -> bool {
    use notify::EventKind;
    let mut touched = false;
    for path in &evt.paths {
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        if crate::agents::coder::index::is_path_excluded(rel) {
            continue;
        }
        let rel_owned = rel.to_path_buf();
        match evt.kind {
            EventKind::Remove(_) => {
                changed.remove(&rel_owned);
                removed.insert(rel_owned);
                touched = true;
            }
            EventKind::Create(_) | EventKind::Modify(_) => {
                removed.remove(&rel_owned);
                changed.insert(rel_owned);
                touched = true;
            }
            _ => {}
        }
    }
    touched
}

async fn code_health(cfg: &DaemonConfig) -> (&'static str, String) {
    let env_kill = std::env::var("GHOST_CODING_AGENT")
        .ok()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("off"));

    let mut setting_kill = false;
    let mut cap: i32 = 200;
    let mut spent: i32 = 0;

    if let Some(pool) = cfg.db.as_deref() {
        if let Some(v) = db::get_setting::<bool>(pool, "coder.kill_switch").await {
            setting_kill = v;
        }
        if let Some(v) = db::get_setting::<i32>(pool, "coder.budget_cents_per_day").await {
            if v > 0 {
                cap = v;
            }
        }
        if let Ok(v) = db::spend_today(pool, "coder").await {
            spent = v;
        }
    }

    let remaining = cap.saturating_sub(spent).max(0);
    let body = json!({
        "kill_switch": env_kill || setting_kill,
        "budget_remaining_cents": remaining,
        "daemon_alive": true,
    })
    .to_string();
    ("200 OK", body)
}

// ---------------------------------------------------------------------------
// Coder endpoints (Phase B.6)
//
// All bearer-authed. The agents self-account via `db::record_spend`; the
// dispatcher path is bypassed here so endpoints can carry an explicit
// `chat_id` from the body through to the coder (needed for diff queueing
// and condensate retrieval).
// ---------------------------------------------------------------------------

fn body_from_raw(raw: &str) -> Option<serde_json::Value> {
    let body_str = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .map_or("", |(_, b)| b);
    serde_json::from_str::<serde_json::Value>(body_str).ok()
}

fn parse_history_from_body(body: &serde_json::Value) -> Vec<serde_json::Value> {
    body["history"]
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
                .take(10)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

async fn code_chat_handler(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    use crate::agents::Agent as _;
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(body) = body_from_raw(raw) else {
        return (
            "400 Bad Request",
            r#"{"error":"body must be JSON"}"#.to_owned(),
        );
    };
    let message = body["message"].as_str().unwrap_or("").trim().to_string();
    if message.is_empty() {
        return (
            "400 Bad Request",
            r#"{"error":"missing message"}"#.to_owned(),
        );
    }
    let chat_id = match body["chat_id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
    {
        Some(id) => id,
        None => uuid::Uuid::new_v4(),
    };
    let history = parse_history_from_body(&body);

    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    let Some(job_id) = db::create_job(pool, &message, "coder", "dashboard", None).await else {
        return (
            "500 Internal Server Error",
            r#"{"error":"failed to create job"}"#.to_owned(),
        );
    };

    let repo_root = crate::agents::coder::repo_root(pool).await;
    let agent = crate::agents::coder::CoderAgent::new(repo_root, chat_id);
    let req = crate::agents::AgentRequest {
        message: message.clone(),
        history,
        source: crate::agents::Source::Dashboard,
        job_id: job_id.clone(),
        sender_phone: None,
    };

    match agent.handle(req, pool).await {
        Ok(resp) => {
            db::update_job_done(pool, &job_id, &resp.text).await;
            let pending_ids = list_pending_diff_ids_for_chat(pool, chat_id).await;
            let body = json!({
                "response": resp.text,
                "job_id": job_id,
                "chat_id": chat_id.to_string(),
                "tokens": {
                    "input": resp.usage.tokens_in,
                    "output": resp.usage.tokens_out,
                },
                "pending_diff_ids": pending_ids,
            })
            .to_string();
            ("200 OK", body)
        }
        Err(e) => {
            db::update_job_failed(pool, &job_id, &e).await;
            let body = json!({ "error": e, "job_id": job_id }).to_string();
            ("502 Bad Gateway", body)
        }
    }
}

async fn code_brainstorm_handler(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    use crate::agents::Agent as _;
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(body) = body_from_raw(raw) else {
        return (
            "400 Bad Request",
            r#"{"error":"body must be JSON"}"#.to_owned(),
        );
    };
    let message = body["message"].as_str().unwrap_or("").trim().to_string();
    if message.is_empty() {
        return (
            "400 Bad Request",
            r#"{"error":"missing message"}"#.to_owned(),
        );
    }
    let history = parse_history_from_body(&body);
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    let Some(job_id) = db::create_job(pool, &message, "brainstorm", "dashboard", None).await else {
        return (
            "500 Internal Server Error",
            r#"{"error":"failed to create job"}"#.to_owned(),
        );
    };

    let req = crate::agents::AgentRequest {
        message: message.clone(),
        history,
        source: crate::agents::Source::Dashboard,
        job_id: job_id.clone(),
        sender_phone: None,
    };
    let agent = crate::agents::brainstorm::BrainstormAgent::new();
    match agent.handle(req, pool).await {
        Ok(resp) => {
            db::update_job_done(pool, &job_id, &resp.text).await;
            let body = json!({
                "response": resp.text,
                "job_id": job_id,
                "tokens": {
                    "input": resp.usage.tokens_in,
                    "output": resp.usage.tokens_out,
                },
            })
            .to_string();
            ("200 OK", body)
        }
        Err(e) => {
            db::update_job_failed(pool, &job_id, &e).await;
            let body = json!({ "error": e, "job_id": job_id }).to_string();
            ("502 Bad Gateway", body)
        }
    }
}

async fn code_orchestrate_handler(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(body) = body_from_raw(raw) else {
        return (
            "400 Bad Request",
            r#"{"error":"body must be JSON"}"#.to_owned(),
        );
    };
    let spec = body["spec"].as_str().unwrap_or("").trim().to_string();
    if spec.is_empty() {
        return ("400 Bad Request", r#"{"error":"missing spec"}"#.to_owned());
    }
    let chat_id = body["chat_id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let repo_root = crate::agents::coder::repo_root(pool).await;
    let agent = crate::agents::orchestrator::OrchestratorAgent::new(repo_root, chat_id);
    match agent.plan(&spec, pool).await {
        Ok(outcome) => {
            let tasks: Vec<serde_json::Value> = outcome
                .tasks
                .iter()
                .map(|(id, t)| {
                    json!({
                        "id": id.to_string(),
                        "task_prompt": t.prompt,
                        "verify_command": t.verify_command,
                    })
                })
                .collect();
            let body = json!({
                "orchestration_id": outcome.orchestration_id.to_string(),
                "tasks": tasks,
            })
            .to_string();
            ("200 OK", body)
        }
        Err(e) => {
            let body = json!({ "error": e }).to_string();
            ("502 Bad Gateway", body)
        }
    }
}

async fn code_orchestrate_run_handler(
    cfg: &DaemonConfig,
    raw: &str,
    id_str: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Ok(orch_id) = uuid::Uuid::parse_str(id_str) else {
        return (
            "400 Bad Request",
            r#"{"error":"invalid orchestration id"}"#.to_owned(),
        );
    };
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    // Verify orchestration exists before kicking off background work.
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT status FROM coder_orchestrations WHERE id = $1")
            .bind(orch_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    if exists.is_none() {
        return ("404 Not Found", r#"{"error":"not found"}"#.to_owned());
    }

    let repo_root = crate::agents::coder::repo_root(pool).await;
    crate::agents::orchestrator::spawn_workers(orch_id, repo_root, pool.clone());
    let body = json!({
        "orchestration_id": orch_id.to_string(),
        "status": "running",
    })
    .to_string();
    ("202 Accepted", body)
}

async fn code_orchestrate_get_handler(
    cfg: &DaemonConfig,
    raw: &str,
    id_str: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Ok(orch_id) = uuid::Uuid::parse_str(id_str) else {
        return (
            "400 Bad Request",
            r#"{"error":"invalid orchestration id"}"#.to_owned(),
        );
    };
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let orch: Option<(String, String, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT status, spec, created_at FROM coder_orchestrations WHERE id = $1")
            .bind(orch_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let Some((status, spec, created_at)) = orch else {
        return ("404 Not Found", r#"{"error":"not found"}"#.to_owned());
    };
    let rows = sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
        ),
    >(
        "SELECT id, task_prompt, status, verify_command, worker_output, completed_at
         FROM coder_tasks WHERE orchestration_id = $1 ORDER BY created_at",
    )
    .bind(orch_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let tasks: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, prompt, status, verify, output, completed_at)| {
            json!({
                "id": id.to_string(),
                "task_prompt": prompt,
                "status": status,
                "verify_command": verify,
                "worker_output": output,
                "completed_at": completed_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    let body = json!({
        "orchestration_id": orch_id.to_string(),
        "spec": spec,
        "status": status,
        "created_at": created_at.to_rfc3339(),
        "tasks": tasks,
    })
    .to_string();
    ("200 OK", body)
}

async fn code_pending_diffs_handler(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let rows = sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            uuid::Uuid,
            String,
            String,
            String,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        "SELECT id, chat_id, path, search, replace, created_at
         FROM coder_pending_diffs WHERE status = 'pending' ORDER BY created_at",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let out: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, chat_id, path, search, replace, created_at)| {
            json!({
                "id": id.to_string(),
                "chat_id": chat_id.to_string(),
                "path": path,
                "search": search,
                "replace": replace,
                "created_at": created_at.to_rfc3339(),
            })
        })
        .collect();
    (
        "200 OK",
        serde_json::to_string(&out).unwrap_or_else(|_| "[]".into()),
    )
}

async fn code_diff_apply_handler(
    cfg: &DaemonConfig,
    raw: &str,
    id_str: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Ok(diff_id) = uuid::Uuid::parse_str(id_str) else {
        return (
            "400 Bad Request",
            r#"{"error":"invalid diff id"}"#.to_owned(),
        );
    };
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT path, search, replace FROM coder_pending_diffs
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(diff_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some((path, search, replace)) = row else {
        return (
            "404 Not Found",
            r#"{"error":"diff not found or already resolved"}"#.to_owned(),
        );
    };

    let repo_root = crate::agents::coder::repo_root(pool).await;
    let Ok(abs) = crate::agents::tools::resolve_within_repo(&repo_root, &path) else {
        return ("400 Bad Request", r#"{"error":"path escape"}"#.to_owned());
    };
    let original = match tokio::fs::read_to_string(&abs).await {
        Ok(s) => s,
        Err(e) => {
            return (
                "500 Internal Server Error",
                json!({ "error": format!("read failed: {e}") }).to_string(),
            );
        }
    };
    if original.matches(search.as_str()).count() != 1 {
        return (
            "409 Conflict",
            json!({ "error": "search string is no longer unique in the file" }).to_string(),
        );
    }
    let updated = original.replacen(&search, &replace, 1);
    if let Err(e) = tokio::fs::write(&abs, updated).await {
        return (
            "500 Internal Server Error",
            json!({ "error": format!("write failed: {e}") }).to_string(),
        );
    }
    let _ = sqlx::query(
        "UPDATE coder_pending_diffs SET status = 'applied', resolved_at = now() WHERE id = $1",
    )
    .bind(diff_id)
    .execute(pool)
    .await;
    let body = json!({ "ok": true, "path": path }).to_string();
    ("200 OK", body)
}

async fn code_diff_reject_handler(
    cfg: &DaemonConfig,
    raw: &str,
    id_str: &str,
) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Ok(diff_id) = uuid::Uuid::parse_str(id_str) else {
        return (
            "400 Bad Request",
            r#"{"error":"invalid diff id"}"#.to_owned(),
        );
    };
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let res = sqlx::query(
        "UPDATE coder_pending_diffs SET status = 'rejected', resolved_at = now()
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(diff_id)
    .execute(pool)
    .await;
    match res {
        Ok(r) if r.rows_affected() == 0 => (
            "404 Not Found",
            r#"{"error":"diff not found or already resolved"}"#.to_owned(),
        ),
        Ok(_) => ("200 OK", r#"{"ok": true}"#.to_owned()),
        Err(e) => (
            "500 Internal Server Error",
            json!({ "error": format!("db error: {e}") }).to_string(),
        ),
    }
}

async fn code_spend_handler(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    let today_cents = db::spend_today(pool, "coder").await.unwrap_or(0);
    let cap_cents = db::get_setting::<i32>(pool, "coder.budget_cents_per_day")
        .await
        .unwrap_or(200);
    let week_cents: i32 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost_cents), 0)::int
         FROM coder_spend
         WHERE agent = 'coder'
           AND day >= ((now() AT TIME ZONE 'America/Denver')::date - INTERVAL '7 days')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    let by_model_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT model, COALESCE(SUM(cost_cents), 0)::bigint
         FROM coder_spend
         WHERE agent = 'coder'
           AND day = (now() AT TIME ZONE 'America/Denver')::date
         GROUP BY model",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut by_model = serde_json::Map::new();
    for (model, cents) in by_model_rows {
        by_model.insert(model, json!(cents));
    }
    let body = json!({
        "today_cents": today_cents,
        "cap_cents": cap_cents,
        "this_week_cents": week_cents,
        "by_model": by_model,
    })
    .to_string();
    ("200 OK", body)
}

async fn list_pending_diff_ids_for_chat(pool: &sqlx::PgPool, chat_id: uuid::Uuid) -> Vec<String> {
    let rows: Vec<(uuid::Uuid,)> = sqlx::query_as(
        "SELECT id FROM coder_pending_diffs
         WHERE chat_id = $1 AND status = 'pending' ORDER BY created_at",
    )
    .bind(chat_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter().map(|(id,)| id.to_string()).collect()
}

// ---------------------------------------------------------------------------
// SSE — token event stream (Phase A.4)
//
// `GET /stream/tokens/:job_id` holds the socket open and writes `data: ...
// \n\n` frames for every TokenEvent matching the job_id. Bearer auth is
// required; `Host` validation already ran upstream before we land here.
// Heartbeat every 30s keeps idle connections alive through proxies. Clients
// disconnect by closing the socket — we detect via write error and exit.
// ---------------------------------------------------------------------------

/// Inspect the request line for `GET /stream/tokens/:job_id`. Returns the
/// `job_id` segment if matched. Query string and trailing slash tolerated.
fn parse_stream_tokens_path(raw: &str) -> Option<String> {
    let first = raw.lines().next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    if method != "GET" {
        return None;
    }
    let path_clean = path.split('?').next().unwrap_or(path);
    let rest = path_clean.strip_prefix("/stream/tokens/")?;
    let job = rest.trim_end_matches('/');
    if job.is_empty() {
        return None;
    }
    Some(job.to_string())
}

async fn stream_tokens_handler(
    stream: &mut TcpStream,
    raw: &str,
    job_id_str: &str,
    allowed_origin: Option<&str>,
) {
    if !auth_matches(raw) {
        write_response(
            stream,
            "401 Unauthorized",
            r#"{"error":"unauthorized"}"#,
            allowed_origin,
            "application/json",
        )
        .await;
        return;
    }

    let Ok(target) = uuid::Uuid::parse_str(job_id_str) else {
        write_response(
            stream,
            "400 Bad Request",
            r#"{"error":"invalid job_id"}"#,
            allowed_origin,
            "application/json",
        )
        .await;
        return;
    };

    // Write the SSE response head ourselves — `write_response` is for
    // one-shot bodies with a known Content-Length.
    let mut head = String::from(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/event-stream\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\n\
         Connection: close\r\n",
    );
    if let Some(origin) = allowed_origin {
        let _ = std::fmt::Write::write_fmt(
            &mut head,
            format_args!(
                "Access-Control-Allow-Origin: {origin}\r\n\
                 Vary: Origin\r\n\
                 Access-Control-Allow-Credentials: true\r\n\
                 Access-Control-Allow-Private-Network: true\r\n"
            ),
        );
    }
    head.push_str("\r\n");

    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    // Initial comment frame — proxies buffer otherwise.
    if stream.write_all(b": ready\n\n").await.is_err() {
        return;
    }

    let mut rx = crate::infra::token_stream::subscribe();
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
    heartbeat.tick().await; // skip immediate tick

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(event) => {
                        if event.job_id != target {
                            continue;
                        }
                        let payload = serde_json::to_string(&event)
                            .unwrap_or_else(|_| "{}".to_string());
                        let frame = format!("data: {payload}\n\n");
                        if stream.write_all(frame.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Dropped some frames — client will reconnect if it cares.
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
            _ = heartbeat.tick() => {
                if stream.write_all(b": heartbeat\n\n").await.is_err() {
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Settings KV endpoints (Phase A)
//
// `GET /settings` returns the full key→value map. `PUT /settings/:key` writes
// one key. Both require bearer auth. Only keys in `WRITABLE_SETTINGS` can be
// mutated — anything else returns 400. The provider router caches settings
// in memory; writes are picked up within 60s.
// ---------------------------------------------------------------------------

const WRITABLE_SETTINGS: &[&str] = &[
    "provider.default",
    "provider.per_agent",
    "coder.budget_cents_per_day",
    "coder.auto_apply",
    "coder.summarize_as_you_go",
    "coder.kill_switch",
];

async fn settings_list(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    match db::list_settings(pool).await {
        Ok(rows) => {
            let map: serde_json::Map<String, serde_json::Value> = rows.into_iter().collect();
            ("200 OK", serde_json::Value::Object(map).to_string())
        }
        Err(e) => {
            eprintln!("[ghost settings] list failed: {e}");
            (
                "500 Internal Server Error",
                r#"{"error":"failed to list settings"}"#.to_owned(),
            )
        }
    }
}

async fn settings_put(cfg: &DaemonConfig, raw: &str, key: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    if !WRITABLE_SETTINGS.contains(&key) {
        return (
            "400 Bad Request",
            json!({ "error": "unknown settings key", "key": key }).to_string(),
        );
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
    let Some(value) = body.get("value").cloned() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: value"}"#.to_owned(),
        );
    };

    match db::set_setting(pool, key, &value).await {
        Ok(()) => (
            "200 OK",
            json!({ "status": "saved", "key": key }).to_string(),
        ),
        Err(e) => {
            eprintln!("[ghost settings] set failed for {key}: {e}");
            (
                "500 Internal Server Error",
                r#"{"error":"failed to save setting"}"#.to_owned(),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Trigger CRUD handlers (Wave 5)
//
// `infra::scheduler` polls `scheduled_triggers` every 30s; these endpoints are
// how Isaac (or the dashboard) registers new cron-fired agent jobs. All three
// require `Authorization: Bearer <GHOST_DAEMON_KEY>` like `POST /sms/send`.
// ---------------------------------------------------------------------------

/// Allow-list of agent names that can be scheduled. Includes agents that
/// don't yet exist in the dispatcher (`chief_of_staff`, `dreamer`) so Isaac
/// can schedule ahead of Wave 5.5 without redeploying.
const SCHEDULABLE_AGENTS: &[&str] = &[
    "research",
    "calendar",
    "docs",
    "chat_dispatcher",
    "director",
    "chief_of_staff",
    "dreamer",
];

/// `GET /triggers` — list all scheduled triggers, newest-first, capped at 100.
async fn triggers_list(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    match db::list_scheduled_triggers(pool, 100).await {
        Ok(triggers) => {
            let items: Vec<serde_json::Value> = triggers
                .iter()
                .map(|t| {
                    json!({
                        "id": t.id.to_string(),
                        "name": t.name,
                        "cron_expr": t.cron_expr,
                        "agent": t.agent,
                        "payload": t.payload,
                        "enabled": t.enabled,
                        "last_fired_at": t.last_fired_at.map(|ts| ts.to_rfc3339()),
                        "next_fire_at": t.next_fire_at.map(|ts| ts.to_rfc3339()),
                    })
                })
                .collect();
            ("200 OK", json!({ "triggers": items }).to_string())
        }
        Err(e) => {
            eprintln!("[ghost daemon] triggers_list query failed: {e}");
            (
                "500 Internal Server Error",
                r#"{"error":"failed to list triggers"}"#.to_owned(),
            )
        }
    }
}

/// `POST /triggers` — register a new cron-fired agent job.
///
/// Body: `{"name":"morning_brief","cron_expr":"0 0 9 * * *","agent":"research",
///         "payload":"?what's new in rust async","enabled":true}`
///
/// `cron_expr` uses the `cron` crate's 6-field format
/// (`<sec> <min> <hr> <dom> <mon> <dow>`). `next_fire_at` is computed from it
/// at insert time. `enabled` defaults to `true`.
async fn trigger_create(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
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

    let Some(name) = body["name"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: name"}"#.to_owned(),
        );
    };
    if name.is_empty() || name.len() > 128 {
        return (
            "400 Bad Request",
            r#"{"error":"name must be non-empty and <=128 chars"}"#.to_owned(),
        );
    }

    let Some(cron_expr) = body["cron_expr"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: cron_expr"}"#.to_owned(),
        );
    };

    let Some(agent) = body["agent"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: agent"}"#.to_owned(),
        );
    };
    if !SCHEDULABLE_AGENTS.contains(&agent) {
        return (
            "400 Bad Request",
            json!({
                "error": format!("unknown agent '{agent}'"),
                "allowed": SCHEDULABLE_AGENTS,
            })
            .to_string(),
        );
    }

    let Some(payload) = body["payload"].as_str() else {
        return (
            "400 Bad Request",
            r#"{"error":"missing field: payload"}"#.to_owned(),
        );
    };
    if payload.is_empty() || payload.len() > 4096 {
        return (
            "400 Bad Request",
            r#"{"error":"payload must be non-empty and <=4096 chars"}"#.to_owned(),
        );
    }

    let enabled = body["enabled"].as_bool().unwrap_or(true);

    let next_fire_at = match next_fire_from_cron(cron_expr) {
        Ok(ts) => ts,
        Err(e) => {
            return ("400 Bad Request", json!({ "error": e }).to_string());
        }
    };

    let new_trigger = db::NewScheduledTrigger {
        name,
        cron_expr,
        agent,
        payload,
        enabled,
        next_fire_at,
    };

    match db::insert_scheduled_trigger(pool, new_trigger).await {
        Ok(id) => (
            "200 OK",
            json!({
                "id": id.to_string(),
                "next_fire_at": next_fire_at.to_rfc3339(),
            })
            .to_string(),
        ),
        Err(e) => {
            eprintln!("[ghost daemon] insert_scheduled_trigger failed: {e}");
            (
                "500 Internal Server Error",
                r#"{"error":"failed to insert trigger"}"#.to_owned(),
            )
        }
    }
}

/// `DELETE /triggers/{id}` — delete a scheduled trigger by UUID.
async fn trigger_delete(cfg: &DaemonConfig, raw: &str, id: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let Ok(parsed) = uuid::Uuid::parse_str(id) else {
        return ("400 Bad Request", r#"{"error":"invalid uuid"}"#.to_owned());
    };
    match db::delete_scheduled_trigger(pool, parsed).await {
        Ok(true) => ("200 OK", r#"{"deleted":true}"#.to_owned()),
        Ok(false) => ("404 Not Found", r#"{"error":"not found"}"#.to_owned()),
        Err(e) => {
            eprintln!("[ghost daemon] delete_scheduled_trigger failed: {e}");
            (
                "500 Internal Server Error",
                r#"{"error":"failed to delete trigger"}"#.to_owned(),
            )
        }
    }
}

/// Parse a 6-field cron expression and return the next fire time in UTC.
/// Mirrors `infra::scheduler::next_fire` so trigger registration computes
/// `next_fire_at` the same way the scheduler will recompute it post-fire.
fn next_fire_from_cron(cron_expr: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    use std::str::FromStr;
    let schedule = cron::Schedule::from_str(cron_expr).map_err(|e| {
        format!(
            "invalid cron expression: {e} \
             (expected 6-field format: '<sec> <min> <hr> <dom> <mon> <dow>')"
        )
    })?;
    schedule
        .upcoming(chrono::Utc)
        .next()
        .ok_or_else(|| "cron produced no upcoming fire time".to_string())
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
// Observability endpoints (Wave 3): /events, /agents/budget, /agents
// ---------------------------------------------------------------------------

/// `GET /events?limit=50&agent=<name>` — recent `ghost_events` rows, newest first.
async fn events_list(cfg: &DaemonConfig, raw: &str, query_string: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    let params = parse_urlencoded(query_string);
    let limit: i64 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let agent = params.get("agent").map(String::as_str);

    let events_result = match agent {
        Some(a) if !a.is_empty() => crate::infra::events::for_agent(pool, a, limit).await,
        _ => crate::infra::events::recent(pool, limit).await,
    };

    let events = match events_result {
        Ok(v) => v,
        Err(e) => {
            return (
                "500 Internal Server Error",
                json!({ "error": format!("failed to load events: {e}") }).to_string(),
            );
        }
    };

    let events_json: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            json!({
                "id": e.id.to_string(),
                "job_id": e.job_id.map(|j| j.to_string()),
                "agent": e.agent,
                "tier": e.tier,
                "input": e.input,
                "output": e.output,
                "tokens_in": e.tokens_in,
                "tokens_out": e.tokens_out,
                "cost_cents": e.cost_cents,
                "outcome": e.outcome.as_str(),
                "human_correction": e.human_correction,
                "created_at": e.created_at.to_rfc3339(),
            })
        })
        .collect();

    ("200 OK", json!({ "events": events_json }).to_string())
}

/// `GET /agents/budget` — today's spend per agent (cents + call count + cap).
async fn agents_budget(cfg: &DaemonConfig, raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    let Some(pool) = cfg.db.as_deref() else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };

    let statuses = match crate::infra::budget::today(pool).await {
        Ok(v) => v,
        Err(e) => {
            return (
                "500 Internal Server Error",
                json!({ "error": format!("failed to load budget: {e}") }).to_string(),
            );
        }
    };

    let today_json: Vec<serde_json::Value> = statuses
        .iter()
        .map(|s| {
            json!({
                "agent": s.agent,
                "spent_cents": s.spent_cents,
                "cap_cents": s.cap_cents,
                "calls_today": s.calls_today,
                "remaining_cents": s.remaining_cents(),
                "is_blown": s.is_blown(),
            })
        })
        .collect();

    let date = chrono::Utc::now().date_naive().to_string();
    (
        "200 OK",
        json!({ "today": today_json, "date": date }).to_string(),
    )
}

/// `GET /agents` — static agent catalogue (name, tier, trigger, implemented).
/// Wave 4+: replace with dispatcher registry query.
fn agents_list(raw: &str) -> (&'static str, String) {
    if !auth_matches(raw) {
        return ("401 Unauthorized", r#"{"error":"unauthorized"}"#.to_owned());
    }
    // Keep in sync with `crate::agents::intent::Intent` variants and
    // `crate::agents::dispatcher::Dispatcher::dispatch` tier assignments.
    let body = json!({
        "agents": [
            {"name": "chat",           "tier": "fast", "implemented": true,  "trigger": "no prefix"},
            {"name": "director",       "tier": "mid",  "implemented": true,  "trigger": "! prefix"},
            {"name": "research",       "tier": "fast", "implemented": true,  "trigger": "? prefix"},
            {"name": "calendar",       "tier": "fast", "implemented": true,  "trigger": "@ prefix"},
            {"name": "chief_of_staff", "tier": "mid",  "implemented": true,  "trigger": "# prefix"},
            {"name": "docs",           "tier": "fast", "implemented": true,  "trigger": "& prefix"},
            {"name": "dreamer",        "tier": "mid",  "implemented": true,  "trigger": "~ prefix"},
            {"name": "scheduled",      "tier": "fast", "implemented": false, "trigger": "> prefix"},
        ]
    });
    ("200 OK", body.to_string())
}

// ---------------------------------------------------------------------------
// Bible endpoints
// ---------------------------------------------------------------------------

/// `GET /bible/stats` -- table row counts.
async fn bible_stats_handler(db: Option<&PgPool>) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let (verses, pericopes, cross_refs, lexicon) = db::bible_stats(pool).await;
    (
        "200 OK",
        json!({
            "verses": verses,
            "pericopes": pericopes,
            "cross_refs": cross_refs,
            "lexicon_entries": lexicon,
        })
        .to_string(),
    )
}

/// `GET /bible/verse/:book/:chapter/:verse` -- single verse lookup.
async fn bible_verse_handler(db: Option<&PgPool>, path: &str) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    // path = "Genesis/1/1" or "1%20John/3/16"
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() != 3 {
        return (
            "400 Bad Request",
            r#"{"error":"expected /bible/verse/:book/:chapter/:verse"}"#.to_owned(),
        );
    }
    let book = url_decode(parts[0]);
    let Ok(chapter) = parts[1].parse::<i32>() else {
        return (
            "400 Bad Request",
            r#"{"error":"invalid chapter number"}"#.to_owned(),
        );
    };
    let Ok(verse) = parts[2].parse::<i32>() else {
        return (
            "400 Bad Request",
            r#"{"error":"invalid verse number"}"#.to_owned(),
        );
    };
    match db::get_bible_verse(pool, &book, chapter, verse).await {
        Some(v) => ("200 OK", json!(v).to_string()),
        None => ("404 Not Found", r#"{"error":"verse not found"}"#.to_owned()),
    }
}

/// `GET /bible/range/:book/:start_ch/:start_v/:end_ch/:end_v` -- verse range.
async fn bible_range_handler(db: Option<&PgPool>, path: &str) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let parts: Vec<&str> = path.splitn(5, '/').collect();
    if parts.len() != 5 {
        return (
            "400 Bad Request",
            r#"{"error":"expected /bible/range/:book/:start_ch/:start_v/:end_ch/:end_v"}"#
                .to_owned(),
        );
    }
    let book = url_decode(parts[0]);
    let nums: Vec<Option<i32>> = parts[1..].iter().map(|s| s.parse().ok()).collect();
    if nums.iter().any(Option::is_none) {
        return (
            "400 Bad Request",
            r#"{"error":"invalid chapter/verse numbers"}"#.to_owned(),
        );
    }
    let verses = db::get_bible_verse_range(
        pool,
        &book,
        nums[0].unwrap(),
        nums[1].unwrap(),
        nums[2].unwrap(),
        nums[3].unwrap(),
    )
    .await;
    ("200 OK", json!({ "verses": verses }).to_string())
}

/// `GET /bible/search?q=...` -- semantic verse search (requires embedding).
async fn bible_search_handler(db: Option<&PgPool>, path: &str) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    // Parse query string from path (e.g. "/bible/search?q=love+your+neighbor")
    let query = path
        .split_once('?')
        .and_then(|(_, qs)| {
            qs.split('&')
                .find_map(|pair| pair.strip_prefix("q=").map(url_decode))
        })
        .unwrap_or_default();
    if query.is_empty() {
        return (
            "400 Bad Request",
            r#"{"error":"missing ?q= parameter"}"#.to_owned(),
        );
    }
    match crate::memory::embed(&query).await {
        Ok(emb) => {
            let results = db::search_bible_verses(pool, &emb, 20).await;
            let items: Vec<_> = results
                .iter()
                .map(|(v, dist)| json!({"verse": v, "distance": dist}))
                .collect();
            ("200 OK", json!({ "results": items }).to_string())
        }
        Err(_) => (
            "503 Service Unavailable",
            r#"{"error":"embedding provider unavailable"}"#.to_owned(),
        ),
    }
}

/// `GET /bible/strongs/:id` -- lexicon lookup + verses containing the Strong's number.
async fn bible_strongs_handler(db: Option<&PgPool>, id: &str) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let strongs_id = url_decode(id);
    let entry = db::get_lexicon_entry(pool, &strongs_id).await;
    let verses = db::search_verses_by_strongs(pool, &strongs_id, 50).await;
    match entry {
        Some(e) => (
            "200 OK",
            json!({ "entry": e, "verses": verses }).to_string(),
        ),
        None => {
            if verses.is_empty() {
                (
                    "404 Not Found",
                    r#"{"error":"Strong's number not found"}"#.to_owned(),
                )
            } else {
                // No lexicon entry but verses contain this Strong's
                (
                    "200 OK",
                    json!({ "entry": null, "verses": verses }).to_string(),
                )
            }
        }
    }
}

/// `GET /bible/crossrefs/:book/:chapter/:verse` -- cross-references from a verse.
async fn bible_crossrefs_handler(db: Option<&PgPool>, path: &str) -> (&'static str, String) {
    let Some(pool) = db else {
        return (
            "503 Service Unavailable",
            r#"{"error":"database not configured"}"#.to_owned(),
        );
    };
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() != 3 {
        return (
            "400 Bad Request",
            r#"{"error":"expected /bible/crossrefs/:book/:chapter/:verse"}"#.to_owned(),
        );
    }
    let book = url_decode(parts[0]);
    let Ok(chapter) = parts[1].parse::<i32>() else {
        return (
            "400 Bad Request",
            r#"{"error":"invalid chapter number"}"#.to_owned(),
        );
    };
    let Ok(verse) = parts[2].parse::<i32>() else {
        return (
            "400 Bad Request",
            r#"{"error":"invalid verse number"}"#.to_owned(),
        );
    };
    let from_refs = db::get_cross_refs_from(pool, &book, chapter, verse).await;
    let to_refs = db::get_cross_refs_to(pool, &book, chapter, verse).await;
    (
        "200 OK",
        json!({ "from": from_refs, "to": to_refs }).to_string(),
    )
}

// ---------------------------------------------------------------------------
// History helpers
// ---------------------------------------------------------------------------

/// Merge loadbearing messages into the recent history, deduplicating by content.
/// Loadbearing messages that already appear in `recent` are skipped. The rest
/// are prepended (oldest first) so the model sees them as earlier context.
fn merge_loadbearing(recent: &mut Vec<serde_json::Value>, loadbearing: Vec<(i64, String, String)>) {
    if loadbearing.is_empty() {
        return;
    }

    // Collect content strings already present in recent history for dedup.
    let existing: std::collections::HashSet<String> = recent
        .iter()
        .filter_map(|m| m["content"].as_str().map(String::from))
        .collect();

    let mut extra: Vec<serde_json::Value> = loadbearing
        .into_iter()
        .filter(|(_, _, content)| !existing.contains(content.as_str()))
        .map(|(_, role, content)| serde_json::json!({"role": role, "content": content}))
        .collect();

    if !extra.is_empty() {
        // Prepend loadbearing messages before the recent window.
        extra.append(recent);
        *recent = extra;
    }
}

// ---------------------------------------------------------------------------
// Form body helpers (Twilio inbound webhook)
// ---------------------------------------------------------------------------

/// Normalize a phone number for comparison: strip everything except digits and
/// a leading `+`. E.g. `"+1-234-567-8900"` → `"+12345678900"`.
pub(crate) fn normalize_phone(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_digit() || (i == 0 && c == '+') {
            out.push(c);
        }
    }
    out
}

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
    // GHOST_ prefix takes precedence; fall back to legacy CLAW_ for backward compat.
    std::env::var_os("GHOST_CONFIG_HOME")
        .or_else(|| std::env::var_os("CLAW_CONFIG_HOME"))
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
            rate_limiter: Arc::new(RateLimiter::new()),
            coder_rebuild_lock: Arc::new(AsyncMutex::new(())),
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

    #[test]
    fn normalize_phone_strips_formatting() {
        assert_eq!(normalize_phone("+1-234-567-8900"), "+12345678900");
        assert_eq!(normalize_phone("(234) 567-8900"), "2345678900");
        assert_eq!(normalize_phone("+12345678900"), "+12345678900");
        assert_eq!(normalize_phone(""), "");
        assert_eq!(normalize_phone("  +1 234  "), "1234");
    }

    #[test]
    fn rate_limiter_allows_within_budget() {
        let rl = RateLimiter::new();
        for _ in 0..RATE_LIMIT_MAX {
            assert!(rl.check("1.2.3.4"));
        }
        // Next request should be denied
        assert!(!rl.check("1.2.3.4"));
        // Different IP is fine
        assert!(rl.check("5.6.7.8"));
    }

    #[test]
    fn redact_secrets_covers_daemon_key() {
        std::env::set_var("GHOST_DAEMON_KEY", "supersecretkey123");
        let output = redact_secrets("token is supersecretkey123 ok");
        assert!(output.contains("***redacted***"));
        assert!(!output.contains("supersecretkey123"));
        std::env::remove_var("GHOST_DAEMON_KEY");
    }

    #[test]
    fn next_fire_from_cron_accepts_six_field() {
        // Every day at 09:00:00 UTC.
        let next = next_fire_from_cron("0 0 9 * * *").expect("valid cron must parse");
        assert!(next > chrono::Utc::now(), "next fire must be in the future");
    }

    #[test]
    fn next_fire_from_cron_rejects_invalid() {
        let err = next_fire_from_cron("not a cron").expect_err("invalid cron must error");
        assert!(err.contains("invalid cron expression"));
    }

    #[test]
    fn next_fire_from_cron_rejects_five_field() {
        // Classic 5-field cron is not supported by the `cron` crate — the
        // leading seconds field is mandatory. Pin that contract here so the
        // /triggers endpoint stays consistent with `infra::scheduler`.
        let err = next_fire_from_cron("0 9 * * *").expect_err("5-field cron must error");
        assert!(err.contains("invalid cron expression"));
    }

    #[test]
    fn schedulable_agents_includes_future_wave_55() {
        // Isaac may schedule chief_of_staff / dreamer triggers before those
        // agents are dispatcher-wired. Rejecting them would force a redeploy
        // post-Wave-5.5, so they must stay in the allow-list.
        assert!(SCHEDULABLE_AGENTS.contains(&"chief_of_staff"));
        assert!(SCHEDULABLE_AGENTS.contains(&"dreamer"));
        assert!(SCHEDULABLE_AGENTS.contains(&"chat_dispatcher"));
    }
}
