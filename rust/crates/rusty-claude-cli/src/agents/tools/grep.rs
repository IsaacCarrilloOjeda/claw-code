use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::process::Command;

use super::{resolve_within_repo, truncate_middle, Tool, ToolCtx, ToolError};

const MATCH_CAP: u32 = 200;
const OUTPUT_CAP: usize = 64 * 1024;

pub struct GrepTool;

#[derive(Deserialize)]
struct Args {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    case_insensitive: Option<bool>,
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn schema(&self) -> Value {
        json!({
            "name": "grep",
            "description": "Regex search across the repo via ripgrep. Caps at 200 matches.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string", "description": "repo-relative path; defaults to repo root" },
                    "case_insensitive": { "type": "boolean" }
                },
                "required": ["pattern"]
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let Args {
            pattern,
            path,
            case_insensitive,
        } = serde_json::from_value(args).map_err(|e| ToolError::BadArgs(e.to_string()))?;

        let search_root = resolve_within_repo(&ctx.repo_root, path.as_deref().unwrap_or(""))?;

        let mut cmd = Command::new("rg");
        cmd.arg("--no-heading")
            .arg("--line-number")
            .arg("--color=never")
            .arg("-m")
            .arg(MATCH_CAP.to_string());
        if case_insensitive.unwrap_or(false) {
            cmd.arg("-i");
        }
        cmd.arg("--").arg(&pattern).arg(&search_root);

        let out = cmd
            .output()
            .await
            .map_err(|e| ToolError::Exec(format!("rg not found or failed: {e}")))?;

        // rg exits 1 on no matches; that's fine.
        let combined = String::from_utf8_lossy(&out.stdout).into_owned();
        if combined.is_empty() && !out.stderr.is_empty() && !out.status.success() {
            let code = out.status.code().unwrap_or(-1);
            if code != 1 {
                let err = String::from_utf8_lossy(&out.stderr).into_owned();
                return Err(ToolError::Exec(format!("rg (exit {code}): {err}")));
            }
        }
        Ok(truncate_middle(&combined, OUTPUT_CAP))
    }
}
