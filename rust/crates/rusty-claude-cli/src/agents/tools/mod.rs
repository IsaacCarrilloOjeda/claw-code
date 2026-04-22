// Tool system for the Coder agent. Hard rules for everything under this
// module:
//   - No arbitrary shell / exec tool. Adding one is out of scope.
//   - No network-calling tools.
//   - No file writes outside the canonicalized `repo_root`.
// Every path-taking tool MUST run `resolve_within_repo` before touching disk.

pub mod cargo;
pub mod diff;
pub mod grep;
pub mod list_dir;
pub mod read;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn schema(&self) -> Value;
    async fn run(&self, args: Value, ctx: &ToolCtx) -> Result<String, ToolError>;
}

pub struct ToolCtx {
    pub repo_root: PathBuf,
    pub pool: PgPool,
    pub auto_apply: bool,
    pub chat_id: Uuid,
}

#[derive(Debug)]
pub enum ToolError {
    BadArgs(String),
    PathEscape,
    NotFound,
    Exec(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadArgs(m) => write!(f, "bad args: {m}"),
            Self::PathEscape => write!(f, "path escapes repo root"),
            Self::NotFound => write!(f, "not found"),
            Self::Exec(m) => write!(f, "exec error: {m}"),
        }
    }
}

impl std::error::Error for ToolError {}

pub fn registry() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(read::ReadTool),
        Box::new(grep::GrepTool),
        Box::new(list_dir::ListDirTool),
        Box::new(diff::DiffTool),
        Box::new(cargo::CargoCheckTool),
        Box::new(cargo::CargoTestTool),
        Box::new(cargo::CargoFmtTool),
    ]
}

/// Join `rel` onto `repo_root`, canonicalize, and assert the result is still
/// inside the canonical `repo_root`. Rejects symlink escapes, absolute paths
/// outside the repo, and `..` traversal. Returns the canonical path on
/// success.
///
/// Empty `rel` (or `.`) resolves to `repo_root` itself.
pub fn resolve_within_repo(repo_root: &Path, rel: &str) -> Result<PathBuf, ToolError> {
    let root = repo_root
        .canonicalize()
        .map_err(|_| ToolError::PathEscape)?;
    let joined = if rel.is_empty() || rel == "." {
        root.clone()
    } else {
        root.join(rel)
    };

    // If the path doesn't exist yet, canonicalize the parent and re-append the
    // final component so new-file paths still get the escape check.
    let canon = if let Ok(p) = joined.canonicalize() {
        p
    } else {
        let parent = joined.parent().ok_or(ToolError::PathEscape)?;
        let file = joined.file_name().ok_or(ToolError::PathEscape)?;
        let parent_canon = parent.canonicalize().map_err(|_| ToolError::PathEscape)?;
        parent_canon.join(file)
    };

    if !canon.starts_with(&root) {
        return Err(ToolError::PathEscape);
    }
    Ok(canon)
}

/// Truncate a string to `max` bytes, inserting a middle marker when cut.
/// Preserves UTF-8 boundaries.
pub fn truncate_middle(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let half = max / 2;
    let head_end = floor_char_boundary(s, half);
    let tail_start = ceil_char_boundary(s, s.len().saturating_sub(half));
    format!(
        "{}\n... [truncated {} bytes] ...\n{}",
        &s[..head_end],
        s.len() - max,
        &s[tail_start..]
    )
}

fn floor_char_boundary(s: &str, i: usize) -> usize {
    let mut j = i.min(s.len());
    while j > 0 && !s.is_char_boundary(j) {
        j -= 1;
    }
    j
}

fn ceil_char_boundary(s: &str, i: usize) -> usize {
    let mut j = i.min(s.len());
    while j < s.len() && !s.is_char_boundary(j) {
        j += 1;
    }
    j
}
