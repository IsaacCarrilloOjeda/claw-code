use std::fmt::Write as _;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::time::timeout;

use super::{truncate_middle, Tool, ToolCtx, ToolError};

const TIMEOUT_SECS: u64 = 120;
const OUTPUT_CAP: usize = 100 * 1024;

fn rust_dir(ctx: &ToolCtx) -> std::path::PathBuf {
    ctx.repo_root.join("rust")
}

async fn run_cargo(cwd: std::path::PathBuf, args: Vec<String>) -> Result<String, ToolError> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&cwd);
    for a in &args {
        cmd.arg(a);
    }
    let fut = cmd.output();
    let out = timeout(Duration::from_secs(TIMEOUT_SECS), fut)
        .await
        .map_err(|_| ToolError::Exec(format!("cargo {} timed out", args.join(" "))))?
        .map_err(|e| ToolError::Exec(format!("cargo spawn failed: {e}")))?;

    let mut combined = String::new();
    let exit = out
        .status
        .code()
        .map_or_else(|| "signal".to_string(), |c| c.to_string());
    let _ = writeln!(combined, "$ cargo {}\nexit: {exit}", args.join(" "));
    if !out.stdout.is_empty() {
        combined.push_str("--- stdout ---\n");
        combined.push_str(&String::from_utf8_lossy(&out.stdout));
    }
    if !out.stderr.is_empty() {
        combined.push_str("\n--- stderr ---\n");
        combined.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    Ok(truncate_middle(&combined, OUTPUT_CAP))
}

pub struct CargoCheckTool;

#[async_trait]
impl Tool for CargoCheckTool {
    fn name(&self) -> &'static str {
        "cargo_check"
    }
    fn schema(&self) -> Value {
        json!({
            "name": "cargo_check",
            "description": "Run `cargo check` in rust/. 120s timeout. Output capped at 100 KB.",
            "input_schema": { "type": "object", "properties": {} }
        })
    }
    async fn run(&self, _args: Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        run_cargo(rust_dir(ctx), vec!["check".into(), "--workspace".into()]).await
    }
}

pub struct CargoFmtTool;

#[async_trait]
impl Tool for CargoFmtTool {
    fn name(&self) -> &'static str {
        "cargo_fmt"
    }
    fn schema(&self) -> Value {
        json!({
            "name": "cargo_fmt",
            "description": "Run `cargo fmt` in rust/. Rewrites files in place.",
            "input_schema": { "type": "object", "properties": {} }
        })
    }
    async fn run(&self, _args: Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        run_cargo(rust_dir(ctx), vec!["fmt".into(), "--all".into()]).await
    }
}

pub struct CargoTestTool;

#[derive(Deserialize)]
struct TestArgs {
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    test_name: Option<String>,
}

#[async_trait]
impl Tool for CargoTestTool {
    fn name(&self) -> &'static str {
        "cargo_test"
    }
    fn schema(&self) -> Value {
        json!({
            "name": "cargo_test",
            "description": "Run `cargo test` in rust/. Optionally narrow to a package or a test name. 120s timeout.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "package": { "type": "string", "description": "-p <package>" },
                    "test_name": { "type": "string", "description": "substring filter passed after --" }
                }
            }
        })
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> Result<String, ToolError> {
        let TestArgs { package, test_name } = if args.is_null() {
            TestArgs {
                package: None,
                test_name: None,
            }
        } else {
            serde_json::from_value(args).map_err(|e| ToolError::BadArgs(e.to_string()))?
        };
        let mut cargo_args = vec!["test".to_string()];
        if let Some(p) = &package {
            cargo_args.push("-p".into());
            cargo_args.push(p.clone());
        }
        cargo_args.push("--bins".into());
        if let Some(t) = &test_name {
            cargo_args.push("--".into());
            cargo_args.push(t.clone());
        }
        run_cargo(rust_dir(ctx), cargo_args).await
    }
}
