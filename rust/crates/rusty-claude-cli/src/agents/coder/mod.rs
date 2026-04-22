//! Coder-agent infrastructure.
//!
//! Prompt C provides the file index + template stamping primitives; Prompt B
//! (the coder agent itself) plugs into them when it lands. Until then this
//! module just exposes `index::search_files` and `templates::stamp` for the
//! daemon endpoints.

pub mod agent;
pub mod index;
pub mod templates;

pub use agent::CoderAgent;

use std::path::PathBuf;

use sqlx::PgPool;

use crate::db;

/// Resolve the directory the coder indexer + watcher should operate on.
///
/// Cascade: `GHOST_CODER_REPO_ROOT` env var → `coder.repo_root` setting
/// (ignored when empty) → `std::env::current_dir()`.
///
/// Returns an owned `PathBuf` even when nothing is configured. Callers that
/// care whether the path exists on disk must check themselves (`exists()` /
/// `is_dir()`); that split lets the watcher skip gracefully on Railway
/// without forcing the indexer endpoints to duplicate the same check.
pub async fn repo_root(pool: &PgPool) -> PathBuf {
    if let Ok(v) = std::env::var("GHOST_CODER_REPO_ROOT") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Some(v) = db::get_setting::<String>(pool, "coder.repo_root").await {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
