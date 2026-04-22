//! Semantic file index: one embedding row per tracked file.
//!
//! The coder agent asks `search_files(query, k)` on each turn to learn where
//! to look in the repo without reading everything. Indexer embeds a short
//! "signature summary" (top-level fn/struct/class signatures + a 200-char
//! fingerprint of the raw body) rather than the whole file — cheap to embed,
//! good enough for file-level retrieval.
//!
//! Re-indexing triggers: manual `POST /code/index/rebuild`, filesystem watcher
//! in `daemon.rs`, and git hooks installed by `scripts/install-coder-git-hook.ps1`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use futures::stream::{self, StreamExt};
use ignore::WalkBuilder;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};

use crate::db::vec_to_pgvector;
use crate::memory;

/// Files larger than this are skipped — indexer signal-to-noise drops fast
/// on big generated files, and embedding them is expensive.
const MAX_FILE_SIZE_BYTES: u64 = 100 * 1024;
/// Window we sniff for null bytes to flag a file as binary.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;
/// Max signature lines we keep from a single file.
const MAX_SIGNATURE_LINES: usize = 30;
/// Chars of raw body appended as a fingerprint so the embedding picks up
/// some per-file uniqueness even when two files have identical signatures.
const RAW_FINGERPRINT_CHARS: usize = 200;
/// Concurrency cap for `index_repo`. Embed calls are I/O-bound; 8 keeps the
/// provider happy without oversubscribing.
const INDEX_CONCURRENCY: usize = 8;

/// Directories we never descend into, beyond what `.gitignore` covers. Defense
/// in depth — a repo without a gitignore still won't try to index `target/`.
const EXCLUDED_DIRS: &[&str] = &["target", "node_modules", ".git", "dist", ".ghost"];

#[derive(Debug)]
pub enum IndexError {
    Io(String),
    Db(String),
    Other(String),
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(s) | Self::Db(s) | Self::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<std::io::Error> for IndexError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<sqlx::Error> for IndexError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e.to_string())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IndexOutcome {
    pub skipped_unchanged: bool,
    pub embedded: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct IndexStats {
    pub files_scanned: usize,
    pub files_embedded: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FileHit {
    pub path: String,
    pub similarity: f32,
    pub summary: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexStoredStats {
    pub total_files: i64,
    pub total_bytes: i64,
    pub last_indexed_at: Option<String>,
    pub null_embeddings: i64,
}

/// Index a single file relative to `repo_root`. Returns `skipped_unchanged`
/// when the stored sha256 already matches the file on disk so callers (the
/// watcher, the git hook) can track no-op churn cheaply.
pub async fn index_file(
    pool: &PgPool,
    repo_root: &Path,
    rel_path: &Path,
) -> Result<IndexOutcome, IndexError> {
    let abs = repo_root.join(rel_path);
    let rel_str = normalize_rel_path(rel_path);

    let data = std::fs::read(&abs)?;
    let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let hash = sha256_hex(&data);

    if let Some(existing) = fetch_sha(pool, &rel_str).await? {
        if existing == hash {
            return Ok(IndexOutcome {
                skipped_unchanged: true,
                embedded: false,
            });
        }
    }

    let summary = build_signature_summary(rel_path, &data);
    let embedding = match memory::embed(&summary).await {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("[coder index] embed failed for {rel_str}: {e}");
            None
        }
    };

    upsert_row(pool, &rel_str, &summary, size, &hash, embedding.as_deref()).await?;

    Ok(IndexOutcome {
        skipped_unchanged: false,
        embedded: embedding.is_some(),
    })
}

/// Walk `repo_root`, respect .gitignore, and index everything that survives
/// the filters. Concurrent embed calls are capped at `INDEX_CONCURRENCY`.
pub async fn index_repo(pool: &PgPool, repo_root: &Path) -> Result<IndexStats, IndexError> {
    let started = Instant::now();
    let paths = collect_paths(repo_root)?;

    let pool_handle = pool.clone();
    let root = repo_root.to_path_buf();

    let results: Vec<Result<IndexOutcome, IndexError>> = stream::iter(paths)
        .map(|rel| {
            let pool = pool_handle.clone();
            let root = root.clone();
            async move { index_file(&pool, &root, &rel).await }
        })
        .buffer_unordered(INDEX_CONCURRENCY)
        .collect()
        .await;

    let mut scanned = 0usize;
    let mut embedded = 0usize;
    for r in &results {
        match r {
            Ok(outcome) => {
                scanned += 1;
                if outcome.embedded {
                    embedded += 1;
                }
            }
            Err(e) => eprintln!("[coder index] file index failed: {e}"),
        }
    }

    Ok(IndexStats {
        files_scanned: scanned,
        files_embedded: embedded,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// Remove a row. Used by the watcher on `Remove` events.
pub async fn remove_path(pool: &PgPool, rel_path: &Path) -> Result<(), IndexError> {
    let rel_str = normalize_rel_path(rel_path);
    sqlx::query("DELETE FROM coder_file_index WHERE path = $1")
        .bind(&rel_str)
        .execute(pool)
        .await?;
    Ok(())
}

/// Top-k nearest-neighbor lookup. Falls back to substring match on
/// `signature_summary` when no embedding provider is configured (so the API
/// contract degrades gracefully on boxes with no `VOYAGE_API_KEY` /
/// `OPENAI_API_KEY`).
pub async fn search_files(
    pool: &PgPool,
    query: &str,
    k: usize,
) -> Result<Vec<FileHit>, IndexError> {
    let limit = i64::try_from(k.max(1)).unwrap_or(5);

    if let Ok(emb) = memory::embed(query).await {
        let emb_str = vec_to_pgvector(&emb);
        let rows = sqlx::query(
            "SELECT path, signature_summary,
                    (1.0 - (embedding <=> $1::vector))::float8 AS similarity
             FROM coder_file_index
             WHERE embedding IS NOT NULL
             ORDER BY embedding <=> $1::vector
             LIMIT $2",
        )
        .bind(&emb_str)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        Ok(rows
            .iter()
            .map(|r| FileHit {
                path: r.try_get::<String, _>("path").unwrap_or_default(),
                summary: r
                    .try_get::<String, _>("signature_summary")
                    .unwrap_or_default(),
                #[allow(clippy::cast_possible_truncation)]
                similarity: r.try_get::<f64, _>("similarity").unwrap_or(0.0) as f32,
            })
            .collect())
    } else {
        // Fallback: no embedding provider. Return rows whose summary
        // literally contains the query string.
        let like = format!("%{query}%");
        let rows = sqlx::query(
            "SELECT path, signature_summary
             FROM coder_file_index
             WHERE signature_summary ILIKE $1
             LIMIT $2",
        )
        .bind(&like)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        Ok(rows
            .iter()
            .map(|r| FileHit {
                path: r.try_get::<String, _>("path").unwrap_or_default(),
                summary: r
                    .try_get::<String, _>("signature_summary")
                    .unwrap_or_default(),
                similarity: 0.0,
            })
            .collect())
    }
}

/// Aggregate index counts for `GET /code/index/stats`.
pub async fn stored_stats(pool: &PgPool) -> Result<IndexStoredStats, IndexError> {
    let row = sqlx::query(
        "SELECT
             COUNT(*)::bigint                                 AS total_files,
             COALESCE(SUM(file_size_bytes), 0)::bigint        AS total_bytes,
             MAX(indexed_at)::text                            AS last_indexed_at,
             COUNT(*) FILTER (WHERE embedding IS NULL)::bigint AS null_embeddings
         FROM coder_file_index",
    )
    .fetch_one(pool)
    .await?;

    Ok(IndexStoredStats {
        total_files: row.try_get("total_files").unwrap_or(0),
        total_bytes: row.try_get("total_bytes").unwrap_or(0),
        last_indexed_at: row
            .try_get::<Option<String>, _>("last_indexed_at")
            .ok()
            .flatten(),
        null_embeddings: row.try_get("null_embeddings").unwrap_or(0),
    })
}

// ---------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------

async fn fetch_sha(pool: &PgPool, rel_str: &str) -> Result<Option<String>, IndexError> {
    let row = sqlx::query("SELECT sha256 FROM coder_file_index WHERE path = $1")
        .bind(rel_str)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.try_get::<String, _>("sha256").ok()))
}

async fn upsert_row(
    pool: &PgPool,
    path: &str,
    summary: &str,
    size: u32,
    sha: &str,
    embedding: Option<&[f32]>,
) -> Result<(), IndexError> {
    if let Some(emb) = embedding {
        let emb_str = vec_to_pgvector(emb);
        sqlx::query(
            "INSERT INTO coder_file_index
               (path, signature_summary, file_size_bytes, sha256, embedding, indexed_at)
             VALUES ($1, $2, $3, $4, $5::vector, now())
             ON CONFLICT (path) DO UPDATE SET
                 signature_summary = EXCLUDED.signature_summary,
                 file_size_bytes   = EXCLUDED.file_size_bytes,
                 sha256            = EXCLUDED.sha256,
                 embedding         = EXCLUDED.embedding,
                 indexed_at        = now()",
        )
        .bind(path)
        .bind(summary)
        .bind(i32::try_from(size).unwrap_or(i32::MAX))
        .bind(sha)
        .bind(&emb_str)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO coder_file_index
               (path, signature_summary, file_size_bytes, sha256, embedding, indexed_at)
             VALUES ($1, $2, $3, $4, NULL, now())
             ON CONFLICT (path) DO UPDATE SET
                 signature_summary = EXCLUDED.signature_summary,
                 file_size_bytes   = EXCLUDED.file_size_bytes,
                 sha256            = EXCLUDED.sha256,
                 embedding         = NULL,
                 indexed_at        = now()",
        )
        .bind(path)
        .bind(summary)
        .bind(i32::try_from(size).unwrap_or(i32::MAX))
        .bind(sha)
        .execute(pool)
        .await?;
    }
    Ok(())
}

fn collect_paths(repo_root: &Path) -> Result<Vec<PathBuf>, IndexError> {
    if !repo_root.exists() {
        return Err(IndexError::Other(format!(
            "repo_root does not exist: {}",
            repo_root.display()
        )));
    }

    let mut out = Vec::new();
    let walker = WalkBuilder::new(repo_root)
        .standard_filters(true)
        .hidden(true)
        .follow_links(false)
        .build();

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(rel) = path.strip_prefix(repo_root) else {
            continue;
        };
        if is_excluded(rel) {
            continue;
        }

        let size = entry.metadata().ok().map_or(0u64, |m| m.len());
        if size > MAX_FILE_SIZE_BYTES {
            continue;
        }

        // Peek at the head of the file for the binary-byte sniff. Files that
        // fail to open here just fall through to the indexer, which will hit
        // the same error on `std::fs::read` and log per-path.
        if let Ok(head) = peek_head(path, BINARY_SNIFF_BYTES) {
            if head.contains(&0u8) {
                continue;
            }
        }

        out.push(rel.to_path_buf());
    }

    Ok(out)
}

fn is_excluded(rel: &Path) -> bool {
    rel.components().any(|c| {
        if let std::path::Component::Normal(s) = c {
            if let Some(s) = s.to_str() {
                return EXCLUDED_DIRS.contains(&s);
            }
        }
        false
    })
}

/// Public wrapper for the watcher in daemon.rs so it can cheaply filter
/// events under excluded directories without reading the file.
pub fn is_path_excluded(rel: &Path) -> bool {
    is_excluded(rel)
}

fn peek_head(path: &Path, max: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

fn normalize_rel_path(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for b in &digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Extract a language-aware signature summary. Rust gets top-level `fn`,
/// `struct`, `enum`, `trait`, `impl`, `mod`, `const`, `type`. JS/TS/JSX/TSX
/// gets `export`, `function`, `class`, `const`, `let`. Anything else falls
/// back to the first 20 non-empty non-comment lines. A 200-char fingerprint
/// of the raw body is appended so embeddings pick up per-file uniqueness.
pub(crate) fn build_signature_summary(rel: &Path, data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data);
    let ext = rel
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let mut lines: Vec<String> = Vec::new();
    let fallback_limit = 20usize;

    match ext.as_str() {
        "rs" => {
            for line in text.lines() {
                if lines.len() >= MAX_SIGNATURE_LINES {
                    break;
                }
                if let Some(sig) = extract_rust_signature(line) {
                    lines.push(sig.to_string());
                }
            }
        }
        "js" | "ts" | "jsx" | "tsx" | "mjs" | "cjs" => {
            for line in text.lines() {
                if lines.len() >= MAX_SIGNATURE_LINES {
                    break;
                }
                if let Some(sig) = extract_js_signature(line) {
                    lines.push(sig.to_string());
                }
            }
        }
        _ => {
            for line in text.lines() {
                if lines.len() >= fallback_limit {
                    break;
                }
                let t = line.trim();
                if t.is_empty() {
                    continue;
                }
                if t.starts_with('#') || t.starts_with("//") || t.starts_with("--") {
                    continue;
                }
                lines.push(t.to_string());
            }
        }
    }

    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str("--- fingerprint ---\n");
    out.push_str(&fingerprint(&text));
    out
}

fn fingerprint(text: &str) -> String {
    text.chars().take(RAW_FINGERPRINT_CHARS).collect()
}

fn extract_rust_signature(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let after_pub = trimmed
        .strip_prefix("pub ")
        .or_else(|| trimmed.strip_prefix("pub(crate) "))
        .or_else(|| trimmed.strip_prefix("pub(super) "))
        .unwrap_or(trimmed)
        .trim_start();

    for kw in [
        "fn ",
        "async fn ",
        "struct ",
        "enum ",
        "trait ",
        "impl ",
        "impl<",
        "mod ",
        "const ",
        "static ",
        "type ",
    ] {
        if let Some(rest) = after_pub.strip_prefix(kw) {
            if rest
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '<')
            {
                return Some(line.trim_end());
            }
        }
    }
    None
}

fn extract_js_signature(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let stripped = trimmed
        .strip_prefix("export default ")
        .or_else(|| trimmed.strip_prefix("export "))
        .unwrap_or(trimmed);
    let stripped = stripped
        .strip_prefix("async ")
        .unwrap_or(stripped)
        .trim_start();

    if trimmed.starts_with("export default") {
        return Some(line.trim_end());
    }

    for kw in ["function ", "class ", "const ", "let ", "var "] {
        if let Some(rest) = stripped.strip_prefix(kw) {
            if rest
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
            {
                return Some(line.trim_end());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_extraction_rust() {
        let src = br#"
use std::path::Path;

pub fn top_level_fn(x: u32) -> u32 { x }
fn private_fn() {}

pub struct Foo {
    pub bar: i32,
}

pub(crate) enum Mode { A, B }

impl Foo {
    pub fn new() -> Self { Self { bar: 0 } }
}

const LIMIT: usize = 10;
type Alias = Vec<u8>;
"#;
        let summary = build_signature_summary(Path::new("sample.rs"), src);
        assert!(summary.contains("pub fn top_level_fn"));
        assert!(summary.contains("fn private_fn"));
        assert!(summary.contains("pub struct Foo"));
        assert!(summary.contains("pub(crate) enum Mode"));
        assert!(summary.contains("impl Foo"));
        assert!(summary.contains("const LIMIT"));
        assert!(summary.contains("type Alias"));
        assert!(summary.contains("--- fingerprint ---"));
    }

    #[test]
    fn signature_extraction_js() {
        let src = br#"
import { foo } from './foo';

export function alpha() {}
export default function Beta() {}
export const gamma = () => 1;
class Delta {}
async function epsilon() {}
"#;
        let summary = build_signature_summary(Path::new("sample.jsx"), src);
        assert!(summary.contains("export function alpha"));
        assert!(summary.contains("export default function Beta"));
        assert!(summary.contains("export const gamma"));
        assert!(summary.contains("class Delta"));
        assert!(summary.contains("async function epsilon"));
    }

    #[test]
    fn signature_extraction_unknown_falls_back_to_body() {
        let src = b"# comment\n\nfirst real line\nsecond real line\n";
        let summary = build_signature_summary(Path::new("notes.txt"), src);
        // Assertion applies to the extracted-signature section only (the
        // fingerprint that follows always quotes raw bytes verbatim).
        let signature_section = summary
            .split_once("--- fingerprint ---")
            .map_or(summary.as_str(), |(head, _)| head);
        assert!(signature_section.contains("first real line"));
        assert!(signature_section.contains("second real line"));
        assert!(!signature_section.contains("# comment"));
    }

    #[test]
    fn is_excluded_catches_common_dirs() {
        assert!(is_excluded(Path::new("target/debug/claw.exe")));
        assert!(is_excluded(Path::new(
            "dashboard/node_modules/react/index.js"
        )));
        assert!(is_excluded(Path::new(".git/HEAD")));
        assert!(is_excluded(Path::new("dist/bundle.js")));
        assert!(is_excluded(Path::new(".ghost/bible-data/kjv.json")));
        assert!(!is_excluded(Path::new("rust/src/main.rs")));
    }

    #[test]
    fn sha256_hex_is_64_chars() {
        assert_eq!(sha256_hex(b"hello").len(), 64);
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"world"));
    }

    #[test]
    fn normalize_rel_path_uses_forward_slash() {
        let p = Path::new("a").join("b").join("c.rs");
        assert_eq!(normalize_rel_path(&p), "a/b/c.rs");
    }
}
