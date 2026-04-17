//! Orchestrator-worker pattern for complex tasks (Phase D).
//!
//! The orchestrator calls a smart model to decompose a spec into a plan of
//! `[DO]` (critical, handled inline) and `[DELEGATE]` (worker, cheap model)
//! items. `execute_plan` then runs workers in parallel, checkpoints their
//! output, and assembles the final result.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use crate::constants::{ANTHROPIC_MESSAGES_URL, HAIKU_MODEL, OPUS_MODEL, SONNET_MODEL};

// ── Types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum WorkerStatus {
    Pending,
    Running,
    Done,
    Failed(String),
    Escalated,
}

#[derive(Debug, Clone)]
pub struct SpecBranch {
    pub hypothesis: String,
    pub worker_prompt: String,
    pub output: Option<String>,
    pub status: WorkerStatus,
}

#[derive(Debug, Clone)]
pub enum PlanItem {
    Critical {
        description: String,
        output: Option<String>,
    },
    Delegate {
        description: String,
        worker_prompt: String,
        output: Option<String>,
        status: WorkerStatus,
    },
    Speculative {
        description: String,
        branches: Vec<SpecBranch>,
        winner: Option<usize>,
    },
}

#[derive(Debug)]
pub struct Plan {
    pub task_description: String,
    pub items: Vec<PlanItem>,
    pub created_at: Instant,
}

// ── Orchestrator system prompt ───────────────────────────────────────────

const ORCHESTRATOR_SYSTEM: &str = "\
You are a task planner. Given a specification, produce a plan.

For each item, decide:
- [DO] -- you handle this yourself (architecture, design, glue logic, anything requiring full context)
- [DELEGATE] -- a cheap, fast model handles this (isolated code, simple transforms, boilerplate)
- [SPECULATE] -- when you're unsure between 2-3 approaches and verification is cheap

DELEGATE prompts must be hyper-specific:
- Exact function signature if applicable
- Numbered behavior steps
- What NOT to add (no extra error handling, no comments, no logging)
- Expected output length hint

Format each line:
[DO] description of what you're doing, then your actual output
[DELEGATE] description ||| the exact prompt to send to the worker model
[SPECULATE] description ||| hypothesis_a: worker_prompt_a ||| hypothesis_b: worker_prompt_b

Produce 3-15 items. If the task is simple enough for one step, output one [DO] item.";

const WORKER_SYSTEM: &str = "You are a precise code/text generator. Follow the instructions \
    exactly. Output only what is requested -- no preamble, no explanation.";

const HEDGING: &[&str] = &[
    "i'm not sure",
    "i cannot",
    "i can't",
    "as an ai",
    "i don't know",
];

// ── Plan parsing ─────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
pub fn parse_plan(response: &str) -> Vec<PlanItem> {
    let mut items = Vec::new();

    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(rest) = strip_tag(trimmed, "[DO]") {
            items.push(PlanItem::Critical {
                description: rest.trim().to_string(),
                output: Some(rest.trim().to_string()),
            });
        } else if let Some(rest) = strip_tag(trimmed, "[SPECULATE]") {
            let rest = rest.trim();
            let segments: Vec<&str> = rest.split("|||").collect();
            if segments.len() >= 3 {
                let desc = segments[0].trim().to_string();
                let branches: Vec<SpecBranch> = segments[1..]
                    .iter()
                    .filter_map(|seg| {
                        let seg = seg.trim();
                        if seg.is_empty() {
                            return None;
                        }
                        let (hypothesis, prompt) = if let Some(colon_idx) = seg.find(':') {
                            (
                                seg[..colon_idx].trim().to_string(),
                                seg[colon_idx + 1..].trim().to_string(),
                            )
                        } else {
                            (seg.to_string(), seg.to_string())
                        };
                        Some(SpecBranch {
                            hypothesis,
                            worker_prompt: prompt,
                            output: None,
                            status: WorkerStatus::Pending,
                        })
                    })
                    .collect();

                if branches.len() >= 2 {
                    items.push(PlanItem::Speculative {
                        description: desc,
                        branches,
                        winner: None,
                    });
                } else {
                    // Speculation needs 2+ branches — fall back to Delegate with first branch
                    if let Some(branch) = branches.into_iter().next() {
                        items.push(PlanItem::Delegate {
                            description: desc,
                            worker_prompt: branch.worker_prompt,
                            output: None,
                            status: WorkerStatus::Pending,
                        });
                    } else {
                        items.push(PlanItem::Critical {
                            description: desc,
                            output: None,
                        });
                    }
                }
            } else {
                // Not enough segments — fall back to Delegate or Critical
                let rest_str = rest.to_string();
                if let Some(sep_idx) = rest_str.find("|||") {
                    let desc = rest_str[..sep_idx].trim().to_string();
                    let prompt = rest_str[sep_idx + 3..].trim().to_string();
                    if prompt.is_empty() {
                        items.push(PlanItem::Critical {
                            description: desc,
                            output: None,
                        });
                    } else {
                        items.push(PlanItem::Delegate {
                            description: desc,
                            worker_prompt: prompt,
                            output: None,
                            status: WorkerStatus::Pending,
                        });
                    }
                } else {
                    items.push(PlanItem::Critical {
                        description: rest_str,
                        output: None,
                    });
                }
            }
        } else if let Some(rest) = strip_tag(trimmed, "[DELEGATE]") {
            let rest = rest.trim();
            if let Some(sep_idx) = rest.find("|||") {
                let desc = rest[..sep_idx].trim().to_string();
                let prompt = rest[sep_idx + 3..].trim().to_string();
                if prompt.is_empty() {
                    // Missing prompt after separator — fall back to critical
                    items.push(PlanItem::Critical {
                        description: desc,
                        output: None,
                    });
                } else {
                    items.push(PlanItem::Delegate {
                        description: desc,
                        worker_prompt: prompt,
                        output: None,
                        status: WorkerStatus::Pending,
                    });
                }
            } else {
                // No ||| separator — treat as critical (model didn't format correctly)
                items.push(PlanItem::Critical {
                    description: rest.to_string(),
                    output: None,
                });
            }
        }
        // Lines not starting with [DO], [DELEGATE], or [SPECULATE] are ignored.
    }

    items
}

/// Case-insensitive tag stripping. Returns the remainder after the tag, or None.
fn strip_tag<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
    let lower = line.to_lowercase();
    let tag_lower = tag.to_lowercase();
    if lower.starts_with(&tag_lower) {
        Some(&line[tag.len()..])
    } else {
        None
    }
}

// ── Orchestrate (Phase D1) ───────────────────────────────────────────────

/// Call a smart model to decompose `spec` into a plan of critical and delegate items.
pub async fn orchestrate(spec: &str, _pool: Option<&sqlx::PgPool>) -> Result<Plan, String> {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    // Use Sonnet for short specs (< 100 whitespace-separated tokens), Opus for complex ones.
    let model = if spec.split_whitespace().count() < 100 {
        SONNET_MODEL
    } else {
        OPUS_MODEL
    };

    eprintln!("[ghost orchestrator] planning with {model} for: {spec}");

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "system": ORCHESTRATOR_SYSTEM,
        "messages": [{"role": "user", "content": spec}],
    });

    let client = crate::http_client::shared_client();
    let resp = client
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(120))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("orchestrator API failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("orchestrator API error {status}: {text}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("orchestrator parse failed: {e}"))?;

    let response_text = json["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let items = parse_plan(&response_text);

    eprintln!(
        "[ghost orchestrator] plan: {} items ({} critical, {} delegate, {} speculative)",
        items.len(),
        items
            .iter()
            .filter(|i| matches!(i, PlanItem::Critical { .. }))
            .count(),
        items
            .iter()
            .filter(|i| matches!(i, PlanItem::Delegate { .. }))
            .count(),
        items
            .iter()
            .filter(|i| matches!(i, PlanItem::Speculative { .. }))
            .count(),
    );

    Ok(Plan {
        task_description: spec.to_string(),
        items,
        created_at: Instant::now(),
    })
}

// ── Worker execution (Phase D2) ──────────────────────────────────────────

/// Heuristic checkpoint: returns true if the output looks acceptable.
fn checkpoint(output: &str, prompt: &str) -> bool {
    if output.trim().is_empty() || output.len() < 20 {
        return false;
    }

    let lower = output.to_lowercase();
    if HEDGING.iter().any(|h| lower.contains(h)) {
        return false;
    }

    // If the prompt looks code-related, check for balanced braces
    let prompt_lower = prompt.to_lowercase();
    let is_code = ["function", "fn ", "struct", "impl", "class ", "def "]
        .iter()
        .any(|kw| prompt_lower.contains(kw));

    if is_code {
        let open_braces = output.matches('{').count();
        let close_braces = output.matches('}').count();
        let open_parens = output.matches('(').count();
        let close_parens = output.matches(')').count();
        if open_braces != close_braces || open_parens != close_parens {
            return false;
        }
    }

    true
}

/// Call Haiku with a worker prompt.
async fn call_worker(api_key: &str, prompt: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": HAIKU_MODEL,
        "max_tokens": 1024,
        "system": WORKER_SYSTEM,
        "messages": [{"role": "user", "content": prompt}],
    });

    let client = crate::http_client::shared_client();
    let resp = client
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(60))
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("worker API failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("worker API error {status}: {text}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("worker parse failed: {e}"))?;

    Ok(json["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

/// Call Sonnet for escalation (failed worker items).
async fn call_escalation(
    api_key: &str,
    prompt: &str,
    failed_output: &str,
) -> Result<String, String> {
    let escalated_prompt = format!(
        "{prompt}\n\n--- PREVIOUS ATTEMPT FAILED ---\nThe following output did not pass validation:\n{failed_output}\n\nPlease produce a correct version."
    );

    let body = serde_json::json!({
        "model": SONNET_MODEL,
        "max_tokens": 2048,
        "system": WORKER_SYSTEM,
        "messages": [{"role": "user", "content": escalated_prompt}],
    });

    let client = crate::http_client::shared_client();
    let resp = client
        .post(ANTHROPIC_MESSAGES_URL)
        .timeout(Duration::from_secs(90))
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("escalation API failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("escalation API error {status}: {text}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("escalation parse failed: {e}"))?;

    Ok(json["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

/// Execute a plan: run DELEGATE items in parallel via cheap model calls,
/// checkpoint outputs, escalate failures, and assemble the final result.
pub async fn execute_plan(plan: &mut Plan, pool: Option<&sqlx::PgPool>) -> Result<String, String> {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    run_delegate_workers(plan, &api_key, pool).await;
    run_speculative_items(plan, &api_key).await;
    escalate_failed_items(plan, &api_key).await;
    assemble_plan_output(plan)
}

/// Spawn parallel Haiku workers for all pending DELEGATE items and collect results.
async fn run_delegate_workers(plan: &mut Plan, api_key: &str, pool: Option<&sqlx::PgPool>) {
    let macros = crate::macros::load_macros();

    let mut delegate_tasks: Vec<(usize, String)> = Vec::new();
    for (idx, item) in plan.items.iter().enumerate() {
        if let PlanItem::Delegate {
            worker_prompt,
            status: WorkerStatus::Pending,
            ..
        } = item
        {
            delegate_tasks.push((idx, worker_prompt.clone()));
        }
    }

    let mut handles = Vec::new();
    for (idx, prompt) in delegate_tasks {
        let key = api_key.to_string();
        // Inject macro dictionary into the worker prompt
        let p = crate::macros::inject_macro_dictionary(&prompt, &macros);
        let pool_clone = pool.cloned();
        let macros_clone = macros.clone();

        let scholar_context = if let Some(ref db_pool) = pool_clone {
            load_scholar_hints(&p, db_pool).await
        } else {
            None
        };

        let enriched_prompt = if let Some(ref ctx) = scholar_context {
            format!("{p}\n\n{ctx}")
        } else {
            p.clone()
        };

        let handle = tokio::spawn(async move {
            let result = run_worker_with_retry(&key, &enriched_prompt, &p).await;
            // Expand any macro references in worker output
            let result = match result {
                WorkerResult::Done(output) => {
                    WorkerResult::Done(crate::macros::expand_macros(&output, &macros_clone))
                }
                WorkerResult::Escalated(output) => {
                    WorkerResult::Escalated(crate::macros::expand_macros(&output, &macros_clone))
                }
            };
            (idx, result)
        });
        handles.push(handle);
    }

    for handle in handles {
        if let Ok((idx, result)) = handle.await {
            let (out_val, status_val) = match result {
                WorkerResult::Done(output) => (output, WorkerStatus::Done),
                WorkerResult::Escalated(output) => (output, WorkerStatus::Escalated),
            };
            if let PlanItem::Delegate {
                output: ref mut out,
                status: ref mut s,
                ..
            } = plan.items[idx]
            {
                *out = Some(out_val);
                *s = status_val;
            }
        }
    }
}

/// Process all pending Speculative items in the plan.
async fn run_speculative_items(plan: &mut Plan, api_key: &str) {
    for item in &mut plan.items {
        if matches!(item, PlanItem::Speculative { .. }) {
            execute_speculative(item, api_key).await;
        }
    }
}

/// Handle escalated items: call Sonnet with original prompt + failed output context.
async fn escalate_failed_items(plan: &mut Plan, api_key: &str) {
    for item in &mut plan.items {
        if let PlanItem::Delegate {
            worker_prompt,
            output,
            status,
            ..
        } = item
        {
            if *status == WorkerStatus::Escalated {
                let failed = output.as_deref().unwrap_or("");
                match call_escalation(api_key, worker_prompt, failed).await {
                    Ok(escalated_output) => {
                        *output = Some(escalated_output);
                        *status = WorkerStatus::Done;
                    }
                    Err(e) => {
                        eprintln!("[ghost orchestrator] escalation failed: {e}");
                        *status = WorkerStatus::Failed(e);
                    }
                }
            }
        }
    }
}

/// Assemble all plan item outputs in order into a single result string.
fn assemble_plan_output(plan: &Plan) -> Result<String, String> {
    let assembled: Vec<String> = plan
        .items
        .iter()
        .filter_map(|item| match item {
            PlanItem::Critical { output, .. } | PlanItem::Delegate { output, .. } => output.clone(),
            PlanItem::Speculative {
                branches, winner, ..
            } => {
                if let Some(idx) = winner {
                    branches.get(*idx).and_then(|b| b.output.clone())
                } else {
                    None
                }
            }
        })
        .collect();

    if assembled.is_empty() {
        return Err("plan produced no output".to_string());
    }

    let result = assembled.join("\n\n");
    eprintln!(
        "[ghost orchestrator] assembled {} section(s), {} bytes, elapsed {:?}",
        assembled.len(),
        result.len(),
        plan.created_at.elapsed(),
    );

    Ok(result)
}

/// Execute a speculative plan item: spawn all branches in parallel, take the
/// first one that passes checkpoint. If none pass, escalate all to Sonnet.
async fn execute_speculative(item: &mut PlanItem, api_key: &str) {
    let PlanItem::Speculative {
        branches, winner, ..
    } = item
    else {
        return;
    };

    let mut handles = Vec::new();
    for (idx, branch) in branches.iter().enumerate() {
        let key = api_key.to_string();
        let prompt = branch.worker_prompt.clone();
        let handle = tokio::spawn(async move {
            let result = call_worker(&key, &prompt).await;
            (idx, prompt, result)
        });
        handles.push(handle);
    }

    // Collect all results
    let mut results: Vec<(usize, String, Result<String, String>)> = Vec::new();
    for handle in handles {
        if let Ok(r) = handle.await {
            results.push(r);
        }
    }

    // Find the first branch that passes checkpoint
    for (idx, prompt, result) in &results {
        if let Ok(output) = result {
            if checkpoint(output, prompt) {
                branches[*idx].output = Some(output.clone());
                branches[*idx].status = WorkerStatus::Done;
                *winner = Some(*idx);
                eprintln!(
                    "[ghost orchestrator] speculative winner: branch {} ({})",
                    idx, branches[*idx].hypothesis
                );
                return;
            }
        }
    }

    // No branch passed — escalate: send all branch outputs to Sonnet
    eprintln!("[ghost orchestrator] no speculative branch passed, escalating to sonnet");
    let mut context = String::from("Multiple approaches were tried but none passed validation:\n");
    for (idx, _, result) in &results {
        let output = match result {
            Ok(o) => o.as_str(),
            Err(e) => e.as_str(),
        };
        let _ = write!(
            context,
            "\n--- Branch {} ({}) ---\n{}\n",
            idx, branches[*idx].hypothesis, output
        );
    }

    // Pick the first branch's prompt as the base for escalation
    let base_prompt = &branches[0].worker_prompt;
    match call_escalation(api_key, base_prompt, &context).await {
        Ok(escalated_output) => {
            branches[0].output = Some(escalated_output);
            branches[0].status = WorkerStatus::Done;
            *winner = Some(0);
        }
        Err(e) => {
            eprintln!("[ghost orchestrator] speculative escalation failed: {e}");
            branches[0].status = WorkerStatus::Failed(e);
        }
    }
}

enum WorkerResult {
    Done(String),
    Escalated(String),
}

/// Run a worker call with one retry. Returns Done if checkpoint passes, Escalated otherwise.
async fn run_worker_with_retry(api_key: &str, prompt: &str, original_prompt: &str) -> WorkerResult {
    // First attempt
    match call_worker(api_key, prompt).await {
        Ok(output) if checkpoint(&output, original_prompt) => WorkerResult::Done(output),
        Ok(output) => {
            eprintln!("[ghost worker] checkpoint failed, retrying");
            // Retry once with error context
            let retry_prompt = format!(
                "{prompt}\n\n--- RETRY ---\nYour previous output did not pass validation. \
                 Ensure output is non-empty, has no hedging phrases, and if code, has balanced braces/parens."
            );
            match call_worker(api_key, &retry_prompt).await {
                Ok(retry_output) if checkpoint(&retry_output, original_prompt) => {
                    WorkerResult::Done(retry_output)
                }
                Ok(retry_output) => {
                    eprintln!("[ghost worker] retry also failed checkpoint, escalating");
                    WorkerResult::Escalated(retry_output)
                }
                Err(e) => {
                    eprintln!("[ghost worker] retry API error: {e}");
                    WorkerResult::Escalated(output)
                }
            }
        }
        Err(e) => {
            eprintln!("[ghost worker] first attempt API error: {e}");
            WorkerResult::Escalated(format!("(worker error: {e})"))
        }
    }
}

/// Check scholar DB for cached solutions or known-bad approaches to enrich a worker prompt.
async fn load_scholar_hints(prompt: &str, pool: &sqlx::PgPool) -> Option<String> {
    let embedding = crate::memory::embed(prompt).await.ok()?;
    let results = crate::db::search_scholar_with_distance(pool, &embedding, 2).await;
    if results.is_empty() {
        return None;
    }

    let mut hints = Vec::new();

    for (sol, distance) in &results {
        if *distance < 0.15 && sol.success_count >= 3 {
            hints.push(format!(
                "CACHED SOLUTION (use if applicable): {}",
                sol.solution
            ));
            break;
        }
        if *distance < 0.25 && !sol.failed_attempts.is_empty() {
            let failed = sol.failed_attempts.join(", ");
            hints.push(format!("DO NOT TRY these approaches: {failed}"));
        }
    }

    if hints.is_empty() {
        None
    } else {
        Some(hints.join("\n"))
    }
}

// ── Public entry point (Phase D2 wiring) ─────────────────────────────────

/// High-level entry: plan then execute. Called for complex tasks that benefit
/// from orchestrator-worker decomposition.
pub async fn dispatch_complex(spec: &str, pool: Option<&sqlx::PgPool>) -> Result<String, String> {
    let mut plan = orchestrate(spec, pool).await?;
    execute_plan(&mut plan, pool).await
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_well_formatted_plan() {
        let response = "\
[DO] Design the module architecture for the auth system
[DELEGATE] Generate the User struct ||| Write a Rust struct `User` with fields: id (Uuid), email (String), password_hash (String), created_at (DateTime). No derives beyond Debug and Clone.
[DO] Wire the auth middleware into the router
[DELEGATE] Write the login handler ||| Write an async fn login(body: LoginRequest) -> Result<Token, Error> that: 1. Validates email format 2. Checks password hash 3. Returns a JWT. No extra error handling.";

        let items = parse_plan(response);
        assert_eq!(items.len(), 4);

        assert!(
            matches!(&items[0], PlanItem::Critical { description, .. } if description.contains("Design"))
        );
        assert!(
            matches!(&items[1], PlanItem::Delegate { description, worker_prompt, .. }
            if description.contains("User struct") && worker_prompt.contains("Rust struct"))
        );
        assert!(
            matches!(&items[2], PlanItem::Critical { description, .. } if description.contains("Wire"))
        );
        assert!(matches!(&items[3], PlanItem::Delegate { worker_prompt, .. }
            if worker_prompt.contains("login")));
    }

    #[test]
    fn parse_single_do_item() {
        let response = "[DO] This task is simple enough to handle in one step";
        let items = parse_plan(response);
        assert_eq!(items.len(), 1);
        assert!(matches!(&items[0], PlanItem::Critical { .. }));
    }

    #[test]
    fn parse_missing_separator_falls_back_to_critical() {
        let response = "[DELEGATE] Generate the config file without a proper separator";
        let items = parse_plan(response);
        assert_eq!(items.len(), 1);
        // No ||| separator → should fall back to Critical
        assert!(matches!(&items[0], PlanItem::Critical { description, .. }
            if description.contains("Generate the config file")));
    }

    #[test]
    fn parse_empty_response() {
        let items = parse_plan("");
        assert!(items.is_empty());
    }

    #[test]
    fn parse_mixed_preserves_order() {
        let response = "\
[DO] First critical task
[DELEGATE] First worker ||| worker prompt one
[DELEGATE] Second worker ||| worker prompt two
[DO] Final critical task";

        let items = parse_plan(response);
        assert_eq!(items.len(), 4);

        assert!(matches!(&items[0], PlanItem::Critical { .. }));
        assert!(matches!(&items[1], PlanItem::Delegate { .. }));
        assert!(matches!(&items[2], PlanItem::Delegate { .. }));
        assert!(matches!(&items[3], PlanItem::Critical { .. }));
    }

    #[test]
    fn parse_ignores_freeform_text() {
        let response = "\
Here is the plan:
[DO] Design the API
Some explanation text
[DELEGATE] Implement endpoint ||| Write fn handle_request()
More context";

        let items = parse_plan(response);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn parse_case_insensitive_tags() {
        let response = "\
[do] lowercase critical item
[Delegate] Mixed case ||| worker prompt here";

        let items = parse_plan(response);
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], PlanItem::Critical { .. }));
        assert!(matches!(&items[1], PlanItem::Delegate { .. }));
    }

    #[test]
    fn parse_delegate_empty_prompt_falls_back() {
        let response = "[DELEGATE] task with empty prompt ||| ";
        let items = parse_plan(response);
        assert_eq!(items.len(), 1);
        // Empty prompt after ||| → falls back to Critical
        assert!(matches!(&items[0], PlanItem::Critical { description, .. }
            if description.contains("task with empty prompt")));
    }

    // ── checkpoint tests ─────────────────────────────────────────────

    #[test]
    fn checkpoint_rejects_empty() {
        assert!(!checkpoint("", "write something"));
    }

    #[test]
    fn checkpoint_rejects_short() {
        assert!(!checkpoint("ok", "write something"));
    }

    #[test]
    fn checkpoint_rejects_hedging() {
        assert!(!checkpoint(
            "I'm not sure about this, but here is an attempt at the code you requested.",
            "write a function"
        ));
    }

    #[test]
    fn checkpoint_accepts_good_output() {
        assert!(checkpoint(
            "pub fn add(a: i32, b: i32) -> i32 { a + b }",
            "write a function to add two numbers"
        ));
    }

    #[test]
    fn checkpoint_rejects_unbalanced_braces() {
        assert!(!checkpoint(
            "fn broken() { let x = 1;",
            "write a fn that does stuff"
        ));
    }

    #[test]
    fn checkpoint_accepts_balanced_code() {
        assert!(checkpoint(
            "fn good() {\n    let x = compute(1, 2);\n    println!(\"{x}\");\n}",
            "write a fn that prints"
        ));
    }

    #[test]
    fn checkpoint_non_code_ignores_braces() {
        // If prompt doesn't look code-related, unbalanced braces are fine
        assert!(checkpoint(
            "The result is { some value that trails off",
            "explain the concept"
        ));
    }

    // ── speculative parsing tests (Phase F2) ────────────────────────

    #[test]
    fn parse_speculate_two_branches() {
        let response =
            "[SPECULATE] Choose sort algorithm ||| bubble_sort: Write a bubble sort fn ||| quick_sort: Write a quicksort fn";
        let items = parse_plan(response);
        assert_eq!(items.len(), 1);
        match &items[0] {
            PlanItem::Speculative {
                description,
                branches,
                winner,
            } => {
                assert!(description.contains("sort algorithm"));
                assert_eq!(branches.len(), 2);
                assert_eq!(branches[0].hypothesis, "bubble_sort");
                assert!(branches[0].worker_prompt.contains("bubble sort"));
                assert_eq!(branches[1].hypothesis, "quick_sort");
                assert!(branches[1].worker_prompt.contains("quicksort"));
                assert!(winner.is_none());
            }
            other => panic!("expected Speculative, got {other:?}"),
        }
    }

    #[test]
    fn parse_speculate_three_branches() {
        let response = "[SPECULATE] Pick approach ||| a: prompt a ||| b: prompt b ||| c: prompt c";
        let items = parse_plan(response);
        assert_eq!(items.len(), 1);
        match &items[0] {
            PlanItem::Speculative { branches, .. } => {
                assert_eq!(branches.len(), 3);
            }
            other => panic!("expected Speculative, got {other:?}"),
        }
    }

    #[test]
    fn parse_speculate_one_branch_falls_back_to_delegate() {
        // Speculation needs 2+ branches — single branch falls back to Delegate
        let response = "[SPECULATE] Only one option ||| only_branch: do the thing";
        let items = parse_plan(response);
        assert_eq!(items.len(), 1);
        assert!(
            matches!(&items[0], PlanItem::Delegate { .. }),
            "single-branch speculate should fall back to Delegate"
        );
    }

    #[test]
    fn parse_speculate_no_separator_falls_back() {
        let response = "[SPECULATE] no branches at all";
        let items = parse_plan(response);
        assert_eq!(items.len(), 1);
        assert!(matches!(&items[0], PlanItem::Critical { .. }));
    }

    #[test]
    fn parse_mixed_with_speculate() {
        let response = "\
[DO] Design the architecture
[SPECULATE] Choose implementation ||| approach_a: Write it with HashMap ||| approach_b: Write it with BTreeMap
[DELEGATE] Write tests ||| Write unit tests for the sort module";
        let items = parse_plan(response);
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], PlanItem::Critical { .. }));
        assert!(matches!(&items[1], PlanItem::Speculative { .. }));
        assert!(matches!(&items[2], PlanItem::Delegate { .. }));
    }
}
