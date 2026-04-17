//! Bible data ingestion pipeline (Phase B1).
//!
//! Reads verse-aligned JSON/TSV files from `.ghost/bible-data/`, parses them,
//! embeds composite strings via `crate::memory::embed_batch()`, and bulk-inserts
//! into the Bible tables created in Phase B0.
//!
//! Run via `claw bible-ingest [--data-dir PATH]`.

use std::path::{Path, PathBuf};
use std::time::Instant;

/// Book metadata: (name, order, testament, `original_lang`).
const BOOKS: &[(&str, i32, &str, &str)] = &[
    ("Genesis", 1, "OT", "hebrew"),
    ("Exodus", 2, "OT", "hebrew"),
    ("Leviticus", 3, "OT", "hebrew"),
    ("Numbers", 4, "OT", "hebrew"),
    ("Deuteronomy", 5, "OT", "hebrew"),
    ("Joshua", 6, "OT", "hebrew"),
    ("Judges", 7, "OT", "hebrew"),
    ("Ruth", 8, "OT", "hebrew"),
    ("1 Samuel", 9, "OT", "hebrew"),
    ("2 Samuel", 10, "OT", "hebrew"),
    ("1 Kings", 11, "OT", "hebrew"),
    ("2 Kings", 12, "OT", "hebrew"),
    ("1 Chronicles", 13, "OT", "hebrew"),
    ("2 Chronicles", 14, "OT", "hebrew"),
    ("Ezra", 15, "OT", "hebrew"),
    ("Nehemiah", 16, "OT", "hebrew"),
    ("Esther", 17, "OT", "hebrew"),
    ("Job", 18, "OT", "hebrew"),
    ("Psalms", 19, "OT", "hebrew"),
    ("Proverbs", 20, "OT", "hebrew"),
    ("Ecclesiastes", 21, "OT", "hebrew"),
    ("Song of Solomon", 22, "OT", "hebrew"),
    ("Isaiah", 23, "OT", "hebrew"),
    ("Jeremiah", 24, "OT", "hebrew"),
    ("Lamentations", 25, "OT", "hebrew"),
    ("Ezekiel", 26, "OT", "hebrew"),
    ("Daniel", 27, "OT", "hebrew"),
    ("Hosea", 28, "OT", "hebrew"),
    ("Joel", 29, "OT", "hebrew"),
    ("Amos", 30, "OT", "hebrew"),
    ("Obadiah", 31, "OT", "hebrew"),
    ("Jonah", 32, "OT", "hebrew"),
    ("Micah", 33, "OT", "hebrew"),
    ("Nahum", 34, "OT", "hebrew"),
    ("Habakkuk", 35, "OT", "hebrew"),
    ("Zephaniah", 36, "OT", "hebrew"),
    ("Haggai", 37, "OT", "hebrew"),
    ("Zechariah", 38, "OT", "hebrew"),
    ("Malachi", 39, "OT", "hebrew"),
    ("Matthew", 40, "NT", "greek"),
    ("Mark", 41, "NT", "greek"),
    ("Luke", 42, "NT", "greek"),
    ("John", 43, "NT", "greek"),
    ("Acts", 44, "NT", "greek"),
    ("Romans", 45, "NT", "greek"),
    ("1 Corinthians", 46, "NT", "greek"),
    ("2 Corinthians", 47, "NT", "greek"),
    ("Galatians", 48, "NT", "greek"),
    ("Ephesians", 49, "NT", "greek"),
    ("Philippians", 50, "NT", "greek"),
    ("Colossians", 51, "NT", "greek"),
    ("1 Thessalonians", 52, "NT", "greek"),
    ("2 Thessalonians", 53, "NT", "greek"),
    ("1 Timothy", 54, "NT", "greek"),
    ("2 Timothy", 55, "NT", "greek"),
    ("Titus", 56, "NT", "greek"),
    ("Philemon", 57, "NT", "greek"),
    ("Hebrews", 58, "NT", "greek"),
    ("James", 59, "NT", "greek"),
    ("1 Peter", 60, "NT", "greek"),
    ("2 Peter", 61, "NT", "greek"),
    ("1 John", 62, "NT", "greek"),
    ("2 John", 63, "NT", "greek"),
    ("3 John", 64, "NT", "greek"),
    ("Jude", 65, "NT", "greek"),
    ("Revelation", 66, "NT", "greek"),
];

pub struct IngestStats {
    pub verses_total: u32,
    pub verses_embedded: u32,
    pub pericopes: u32,
    pub cross_refs: u32,
    pub lexicon_entries: u32,
    pub elapsed_secs: u64,
}

fn book_metadata(name: &str) -> Option<(i32, &'static str, &'static str)> {
    BOOKS
        .iter()
        .find(|(n, _, _, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, order, test, lang)| (*order, *test, *lang))
}

/// Determine original language for a verse, handling Aramaic portions of
/// Daniel (2:4b-7:28) and Ezra (4:8-6:18, 7:12-26).
fn original_lang_for_verse<'a>(
    book: &str,
    chapter: i32,
    _verse: i32,
    default_lang: &'a str,
) -> &'a str {
    if book.eq_ignore_ascii_case("Daniel") && ((3..=7).contains(&chapter) || chapter == 2) {
        return "aramaic";
    }
    if book.eq_ignore_ascii_case("Ezra")
        && ((5..=6).contains(&chapter) || chapter == 4 || chapter == 7)
    {
        return "aramaic";
    }
    default_lang
}

fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("GHOST_BIBLE_DATA_PATH") {
        return PathBuf::from(p);
    }
    PathBuf::from(".ghost/bible-data")
}

// ---------------------------------------------------------------------------
// JSON types for parsing data files
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct KjvVerse {
    book: String,
    chapter: i32,
    verse: i32,
    text: String,
}

#[derive(serde::Deserialize)]
struct OriginalVerse {
    book: String,
    chapter: i32,
    verse: i32,
    text: String,
    #[serde(default)]
    strongs: Vec<String>,
    #[serde(default)]
    morphology: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct LexiconJson {
    strongs_id: String,
    original_word: String,
    transliteration: String,
    #[serde(default)]
    definition: String,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    semantic_range: Vec<String>,
}

#[derive(serde::Deserialize)]
struct PericopeJson {
    title: String,
    start_book: String,
    start_chapter: i32,
    start_verse: i32,
    end_book: String,
    end_chapter: i32,
    end_verse: i32,
    #[serde(default)]
    genre: Option<String>,
}

// ---------------------------------------------------------------------------
// Ingestion pipeline
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
pub async fn ingest_bible(
    pool: &sqlx::PgPool,
    custom_dir: Option<&str>,
) -> Result<IngestStats, String> {
    let start = Instant::now();
    let dir = custom_dir.map_or_else(data_dir, PathBuf::from);

    if !dir.exists() {
        return Err(format!(
            "data directory not found: {}. Place Bible data files there or set GHOST_BIBLE_DATA_PATH.",
            dir.display()
        ));
    }

    // 1. Load KJV (required)
    let kjv_path = dir.join("kjv.json");
    if !kjv_path.exists() {
        return Err(format!("kjv.json not found in {}", dir.display()));
    }
    let kjv_data =
        std::fs::read_to_string(&kjv_path).map_err(|e| format!("failed to read kjv.json: {e}"))?;
    let kjv_verses: Vec<KjvVerse> =
        serde_json::from_str(&kjv_data).map_err(|e| format!("failed to parse kjv.json: {e}"))?;
    eprintln!("[bible ingest] loaded {} KJV verses", kjv_verses.len());

    // 2. Load optional data files
    let web_verses = load_optional_json::<KjvVerse>(&dir, "web.json");
    let hebrew_verses = load_optional_json::<OriginalVerse>(&dir, "hebrew-wlc.json");
    let greek_verses = load_optional_json::<OriginalVerse>(&dir, "greek-ugnt.json");

    // Build lookup maps for optional data
    let web_map = build_verse_map_kjv(web_verses.as_ref());
    let hebrew_map = build_verse_map_original(hebrew_verses.as_ref());
    let greek_map = build_verse_map_original(greek_verses.as_ref());

    // 3. Ingest lexicon entries first
    let mut lexicon_count: u32 = 0;
    for filename in &["strongs-hebrew.json", "strongs-greek.json"] {
        let lang = if filename.contains("hebrew") {
            "hebrew"
        } else {
            "greek"
        };
        if let Some(entries) = load_optional_json::<LexiconJson>(&dir, filename) {
            for entry in &entries {
                let stored = crate::db::insert_lexicon_entry(
                    pool,
                    &entry.strongs_id,
                    &entry.original_word,
                    &entry.transliteration,
                    lang,
                    entry.root.as_deref(),
                    &entry.definition,
                    0,
                    &entry.semantic_range,
                )
                .await;
                if stored {
                    lexicon_count += 1;
                }
            }
            eprintln!("[bible ingest] ingested {lexicon_count} lexicon entries from {filename}");
        }
    }

    // 4. Ingest pericopes
    let mut pericope_count: u32 = 0;
    let mut pericope_ranges: Vec<(String, String, i32, i32, String, i32, i32)> = Vec::new();
    if let Some(pericopes) = load_optional_json::<PericopeJson>(&dir, "pericopes.json") {
        for p in &pericopes {
            let id = crate::db::insert_bible_pericope(
                pool,
                &p.title,
                &p.start_book,
                p.start_chapter,
                p.start_verse,
                &p.end_book,
                p.end_chapter,
                p.end_verse,
                p.genre.as_deref(),
                None,
                None,
            )
            .await;
            if let Some(pericope_uuid) = id {
                pericope_ranges.push((
                    pericope_uuid,
                    p.start_book.clone(),
                    p.start_chapter,
                    p.start_verse,
                    p.end_book.clone(),
                    p.end_chapter,
                    p.end_verse,
                ));
                pericope_count += 1;
            }
        }
        eprintln!("[bible ingest] ingested {pericope_count} pericopes");
    }

    // 5. Ingest verses with batch embedding
    let mut verses_total: u32 = 0;
    let mut verses_embedded: u32 = 0;

    // Collect all verses into batches for embedding
    let mut batch_texts: Vec<String> = Vec::new();
    #[allow(clippy::type_complexity)]
    let mut batch_meta: Vec<(
        String,
        i32,
        i32,
        i32,
        String,
        String,
        String,
        String,
        Option<String>,
        Vec<String>,
        Option<serde_json::Value>,
        Option<String>,
    )> = Vec::new();

    for kjv in &kjv_verses {
        let Some((book_order, testament, default_lang)) = book_metadata(&kjv.book) else {
            continue;
        };

        let lang = original_lang_for_verse(&kjv.book, kjv.chapter, kjv.verse, default_lang);

        // Merge data from optional sources
        let key = verse_key(&kjv.book, kjv.chapter, kjv.verse);
        let web_text = web_map.get(&key).map(String::as_str);

        let (original_text, strongs, morphology) = if testament == "OT" {
            if let Some(hv) = hebrew_map.get(&key) {
                (hv.text.as_str(), &hv.strongs, hv.morphology.as_ref())
            } else {
                ("", &vec![] as &Vec<String>, None)
            }
        } else if let Some(gv) = greek_map.get(&key) {
            (gv.text.as_str(), &gv.strongs, gv.morphology.as_ref())
        } else {
            ("", &vec![] as &Vec<String>, None)
        };

        // Find matching pericope
        let pericope_id = find_pericope(&pericope_ranges, &kjv.book, kjv.chapter, kjv.verse);

        // Build composite embedding string
        let strongs_joined = strongs.join(" ");
        let composite = format!(
            "{} {}:{} | {} | {} | {}",
            kjv.book, kjv.chapter, kjv.verse, original_text, kjv.text, strongs_joined
        );

        batch_texts.push(composite);
        batch_meta.push((
            kjv.book.clone(),
            kjv.chapter,
            kjv.verse,
            book_order,
            testament.to_string(),
            lang.to_string(),
            original_text.to_string(),
            kjv.text.clone(),
            web_text.map(ToString::to_string),
            strongs.clone(),
            morphology.cloned(),
            pericope_id,
        ));
    }

    // Process in batches of 128
    let batch_size = 128;
    let total = batch_texts.len();
    for chunk_start in (0..total).step_by(batch_size) {
        let chunk_end = (chunk_start + batch_size).min(total);
        let texts = &batch_texts[chunk_start..chunk_end];

        let embeddings = match crate::memory::embed_batch(texts).await {
            Ok(embs) => embs,
            Err(e) => {
                eprintln!("[bible ingest] batch embed failed ({e}), inserting without embeddings");
                vec![]
            }
        };

        for (i, meta) in batch_meta[chunk_start..chunk_end].iter().enumerate() {
            let embedding = embeddings.get(i).map(Vec::as_slice);

            let stored = crate::db::insert_bible_verse(
                pool,
                &meta.0,           // book
                meta.1,            // chapter
                meta.2,            // verse
                meta.3,            // book_order
                &meta.4,           // testament
                &meta.5,           // original_lang
                &meta.6,           // original_text
                &meta.7,           // kjv_text
                meta.8.as_deref(), // web_text
                &meta.9,           // strongs
                meta.10.as_ref(),  // morphology
                embedding,
                meta.11.as_deref(), // pericope_id
            )
            .await;

            if stored {
                verses_total += 1;
                if embedding.is_some() {
                    verses_embedded += 1;
                }
            }
        }

        if (chunk_start + batch_size) % 500 < batch_size && chunk_start > 0 {
            eprintln!(
                "[bible ingest] {}/{total} verses embedded...",
                chunk_start + batch_size
            );
        }

        // Small delay between batches to respect rate limits
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    eprintln!("[bible ingest] verses: {verses_total} total, {verses_embedded} with embeddings");

    // 6. Ingest cross-references
    let mut crossref_count: u32 = 0;
    let xref_path = dir.join("cross-refs.tsv");
    if xref_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&xref_path) {
            for line in content.lines().skip(1) {
                // Skip header
                let fields: Vec<&str> = line.split('\t').collect();
                if fields.len() >= 7 {
                    let stored = crate::db::insert_bible_cross_ref(
                        pool,
                        fields[0],                      // source_book
                        fields[1].parse().unwrap_or(0), // source_chapter
                        fields[2].parse().unwrap_or(0), // source_verse
                        fields[3],                      // target_book
                        fields[4].parse().unwrap_or(0), // target_chapter
                        fields[5].parse().unwrap_or(0), // target_verse
                        fields[6],                      // rel_type
                        "TSK",
                    )
                    .await;
                    if stored {
                        crossref_count += 1;
                    }
                }
            }
            eprintln!("[bible ingest] ingested {crossref_count} cross-references");
        }
    }

    let elapsed = start.elapsed().as_secs();
    let stats = IngestStats {
        verses_total,
        verses_embedded,
        pericopes: pericope_count,
        cross_refs: crossref_count,
        lexicon_entries: lexicon_count,
        elapsed_secs: elapsed,
    };

    eprintln!(
        "[bible ingest] done in {elapsed}s: {} verses ({} embedded), {} pericopes, {} cross-refs, {} lexicon",
        stats.verses_total, stats.verses_embedded, stats.pericopes, stats.cross_refs, stats.lexicon_entries
    );

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_optional_json<T: serde::de::DeserializeOwned>(
    dir: &Path,
    filename: &str,
) -> Option<Vec<T>> {
    let path = dir.join(filename);
    if !path.exists() {
        eprintln!("[bible ingest] {filename} not found, skipping");
        return None;
    }
    let data = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str(&data) {
        Ok(parsed) => {
            eprintln!("[bible ingest] loaded {filename}");
            Some(parsed)
        }
        Err(e) => {
            eprintln!("[bible ingest] failed to parse {filename}: {e}");
            None
        }
    }
}

fn verse_key(book: &str, chapter: i32, verse: i32) -> String {
    format!("{book}:{chapter}:{verse}")
}

fn build_verse_map_kjv(
    verses: Option<&Vec<KjvVerse>>,
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Some(vs) = verses {
        for v in vs {
            map.insert(verse_key(&v.book, v.chapter, v.verse), v.text.clone());
        }
    }
    map
}

fn build_verse_map_original(
    verses: Option<&Vec<OriginalVerse>>,
) -> std::collections::HashMap<String, OriginalVerse> {
    let mut map = std::collections::HashMap::new();
    if let Some(vs) = verses {
        for v in vs {
            map.insert(
                verse_key(&v.book, v.chapter, v.verse),
                OriginalVerse {
                    book: v.book.clone(),
                    chapter: v.chapter,
                    verse: v.verse,
                    text: v.text.clone(),
                    strongs: v.strongs.clone(),
                    morphology: v.morphology.clone(),
                },
            );
        }
    }
    map
}

fn find_pericope(
    ranges: &[(String, String, i32, i32, String, i32, i32)],
    book: &str,
    chapter: i32,
    verse: i32,
) -> Option<String> {
    for (id, start_book, start_ch, start_v, end_book, end_ch, end_v) in ranges {
        if start_book != book || end_book != book {
            continue;
        }
        let after_start = chapter > *start_ch || (chapter == *start_ch && verse >= *start_v);
        let before_end = chapter < *end_ch || (chapter == *end_ch && verse <= *end_v);
        if after_start && before_end {
            return Some(id.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_metadata_lookup() {
        let (order, testament, lang) = book_metadata("Genesis").unwrap();
        assert_eq!(order, 1);
        assert_eq!(testament, "OT");
        assert_eq!(lang, "hebrew");
    }

    #[test]
    fn book_metadata_nt() {
        let (order, testament, lang) = book_metadata("Matthew").unwrap();
        assert_eq!(order, 40);
        assert_eq!(testament, "NT");
        assert_eq!(lang, "greek");
    }

    #[test]
    fn book_metadata_case_insensitive() {
        assert!(book_metadata("genesis").is_some());
        assert!(book_metadata("REVELATION").is_some());
    }

    #[test]
    fn book_metadata_unknown() {
        assert!(book_metadata("Foobar").is_none());
    }

    #[test]
    fn aramaic_detection_daniel() {
        assert_eq!(original_lang_for_verse("Daniel", 3, 1, "hebrew"), "aramaic");
        assert_eq!(original_lang_for_verse("Daniel", 7, 1, "hebrew"), "aramaic");
        assert_eq!(original_lang_for_verse("Daniel", 1, 1, "hebrew"), "hebrew");
        assert_eq!(original_lang_for_verse("Daniel", 8, 1, "hebrew"), "hebrew");
    }

    #[test]
    fn aramaic_detection_ezra() {
        assert_eq!(original_lang_for_verse("Ezra", 4, 8, "hebrew"), "aramaic");
        assert_eq!(original_lang_for_verse("Ezra", 5, 1, "hebrew"), "aramaic");
        assert_eq!(original_lang_for_verse("Ezra", 1, 1, "hebrew"), "hebrew");
    }

    #[test]
    fn verse_key_format() {
        assert_eq!(verse_key("John", 3, 16), "John:3:16");
    }

    #[test]
    fn parse_kjv_verse_json() {
        let json =
            r#"{"book": "Genesis", "chapter": 1, "verse": 1, "text": "In the beginning..."}"#;
        let v: KjvVerse = serde_json::from_str(json).unwrap();
        assert_eq!(v.book, "Genesis");
        assert_eq!(v.chapter, 1);
        assert_eq!(v.verse, 1);
        assert_eq!(v.text, "In the beginning...");
    }

    #[test]
    fn parse_original_verse_with_strongs() {
        let json = r#"{
            "book": "Genesis", "chapter": 1, "verse": 1,
            "text": "test hebrew",
            "strongs": ["H7225", "H1254"],
            "morphology": {"words": []}
        }"#;
        let v: OriginalVerse = serde_json::from_str(json).unwrap();
        assert_eq!(v.strongs, vec!["H7225", "H1254"]);
        assert!(v.morphology.is_some());
    }

    #[test]
    fn parse_lexicon_entry() {
        let json = r#"{
            "strongs_id": "H1254",
            "original_word": "bara",
            "transliteration": "bara",
            "definition": "to create",
            "root": "br'",
            "semantic_range": ["create", "shape"]
        }"#;
        let e: LexiconJson = serde_json::from_str(json).unwrap();
        assert_eq!(e.strongs_id, "H1254");
        assert_eq!(e.definition, "to create");
    }

    #[test]
    fn composite_embedding_string_format() {
        let composite = format!(
            "{} {}:{} | {} | {} | {}",
            "John", 3, 16, "greek text", "For God so loved", "G3588 G2316"
        );
        assert!(composite.contains("John 3:16"));
        assert!(composite.contains("greek text"));
        assert!(composite.contains("For God so loved"));
        assert!(composite.contains("G3588"));
    }

    #[test]
    fn find_pericope_match() {
        let ranges = vec![(
            "uuid1".to_string(),
            "Genesis".to_string(),
            1,
            1,
            "Genesis".to_string(),
            2,
            3,
        )];
        assert_eq!(
            find_pericope(&ranges, "Genesis", 1, 5),
            Some("uuid1".to_string())
        );
        assert_eq!(
            find_pericope(&ranges, "Genesis", 2, 3),
            Some("uuid1".to_string())
        );
        assert_eq!(find_pericope(&ranges, "Genesis", 3, 1), None);
        assert_eq!(find_pericope(&ranges, "Exodus", 1, 1), None);
    }
}
