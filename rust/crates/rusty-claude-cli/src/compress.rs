//! Filler stripping and response confidence heuristics for the cascade dispatcher.

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
}
