//! Filler stripping, response confidence heuristics, and prompt compression
//! for the cascade dispatcher.

use std::sync::OnceLock;

const FILLER_PREFIXES: &[&str] = &[
    "okay so ", "hey so ", "ok so ", "hello ", "hey ", "umm ", "uhh ", "hmm ", "hi ", "so ", "um ",
];

const FILLER_SUFFIXES: &[&str] = &[
    "i hope this isn't too much",
    "i hope this isnt too much",
    "sorry if that's a lot",
    "sorry if thats a lot",
    "does that make sense",
    "if that makes sense",
    "no worries if not",
    "thank you!",
    "thank you",
    "thanks!",
    "thanks",
    "please!",
    "please",
    "haha",
    "lol",
];

const HEDGING_PHRASES: &[&str] = &[
    "i'm not sure",
    "i can't",
    "i don't know",
    "i cannot",
    "as an ai",
];

/// Strip filler greetings/sign-offs from the start and end of a message.
///
/// Only trims prefix and suffix patterns — never touches the middle of the
/// message. Returns the original (trimmed) input if stripping would reduce
/// the result to fewer than 5 characters.
pub fn strip_filler(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }

    let mut start = 0usize;
    let mut end = trimmed.len();

    // Strip prefixes (longest-first, repeat until none match)
    loop {
        let remaining = &trimmed[start..end];
        let lower = remaining.to_lowercase();
        let mut matched = false;
        for prefix in FILLER_PREFIXES {
            if lower.starts_with(prefix) {
                start += prefix.len();
                matched = true;
                break;
            }
        }
        if !matched {
            break;
        }
    }

    // Strip suffixes (longest-first, repeat until none match)
    loop {
        let remaining = &trimmed[start..end];
        let lower = remaining.to_lowercase();
        let trimmed_lower = lower.trim_end();
        let trailing_ws = lower.len() - trimmed_lower.len();
        let mut matched = false;
        for suffix in FILLER_SUFFIXES {
            if trimmed_lower.ends_with(suffix) {
                end -= suffix.len() + trailing_ws;
                matched = true;
                break;
            }
        }
        if !matched {
            break;
        }
    }

    if start >= end {
        return trimmed.to_string();
    }

    let result: String = trimmed[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if result.len() < 5 {
        return trimmed.to_string();
    }

    result
}

// ── Intake (prompt polisher) ──────────────────────────────────────────

const AMBIGUITY_SIGNALS: &[&str] = &[
    "or something",
    "kind of",
    "sort of",
    "maybe",
    "i think",
    "i guess",
    "not sure",
    "somehow",
    "whatever",
    "stuff",
    "things",
    "the thing",
    "you know",
    "like ",
    "basically",
    "idk",
    "i dunno",
];

pub fn needs_intake(message: &str) -> bool {
    let word_count = message.split_whitespace().count();

    if word_count < 15 {
        return false;
    }

    if word_count >= 50 {
        return true;
    }

    // 15-49 words: check for ambiguity signals
    let lower = message.to_lowercase();
    AMBIGUITY_SIGNALS.iter().any(|s| lower.contains(s))
}

pub async fn intake_polish(message: &str) -> Result<String, String> {
    if !needs_intake(message) {
        return Ok(message.to_string());
    }

    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    let system = "You are a prompt polisher. Your ONLY job is to rewrite the user's message \
        as a clear, unambiguous request. Rules:\n\
        1. Remove filler, hedging, and repeated ideas\n\
        2. Make implicit requests explicit\n\
        3. If the user mentions multiple things, list them as numbered items\n\
        4. Keep the user's intent exactly — do NOT add requirements they didn't mention\n\
        5. Return ONLY the rewritten message. No preamble, no quotes, no explanation.\n\
        6. If the message is already clear, return it unchanged.\n\
        7. Never exceed 2x the original word count.";

    let body = serde_json::json!({
        "model": crate::constants::HAIKU_MODEL,
        "max_tokens": 512,
        "system": system,
        "messages": [{"role": "user", "content": message}],
    });

    let client = crate::http_client::shared_client();
    let resp = client
        .post(crate::constants::ANTHROPIC_MESSAGES_URL)
        .timeout(std::time::Duration::from_secs(15))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("intake API failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("intake API error: {}", resp.status()));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("intake parse failed: {e}"))?;

    let polished = json["content"][0]["text"]
        .as_str()
        .unwrap_or(message)
        .to_string();

    // Safety: if polished is way longer, model hallucinated — use original
    if polished.split_whitespace().count() > message.split_whitespace().count() * 2 {
        eprintln!("[ghost intake] polished too long, using original");
        return Ok(message.to_string());
    }

    eprintln!(
        "[ghost intake] polished: {} -> {} words",
        message.split_whitespace().count(),
        polished.split_whitespace().count()
    );

    Ok(polished)
}

/// Heuristic check: returns `true` if the response looks like the model
/// punted or produced a low-quality answer worth escalating to a higher tier.
pub fn is_low_confidence(response: &str) -> bool {
    if response.trim().is_empty() {
        return true;
    }
    if response.len() < 40 {
        return true;
    }
    let lower = response.to_lowercase();
    HEDGING_PHRASES.iter().any(|h| lower.contains(h))
}

// ── Abbreviations (Phase E2 — prompt compression) ───────────────────────

/// Cached abbreviation pairs, loaded once per process lifetime.
static ABBREVIATIONS: OnceLock<Vec<(String, String)>> = OnceLock::new();

/// Load abbreviation pairs from `.ghost/abbreviations.toml` (or `GHOST_ABBREVIATIONS_PATH`).
/// Returns pairs sorted by pattern length descending so longer matches take priority.
/// Cached in a `OnceLock` — the file is only read once per daemon lifetime.
pub fn load_abbreviations() -> &'static [(String, String)] {
    ABBREVIATIONS.get_or_init(|| {
        let path = std::env::var("GHOST_ABBREVIATIONS_PATH").unwrap_or_else(|_| {
            let mut p = std::env::current_dir().unwrap_or_default();
            p.push(".ghost");
            p.push("abbreviations.toml");
            p.to_string_lossy().to_string()
        });

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[ghost abbreviations] failed to read {path}: {e}");
                return Vec::new();
            }
        };

        parse_abbreviations_toml(&content)
    })
}

/// Parse abbreviation pairs from a TOML string. Exported for testing.
pub fn parse_abbreviations_toml(content: &str) -> Vec<(String, String)> {
    let table: toml::Value = match content.parse() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[ghost abbreviations] TOML parse error: {e}");
            return Vec::new();
        }
    };

    let Some(abbr_table) = table.get("abbreviations").and_then(|v| v.as_table()) else {
        return Vec::new();
    };

    let mut pairs: Vec<(String, String)> = abbr_table
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
        .filter(|(_, v)| !v.is_empty())
        .collect();

    // Sort by pattern length descending — longest match first
    pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    pairs
}

/// Apply abbreviations: case-insensitive replacement of known phrases.
/// Longest patterns match first to avoid partial replacements.
pub fn apply_abbreviations(message: &str) -> String {
    apply_abbreviations_with(message, load_abbreviations())
}

/// Inner function that accepts abbreviation pairs directly (for testing).
pub fn apply_abbreviations_with(message: &str, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return message.to_string();
    }

    let mut result = message.to_string();
    for (pattern, replacement) in pairs {
        result = case_insensitive_replace(&result, pattern, replacement);
    }
    result
}

/// Replace all case-insensitive occurrences of `pattern` in `text` with `replacement`.
fn case_insensitive_replace(text: &str, pattern: &str, replacement: &str) -> String {
    let lower_text = text.to_lowercase();
    let lower_pattern = pattern.to_lowercase();
    let pattern_len = pattern.len();

    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;

    for (start, _) in lower_text.match_indices(&lower_pattern) {
        if start < last_end {
            continue; // skip overlapping matches
        }
        result.push_str(&text[last_end..start]);
        result.push_str(replacement);
        last_end = start + pattern_len;
    }

    result.push_str(&text[last_end..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_filler ──────────────────────────────────────────────

    #[test]
    fn strip_filler_removes_prefix() {
        assert_eq!(strip_filler("hey what time is it"), "what time is it");
    }

    #[test]
    fn strip_filler_removes_suffix() {
        assert_eq!(strip_filler("what time is it thanks"), "what time is it");
    }

    #[test]
    fn strip_filler_removes_both() {
        assert_eq!(
            strip_filler("hey so what time is it thanks!"),
            "what time is it"
        );
    }

    #[test]
    fn strip_filler_chains_prefixes() {
        assert_eq!(strip_filler("hey so um what time is it"), "what time is it");
    }

    #[test]
    fn strip_filler_case_insensitive() {
        assert_eq!(
            strip_filler("Hey So What time is it Thanks"),
            "What time is it"
        );
    }

    #[test]
    fn strip_filler_preserves_mid_sentence() {
        let input = "can you please help me with um the thing";
        assert_eq!(strip_filler(input), input);
    }

    #[test]
    fn strip_filler_returns_original_if_too_short() {
        assert_eq!(strip_filler("hey thanks"), "hey thanks");
    }

    #[test]
    fn strip_filler_no_op_on_clean_input() {
        assert_eq!(
            strip_filler("what is the weather in boston"),
            "what is the weather in boston"
        );
    }

    #[test]
    fn strip_filler_collapses_whitespace() {
        assert_eq!(strip_filler("hey   what  time  is it"), "what time is it");
    }

    // ── is_low_confidence ─────────────────────────────────────────

    #[test]
    fn is_low_confidence_empty() {
        assert!(is_low_confidence(""));
    }

    #[test]
    fn is_low_confidence_whitespace() {
        assert!(is_low_confidence("   \n  "));
    }

    #[test]
    fn is_low_confidence_short() {
        assert!(is_low_confidence("Yes."));
    }

    #[test]
    fn is_low_confidence_hedging() {
        assert!(is_low_confidence(
            "I'm not sure about that, but here's what I think about the situation at hand."
        ));
    }

    #[test]
    fn is_low_confidence_good_response() {
        assert!(!is_low_confidence(
            "The weather in Boston is currently 72 degrees Fahrenheit with partly cloudy skies and a light breeze from the northeast."
        ));
    }

    #[test]
    fn is_low_confidence_as_an_ai() {
        assert!(is_low_confidence(
            "As an AI language model, I don't have personal experiences or real-time information."
        ));
    }

    // ── needs_intake ─────────────────────────────────────────────

    #[test]
    fn needs_intake_short_message() {
        assert!(!needs_intake("what time is it in boston"));
    }

    #[test]
    fn needs_intake_long_message() {
        let msg = "word ".repeat(50);
        assert!(needs_intake(msg.trim()));
    }

    #[test]
    fn needs_intake_medium_with_ambiguity() {
        assert!(needs_intake(
            "I think maybe we should update the daemon file or something because it has been kind of broken lately and stuff"
        ));
    }

    #[test]
    fn needs_intake_medium_without_ambiguity() {
        assert!(!needs_intake(
            "update the daemon endpoint to return the correct status code when the database connection pool is exhausted"
        ));
    }

    #[test]
    fn needs_intake_exactly_fifteen_no_signal() {
        // exactly 15 words, no ambiguity signal → in range but no signal, so false
        assert!(!needs_intake(
            "please update the daemon endpoint to return the correct status code when pool is down"
        ));
    }

    #[test]
    fn needs_intake_exactly_fifty_words() {
        let msg = "word ".repeat(50);
        assert!(needs_intake(msg.trim()));
    }

    // ── apply_abbreviations ─────────────────────────────────────

    fn test_pairs() -> Vec<(String, String)> {
        parse_abbreviations_toml(
            r#"
[abbreviations]
"the daemon file" = "daemon.rs"
"the daemon" = "daemon.rs"
"the memory stuff" = "director_notes table + memory.rs"
"the memory" = "memory.rs"
"#,
        )
    }

    #[test]
    fn abbreviation_known_replaced() {
        let pairs = test_pairs();
        let result = apply_abbreviations_with("update the daemon", &pairs);
        assert_eq!(result, "update daemon.rs");
    }

    #[test]
    fn abbreviation_case_insensitive() {
        let pairs = test_pairs();
        let result = apply_abbreviations_with("check The Daemon for issues", &pairs);
        assert_eq!(result, "check daemon.rs for issues");
    }

    #[test]
    fn abbreviation_unknown_unchanged() {
        let pairs = test_pairs();
        let result = apply_abbreviations_with("update the frobnicator", &pairs);
        assert_eq!(result, "update the frobnicator");
    }

    #[test]
    fn abbreviation_longer_pattern_priority() {
        let pairs = test_pairs();
        // "the memory stuff" (longer) should match before "the memory"
        let result = apply_abbreviations_with("fix the memory stuff", &pairs);
        assert_eq!(result, "fix director_notes table + memory.rs");
    }

    #[test]
    fn abbreviation_in_sentence_context() {
        let pairs = test_pairs();
        let result =
            apply_abbreviations_with("please update the daemon file and restart it", &pairs);
        assert_eq!(result, "please update daemon.rs and restart it");
    }

    #[test]
    fn abbreviation_empty_pairs() {
        let result = apply_abbreviations_with("nothing to replace here", &[]);
        assert_eq!(result, "nothing to replace here");
    }
}
