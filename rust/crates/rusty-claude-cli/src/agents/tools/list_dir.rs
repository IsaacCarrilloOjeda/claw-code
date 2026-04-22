use async_trait::async_trait;
use ignore::WalkBuilder;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{resolve_within_repo, Tool, ToolCtx, ToolError};

const ENTRY_CAP: usize = 500;

pub struct ListDirTool;

#[derive(Deserialize)]
struct Args {
    path: String,
    #[serde(default)]
    recursive: Option<bool>,
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "list_dir",
            "description": "List files in a directory, honoring .gitignore. Caps at 500 entries.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "repo-relative path" },
                    "recursive": { "type": "boolean" }
                },
                "required": ["path"]
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let Args { path, recursive } =
            serde_json::from_value(args).map_err(|e| ToolError::BadArgs(e.to_string()))?;
        let root = resolve_within_repo(&ctx.repo_root, &path)?;
        let root_for_strip = ctx.repo_root.canonicalize().unwrap_or(root.clone());
        let recursive = recursive.unwrap_or(false);

        let entries = tokio::task::spawn_blocking(move || {
            let mut builder = WalkBuilder::new(&root);
            builder.hidden(false);
            if !recursive {
                builder.max_depth(Some(1));
            }
            let mut out: Vec<String> = Vec::new();
            for entry in builder.build().flatten() {
                if entry.depth() == 0 {
                    continue;
                }
                let rel = entry
                    .path()
                    .strip_prefix(&root_for_strip)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                let suffix = if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                    "/"
                } else {
                    ""
                };
                out.push(format!("{rel}{suffix}"));
                if out.len() >= ENTRY_CAP {
                    out.push("... [truncated]".into());
                    break;
                }
            }
            out
        })
        .await
        .map_err(|e| ToolError::Exec(e.to_string()))?;

        Ok(entries.join("\n"))
    }
}
