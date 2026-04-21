#[derive(Debug, Clone, PartialEq)]
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

/// Classify a raw inbound message. Returns the intent and the message with its
/// leading prefix stripped (whitespace-trimmed).
///
/// Wave 2: move the real prefix parsing here from `daemon.rs` and `chat_dispatcher.rs`.
/// For now this is a correct-but-minimal implementation so callers can start using it.
pub fn classify(raw: &str) -> (Intent, String) {
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.strip_prefix('!') {
        return (Intent::Director, rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix('?') {
        return (Intent::Research, rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix('>') {
        return (Intent::Scheduled, rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix('@') {
        return (Intent::Calendar, rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
        return (Intent::ChiefOfStaff, rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix('&') {
        return (Intent::Docs, rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix('.') {
        return (Intent::Ignore, rest.trim().to_string());
    }
    (Intent::Chat, trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_no_prefix_as_chat() {
        let (intent, body) = classify("hello there");
        assert_eq!(intent, Intent::Chat);
        assert_eq!(body, "hello there");
    }

    #[test]
    fn classifies_bang_prefix_as_director() {
        let (intent, body) = classify("!do the thing");
        assert_eq!(intent, Intent::Director);
        assert_eq!(body, "do the thing");
    }

    #[test]
    fn classifies_question_prefix_as_research() {
        let (intent, body) = classify("?what is rust");
        assert_eq!(intent, Intent::Research);
        assert_eq!(body, "what is rust");
    }

    #[test]
    fn classifies_gt_prefix_as_scheduled() {
        let (intent, body) = classify("> morning-brief");
        assert_eq!(intent, Intent::Scheduled);
        assert_eq!(body, "morning-brief");
    }

    #[test]
    fn classifies_at_prefix_as_calendar() {
        let (intent, body) = classify("@what's on my schedule today");
        assert_eq!(intent, Intent::Calendar);
        assert_eq!(body, "what's on my schedule today");
    }

    #[test]
    fn classifies_hash_prefix_as_chief_of_staff() {
        let (intent, body) = classify("#plan my week around the mobile launch");
        assert_eq!(intent, Intent::ChiefOfStaff);
        assert_eq!(body, "plan my week around the mobile launch");
    }

    #[test]
    fn classifies_ampersand_prefix_as_docs() {
        let (intent, body) = classify("&create \"Team Notes\"");
        assert_eq!(intent, Intent::Docs);
        assert_eq!(body, "create \"Team Notes\"");
    }

    #[test]
    fn classifies_dot_prefix_as_ignore() {
        let (intent, body) = classify(".silent note");
        assert_eq!(intent, Intent::Ignore);
        assert_eq!(body, "silent note");
    }

    #[test]
    fn strips_leading_whitespace_before_prefix() {
        let (intent, body) = classify("   !routed");
        assert_eq!(intent, Intent::Director);
        assert_eq!(body, "routed");
    }

    #[test]
    fn whitespace_only_input_is_chat() {
        let (intent, body) = classify("   ");
        assert_eq!(intent, Intent::Chat);
        assert_eq!(body, "");
    }
}
