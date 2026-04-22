use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{resolve_within_repo, Tool, ToolCtx, ToolError};

pub struct DiffTool;

#[derive(Deserialize)]
struct Args {
    path: String,
    search: String,
    replace: String,
}

#[async_trait]
impl Tool for DiffTool {
    fn name(&self) -> &'static str {
        "diff"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "diff",
            "description": "Exact-string search/replace in a file. When coder.auto_apply is false, the change is queued for approval instead of written. The search string must appear exactly once.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "repo-relative path" },
                    "search": { "type": "string", "description": "exact text to find (must appear exactly once)" },
                    "replace": { "type": "string", "description": "replacement text" }
                },
                "required": ["path", "search", "replace"]
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let Args {
            path,
            search,
            replace,
        } = serde_json::from_value(args).map_err(|e| ToolError::BadArgs(e.to_string()))?;

        let abs = resolve_within_repo(&ctx.repo_root, &path)?;
        let original = tokio::fs::read_to_string(&abs)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => ToolError::NotFound,
                _ => ToolError::Exec(e.to_string()),
            })?;

        let occurrences = count_occurrences(&original, &search);
        if occurrences == 0 {
            return Err(ToolError::BadArgs("search string not found".into()));
        }
        if occurrences > 1 {
            return Err(ToolError::BadArgs(format!(
                "search string appears {occurrences} times; must be unique"
            )));
        }

        if ctx.auto_apply {
            let updated = original.replacen(&search, &replace, 1);
            let delta = line_delta(&original, &updated);
            tokio::fs::write(&abs, &updated)
                .await
                .map_err(|e| ToolError::Exec(e.to_string()))?;
            Ok(format!("applied to {path} ({delta} line delta)"))
        } else {
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO coder_pending_diffs (chat_id, path, search, replace) \
                 VALUES ($1, $2, $3, $4) RETURNING id",
            )
            .bind(ctx.chat_id)
            .bind(&path)
            .bind(&search)
            .bind(&replace)
            .fetch_one(&ctx.pool)
            .await
            .map_err(|e| ToolError::Exec(format!("db insert: {e}")))?;
            Ok(format!("queued as diff {id}. awaiting approval."))
        }
    }
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(idx) = haystack[start..].find(needle) {
        count += 1;
        start += idx + needle.len();
    }
    count
}

fn line_delta(before: &str, after: &str) -> i64 {
    let b = i64::try_from(before.lines().count()).unwrap_or(i64::MAX);
    let a = i64::try_from(after.lines().count()).unwrap_or(i64::MAX);
    a - b
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use tempfile::tempdir;

    fn lazy_pool() -> sqlx::PgPool {
        PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:5432/unused")
            .expect("lazy pool")
    }

    /// DB-backed; skipped when DATABASE_URL is unset so CI without a DB
    /// doesn't fail.
    #[tokio::test]
    async fn queues_when_auto_apply_off() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("skipping: DATABASE_URL not set");
            return;
        };
        let pool = PgPoolOptions::new().connect(&url).await.unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();

        let dir = tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();
        let before = tokio::fs::read_to_string(&path).await.unwrap();

        let chat_id = Uuid::new_v4();
        let ctx = ToolCtx {
            repo_root: dir.path().to_path_buf(),
            pool: pool.clone(),
            auto_apply: false,
            chat_id,
        };
        let args = json!({ "path": "hello.txt", "search": "world", "replace": "friend" });
        let out = DiffTool.run(args, &ctx).await.unwrap();
        assert!(out.starts_with("queued as diff "));
        let after = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(before, after, "file must not change when auto_apply=false");

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM coder_pending_diffs WHERE chat_id = $1")
                .bind(chat_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);

        sqlx::query("DELETE FROM coder_pending_diffs WHERE chat_id = $1")
            .bind(chat_id)
            .execute(&pool)
            .await
            .ok();
    }

    #[tokio::test]
    async fn writes_when_auto_apply_on() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let ctx = ToolCtx {
            repo_root: dir.path().to_path_buf(),
            pool: lazy_pool(),
            auto_apply: true,
            chat_id: Uuid::nil(),
        };
        let args = json!({ "path": "hello.txt", "search": "world", "replace": "friend" });
        let out = DiffTool.run(args, &ctx).await.unwrap();
        assert!(out.starts_with("applied to "));
        let after = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(after, "hello friend");
    }

    #[tokio::test]
    async fn rejects_non_unique_search() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dup.txt");
        tokio::fs::write(&path, "a a a").await.unwrap();
        let ctx = ToolCtx {
            repo_root: dir.path().to_path_buf(),
            pool: lazy_pool(),
            auto_apply: true,
            chat_id: Uuid::nil(),
        };
        let args = json!({ "path": "dup.txt", "search": "a", "replace": "b" });
        let err = DiffTool.run(args, &ctx).await.unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }
}
