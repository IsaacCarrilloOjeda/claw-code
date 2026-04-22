use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{resolve_within_repo, Tool, ToolCtx, ToolError};

const LINE_CAP: usize = 500;

pub struct ReadTool;

#[derive(Deserialize)]
struct Args {
    path: String,
    #[serde(default)]
    start_line: Option<u32>,
    #[serde(default)]
    end_line: Option<u32>,
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &'static str {
        "read"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "read",
            "description": "Read a text file from the repo. Optionally slice to a line range. Caps at 500 lines.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "repo-relative path" },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "end_line": { "type": "integer", "minimum": 1 }
                },
                "required": ["path"]
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let Args {
            path,
            start_line,
            end_line,
        } = serde_json::from_value(args).map_err(|e| ToolError::BadArgs(e.to_string()))?;

        let abs = resolve_within_repo(&ctx.repo_root, &path)?;
        let content = tokio::fs::read_to_string(&abs)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => ToolError::NotFound,
                _ => ToolError::Exec(e.to_string()),
            })?;

        let mut lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        if let (Some(start), Some(end)) = (start_line, end_line) {
            let s = (start.saturating_sub(1) as usize).min(total);
            let e = (end as usize).min(total);
            if s < e {
                lines = lines[s..e].to_vec();
            } else {
                lines.clear();
            }
        } else if let Some(start) = start_line {
            let s = (start.saturating_sub(1) as usize).min(total);
            lines = lines[s..].to_vec();
        } else if let Some(end) = end_line {
            let e = (end as usize).min(total);
            lines = lines[..e].to_vec();
        }

        let truncated = lines.len() > LINE_CAP;
        if truncated {
            lines.truncate(LINE_CAP);
        }
        let mut out = lines.join("\n");
        if truncated {
            out.push_str("\n... [truncated]");
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn lazy_pool() -> sqlx::PgPool {
        PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:5432/unused")
            .expect("lazy pool")
    }

    #[tokio::test]
    async fn path_escape_rejected() {
        let dir = tempdir().unwrap();
        let ctx = ToolCtx {
            repo_root: dir.path().to_path_buf(),
            pool: lazy_pool(),
            auto_apply: false,
            chat_id: Uuid::nil(),
        };
        let args = json!({ "path": "../../etc/passwd" });
        let err = ReadTool.run(args, &ctx).await.unwrap_err();
        assert!(matches!(err, ToolError::PathEscape));
    }

    #[tokio::test]
    async fn reads_and_caps() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        let body = (1..=600)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(&path, &body).await.unwrap();

        let ctx = ToolCtx {
            repo_root: dir.path().to_path_buf(),
            pool: lazy_pool(),
            auto_apply: false,
            chat_id: Uuid::nil(),
        };
        let out = ReadTool
            .run(json!({ "path": "a.txt" }), &ctx)
            .await
            .unwrap();
        assert!(out.ends_with("[truncated]"));
        assert!(out.contains("500\n"));
        assert!(!out.contains("501\n"));
    }
}
