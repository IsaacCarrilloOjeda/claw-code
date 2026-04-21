//! Prompt-cache shaping for Anthropic's `system` field.
//!
//! Anthropic prompt caching expects `system` as an array of content blocks,
//! with `cache_control: {"type": "ephemeral"}` on any block we want cached.
//! We split the system prompt into a stable prefix (core context — same text
//! across a session, high hit rate) and a dynamic suffix (memory, schedule,
//! per-turn context — changes each turn, not worth caching).

use serde_json::{json, Value};

/// Build an Anthropic-compatible `system` field as a JSON array with
/// `cache_control` on the stable prefix. Pass the core context (stable) and
/// the dynamic suffix. If `dynamic` is empty, returns a single cached block.
pub fn build_cached_system(stable: &str, dynamic: &str) -> Value {
    if dynamic.is_empty() {
        json!([
            {
                "type": "text",
                "text": stable,
                "cache_control": {"type": "ephemeral"}
            }
        ])
    } else {
        json!([
            {
                "type": "text",
                "text": stable,
                "cache_control": {"type": "ephemeral"}
            },
            {
                "type": "text",
                "text": dynamic
            }
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_blocks_when_dynamic_present() {
        let v = build_cached_system("core prefix", "dynamic suffix");
        let arr = v.as_array().expect("system must be a JSON array");
        assert_eq!(arr.len(), 2, "expected stable + dynamic blocks");

        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], "core prefix");
        assert_eq!(
            arr[0]["cache_control"]["type"], "ephemeral",
            "stable block must be marked ephemeral-cached"
        );

        assert_eq!(arr[1]["type"], "text");
        assert_eq!(arr[1]["text"], "dynamic suffix");
        assert!(
            arr[1].get("cache_control").is_none(),
            "dynamic block must NOT carry cache_control"
        );
    }

    #[test]
    fn single_cached_block_when_dynamic_empty() {
        let v = build_cached_system("core only", "");
        let arr = v.as_array().expect("system must be a JSON array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["text"], "core only");
        assert_eq!(arr[0]["cache_control"]["type"], "ephemeral");
    }
}
