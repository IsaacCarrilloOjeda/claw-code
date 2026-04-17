//! Bible Agent — query classification, context assembly, and formatting.
//!
//! Classifies user messages into Bible query types (reference, word study,
//! topical, or not-Bible), retrieves relevant data from all four Bible tables,
//! and assembles a rich context block for injection into the system prompt.

use std::fmt::Write as _;

use crate::db::{BibleCrossRef, BiblePericope, BibleVerse, LexiconEntry};

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum BibleQueryType {
    Reference(VerseRef),
    WordStudy(String),
    Topical(String),
    NotBible,
}

#[derive(Debug, PartialEq)]
pub struct VerseRef {
    pub book: String,
    pub start_chapter: i32,
    pub start_verse: Option<i32>,
    pub end_chapter: Option<i32>,
    pub end_verse: Option<i32>,
}

// ---------------------------------------------------------------------------
// Book name normalization
// ---------------------------------------------------------------------------

/// Canonical book names with common abbreviations.
/// Each entry: (canonical, &[abbreviations]).
const BOOK_ALIASES: &[(&str, &[&str])] = &[
    ("Genesis", &["gen", "ge", "gn"]),
    ("Exodus", &["exod", "exo", "ex"]),
    ("Leviticus", &["lev", "le", "lv"]),
    ("Numbers", &["num", "nu", "nm", "nb"]),
    ("Deuteronomy", &["deut", "de", "dt"]),
    ("Joshua", &["josh", "jos", "jsh"]),
    ("Judges", &["judg", "jdg", "jg", "jdgs"]),
    ("Ruth", &["rth", "ru"]),
    (
        "1 Samuel",
        &[
            "1sam",
            "1sa",
            "1 sam",
            "1 sa",
            "1st samuel",
            "1st sam",
            "i samuel",
            "i sam",
        ],
    ),
    (
        "2 Samuel",
        &[
            "2sam",
            "2sa",
            "2 sam",
            "2 sa",
            "2nd samuel",
            "2nd sam",
            "ii samuel",
            "ii sam",
        ],
    ),
    (
        "1 Kings",
        &[
            "1kgs",
            "1ki",
            "1 kgs",
            "1 ki",
            "1st kings",
            "1st kgs",
            "i kings",
            "i kgs",
        ],
    ),
    (
        "2 Kings",
        &[
            "2kgs",
            "2ki",
            "2 kgs",
            "2 ki",
            "2nd kings",
            "2nd kgs",
            "ii kings",
            "ii kgs",
        ],
    ),
    (
        "1 Chronicles",
        &[
            "1chr",
            "1ch",
            "1 chr",
            "1 ch",
            "1st chronicles",
            "i chronicles",
        ],
    ),
    (
        "2 Chronicles",
        &[
            "2chr",
            "2ch",
            "2 chr",
            "2 ch",
            "2nd chronicles",
            "ii chronicles",
        ],
    ),
    ("Ezra", &["ezr", "ez"]),
    ("Nehemiah", &["neh", "ne"]),
    ("Esther", &["esth", "est", "es"]),
    ("Job", &["jb"]),
    ("Psalms", &["ps", "psa", "psm", "pss", "psalm"]),
    ("Proverbs", &["prov", "pro", "prv", "pr"]),
    ("Ecclesiastes", &["eccl", "ecc", "ec", "eccles"]),
    (
        "Song of Solomon",
        &["song", "sos", "sg", "song of songs", "canticles"],
    ),
    ("Isaiah", &["isa", "is"]),
    ("Jeremiah", &["jer", "je", "jr"]),
    ("Lamentations", &["lam", "la"]),
    ("Ezekiel", &["ezek", "eze", "ezk"]),
    ("Daniel", &["dan", "da", "dn"]),
    ("Hosea", &["hos", "ho"]),
    ("Joel", &["joe", "jl"]),
    ("Amos", &["am"]),
    ("Obadiah", &["obad", "ob"]),
    ("Jonah", &["jon", "jnh"]),
    ("Micah", &["mic", "mc"]),
    ("Nahum", &["nah", "na"]),
    ("Habakkuk", &["hab", "hb"]),
    ("Zephaniah", &["zeph", "zep", "zp"]),
    ("Haggai", &["hag", "hg"]),
    ("Zechariah", &["zech", "zec", "zc"]),
    ("Malachi", &["mal", "ml"]),
    // NT
    ("Matthew", &["matt", "mat", "mt"]),
    ("Mark", &["mrk", "mk", "mr"]),
    ("Luke", &["luk", "lk"]),
    ("John", &["jhn", "jn"]),
    ("Acts", &["act", "ac"]),
    ("Romans", &["rom", "ro", "rm"]),
    (
        "1 Corinthians",
        &[
            "1cor",
            "1co",
            "1 cor",
            "1 co",
            "1st corinthians",
            "i corinthians",
        ],
    ),
    (
        "2 Corinthians",
        &[
            "2cor",
            "2co",
            "2 cor",
            "2 co",
            "2nd corinthians",
            "ii corinthians",
        ],
    ),
    ("Galatians", &["gal", "ga"]),
    ("Ephesians", &["eph", "ep"]),
    ("Philippians", &["phil", "php", "pp"]),
    ("Colossians", &["col", "co"]),
    (
        "1 Thessalonians",
        &[
            "1thess",
            "1th",
            "1 thess",
            "1 th",
            "1st thessalonians",
            "i thessalonians",
        ],
    ),
    (
        "2 Thessalonians",
        &[
            "2thess",
            "2th",
            "2 thess",
            "2 th",
            "2nd thessalonians",
            "ii thessalonians",
        ],
    ),
    (
        "1 Timothy",
        &["1tim", "1ti", "1 tim", "1 ti", "1st timothy", "i timothy"],
    ),
    (
        "2 Timothy",
        &["2tim", "2ti", "2 tim", "2 ti", "2nd timothy", "ii timothy"],
    ),
    ("Titus", &["tit", "ti"]),
    ("Philemon", &["phlm", "phm"]),
    ("Hebrews", &["heb"]),
    ("James", &["jas", "jm"]),
    (
        "1 Peter",
        &[
            "1pet",
            "1pe",
            "1pt",
            "1 pet",
            "1 pe",
            "1st peter",
            "i peter",
        ],
    ),
    (
        "2 Peter",
        &[
            "2pet",
            "2pe",
            "2pt",
            "2 pet",
            "2 pe",
            "2nd peter",
            "ii peter",
        ],
    ),
    (
        "1 John",
        &["1jn", "1jhn", "1 jn", "1 jhn", "1st john", "i john"],
    ),
    (
        "2 John",
        &["2jn", "2jhn", "2 jn", "2 jhn", "2nd john", "ii john"],
    ),
    (
        "3 John",
        &["3jn", "3jhn", "3 jn", "3 jhn", "3rd john", "iii john"],
    ),
    ("Jude", &["jud", "jde"]),
    ("Revelation", &["rev", "re", "revelations"]),
];

/// Normalize a book name/abbreviation to the canonical form used in the database.
pub fn normalize_book_name(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let lower = trimmed.to_lowercase();

    for &(canonical, aliases) in BOOK_ALIASES {
        if lower == canonical.to_lowercase() {
            return Some(canonical.to_string());
        }
        for &alias in aliases {
            if lower == alias {
                return Some(canonical.to_string());
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// !bible prefix
// ---------------------------------------------------------------------------

/// Strip the `!bible` prefix from a message if present.
pub fn strip_bible_prefix(message: &str) -> (String, bool) {
    let trimmed = message.trim();

    if trimmed.len() > 6 {
        let prefix = &trimmed[..6];
        if prefix.eq_ignore_ascii_case("!bible") {
            let rest_char = trimmed.as_bytes()[6];
            if rest_char == b' ' || rest_char == b':' {
                let rest = trimmed[7..].trim().to_string();
                return (rest, true);
            }
        }
    }

    (trimmed.to_string(), false)
}

// ---------------------------------------------------------------------------
// Verse reference parsing
// ---------------------------------------------------------------------------

fn parse_verse_ref(input: &str) -> Option<VerseRef> {
    let s = input.trim();

    let mut split_pos = None;
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'0'..=b'9' | b':' | b'-' => {}
            b' ' => {
                if i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                    split_pos = Some(i);
                }
                break;
            }
            _ => break,
        }
    }

    let split_pos = split_pos?;
    let book_part = s[..split_pos].trim();
    let ref_part = &s[split_pos + 1..];

    let book = normalize_book_name(book_part)?;

    if let Some(colon_pos) = ref_part.find(':') {
        let chapter: i32 = ref_part[..colon_pos].parse().ok()?;
        let verse_part = &ref_part[colon_pos + 1..];

        if let Some(dash_pos) = verse_part.find('-') {
            let start_verse: i32 = verse_part[..dash_pos].parse().ok()?;
            let end_verse: i32 = verse_part[dash_pos + 1..].parse().ok()?;
            Some(VerseRef {
                book,
                start_chapter: chapter,
                start_verse: Some(start_verse),
                end_chapter: Some(chapter),
                end_verse: Some(end_verse),
            })
        } else {
            let verse_num: i32 = verse_part.parse().ok()?;
            Some(VerseRef {
                book,
                start_chapter: chapter,
                start_verse: Some(verse_num),
                end_chapter: None,
                end_verse: None,
            })
        }
    } else {
        let chapter: i32 = ref_part.parse().ok()?;
        Some(VerseRef {
            book,
            start_chapter: chapter,
            start_verse: None,
            end_chapter: None,
            end_verse: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Query classification
// ---------------------------------------------------------------------------

/// Classify a user message into a Bible query type.
pub fn classify_query(message: &str) -> BibleQueryType {
    let trimmed = message.trim();
    let lower = trimmed.to_lowercase();

    // 1. Reference detection
    if let Some(vref) = parse_verse_ref(trimmed) {
        return BibleQueryType::Reference(vref);
    }

    for prefix in &["read ", "show me ", "show ", "look up ", "lookup "] {
        if lower.starts_with(prefix) {
            if let Some(vref) = parse_verse_ref(&trimmed[prefix.len()..]) {
                return BibleQueryType::Reference(vref);
            }
        }
    }

    // 2. Word study detection
    if let Some(strongs) = extract_strongs_number(&lower) {
        return BibleQueryType::WordStudy(strongs);
    }

    // "what does X mean" — only match if "mean" appears after the term
    if lower.contains("what does ") && lower.contains(" mean") {
        if let Some(term) = extract_study_term(&lower, "what does ") {
            return BibleQueryType::WordStudy(term);
        }
    }

    let word_study_patterns = [
        "hebrew word for ",
        "greek word for ",
        "original word for ",
        "root word for ",
        "meaning of ",
        "strong's ",
        "strongs ",
    ];

    for pattern in &word_study_patterns {
        if lower.contains(pattern) {
            if let Some(term) = extract_study_term(&lower, pattern) {
                return BibleQueryType::WordStudy(term);
            }
        }
    }

    // 3. Topical detection
    let topical_patterns = [
        "what does the bible say about ",
        "what does the bible teach about ",
        "scripture about ",
        "scriptures about ",
        "biblical view of ",
        "biblical view on ",
        "bible and ",
        "verse about ",
        "verses about ",
        "biblical ",
        "according to the bible",
        "in the bible",
    ];

    for pattern in &topical_patterns {
        if lower.contains(pattern) {
            if let Some(topic) = extract_topic(&lower, pattern) {
                return BibleQueryType::Topical(topic);
            }
        }
    }

    BibleQueryType::NotBible
}

fn extract_strongs_number(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        if (bytes[i] == b'h' || bytes[i] == b'g')
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
        {
            if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
                continue;
            }
            let prefix = (bytes[i] as char).to_uppercase().next().unwrap();
            let mut end = i + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            let digits = &text[i + 1..end];
            if !digits.is_empty()
                && digits.len() <= 5
                && (end >= bytes.len() || !bytes[end].is_ascii_alphanumeric())
            {
                return Some(format!("{prefix}{digits}"));
            }
        }
    }
    None
}

fn extract_study_term(lower: &str, pattern: &str) -> Option<String> {
    let after = lower.split(pattern).nth(1)?;
    let term = if pattern.starts_with("what does") {
        after.split(" mean").next().unwrap_or(after).trim()
    } else {
        after.split(['?', '.', '!']).next().unwrap_or(after).trim()
    };

    if term.is_empty() {
        return None;
    }
    Some(term.to_string())
}

fn extract_topic(lower: &str, pattern: &str) -> Option<String> {
    let after = lower.split(pattern).nth(1)?;
    let topic = after.split(['?', '.', '!']).next().unwrap_or(after).trim();

    if topic.is_empty() {
        let before = lower.split(pattern).next()?.trim();
        if !before.is_empty() {
            return Some(before.to_string());
        }
        return None;
    }
    Some(topic.to_string())
}

// ---------------------------------------------------------------------------
// Context assembly — the main entry point called from chat_dispatcher
// ---------------------------------------------------------------------------

const CONTEXT_CAP: usize = 6000;
const REF_XREF_LIMIT: usize = 5;
const TOPICAL_VERSE_LIMIT: i64 = 8;
const TOPICAL_PERICOPE_LIMIT: i64 = 3;
const TOPICAL_XREF_PER_VERSE: usize = 3;
const WORD_STUDY_VERSE_LIMIT: i64 = 10;

/// Build a Bible context block to inject into the system prompt.
pub async fn load_bible_context(
    message: &str,
    query_embedding: Option<&[f32]>,
    pool: Option<&sqlx::PgPool>,
) -> String {
    let Some(pool) = pool else {
        return String::new();
    };

    let query_type = classify_query(message);
    if matches!(query_type, BibleQueryType::NotBible) {
        return String::new();
    }

    match query_type {
        BibleQueryType::Reference(ref vref) => load_reference_context(vref, pool).await,
        BibleQueryType::WordStudy(ref term) => {
            load_word_study_context(term, query_embedding, pool).await
        }
        BibleQueryType::Topical(ref _topic) => load_topical_context(query_embedding, pool).await,
        BibleQueryType::NotBible => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Reference context
// ---------------------------------------------------------------------------

async fn load_reference_context(vref: &VerseRef, pool: &sqlx::PgPool) -> String {
    let verses = if let Some(sv) = vref.start_verse {
        if let Some(ev) = vref.end_verse {
            let ec = vref.end_chapter.unwrap_or(vref.start_chapter);
            crate::db::get_bible_verse_range(pool, &vref.book, vref.start_chapter, sv, ec, ev).await
        } else {
            match crate::db::get_bible_verse(pool, &vref.book, vref.start_chapter, sv).await {
                Some(v) => vec![v],
                None => vec![],
            }
        }
    } else {
        crate::db::get_bible_verse_range(
            pool,
            &vref.book,
            vref.start_chapter,
            1,
            vref.start_chapter,
            200,
        )
        .await
    };

    if verses.is_empty() {
        return String::new();
    }

    let mut all_xrefs = Vec::new();
    for v in &verses {
        let xrefs = crate::db::get_cross_refs_from(pool, &v.book, v.chapter, v.verse).await;
        all_xrefs.extend(xrefs.into_iter().take(REF_XREF_LIMIT));
    }

    let mut all_lexicon = Vec::new();
    let mut seen_strongs = std::collections::HashSet::new();
    for v in &verses {
        for s in &v.strongs {
            if seen_strongs.insert(s.clone()) {
                if let Some(entry) = crate::db::get_lexicon_entry(pool, s).await {
                    all_lexicon.push(entry);
                }
            }
        }
    }

    let verses_with_dist: Vec<(BibleVerse, Option<f64>)> =
        verses.into_iter().map(|v| (v, None)).collect();
    let pericopes: Vec<(BiblePericope, Option<f64>)> = Vec::new();

    format_bible_context(
        &BibleQueryType::Reference(VerseRef {
            book: vref.book.clone(),
            start_chapter: vref.start_chapter,
            start_verse: vref.start_verse,
            end_chapter: vref.end_chapter,
            end_verse: vref.end_verse,
        }),
        &verses_with_dist,
        &pericopes,
        &all_xrefs,
        &all_lexicon,
    )
}

// ---------------------------------------------------------------------------
// Word study context
// ---------------------------------------------------------------------------

async fn load_word_study_context(
    term: &str,
    query_embedding: Option<&[f32]>,
    pool: &sqlx::PgPool,
) -> String {
    let mut lexicon_entries = Vec::new();
    let mut verses = Vec::new();

    if let Some(strongs) = extract_strongs_number(&term.to_lowercase()) {
        if let Some(entry) = crate::db::get_lexicon_entry(pool, &strongs).await {
            if let Some(ref root) = entry.root {
                let related = crate::db::search_lexicon_by_root(pool, root).await;
                for r in related {
                    if r.strongs_id != entry.strongs_id {
                        lexicon_entries.push(r);
                    }
                }
            }
            lexicon_entries.insert(0, entry);
        }
        let strongs_verses =
            crate::db::search_verses_by_strongs(pool, &strongs, WORD_STUDY_VERSE_LIMIT).await;
        for v in strongs_verses {
            verses.push((v, None));
        }
    } else if let Some(emb) = query_embedding {
        let results = crate::db::search_bible_verses(pool, emb, WORD_STUDY_VERSE_LIMIT).await;
        for (v, d) in results {
            verses.push((v, Some(d)));
        }
    }

    let mut all_xrefs = Vec::new();
    for (v, _) in verses.iter().take(3) {
        let xrefs = crate::db::get_cross_refs_from(pool, &v.book, v.chapter, v.verse).await;
        all_xrefs.extend(xrefs.into_iter().take(3));
    }

    if verses.is_empty() && lexicon_entries.is_empty() {
        return String::new();
    }

    let pericopes: Vec<(BiblePericope, Option<f64>)> = Vec::new();
    format_bible_context(
        &BibleQueryType::WordStudy(term.to_string()),
        &verses,
        &pericopes,
        &all_xrefs,
        &lexicon_entries,
    )
}

// ---------------------------------------------------------------------------
// Topical context
// ---------------------------------------------------------------------------

async fn load_topical_context(query_embedding: Option<&[f32]>, pool: &sqlx::PgPool) -> String {
    let Some(emb) = query_embedding else {
        return String::new();
    };

    let verse_results = crate::db::search_bible_verses(pool, emb, TOPICAL_VERSE_LIMIT).await;
    let pericope_results =
        crate::db::search_bible_pericopes(pool, emb, TOPICAL_PERICOPE_LIMIT).await;

    if verse_results.is_empty() && pericope_results.is_empty() {
        return String::new();
    }

    let verses: Vec<(BibleVerse, Option<f64>)> = verse_results
        .into_iter()
        .map(|(v, d)| (v, Some(d)))
        .collect();

    let pericopes: Vec<(BiblePericope, Option<f64>)> = pericope_results
        .into_iter()
        .map(|(p, d)| (p, Some(d)))
        .collect();

    let mut all_xrefs = Vec::new();
    for (v, _) in verses.iter().take(TOPICAL_XREF_PER_VERSE) {
        let xrefs = crate::db::get_cross_refs_from(pool, &v.book, v.chapter, v.verse).await;
        all_xrefs.extend(xrefs.into_iter().take(TOPICAL_XREF_PER_VERSE));
    }

    let lexicon: Vec<LexiconEntry> = Vec::new();
    format_bible_context(
        &BibleQueryType::Topical(String::new()),
        &verses,
        &pericopes,
        &all_xrefs,
        &lexicon,
    )
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn format_bible_context(
    query_type: &BibleQueryType,
    verses: &[(BibleVerse, Option<f64>)],
    pericopes: &[(BiblePericope, Option<f64>)],
    cross_refs: &[BibleCrossRef],
    lexicon: &[LexiconEntry],
) -> String {
    if verses.is_empty() && pericopes.is_empty() && lexicon.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(4096);
    out.push_str("## Bible study context\n");

    match query_type {
        BibleQueryType::Reference(_) => out.push_str("Query type: reference\n"),
        BibleQueryType::WordStudy(_) => out.push_str("Query type: word study\n"),
        BibleQueryType::Topical(_) => out.push_str("Query type: topical\n"),
        BibleQueryType::NotBible => {}
    }

    format_verses(&mut out, verses, lexicon);
    format_cross_refs(&mut out, cross_refs);
    format_lexicon(&mut out, lexicon);
    format_pericopes(&mut out, pericopes);

    // Final cap enforcement
    if out.len() > CONTEXT_CAP {
        out.truncate(CONTEXT_CAP);
        if let Some(pos) = out.rfind('\n') {
            out.truncate(pos + 1);
        }
        out.push_str("...(truncated)\n");
    }

    out
}

fn format_verses(out: &mut String, verses: &[(BibleVerse, Option<f64>)], lexicon: &[LexiconEntry]) {
    if verses.is_empty() {
        return;
    }

    out.push_str("\n### Verses\n");
    for (v, dist) in verses {
        let _ = write!(out, "**{} {}:{}**", v.book, v.chapter, v.verse);
        if let Some(d) = dist {
            let _ = write!(out, " (similarity: {d:.3})");
        }
        out.push('\n');

        if !v.original_text.is_empty() {
            let lang_label = match v.original_lang.as_str() {
                "hebrew" => "Hebrew",
                "aramaic" => "Aramaic",
                "greek" => "Greek",
                _ => &v.original_lang,
            };
            let _ = writeln!(out, "Original ({lang_label}): {}", v.original_text);
        }

        let _ = writeln!(out, "KJV: {}", v.kjv_text);

        if let Some(ref web) = v.web_text {
            let _ = writeln!(out, "WEB: {web}");
        }

        if !v.strongs.is_empty() {
            let glosses: Vec<String> = v
                .strongs
                .iter()
                .filter_map(|s| {
                    lexicon
                        .iter()
                        .find(|e| e.strongs_id == *s)
                        .map(|e| {
                            format!(
                                "{} ({}: {})",
                                s,
                                e.transliteration,
                                short_def(&e.definition)
                            )
                        })
                        .or_else(|| Some(s.clone()))
                })
                .collect();
            let _ = writeln!(out, "Strong's: {}", glosses.join(" | "));
        }

        out.push('\n');

        if out.len() > CONTEXT_CAP {
            break;
        }
    }
}

fn format_cross_refs(out: &mut String, cross_refs: &[BibleCrossRef]) {
    if cross_refs.is_empty() || out.len() >= CONTEXT_CAP {
        return;
    }

    out.push_str("### Cross-references\n");
    for xr in cross_refs {
        let _ = writeln!(
            out,
            "- {} {}:{} -> {} {}:{} ({})",
            xr.source_book,
            xr.source_chapter,
            xr.source_verse,
            xr.target_book,
            xr.target_chapter,
            xr.target_verse,
            xr.rel_type
        );

        if out.len() > CONTEXT_CAP {
            break;
        }
    }
    out.push('\n');
}

fn format_lexicon(out: &mut String, lexicon: &[LexiconEntry]) {
    if lexicon.is_empty() || out.len() >= CONTEXT_CAP {
        return;
    }

    out.push_str("### Word study\n");
    for entry in lexicon {
        let _ = writeln!(
            out,
            "**{} ({}, {})**",
            entry.strongs_id, entry.transliteration, entry.original_word
        );
        if let Some(ref root) = entry.root {
            let _ = writeln!(out, "Root: {root}");
        }
        let _ = writeln!(out, "Definition: {}", entry.definition);
        if !entry.semantic_range.is_empty() {
            let _ = writeln!(out, "Semantic range: {}", entry.semantic_range.join(", "));
        }
        if entry.usage_count > 0 {
            let _ = writeln!(out, "Occurrences: {}", entry.usage_count);
        }
        out.push('\n');

        if out.len() > CONTEXT_CAP {
            break;
        }
    }
}

fn format_pericopes(out: &mut String, pericopes: &[(BiblePericope, Option<f64>)]) {
    if pericopes.is_empty() || out.len() >= CONTEXT_CAP {
        return;
    }

    out.push_str("### Thematic context\n");
    for (p, dist) in pericopes {
        let _ = write!(
            out,
            "Pericope: \"{}\" ({} {}:{} -- {} {}:{})",
            p.title,
            p.start_book,
            p.start_chapter,
            p.start_verse,
            p.end_book,
            p.end_chapter,
            p.end_verse
        );
        if let Some(ref genre) = p.genre {
            let _ = write!(out, " [{genre}]");
        }
        if let Some(d) = dist {
            let _ = write!(out, " (similarity: {d:.3})");
        }
        out.push('\n');
        if let Some(ref summary) = p.summary {
            let _ = writeln!(out, "Summary: {summary}");
        }
        out.push('\n');

        if out.len() > CONTEXT_CAP {
            break;
        }
    }
}

fn short_def(def: &str) -> &str {
    if def.len() <= 50 {
        return def;
    }
    if let Some(pos) = def[..50].rfind([',', ';']) {
        &def[..pos]
    } else {
        &def[..50]
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_john_3_16() {
        match classify_query("John 3:16") {
            BibleQueryType::Reference(vref) => {
                assert_eq!(vref.book, "John");
                assert_eq!(vref.start_chapter, 3);
                assert_eq!(vref.start_verse, Some(16));
            }
            other => panic!("expected Reference, got {other:?}"),
        }
    }

    #[test]
    fn classify_romans_range() {
        match classify_query("Romans 8:28-30") {
            BibleQueryType::Reference(vref) => {
                assert_eq!(vref.book, "Romans");
                assert_eq!(vref.start_chapter, 8);
                assert_eq!(vref.start_verse, Some(28));
                assert_eq!(vref.end_verse, Some(30));
            }
            other => panic!("expected Reference, got {other:?}"),
        }
    }

    #[test]
    fn classify_genesis_whole_chapter() {
        match classify_query("Genesis 1") {
            BibleQueryType::Reference(vref) => {
                assert_eq!(vref.book, "Genesis");
                assert_eq!(vref.start_chapter, 1);
                assert_eq!(vref.start_verse, None);
            }
            other => panic!("expected Reference, got {other:?}"),
        }
    }

    #[test]
    fn classify_1corinthians_range() {
        match classify_query("1 Corinthians 13:4-7") {
            BibleQueryType::Reference(vref) => {
                assert_eq!(vref.book, "1 Corinthians");
                assert_eq!(vref.start_chapter, 13);
                assert_eq!(vref.start_verse, Some(4));
                assert_eq!(vref.end_verse, Some(7));
            }
            other => panic!("expected Reference, got {other:?}"),
        }
    }

    #[test]
    fn classify_psalm_23() {
        match classify_query("Psalm 23") {
            BibleQueryType::Reference(vref) => {
                assert_eq!(vref.book, "Psalms");
                assert_eq!(vref.start_chapter, 23);
                assert_eq!(vref.start_verse, None);
            }
            other => panic!("expected Reference, got {other:?}"),
        }
    }

    #[test]
    fn classify_gen_abbreviation() {
        match classify_query("Gen 1:1") {
            BibleQueryType::Reference(vref) => {
                assert_eq!(vref.book, "Genesis");
                assert_eq!(vref.start_chapter, 1);
                assert_eq!(vref.start_verse, Some(1));
            }
            other => panic!("expected Reference, got {other:?}"),
        }
    }

    #[test]
    fn classify_agape_word_study() {
        match classify_query("what does agape mean") {
            BibleQueryType::WordStudy(term) => {
                assert_eq!(term, "agape");
            }
            other => panic!("expected WordStudy, got {other:?}"),
        }
    }

    #[test]
    fn classify_strongs_number() {
        match classify_query("Strong's H1254") {
            BibleQueryType::WordStudy(term) => {
                assert_eq!(term, "H1254");
            }
            other => panic!("expected WordStudy, got {other:?}"),
        }
    }

    #[test]
    fn classify_hebrew_word_study() {
        match classify_query("hebrew word for love") {
            BibleQueryType::WordStudy(term) => {
                assert_eq!(term, "love");
            }
            other => panic!("expected WordStudy, got {other:?}"),
        }
    }

    #[test]
    fn classify_topical_patience() {
        match classify_query("what does the bible say about patience") {
            BibleQueryType::Topical(topic) => {
                assert_eq!(topic, "patience");
            }
            other => panic!("expected Topical, got {other:?}"),
        }
    }

    #[test]
    fn classify_topical_forgiveness() {
        match classify_query("verses about forgiveness") {
            BibleQueryType::Topical(topic) => {
                assert_eq!(topic, "forgiveness");
            }
            other => panic!("expected Topical, got {other:?}"),
        }
    }

    #[test]
    fn classify_not_bible_weather() {
        assert_eq!(
            classify_query("what's the weather"),
            BibleQueryType::NotBible
        );
    }

    #[test]
    fn classify_not_bible_debug() {
        assert_eq!(
            classify_query("help me debug this function"),
            BibleQueryType::NotBible
        );
    }

    #[test]
    fn normalize_gen() {
        assert_eq!(normalize_book_name("Gen"), Some("Genesis".to_string()));
    }

    #[test]
    fn normalize_1cor() {
        assert_eq!(
            normalize_book_name("1Cor"),
            Some("1 Corinthians".to_string())
        );
    }

    #[test]
    fn normalize_ps() {
        assert_eq!(normalize_book_name("Ps"), Some("Psalms".to_string()));
    }

    #[test]
    fn normalize_psalm() {
        assert_eq!(normalize_book_name("Psalm"), Some("Psalms".to_string()));
    }

    #[test]
    fn normalize_song_of_songs() {
        assert_eq!(
            normalize_book_name("Song of Songs"),
            Some("Song of Solomon".to_string())
        );
    }

    #[test]
    fn normalize_rev() {
        assert_eq!(normalize_book_name("Rev"), Some("Revelation".to_string()));
    }

    #[test]
    fn normalize_unknown() {
        assert_eq!(normalize_book_name("Foobar"), None);
    }

    #[test]
    fn strip_prefix_space() {
        let (msg, forced) = strip_bible_prefix("!bible what is love");
        assert_eq!(msg, "what is love");
        assert!(forced);
    }

    #[test]
    fn strip_prefix_uppercase() {
        let (msg, forced) = strip_bible_prefix("!Bible John 3:16");
        assert_eq!(msg, "John 3:16");
        assert!(forced);
    }

    #[test]
    fn strip_no_prefix() {
        let (msg, forced) = strip_bible_prefix("what is love");
        assert_eq!(msg, "what is love");
        assert!(!forced);
    }

    #[test]
    fn parse_john_3_16() {
        let vref = parse_verse_ref("John 3:16").unwrap();
        assert_eq!(vref.book, "John");
        assert_eq!(vref.start_chapter, 3);
        assert_eq!(vref.start_verse, Some(16));
        assert_eq!(vref.end_chapter, None);
        assert_eq!(vref.end_verse, None);
    }

    #[test]
    fn parse_romans_range() {
        let vref = parse_verse_ref("Romans 8:28-30").unwrap();
        assert_eq!(vref.book, "Romans");
        assert_eq!(vref.start_chapter, 8);
        assert_eq!(vref.start_verse, Some(28));
        assert_eq!(vref.end_verse, Some(30));
    }

    #[test]
    fn parse_genesis_chapter() {
        let vref = parse_verse_ref("Genesis 1").unwrap();
        assert_eq!(vref.book, "Genesis");
        assert_eq!(vref.start_chapter, 1);
        assert_eq!(vref.start_verse, None);
    }

    #[test]
    fn format_empty_inputs() {
        let result = format_bible_context(
            &BibleQueryType::Reference(VerseRef {
                book: "John".into(),
                start_chapter: 3,
                start_verse: Some(16),
                end_chapter: None,
                end_verse: None,
            }),
            &[],
            &[],
            &[],
            &[],
        );
        assert!(result.is_empty());
    }

    #[test]
    fn format_verse_shows_original() {
        let verse = BibleVerse {
            id: "test".into(),
            book: "Genesis".into(),
            chapter: 1,
            verse: 1,
            book_order: 1,
            testament: "OT".into(),
            original_lang: "hebrew".into(),
            original_text: "test hebrew text".into(),
            kjv_text: "In the beginning".into(),
            web_text: Some("In the beginning,".into()),
            strongs: vec![],
            morphology: None,
        };
        let result = format_bible_context(
            &BibleQueryType::Reference(VerseRef {
                book: "Genesis".into(),
                start_chapter: 1,
                start_verse: Some(1),
                end_chapter: None,
                end_verse: None,
            }),
            &[(verse, None)],
            &[],
            &[],
            &[],
        );
        assert!(result.contains("test hebrew text"));
        assert!(result.contains("In the beginning"));
    }

    #[test]
    fn format_includes_cross_refs() {
        let verse = BibleVerse {
            id: "test".into(),
            book: "Genesis".into(),
            chapter: 1,
            verse: 1,
            book_order: 1,
            testament: "OT".into(),
            original_lang: "hebrew".into(),
            original_text: String::new(),
            kjv_text: "In the beginning".into(),
            web_text: None,
            strongs: vec![],
            morphology: None,
        };
        let xref = BibleCrossRef {
            id: "xr1".into(),
            source_book: "Genesis".into(),
            source_chapter: 1,
            source_verse: 1,
            target_book: "John".into(),
            target_chapter: 1,
            target_verse: 1,
            rel_type: "thematic".into(),
            source_dataset: "TSK".into(),
        };
        let result = format_bible_context(
            &BibleQueryType::Reference(VerseRef {
                book: "Genesis".into(),
                start_chapter: 1,
                start_verse: Some(1),
                end_chapter: None,
                end_verse: None,
            }),
            &[(verse, None)],
            &[],
            &[xref],
            &[],
        );
        assert!(result.contains("Cross-references"));
        assert!(result.contains("John 1:1"));
    }

    #[test]
    fn format_includes_lexicon() {
        let entry = LexiconEntry {
            strongs_id: "H1254".into(),
            original_word: "bara".into(),
            transliteration: "bara".into(),
            lang: "hebrew".into(),
            root: Some("br'".into()),
            definition: "to create, shape, form".into(),
            usage_count: 54,
            semantic_range: vec!["create".into(), "shape".into()],
        };
        let result = format_bible_context(
            &BibleQueryType::WordStudy("H1254".into()),
            &[],
            &[],
            &[],
            &[entry],
        );
        assert!(result.contains("Word study"));
        assert!(result.contains("H1254"));
        assert!(result.contains("to create"));
    }

    #[test]
    fn format_context_cap_enforced() {
        let verses: Vec<(BibleVerse, Option<f64>)> = (1..=50)
            .map(|i| {
                (
                    BibleVerse {
                        id: format!("id{i}"),
                        book: "Psalms".into(),
                        chapter: 119,
                        verse: i,
                        book_order: 19,
                        testament: "OT".into(),
                        original_lang: "hebrew".into(),
                        original_text: "a".repeat(100),
                        kjv_text: "b".repeat(100),
                        web_text: Some("c".repeat(100)),
                        strongs: vec![],
                        morphology: None,
                    },
                    Some(0.5),
                )
            })
            .collect();

        let result = format_bible_context(
            &BibleQueryType::Topical("test".into()),
            &verses,
            &[],
            &[],
            &[],
        );
        assert!(result.len() <= CONTEXT_CAP + 50);
    }
}
