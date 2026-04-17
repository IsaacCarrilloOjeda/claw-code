//! Code macro system (Phase E1).
//!
//! Loads shorthand macro definitions from `.ghost/macros.toml` and expands
//! them in AI outputs. Worker prompts get a preamble listing available macros,
//! and post-processing replaces `MACRO_NAME("arg1", "arg2")` with the full
//! expansion.

use std::collections::HashMap;
use std::fmt::Write;

#[derive(Debug, Clone)]
pub struct Macro {
    pub name: String,
    pub params: Vec<String>,
    pub expansion: String,
}

/// Load macros from the TOML file at `.ghost/macros.toml` (or `GHOST_MACROS_PATH`).
/// Returns an empty map on any error (missing file, parse failure, etc.).
pub fn load_macros() -> HashMap<String, Macro> {
    let path = std::env::var("GHOST_MACROS_PATH").unwrap_or_else(|_| {
        let mut p = std::env::current_dir().unwrap_or_default();
        p.push(".ghost");
        p.push("macros.toml");
        p.to_string_lossy().to_string()
    });

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ghost macros] failed to read {path}: {e}");
            return HashMap::new();
        }
    };

    parse_macros_toml(&content)
}

/// Parse macros from a TOML string. Exported for testing.
pub fn parse_macros_toml(content: &str) -> HashMap<String, Macro> {
    let table: toml::Value = match content.parse() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[ghost macros] TOML parse error: {e}");
            return HashMap::new();
        }
    };

    let Some(macros_table) = table.get("macros").and_then(|v| v.as_table()) else {
        return HashMap::new();
    };

    let mut result = HashMap::new();

    for (name, value) in macros_table {
        let params: Vec<String> = value
            .get("params")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let expansion = value
            .get("expansion")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if expansion.is_empty() {
            continue;
        }

        result.insert(
            name.clone(),
            Macro {
                name: name.clone(),
                params,
                expansion,
            },
        );
    }

    result
}

/// Expand macro invocations in `text`. Matches `MACRO_NAME("arg1", "arg2")`
/// patterns and substitutes `{param_name}` placeholders in the expansion.
/// Unknown macros or wrong param counts are left as-is.
pub fn expand_macros(text: &str, macros: &HashMap<String, Macro>) -> String {
    if macros.is_empty() {
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while !remaining.is_empty() {
        // Find the next potential macro call: uppercase letters/underscores followed by '('
        if let Some(start) = find_macro_start(remaining) {
            // Push everything before the potential macro
            result.push_str(&remaining[..start]);
            let after_start = &remaining[start..];

            if let Some((name, args, consumed)) = parse_macro_call(after_start) {
                if let Some(mac) = macros.get(&name) {
                    if mac.params.len() == args.len() {
                        // Expand: substitute {param_name} with provided args
                        let expanded = substitute(&mac.expansion, &mac.params, &args);
                        result.push_str(&expanded);
                        remaining = &after_start[consumed..];
                        continue;
                    }
                }
                // Unknown macro or wrong param count — leave as-is
                result.push_str(&after_start[..1]);
                remaining = &after_start[1..];
            } else {
                // Not a valid macro call pattern — advance past the first char
                result.push_str(&after_start[..1]);
                remaining = &after_start[1..];
            }
        } else {
            result.push_str(remaining);
            break;
        }
    }

    result
}

/// Prepend a preamble to a worker prompt listing available macros.
pub fn inject_macro_dictionary(prompt: &str, macros: &HashMap<String, Macro>) -> String {
    if macros.is_empty() {
        return prompt.to_string();
    }

    let mut preamble =
        String::from("Available code macros (use these instead of writing the code):\n");

    let mut names: Vec<&String> = macros.keys().collect();
    names.sort();

    for name in names {
        let mac = &macros[name];
        let params_str = mac
            .params
            .iter()
            .map(|p| format!("\"{p}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(preamble, "- {name}({params_str})");
    }

    preamble.push_str("When you can use a macro, write MACRO_NAME(\"args\") on its own line.\n\n");
    preamble.push_str(prompt);
    preamble
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Find the byte offset of the next potential macro name start (sequence of
/// uppercase ASCII letters and underscores, at least 2 chars, followed by `(`).
fn find_macro_start(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if is_macro_char(bytes[i]) {
            let start = i;
            while i < len && is_macro_char(bytes[i]) {
                i += 1;
            }
            let name_len = i - start;
            if name_len >= 2 && i < len && bytes[i] == b'(' {
                return Some(start);
            }
        } else {
            i += 1;
        }
    }

    None
}

fn is_macro_char(b: u8) -> bool {
    b.is_ascii_uppercase() || b == b'_'
}

/// Try to parse a macro call at the start of `text`. Returns `(name, args, bytes_consumed)`
/// or `None` if the pattern doesn't match.
fn parse_macro_call(text: &str) -> Option<(String, Vec<String>, usize)> {
    let bytes = text.as_bytes();
    let mut i = 0;

    // Read name
    while i < bytes.len() && is_macro_char(bytes[i]) {
        i += 1;
    }
    if i < 2 || i >= bytes.len() || bytes[i] != b'(' {
        return None;
    }
    let name = text[..i].to_string();
    i += 1; // skip '('

    let mut args = Vec::new();

    // Parse comma-separated quoted args
    loop {
        // Skip whitespace
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }

        if i >= bytes.len() {
            return None;
        }

        // Check for closing paren (empty args or trailing)
        if bytes[i] == b')' {
            return Some((name, args, i + 1));
        }

        // Skip comma between args
        if !args.is_empty() {
            if bytes[i] != b',' {
                return None;
            }
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
        }

        if i >= bytes.len() {
            return None;
        }

        // Expect a quoted string
        if bytes[i] != b'"' {
            return None;
        }
        i += 1; // skip opening quote

        let arg_start = i;
        while i < bytes.len() && bytes[i] != b'"' {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                i += 2; // skip escaped char
            } else {
                i += 1;
            }
        }
        if i >= bytes.len() {
            return None;
        }
        args.push(text[arg_start..i].to_string());
        i += 1; // skip closing quote
    }
}

/// Substitute `{param_name}` placeholders in the template with argument values.
fn substitute(template: &str, params: &[String], args: &[String]) -> String {
    let mut result = template.to_string();
    for (param, arg) in params.iter().zip(args.iter()) {
        let placeholder = format!("{{{param}}}");
        result = result.replace(&placeholder, arg);
    }
    result
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TOML: &str = r#"
[macros.AUTH_BEARER]
params = ["key_var"]
expansion = """
let expected = std::env::var("{key_var}").unwrap_or_default();
"""

[macros.DB_QUERY_EMBED]
params = ["table", "embed_col", "limit"]
expansion = """
SELECT * FROM {table} WHERE {embed_col} IS NOT NULL LIMIT {limit}
"""
"#;

    #[test]
    fn load_macros_from_toml_string() {
        let macros = parse_macros_toml(TEST_TOML);
        assert_eq!(macros.len(), 2);
        assert!(macros.contains_key("AUTH_BEARER"));
        assert!(macros.contains_key("DB_QUERY_EMBED"));
        assert_eq!(macros["AUTH_BEARER"].params, vec!["key_var"]);
        assert_eq!(
            macros["DB_QUERY_EMBED"].params,
            vec!["table", "embed_col", "limit"]
        );
    }

    #[test]
    fn expand_single_macro_with_params() {
        let macros = parse_macros_toml(TEST_TOML);
        let input = r#"AUTH_BEARER("GHOST_DAEMON_KEY")"#;
        let expanded = expand_macros(input, &macros);
        assert!(expanded.contains(r#"std::env::var("GHOST_DAEMON_KEY")"#));
        assert!(!expanded.contains("AUTH_BEARER"));
    }

    #[test]
    fn expand_macro_in_surrounding_text() {
        let macros = parse_macros_toml(TEST_TOML);
        let input = r#"before text AUTH_BEARER("MY_KEY") after text"#;
        let expanded = expand_macros(input, &macros);
        assert!(expanded.starts_with("before text "));
        assert!(expanded.ends_with(" after text"));
        assert!(expanded.contains(r#"std::env::var("MY_KEY")"#));
    }

    #[test]
    fn unknown_macro_left_as_is() {
        let macros = parse_macros_toml(TEST_TOML);
        let input = r#"UNKNOWN_MACRO("arg1")"#;
        let expanded = expand_macros(input, &macros);
        assert_eq!(expanded, input);
    }

    #[test]
    fn wrong_param_count_left_as_is() {
        let macros = parse_macros_toml(TEST_TOML);
        // AUTH_BEARER expects 1 param, giving it 2
        let input = r#"AUTH_BEARER("a", "b")"#;
        let expanded = expand_macros(input, &macros);
        assert_eq!(expanded, input);
    }

    #[test]
    fn empty_macros_file_returns_empty() {
        let macros = parse_macros_toml("");
        assert!(macros.is_empty());
    }

    #[test]
    fn multi_param_expansion() {
        let macros = parse_macros_toml(TEST_TOML);
        let input = r#"DB_QUERY_EMBED("scholar_solutions", "problem_embed", "5")"#;
        let expanded = expand_macros(input, &macros);
        assert!(expanded.contains("scholar_solutions"));
        assert!(expanded.contains("problem_embed"));
        assert!(expanded.contains("LIMIT 5"));
    }

    #[test]
    fn inject_dictionary_adds_preamble() {
        let macros = parse_macros_toml(TEST_TOML);
        let prompt = "Write a function";
        let injected = inject_macro_dictionary(prompt, &macros);
        assert!(injected.starts_with("Available code macros"));
        assert!(injected.contains("AUTH_BEARER"));
        assert!(injected.contains("DB_QUERY_EMBED"));
        assert!(injected.ends_with("Write a function"));
    }

    #[test]
    fn inject_dictionary_empty_macros_returns_prompt() {
        let macros = HashMap::new();
        let prompt = "Write a function";
        let injected = inject_macro_dictionary(prompt, &macros);
        assert_eq!(injected, prompt);
    }
}
