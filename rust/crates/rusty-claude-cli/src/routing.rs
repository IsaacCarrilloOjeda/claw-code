//! Provider routing for `CheetahClaws`.
//!
//! `route_model(prompt)` picks the cheapest model that can handle the task.
//! Only active when the user is running the default model (no explicit `--model` flag).
//! Opt-in via env: set `CLAW_ROUTING=1` to enable.
//!
//! Tiers (cheapest → most capable):
//!   fast   – `gpt-4o-mini`     (`OpenAI`)  short questions, summaries
//!   code   – `deepseek-chat`   (`DeepSeek`, OpenAI-compat) bulk codegen
//!   mid    – `claude-sonnet-4-6`          architecture, design, review
//!   full   – `claude-opus-4-6`            default / fallback
//!
//! Required env vars per tier:
//!   fast / code  → `OPENAI_API_KEY`  (`DeepSeek` uses OpenAI-compat endpoint)
//!   mid / full   → `ANTHROPIC_API_KEY`

/// Returns the model string to use for this prompt, or `None` if routing is
/// disabled or the current model is already overridden.
pub fn route_model(prompt: &str) -> Option<String> {
    // GHOST_ prefix takes precedence; fall back to legacy CLAW_ROUTING.
    let routing_val = std::env::var("GHOST_ROUTING")
        .or_else(|_| std::env::var("CLAW_ROUTING"))
        .unwrap_or_default();
    if routing_val != "1" {
        return None;
    }
    Some(classify(prompt, has_openai_key()).to_string())
}

fn classify(prompt: &str, has_openai: bool) -> &'static str {
    let lower = prompt.to_lowercase();
    let word_count = lower.split_whitespace().count();

    let is_code = has_any(&lower, CODE_SIGNALS);
    let is_arch = has_any(&lower, ARCH_SIGNALS);

    // Fast tier: short questions / lookups / rewrites with no heavy reasoning
    if word_count <= 20 && !is_code && !is_arch && has_openai {
        return "gpt-4o-mini";
    }

    // Code tier: bulk generation, refactoring, tests
    if is_code && !is_arch {
        if has_openai {
            return "deepseek-chat";
        }
        return "claude-sonnet-4-6"; // fallback if no OpenAI key
    }

    // Architecture tier: design, review, planning, explanation
    if is_arch {
        return "claude-sonnet-4-6";
    }

    // Default: Opus for anything complex or ambiguous
    "claude-opus-4-6"
}

fn has_any(text: &str, signals: &[&str]) -> bool {
    signals.iter().any(|s| text.contains(s))
}

fn has_openai_key() -> bool {
    std::env::var("OPENAI_API_KEY")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

const CODE_SIGNALS: &[&str] = &[
    "write ",
    "implement",
    "generate ",
    "refactor",
    "function",
    "class ",
    "struct ",
    "method",
    "algorithm",
    "script",
    "module",
    "test ",
    "tests",
    "fix the ",
    "fix this",
    "bug",
    "error",
    "compile",
    "build ",
    "code ",
    "endpoint",
    "api ",
    "database",
    "query",
    "migration",
];

const ARCH_SIGNALS: &[&str] = &[
    "architect",
    "design ",
    "plan ",
    "planning",
    "system ",
    "tradeoff",
    "review ",
    "explain",
    "understand",
    "how does",
    "why does",
    "should i",
    "best way",
    "approach",
    "strategy",
    "decision",
    "compare",
    "evaluate",
    "pros and cons",
    "diagram",
    "structure",
    "pattern",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_question_routes_fast_when_key_available() {
        assert_eq!(
            classify("what is the capital of france", true),
            "gpt-4o-mini"
        );
    }

    #[test]
    fn short_question_falls_back_to_opus_without_openai_key() {
        assert_eq!(
            classify("what is the capital of france", false),
            "claude-opus-4-6"
        );
    }

    #[test]
    fn code_heavy_prompt_routes_code_tier_with_key() {
        assert_eq!(
            classify("write a function to parse JSON and handle errors", true),
            "deepseek-chat"
        );
    }

    #[test]
    fn code_heavy_prompt_falls_back_to_sonnet_without_key() {
        assert_eq!(
            classify("write a function to parse JSON and handle errors", false),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn architecture_prompt_routes_mid_tier() {
        assert_eq!(
            classify(
                "design a system for real-time data ingestion at scale",
                true
            ),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            classify(
                "design a system for real-time data ingestion at scale",
                false
            ),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn ambiguous_long_prompt_defaults_to_opus() {
        let prompt = "i have a really long thought to share with you about how my day went \
                      and i want to know what you think about all the small things that happened";
        assert_eq!(classify(prompt, true), "claude-opus-4-6");
    }
}
