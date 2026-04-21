# Worker A (Wave 5) — Chief of Staff agent + Docs dispatcher registration

One-line task: ship `agents/chief_of_staff.rs` (a Sonnet-driven orchestrator that decomposes compound asks and delegates to already-registered agents via `Dispatcher::dispatch`), and while you're in the dispatcher/intent layer, register the Docs agent (Wave 4.5 loose end).

You are the ONLY worker touching `agents/intent.rs` and `agents/dispatcher.rs` this wave. B and C stay out of both files.

---

## Read first

1. `CLAUDE.md` + `ARCHITECTURE.md`.
2. `rust/crates/rusty-claude-cli/src/agents/mod.rs` — the `Agent` trait + `AgentRequest`/`AgentResponse`/`Usage`/`Source`.
3. `rust/crates/rusty-claude-cli/src/agents/dispatcher.rs` — the full file. You'll add two arms (ChiefOfStaff, Docs). Also re-read how Calendar was registered in Wave 3.5 and Research was registered in Wave 4.
4. `rust/crates/rusty-claude-cli/src/agents/intent.rs` — you'll add two new variants + prefixes.
5. `rust/crates/rusty-claude-cli/src/agents/research.rs` — closest Wave 4 reference for a Sonnet-ish agent that composes several stages.
6. `rust/crates/rusty-claude-cli/src/agents/docs.rs` — already exists, already implements `Agent`. You are registering the existing struct; do NOT edit docs.rs.
7. `rust/crates/rusty-claude-cli/src/director.rs` — reference for how `sonnet_reply` builds an Anthropic messages request + pulls `Usage`.
8. `rust/crates/rusty-claude-cli/src/constants.rs` — `SONNET_MODEL`, `ANTHROPIC_API_URL`.
9. `rust/crates/rusty-claude-cli/src/http_client.rs` — `shared_client()` only.

---

## Scope — exactly these files

| Path | Action |
|---|---|
| `rust/crates/rusty-claude-cli/src/agents/chief_of_staff.rs` | **Create** |
| `rust/crates/rusty-claude-cli/src/agents/mod.rs` | **Edit** — add `pub mod chief_of_staff;` (one line; B also appends here — trivial merge) |
| `rust/crates/rusty-claude-cli/src/agents/intent.rs` | **Edit** — add `Intent::ChiefOfStaff` + `Intent::Docs`, plus prefix arms + tests |
| `rust/crates/rusty-claude-cli/src/agents/dispatcher.rs` | **Edit** — add ChiefOfStaff + Docs arms; import both |

**Do NOT touch:** `daemon.rs` (C owns it), `db.rs` (B + C append-only), any migration, any dashboard file, any other agent file, `director.rs`, `chat_dispatcher.rs`, `infra/*`.

---

## Design (already decided)

### Prefixes (locked)

| Prefix | Intent | Agent |
|---|---|---|
| `#` | `ChiefOfStaff` | `agents/chief_of_staff.rs` (new) |
| `&` | `Docs` | `agents/docs.rs` (already exists) |

Do NOT pick different prefixes. `#` and `&` were chosen because neither collides with shell metacharacters SMS clients commonly mangle, and both are unused.

### intent.rs changes

Add two variants to the enum (order doesn't matter functionally, but keep alphabetical-ish for consistency):

```rust
pub enum Intent {
    Chat,
    Director,
    Research,
    Scheduled,
    Calendar,
    ChiefOfStaff,
    Docs,
    Ignore,
}
```

Add two prefix arms in `classify`, in the same style as the existing arms:

```rust
if let Some(rest) = trimmed.strip_prefix('#') {
    return (Intent::ChiefOfStaff, rest.trim().to_string());
}
if let Some(rest) = trimmed.strip_prefix('&') {
    return (Intent::Docs, rest.trim().to_string());
}
```

Add two tests mirroring `classifies_at_prefix_as_calendar`:
- `classifies_hash_prefix_as_chief_of_staff`
- `classifies_ampersand_prefix_as_docs`

### dispatcher.rs changes

Add two imports next to the existing agent imports:
```rust
use super::chief_of_staff::ChiefOfStaff;
use super::docs::Docs;
```

Extend the `(agent_name, tier)` match on `intent`:
```rust
Intent::ChiefOfStaff => ("chief_of_staff", ModelTier::Mid),
Intent::Docs => ("docs", ModelTier::Fast),
```

Extend the `call_result` match on `intent` with two new arms, mirroring Calendar/Research exactly:
```rust
Intent::ChiefOfStaff => ChiefOfStaff::new()
    .handle(req.clone(), pool)
    .await
    .map(|resp| (resp.text, resp.usage)),
Intent::Docs => Docs::new()
    .handle(req.clone(), pool)
    .await
    .map(|resp| (resp.text, resp.usage)),
```

Update the two `unreachable!` comments if needed. Don't touch anything else in dispatcher.rs.

### agents/mod.rs

Append one line: `pub mod chief_of_staff;`. B also appends `pub mod dreamer;`. Your add and B's add are both trivial one-line appends — if you see B's line already there when you edit, just add yours next to it. If you see theirs missing, just add yours and leave a spot for it.

### chief_of_staff.rs — `Agent` trait impl

```rust
//! Chief of Staff — Sonnet-driven orchestrator.
//!
//! Decomposes a compound ask into a plan of sub-agent calls, executes each
//! leg by re-entering `Dispatcher::dispatch` (so budget/events are recorded
//! per leg), then composes a final reply from the aggregated outputs.
//!
//! Budget/cost model: this agent's declared_tier is `Mid` (Sonnet). The
//! dispatcher debits the `chief_of_staff` line for THIS call's Sonnet
//! tokens only. Sub-legs (Research, Calendar, Docs) debit their own lines
//! via their own dispatcher entry, same as if Isaac had called them
//! directly — double-counting is intentional and correct.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;

use super::dispatcher::Dispatcher;
use super::{Agent, AgentRequest, AgentResponse, ModelTier, Source, Usage};
use crate::constants::{ANTHROPIC_API_URL, SONNET_MODEL};
use crate::http_client::shared_client;
```

#### Structure

```rust
pub struct ChiefOfStaff;

impl ChiefOfStaff {
    pub fn new() -> Self { Self }
}
impl Default for ChiefOfStaff { fn default() -> Self { Self::new() } }

#[async_trait]
impl Agent for ChiefOfStaff {
    fn name(&self) -> &'static str { "chief_of_staff" }
    fn declared_tier(&self) -> ModelTier { ModelTier::Mid }
    fn requires_approval(&self, _req: &AgentRequest) -> bool { false }
    async fn handle(&self, req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String> {
        let plan = build_plan(&req.message).await?;
        let mut leg_outputs: Vec<(String, String)> = Vec::new();
        for leg in &plan.legs {
            let prefixed = format_leg_message(leg);
            let sub = AgentRequest {
                message: prefixed,
                history: Vec::new(),
                source: req.source,
                job_id: format!("{}-{}", req.job_id, leg.agent),
                sender_phone: req.sender_phone.clone(),
            };
            match Dispatcher::new().dispatch(sub, pool).await {
                Ok(resp) => leg_outputs.push((leg.agent.clone(), resp.text)),
                Err(e) => leg_outputs.push((leg.agent.clone(), format!("(error: {e})"))),
            }
        }
        let (summary, usage) = compose_reply(&req.message, &plan, &leg_outputs).await?;
        Ok(AgentResponse { text: summary, usage, tier: ModelTier::Mid })
    }
}
```

#### The plan shape

Plans come back from Sonnet as JSON. Parse via `serde_json::Value` first, then extract.

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub legs: Vec<PlanLeg>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanLeg {
    pub agent: String,   // "research" | "calendar" | "docs"
    pub prompt: String,  // prefix-less payload to feed through the dispatcher
}
```

#### `build_plan` — Sonnet call #1 (planning)

System prompt:
```
You are the Chief of Staff orchestrator for Isaac's GHOST system.
Given a compound request, produce a JSON plan that decomposes it into
ordered sub-agent calls. Available sub-agents:
- research — web search + summary. Use for "look up", "find out", "what is".
- calendar — Google Calendar. Use for list/create events. Supports:
    list ("what's on my calendar today")
    create "Summary" at <RFC3339> for <duration>
- docs — Google Docs. Supports create/read/append. For create, pass
    'create "Title"'. For append, pass the doc URL followed by the content.

Respond with ONLY a JSON object. No prose.

Schema:
{
  "goal": "<one-line restatement of the overall ask>",
  "legs": [
    { "agent": "research|calendar|docs", "prompt": "<plain text payload>" }
  ]
}

Keep legs minimal — 1 to 3 max. If the ask needs only one agent, still
emit a single-leg plan. If none of the above agents fit, emit { "goal": "...", "legs": [] }.
```

User message: the raw `message` (already prefix-stripped by the dispatcher).

Model: `SONNET_MODEL`, `max_tokens: 512`.

Parse: extract the first `content[*].text` string, strip any leading ```json``` code fence if present, `serde_json::from_str::<Plan>`. On parse failure: `Err(format!("could not parse Chief of Staff plan: {e}: raw = {raw}"))`.

Usage from this call is NOT returned from `build_plan`; it's added to the compose-call usage at the end.

Actually — simpler: have `build_plan` also return its `Usage` and sum both into the final `Usage`. Shape:

```rust
async fn build_plan(user_msg: &str) -> Result<(Plan, Usage), String> { ... }
async fn compose_reply(...) -> Result<(String, Usage), String> { ... }
```

Then in `handle`:
```rust
let (plan, plan_usage) = build_plan(&req.message).await?;
// ...
let (summary, compose_usage) = compose_reply(&req.message, &plan, &leg_outputs).await?;
let usage = Usage {
    tokens_in: plan_usage.tokens_in + compose_usage.tokens_in,
    tokens_out: plan_usage.tokens_out + compose_usage.tokens_out,
};
```

#### `format_leg_message`

Prefix the leg's `prompt` so the dispatcher re-classifies it to the right agent:

```rust
pub(crate) fn format_leg_message(leg: &PlanLeg) -> String {
    match leg.agent.as_str() {
        "research" => format!("?{}", leg.prompt),
        "calendar" => format!("@{}", leg.prompt),
        "docs"     => format!("&{}", leg.prompt),
        _          => leg.prompt.clone(), // fall through — Chat
    }
}
```

#### `compose_reply` — Sonnet call #2 (composition)

System prompt:
```
You are the Chief of Staff. You just orchestrated sub-agents to answer
Isaac's request. Produce a single concise reply that combines what the
sub-agents found. If a leg errored, surface the error briefly. No preamble,
no "as an AI"; write like Isaac's chief of staff briefing him in 3–6 lines.
```

User message: something like:

```
Original request: {original_message}

Plan: {plan_json}

Leg results:
- {agent}: {output}
- ...
```

Model: `SONNET_MODEL`, `max_tokens: 1024`. Return `(text, usage)`.

#### Anthropic request helpers

If your file ends up duplicating Anthropic-call boilerplate, do NOT refactor into a new shared module this wave — keep the two private async fns self-contained. The dedup lands in Wave 6 once Dreamer also needs it.

### Edge cases

- Empty `req.message` (after `#` stripping) → `Err("chief of staff needs an actual request".into())`.
- `ANTHROPIC_API_KEY` missing → `Err("ANTHROPIC_API_KEY not set".into())`.
- Zero-leg plan (Sonnet decided none of the agents fit) → skip the orchestration loop; `compose_reply` still runs and explains that none of the sub-agents matched.
- Any leg's dispatch error → don't fail the whole request; record the error string as that leg's output and keep going.

### Unit tests (at least these four)

1. `format_leg_message` prefixes correctly for each known agent and passes through unknowns unchanged.
2. `Plan` / `PlanLeg` parse from a representative JSON blob (use a hardcoded string).
3. `requires_approval` is false for all inputs.
4. Plan-JSON parser strips a ```json``` code fence if Sonnet wraps its response.

No network, no DB.

---

## Constraints — do NOT

- Do NOT add new crate dependencies. `async_trait`, `serde`, `serde_json`, `reqwest` (via `shared_client`) are in tree.
- Do NOT register the agent recursively — `build_plan` must NEVER emit an `agent: "chief_of_staff"` leg. Guard: if one slips through, treat it as Chat.
- Do NOT implement a retry loop on Sonnet plan-parse failures — error out cleanly.
- Do NOT let leg-output text flow back into `build_plan` for re-planning. Single-shot plan, single-shot compose. Multi-turn planning is Wave 6.
- Do NOT touch `daemon.rs`, `chat_dispatcher.rs`, `director.rs`, `db.rs`, any migration, any infra module.
- Do NOT modify the `Agent` trait.
- Do NOT alter Calendar/Research/Docs files. You're only wiring Docs into the dispatcher/intent — its code stays untouched.

---

## Implementation order

1. Edit `agents/intent.rs`: add two enum variants + two prefix arms + two tests.
2. Create `agents/chief_of_staff.rs` with the `Agent` trait impl + helpers + tests.
3. Add `pub mod chief_of_staff;` to `agents/mod.rs`.
4. Edit `agents/dispatcher.rs`: two imports, two match arms in each of the two match statements.
5. `cargo fmt && cargo clippy -p rusty-claude-cli --bins -- -D warnings`.
6. `cargo test -p rusty-claude-cli --bins`.
7. All green → done.

---

## Verification

```bash
cd rust
cargo fmt
cargo clippy -p rusty-claude-cli --bins -- -D warnings
cargo test -p rusty-claude-cli --bins
```

All green. Pre-existing plugin-lifecycle failures don't count (see CLAUDE.md).

---

## Done criteria

- `agents/chief_of_staff.rs` exists, implements `Agent`, compiles clippy-clean.
- `agents/intent.rs` has `ChiefOfStaff` + `Docs` variants and `#` / `&` prefix handling, with two new tests.
- `agents/dispatcher.rs` routes `Intent::ChiefOfStaff` → `ChiefOfStaff::new().handle(...)` and `Intent::Docs` → `Docs::new().handle(...)`.
- `agents/mod.rs` has `pub mod chief_of_staff;`.
- Scoped clippy green, scoped tests green.

---

## If you hit ambiguity

Stop and flag:
- If `SONNET_MODEL` / `ANTHROPIC_API_URL` have moved or renamed, match reality.
- If Calendar/Research/Docs `handle` signatures differ from `handle(&self, req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String>`, something shifted — flag it, do NOT edit the trait.
- If B's `pub mod dreamer;` line is already present in `agents/mod.rs` when you edit, leave it alone — add yours beside it.
- If `#` or `&` is already taken in `intent.rs` for some reason (shouldn't be), flag it — don't silently change the prefix.
