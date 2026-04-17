//! Postgres database layer for GHOST daemon.
//!
//! Connects via `DATABASE_URL` env var (injected by Railway).
//! Runs embedded migrations on startup, then seeds the `director_config`
//! singleton if absent.
//!
//! All queries use runtime-checked `sqlx::query` / `sqlx::query_as` (not the
//! compile-time macros) so the Docker build does not require a live database.

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

/// Build a connection pool and run embedded migrations. Returns `None` if
/// `DATABASE_URL` is not set (local dev without Postgres — daemon starts fine,
/// /jobs and /director/* will return 503 until a database is configured).
pub async fn init_pool() -> Option<PgPool> {
    let url = std::env::var("DATABASE_URL").ok()?;

    let pool = match PgPoolOptions::new().max_connections(5).connect(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[ghost db] failed to connect to Postgres: {e}");
            return None;
        }
    };

    // Migrations are embedded at compile time from rust/migrations/.
    // sqlx::migrate! does NOT need SQLX_OFFLINE — it just reads .sql files at build time.
    if let Err(e) = sqlx::migrate!("../../migrations").run(&pool).await {
        eprintln!("[ghost db] migration failed: {e}");
        return None;
    }

    eprintln!("[ghost db] migrations applied, connected to Postgres");
    Some(pool)
}

// ---------------------------------------------------------------------------
// Job model
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct Job {
    pub id: String,
    pub status: String,
    pub input: String,
    pub output: Option<String>,
    pub agent: Option<String>,
    pub source: Option<String>,
    pub phone_from: Option<String>,
    pub requires_confirmation: bool,
    pub confirmation_token: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

pub async fn list_jobs(pool: &PgPool, limit: i64) -> Vec<Job> {
    let rows = sqlx::query(
        "SELECT id::text, status, input, output, agent, source, phone_from,
                requires_confirmation, confirmation_token,
                created_at::text, updated_at::text, completed_at::text
         FROM jobs
         ORDER BY created_at DESC
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter().map(row_to_job).collect()
}

pub async fn get_job(pool: &PgPool, id: &str) -> Option<Job> {
    // Validate that the id is a UUID string before passing to the query.
    uuid::Uuid::parse_str(id).ok()?;

    let row = sqlx::query(
        "SELECT id::text, status, input, output, agent, source, phone_from,
                requires_confirmation, confirmation_token,
                created_at::text, updated_at::text, completed_at::text
         FROM jobs
         WHERE id = $1::uuid",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .ok()??;

    Some(row_to_job(&row))
}

fn row_to_job(row: &sqlx::postgres::PgRow) -> Job {
    Job {
        id: row.try_get("id").unwrap_or_default(),
        status: row.try_get("status").unwrap_or_default(),
        input: row.try_get("input").unwrap_or_default(),
        output: row.try_get("output").unwrap_or(None),
        agent: row.try_get("agent").unwrap_or(None),
        source: row.try_get("source").unwrap_or(None),
        phone_from: row.try_get("phone_from").unwrap_or(None),
        requires_confirmation: row.try_get("requires_confirmation").unwrap_or(false),
        confirmation_token: row.try_get("confirmation_token").unwrap_or(None),
        created_at: row.try_get("created_at").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
        completed_at: row.try_get("completed_at").unwrap_or(None),
    }
}

/// Create a new job with `status = 'running'`. Returns the UUID string on
/// success, `None` on insert failure.
pub async fn create_job(
    pool: &PgPool,
    input: &str,
    agent: &str,
    source: &str,
    phone_from: Option<&str>,
) -> Option<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let result = sqlx::query(
        "INSERT INTO jobs (id, status, input, agent, source, phone_from)
         VALUES ($1::uuid, 'running', $2, $3, $4, $5)",
    )
    .bind(&id)
    .bind(input)
    .bind(agent)
    .bind(source)
    .bind(phone_from)
    .execute(pool)
    .await;

    match result {
        Ok(_) => Some(id),
        Err(e) => {
            eprintln!("[ghost db] create_job failed: {e}");
            None
        }
    }
}

/// Mark a job as done, storing the output.
pub async fn update_job_done(pool: &PgPool, id: &str, output: &str) {
    let _ = sqlx::query(
        "UPDATE jobs
         SET status = 'done', output = $1, completed_at = now(), updated_at = now()
         WHERE id = $2::uuid",
    )
    .bind(output)
    .bind(id)
    .execute(pool)
    .await;
}

/// Mark a job as failed, storing the error message in `output`.
pub async fn update_job_failed(pool: &PgPool, id: &str, error: &str) {
    let _ = sqlx::query(
        "UPDATE jobs
         SET status = 'failed', output = $1, completed_at = now(), updated_at = now()
         WHERE id = $2::uuid",
    )
    .bind(error)
    .bind(id)
    .execute(pool)
    .await;
}

/// Mark jobs stuck in `running` for over 1 hour as `failed`.
/// Called at startup and on each health-check cycle.
pub async fn cleanup_stale_jobs(pool: &PgPool) {
    let result = sqlx::query(
        "UPDATE jobs
         SET status = 'failed',
             output = 'stale: daemon restarted or timed out',
             completed_at = now(),
             updated_at = now()
         WHERE status = 'running'
           AND created_at < now() - interval '1 hour'",
    )
    .execute(pool)
    .await;

    if let Ok(r) = result {
        let n = r.rows_affected();
        if n > 0 {
            eprintln!("[ghost db] cleaned up {n} stale job(s)");
        }
    }
}

// ---------------------------------------------------------------------------
// Director config model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DirectorConfig {
    pub id: i32,
    pub primary_model: String,
    pub fallback_model: String,
    pub primary_healthy: bool,
    pub fallback_healthy: bool,
    pub last_health_check: Option<String>,
    pub updated_at: String,
}

pub async fn get_director_config(pool: &PgPool) -> Option<DirectorConfig> {
    let row = sqlx::query(
        "SELECT id, primary_model, fallback_model, primary_healthy, fallback_healthy,
                last_health_check::text, updated_at::text
         FROM director_config
         WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .ok()??;

    Some(DirectorConfig {
        id: row.try_get("id").unwrap_or(1),
        primary_model: row
            .try_get("primary_model")
            .unwrap_or_else(|_| "claude-sonnet-4-6".into()),
        fallback_model: row
            .try_get("fallback_model")
            .unwrap_or_else(|_| "gpt-4o".into()),
        primary_healthy: row.try_get("primary_healthy").unwrap_or(true),
        fallback_healthy: row.try_get("fallback_healthy").unwrap_or(true),
        last_health_check: row.try_get("last_health_check").unwrap_or(None),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
    })
}

/// Allowed models that can be set via the config endpoint.
const ALLOWED_MODELS: &[&str] = &[
    "claude-sonnet-4-6",
    "claude-opus-4-6",
    "claude-haiku-4-5-20251001",
    "gpt-4o",
    "gpt-4o-mini",
    "deepseek-chat",
];

pub async fn update_director_config(
    pool: &PgPool,
    primary_model: Option<&str>,
    fallback_model: Option<&str>,
) -> Result<DirectorConfig, String> {
    // Validate model names against allow-list.
    if let Some(m) = primary_model {
        if !ALLOWED_MODELS.contains(&m) {
            return Err(format!("unknown primary model: {m}"));
        }
    }
    if let Some(m) = fallback_model {
        if !ALLOWED_MODELS.contains(&m) {
            return Err(format!("unknown fallback model: {m}"));
        }
    }

    sqlx::query(
        "UPDATE director_config
         SET primary_model  = COALESCE($1, primary_model),
             fallback_model = COALESCE($2, fallback_model),
             updated_at     = now()
         WHERE id = 1",
    )
    .bind(primary_model)
    .bind(fallback_model)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    get_director_config(pool)
        .await
        .ok_or_else(|| "config row missing after update".into())
}

// ---------------------------------------------------------------------------
// Memory notes model (Phase 2)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct MemoryNote {
    pub id: String,
    pub category: String,
    pub content: String,
    pub confidence: f64,
    pub created_at: String,
}

/// Format a float vector as a pgvector literal: `[v1,v2,...]`
pub fn vec_to_pgvector(v: &[f32]) -> String {
    let inner = v
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("[{inner}]")
}

/// Insert a new memory note. `embedding` may be `None` when `OPENAI_API_KEY`
/// is absent — the note is stored without a vector and won't appear in
/// semantic search results but will still show in the memory panel.
pub async fn insert_note(
    pool: &PgPool,
    category: &str,
    content: &str,
    embedding: Option<&[f32]>,
) -> bool {
    let id = uuid::Uuid::new_v4().to_string();
    if let Some(emb) = embedding {
        let embedding_str = vec_to_pgvector(emb);
        sqlx::query(
            "INSERT INTO director_notes (id, category, content, embedding)
             VALUES ($1::uuid, $2, $3, $4::vector)",
        )
        .bind(&id)
        .bind(category)
        .bind(content)
        .bind(&embedding_str)
        .execute(pool)
        .await
        .is_ok()
    } else {
        sqlx::query(
            "INSERT INTO director_notes (id, category, content)
             VALUES ($1::uuid, $2, $3)",
        )
        .bind(&id)
        .bind(category)
        .bind(content)
        .execute(pool)
        .await
        .is_ok()
    }
}

/// Semantic search: return up to `limit` notes ordered by cosine similarity to
/// the query embedding. Skips expired notes.
pub async fn search_notes(pool: &PgPool, embedding: &[f32], limit: i64) -> Vec<MemoryNote> {
    let embedding_str = vec_to_pgvector(embedding);
    let rows = sqlx::query(
        "SELECT id::text, category, content, confidence, created_at::text
         FROM director_notes
         WHERE expires_at IS NULL AND embedding IS NOT NULL
         ORDER BY embedding <=> $1::vector
         LIMIT $2",
    )
    .bind(&embedding_str)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter()
        .map(|row| MemoryNote {
            id: row.try_get("id").unwrap_or_default(),
            category: row.try_get("category").unwrap_or_default(),
            content: row.try_get("content").unwrap_or_default(),
            confidence: row.try_get("confidence").unwrap_or(1.0),
            created_at: row.try_get("created_at").unwrap_or_default(),
        })
        .collect()
}

/// List all non-expired notes (for the dashboard memory panel).
pub async fn list_notes(pool: &PgPool, limit: i64) -> Vec<MemoryNote> {
    let rows = sqlx::query(
        "SELECT id::text, category, content, confidence, created_at::text
         FROM director_notes
         WHERE expires_at IS NULL
         ORDER BY created_at DESC
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter()
        .map(|row| MemoryNote {
            id: row.try_get("id").unwrap_or_default(),
            category: row.try_get("category").unwrap_or_default(),
            content: row.try_get("content").unwrap_or_default(),
            confidence: row.try_get("confidence").unwrap_or(1.0),
            created_at: row.try_get("created_at").unwrap_or_default(),
        })
        .collect()
}

/// Delete a note by UUID. Returns true if a row was deleted.
pub async fn delete_note(pool: &PgPool, id: &str) -> bool {
    if uuid::Uuid::parse_str(id).is_err() {
        return false;
    }
    sqlx::query("DELETE FROM director_notes WHERE id = $1::uuid")
        .bind(id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected() > 0)
        .unwrap_or(false)
}

/// Confidence decay: called by the background task every 24h.
/// Reduces confidence by 5% on notes older than 30 days; marks notes below
/// 0.1 confidence as expired.
pub async fn decay_notes_confidence(pool: &PgPool) {
    let _ = sqlx::query(
        "UPDATE director_notes
         SET confidence = confidence * 0.95,
             expires_at = CASE
                 WHEN confidence * 0.95 < 0.1 THEN now()
                 ELSE expires_at
             END
         WHERE created_at < now() - interval '30 days'
           AND expires_at IS NULL",
    )
    .execute(pool)
    .await;
}

// ---------------------------------------------------------------------------
// Scholar solutions model (Phase 3)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct ScholarSolution {
    pub id: String,
    pub problem_sig: String,
    pub solution: String,
    pub solution_lang: Option<String>,
    pub context_file: Option<String>,
    pub failed_attempts: Vec<String>,
    pub success_count: i32,
    pub last_used_at: String,
}

/// Semantic search on `scholar_solutions` by cosine similarity on `problem_embed`.
pub async fn search_scholar(pool: &PgPool, embedding: &[f32], limit: i64) -> Vec<ScholarSolution> {
    let embedding_str = vec_to_pgvector(embedding);
    let rows = sqlx::query(
        "SELECT id::text, problem_sig, solution, solution_lang, context_file,
                failed_attempts, success_count, last_used_at::text,
                (problem_embed <=> $1::vector) AS distance
         FROM scholar_solutions
         WHERE problem_embed IS NOT NULL
         ORDER BY problem_embed <=> $1::vector
         LIMIT $2",
    )
    .bind(&embedding_str)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter()
        .map(|row| ScholarSolution {
            id: row.try_get("id").unwrap_or_default(),
            problem_sig: row.try_get("problem_sig").unwrap_or_default(),
            solution: row.try_get("solution").unwrap_or_default(),
            solution_lang: row.try_get("solution_lang").unwrap_or(None),
            context_file: row.try_get("context_file").unwrap_or(None),
            failed_attempts: row
                .try_get::<Vec<String>, _>("failed_attempts")
                .unwrap_or_default(),
            success_count: row.try_get("success_count").unwrap_or(1),
            last_used_at: row.try_get("last_used_at").unwrap_or_default(),
        })
        .collect()
}

/// Semantic search returning results with their cosine distance for threshold checks.
pub async fn search_scholar_with_distance(
    pool: &PgPool,
    embedding: &[f32],
    limit: i64,
) -> Vec<(ScholarSolution, f64)> {
    let embedding_str = vec_to_pgvector(embedding);
    let rows = sqlx::query(
        "SELECT id::text, problem_sig, solution, solution_lang, context_file,
                failed_attempts, success_count, last_used_at::text,
                (problem_embed <=> $1::vector) AS distance
         FROM scholar_solutions
         WHERE problem_embed IS NOT NULL
         ORDER BY problem_embed <=> $1::vector
         LIMIT $2",
    )
    .bind(&embedding_str)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter()
        .map(|row| {
            let sol = ScholarSolution {
                id: row.try_get("id").unwrap_or_default(),
                problem_sig: row.try_get("problem_sig").unwrap_or_default(),
                solution: row.try_get("solution").unwrap_or_default(),
                solution_lang: row.try_get("solution_lang").unwrap_or(None),
                context_file: row.try_get("context_file").unwrap_or(None),
                failed_attempts: row
                    .try_get::<Vec<String>, _>("failed_attempts")
                    .unwrap_or_default(),
                success_count: row.try_get("success_count").unwrap_or(1),
                last_used_at: row.try_get("last_used_at").unwrap_or_default(),
            };
            let distance: f64 = row.try_get("distance").unwrap_or(1.0);
            (sol, distance)
        })
        .collect()
}

/// Insert a new scholar solution. Returns true on success.
pub async fn insert_scholar(
    pool: &PgPool,
    problem_sig: &str,
    problem_embed: Option<&[f32]>,
    solution: &str,
    solution_lang: Option<&str>,
    context_file: Option<&str>,
) -> bool {
    let id = uuid::Uuid::new_v4().to_string();
    if let Some(emb) = problem_embed {
        let embedding_str = vec_to_pgvector(emb);
        sqlx::query(
            "INSERT INTO scholar_solutions (id, problem_sig, problem_embed, solution, solution_lang, context_file)
             VALUES ($1::uuid, $2, $3::vector, $4, $5, $6)",
        )
        .bind(&id)
        .bind(problem_sig)
        .bind(&embedding_str)
        .bind(solution)
        .bind(solution_lang)
        .bind(context_file)
        .execute(pool)
        .await
        .is_ok()
    } else {
        sqlx::query(
            "INSERT INTO scholar_solutions (id, problem_sig, solution, solution_lang, context_file)
             VALUES ($1::uuid, $2, $3, $4, $5)",
        )
        .bind(&id)
        .bind(problem_sig)
        .bind(solution)
        .bind(solution_lang)
        .bind(context_file)
        .execute(pool)
        .await
        .is_ok()
    }
}

/// Bump `success_count` and update `last_used_at` for a scholar solution.
pub async fn increment_scholar_success(pool: &PgPool, id: &str) {
    let _ = sqlx::query(
        "UPDATE scholar_solutions
         SET success_count = success_count + 1, last_used_at = now()
         WHERE id = $1::uuid",
    )
    .bind(id)
    .execute(pool)
    .await;
}

/// Append a failed attempt description to a scholar solution.
pub async fn add_scholar_failed_attempt(pool: &PgPool, id: &str, attempt: &str) {
    let _ = sqlx::query(
        "UPDATE scholar_solutions
         SET failed_attempts = array_append(failed_attempts, $2)
         WHERE id = $1::uuid",
    )
    .bind(id)
    .bind(attempt)
    .execute(pool)
    .await;
}

// ---------------------------------------------------------------------------
// Response cache model (Phase F1 — token recycling)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct CachedResponse {
    pub id: String,
    pub query_text: String,
    pub response_text: String,
    pub hit_count: i32,
}

/// Semantic search on `response_cache` by cosine similarity on `query_embed`.
pub async fn search_response_cache(
    pool: &PgPool,
    embedding: &[f32],
    limit: i64,
) -> Vec<(CachedResponse, f64)> {
    let embedding_str = vec_to_pgvector(embedding);
    let rows = sqlx::query(
        "SELECT id::text, query_text, response_text, hit_count,
                (query_embed <=> $1::vector) AS distance
         FROM response_cache
         WHERE query_embed IS NOT NULL
         ORDER BY query_embed <=> $1::vector
         LIMIT $2",
    )
    .bind(&embedding_str)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter()
        .map(|row| {
            let cached = CachedResponse {
                id: row.try_get("id").unwrap_or_default(),
                query_text: row.try_get("query_text").unwrap_or_default(),
                response_text: row.try_get("response_text").unwrap_or_default(),
                hit_count: row.try_get("hit_count").unwrap_or(0),
            };
            let distance: f64 = row.try_get("distance").unwrap_or(1.0);
            (cached, distance)
        })
        .collect()
}

/// Insert a new response cache entry. Returns true on success.
pub async fn insert_response_cache(
    pool: &PgPool,
    query_text: &str,
    query_embed: Option<&[f32]>,
    response_text: &str,
) -> bool {
    let id = uuid::Uuid::new_v4().to_string();
    if let Some(emb) = query_embed {
        let embedding_str = vec_to_pgvector(emb);
        sqlx::query(
            "INSERT INTO response_cache (id, query_text, query_embed, response_text)
             VALUES ($1::uuid, $2, $3::vector, $4)",
        )
        .bind(&id)
        .bind(query_text)
        .bind(&embedding_str)
        .bind(response_text)
        .execute(pool)
        .await
        .is_ok()
    } else {
        sqlx::query(
            "INSERT INTO response_cache (id, query_text, response_text)
             VALUES ($1::uuid, $2, $3)",
        )
        .bind(&id)
        .bind(query_text)
        .bind(response_text)
        .execute(pool)
        .await
        .is_ok()
    }
}

/// Bump `hit_count` and update `last_hit_at` for a cached response.
pub async fn increment_cache_hit(pool: &PgPool, id: &str) {
    let _ = sqlx::query(
        "UPDATE response_cache
         SET hit_count = hit_count + 1, last_hit_at = now()
         WHERE id = $1::uuid",
    )
    .bind(id)
    .execute(pool)
    .await;
}

/// Delete stale response cache entries: older than 90 days with fewer than 3 hits.
pub async fn cleanup_response_cache(pool: &PgPool) {
    let result = sqlx::query(
        "DELETE FROM response_cache
         WHERE created_at < now() - interval '90 days'
           AND hit_count < 3",
    )
    .execute(pool)
    .await;

    if let Ok(r) = result {
        let n = r.rows_affected();
        if n > 0 {
            eprintln!("[ghost db] cleaned up {n} stale response cache entries");
        }
    }
}

// ---------------------------------------------------------------------------
// Bible Agent models (Phase B0)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct BibleVerse {
    pub id: String,
    pub book: String,
    pub chapter: i32,
    pub verse: i32,
    pub book_order: i32,
    pub testament: String,
    pub original_lang: String,
    pub original_text: String,
    pub kjv_text: String,
    pub web_text: Option<String>,
    pub strongs: Vec<String>,
    pub morphology: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BiblePericope {
    pub id: String,
    pub title: String,
    pub start_book: String,
    pub start_chapter: i32,
    pub start_verse: i32,
    pub end_book: String,
    pub end_chapter: i32,
    pub end_verse: i32,
    pub genre: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BibleCrossRef {
    pub id: String,
    pub source_book: String,
    pub source_chapter: i32,
    pub source_verse: i32,
    pub target_book: String,
    pub target_chapter: i32,
    pub target_verse: i32,
    pub rel_type: String,
    pub source_dataset: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LexiconEntry {
    pub strongs_id: String,
    pub original_word: String,
    pub transliteration: String,
    pub lang: String,
    pub root: Option<String>,
    pub definition: String,
    pub usage_count: i32,
    pub semantic_range: Vec<String>,
}

fn row_to_bible_verse(row: &sqlx::postgres::PgRow) -> BibleVerse {
    BibleVerse {
        id: row.try_get("id").unwrap_or_default(),
        book: row.try_get("book").unwrap_or_default(),
        chapter: row.try_get("chapter").unwrap_or(0),
        verse: row.try_get("verse").unwrap_or(0),
        book_order: row.try_get("book_order").unwrap_or(0),
        testament: row.try_get("testament").unwrap_or_default(),
        original_lang: row.try_get("original_lang").unwrap_or_default(),
        original_text: row.try_get("original_text").unwrap_or_default(),
        kjv_text: row.try_get("kjv_text").unwrap_or_default(),
        web_text: row.try_get("web_text").unwrap_or(None),
        strongs: row.try_get::<Vec<String>, _>("strongs").unwrap_or_default(),
        morphology: row.try_get("morphology").unwrap_or(None),
    }
}

fn row_to_bible_pericope(row: &sqlx::postgres::PgRow) -> BiblePericope {
    BiblePericope {
        id: row.try_get("id").unwrap_or_default(),
        title: row.try_get("title").unwrap_or_default(),
        start_book: row.try_get("start_book").unwrap_or_default(),
        start_chapter: row.try_get("start_chapter").unwrap_or(0),
        start_verse: row.try_get("start_verse").unwrap_or(0),
        end_book: row.try_get("end_book").unwrap_or_default(),
        end_chapter: row.try_get("end_chapter").unwrap_or(0),
        end_verse: row.try_get("end_verse").unwrap_or(0),
        genre: row.try_get("genre").unwrap_or(None),
        summary: row.try_get("summary").unwrap_or(None),
    }
}

fn row_to_cross_ref(row: &sqlx::postgres::PgRow) -> BibleCrossRef {
    BibleCrossRef {
        id: row.try_get("id").unwrap_or_default(),
        source_book: row.try_get("source_book").unwrap_or_default(),
        source_chapter: row.try_get("source_chapter").unwrap_or(0),
        source_verse: row.try_get("source_verse").unwrap_or(0),
        target_book: row.try_get("target_book").unwrap_or_default(),
        target_chapter: row.try_get("target_chapter").unwrap_or(0),
        target_verse: row.try_get("target_verse").unwrap_or(0),
        rel_type: row.try_get("rel_type").unwrap_or_default(),
        source_dataset: row.try_get("source_dataset").unwrap_or_default(),
    }
}

fn row_to_lexicon_entry(row: &sqlx::postgres::PgRow) -> LexiconEntry {
    LexiconEntry {
        strongs_id: row.try_get("strongs_id").unwrap_or_default(),
        original_word: row.try_get("original_word").unwrap_or_default(),
        transliteration: row.try_get("transliteration").unwrap_or_default(),
        lang: row.try_get("lang").unwrap_or_default(),
        root: row.try_get("root").unwrap_or(None),
        definition: row.try_get("definition").unwrap_or_default(),
        usage_count: row.try_get("usage_count").unwrap_or(0),
        semantic_range: row
            .try_get::<Vec<String>, _>("semantic_range")
            .unwrap_or_default(),
    }
}

/// Semantic search on `bible_verses` by embedding similarity. Returns (verse, distance).
pub async fn search_bible_verses(
    pool: &PgPool,
    embedding: &[f32],
    limit: i64,
) -> Vec<(BibleVerse, f64)> {
    let embedding_str = vec_to_pgvector(embedding);
    let rows = sqlx::query(
        "SELECT id::text, book, chapter, verse, book_order, testament, original_lang,
                original_text, kjv_text, web_text, strongs, morphology,
                (embedding <=> $1::vector) AS distance
         FROM bible_verses
         WHERE embedding IS NOT NULL
         ORDER BY embedding <=> $1::vector
         LIMIT $2",
    )
    .bind(&embedding_str)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter()
        .map(|row| {
            let v = row_to_bible_verse(row);
            let distance: f64 = row.try_get("distance").unwrap_or(1.0);
            (v, distance)
        })
        .collect()
}

/// Semantic search on `bible_pericopes` by embedding similarity.
pub async fn search_bible_pericopes(
    pool: &PgPool,
    embedding: &[f32],
    limit: i64,
) -> Vec<(BiblePericope, f64)> {
    let embedding_str = vec_to_pgvector(embedding);
    let rows = sqlx::query(
        "SELECT id::text, title, start_book, start_chapter, start_verse,
                end_book, end_chapter, end_verse, genre, summary,
                (embedding <=> $1::vector) AS distance
         FROM bible_pericopes
         WHERE embedding IS NOT NULL
         ORDER BY embedding <=> $1::vector
         LIMIT $2",
    )
    .bind(&embedding_str)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter()
        .map(|row| {
            let p = row_to_bible_pericope(row);
            let distance: f64 = row.try_get("distance").unwrap_or(1.0);
            (p, distance)
        })
        .collect()
}

/// Fetch a single verse by book/chapter/verse.
pub async fn get_bible_verse(
    pool: &PgPool,
    book: &str,
    chapter: i32,
    verse: i32,
) -> Option<BibleVerse> {
    let row = sqlx::query(
        "SELECT id::text, book, chapter, verse, book_order, testament, original_lang,
                original_text, kjv_text, web_text, strongs, morphology
         FROM bible_verses
         WHERE book = $1 AND chapter = $2 AND verse = $3",
    )
    .bind(book)
    .bind(chapter)
    .bind(verse)
    .fetch_optional(pool)
    .await
    .ok()??;

    Some(row_to_bible_verse(&row))
}

/// Fetch a range of verses (e.g., Romans 8:28-30). Ordered by chapter, verse.
pub async fn get_bible_verse_range(
    pool: &PgPool,
    book: &str,
    start_chapter: i32,
    start_verse: i32,
    end_chapter: i32,
    end_verse: i32,
) -> Vec<BibleVerse> {
    let rows = if start_chapter == end_chapter {
        sqlx::query(
            "SELECT id::text, book, chapter, verse, book_order, testament, original_lang,
                    original_text, kjv_text, web_text, strongs, morphology
             FROM bible_verses
             WHERE book = $1 AND chapter = $2 AND verse >= $3 AND verse <= $4
             ORDER BY chapter, verse",
        )
        .bind(book)
        .bind(start_chapter)
        .bind(start_verse)
        .bind(end_verse)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
    } else {
        sqlx::query(
            "SELECT id::text, book, chapter, verse, book_order, testament, original_lang,
                    original_text, kjv_text, web_text, strongs, morphology
             FROM bible_verses
             WHERE book = $1
               AND ((chapter = $2 AND verse >= $3)
                    OR (chapter > $2 AND chapter < $4)
                    OR (chapter = $4 AND verse <= $5))
             ORDER BY chapter, verse",
        )
        .bind(book)
        .bind(start_chapter)
        .bind(start_verse)
        .bind(end_chapter)
        .bind(end_verse)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
    };

    rows.iter().map(row_to_bible_verse).collect()
}

/// Get all cross-references FROM a given verse (1-hop outbound).
pub async fn get_cross_refs_from(
    pool: &PgPool,
    book: &str,
    chapter: i32,
    verse: i32,
) -> Vec<BibleCrossRef> {
    let rows = sqlx::query(
        "SELECT id::text, source_book, source_chapter, source_verse,
                target_book, target_chapter, target_verse, rel_type, source_dataset
         FROM bible_cross_refs
         WHERE source_book = $1 AND source_chapter = $2 AND source_verse = $3",
    )
    .bind(book)
    .bind(chapter)
    .bind(verse)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter().map(row_to_cross_ref).collect()
}

/// Get all cross-references TO a given verse (1-hop inbound).
pub async fn get_cross_refs_to(
    pool: &PgPool,
    book: &str,
    chapter: i32,
    verse: i32,
) -> Vec<BibleCrossRef> {
    let rows = sqlx::query(
        "SELECT id::text, source_book, source_chapter, source_verse,
                target_book, target_chapter, target_verse, rel_type, source_dataset
         FROM bible_cross_refs
         WHERE target_book = $1 AND target_chapter = $2 AND target_verse = $3",
    )
    .bind(book)
    .bind(chapter)
    .bind(verse)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter().map(row_to_cross_ref).collect()
}

/// Look up a Strong's number (e.g., "H1254" or "G26").
pub async fn get_lexicon_entry(pool: &PgPool, strongs_id: &str) -> Option<LexiconEntry> {
    let row = sqlx::query(
        "SELECT strongs_id, original_word, transliteration, lang, root,
                definition, usage_count, semantic_range
         FROM bible_lexicon
         WHERE strongs_id = $1",
    )
    .bind(strongs_id)
    .fetch_optional(pool)
    .await
    .ok()??;

    Some(row_to_lexicon_entry(&row))
}

/// Find all verses containing a specific Strong's number.
pub async fn search_verses_by_strongs(
    pool: &PgPool,
    strongs_id: &str,
    limit: i64,
) -> Vec<BibleVerse> {
    let rows = sqlx::query(
        "SELECT id::text, book, chapter, verse, book_order, testament, original_lang,
                original_text, kjv_text, web_text, strongs, morphology
         FROM bible_verses
         WHERE $1 = ANY(strongs)
         ORDER BY book_order, chapter, verse
         LIMIT $2",
    )
    .bind(strongs_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter().map(row_to_bible_verse).collect()
}

/// Find all lexicon entries sharing the same root.
pub async fn search_lexicon_by_root(pool: &PgPool, root: &str) -> Vec<LexiconEntry> {
    let rows = sqlx::query(
        "SELECT strongs_id, original_word, transliteration, lang, root,
                definition, usage_count, semantic_range
         FROM bible_lexicon
         WHERE root = $1",
    )
    .bind(root)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.iter().map(row_to_lexicon_entry).collect()
}

/// Insert a verse. Returns true on success. Skips if (book, chapter, verse) already exists.
#[allow(clippy::too_many_arguments)]
pub async fn insert_bible_verse(
    pool: &PgPool,
    book: &str,
    chapter: i32,
    verse: i32,
    book_order: i32,
    testament: &str,
    original_lang: &str,
    original_text: &str,
    kjv_text: &str,
    web_text: Option<&str>,
    strongs: &[String],
    morphology: Option<&serde_json::Value>,
    embedding: Option<&[f32]>,
    pericope_id: Option<&str>,
) -> bool {
    let id = uuid::Uuid::new_v4().to_string();
    if let Some(emb) = embedding {
        let embedding_str = vec_to_pgvector(emb);
        sqlx::query(
            "INSERT INTO bible_verses (id, book, chapter, verse, book_order, testament,
                    original_lang, original_text, kjv_text, web_text, strongs, morphology,
                    embedding, pericope_id)
             VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13::vector, $14::uuid)
             ON CONFLICT (book, chapter, verse) DO NOTHING",
        )
        .bind(&id)
        .bind(book)
        .bind(chapter)
        .bind(verse)
        .bind(book_order)
        .bind(testament)
        .bind(original_lang)
        .bind(original_text)
        .bind(kjv_text)
        .bind(web_text)
        .bind(strongs)
        .bind(morphology)
        .bind(&embedding_str)
        .bind(pericope_id)
        .execute(pool)
        .await
        .is_ok()
    } else {
        sqlx::query(
            "INSERT INTO bible_verses (id, book, chapter, verse, book_order, testament,
                    original_lang, original_text, kjv_text, web_text, strongs, morphology,
                    pericope_id)
             VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13::uuid)
             ON CONFLICT (book, chapter, verse) DO NOTHING",
        )
        .bind(&id)
        .bind(book)
        .bind(chapter)
        .bind(verse)
        .bind(book_order)
        .bind(testament)
        .bind(original_lang)
        .bind(original_text)
        .bind(kjv_text)
        .bind(web_text)
        .bind(strongs)
        .bind(morphology)
        .bind(pericope_id)
        .execute(pool)
        .await
        .is_ok()
    }
}

/// Insert a pericope. Returns the generated UUID.
#[allow(clippy::too_many_arguments)]
pub async fn insert_bible_pericope(
    pool: &PgPool,
    title: &str,
    start_book: &str,
    start_chapter: i32,
    start_verse: i32,
    end_book: &str,
    end_chapter: i32,
    end_verse: i32,
    genre: Option<&str>,
    summary: Option<&str>,
    embedding: Option<&[f32]>,
) -> Option<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let result = if let Some(emb) = embedding {
        let embedding_str = vec_to_pgvector(emb);
        sqlx::query(
            "INSERT INTO bible_pericopes (id, title, start_book, start_chapter, start_verse,
                    end_book, end_chapter, end_verse, genre, summary, embedding)
             VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11::vector)",
        )
        .bind(&id)
        .bind(title)
        .bind(start_book)
        .bind(start_chapter)
        .bind(start_verse)
        .bind(end_book)
        .bind(end_chapter)
        .bind(end_verse)
        .bind(genre)
        .bind(summary)
        .bind(&embedding_str)
        .execute(pool)
        .await
    } else {
        sqlx::query(
            "INSERT INTO bible_pericopes (id, title, start_book, start_chapter, start_verse,
                    end_book, end_chapter, end_verse, genre, summary)
             VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&id)
        .bind(title)
        .bind(start_book)
        .bind(start_chapter)
        .bind(start_verse)
        .bind(end_book)
        .bind(end_chapter)
        .bind(end_verse)
        .bind(genre)
        .bind(summary)
        .execute(pool)
        .await
    };

    match result {
        Ok(_) => Some(id),
        Err(e) => {
            eprintln!("[ghost db] insert_bible_pericope failed: {e}");
            None
        }
    }
}

/// Insert a cross-reference. Skips duplicates silently.
#[allow(clippy::too_many_arguments)]
pub async fn insert_bible_cross_ref(
    pool: &PgPool,
    source_book: &str,
    source_chapter: i32,
    source_verse: i32,
    target_book: &str,
    target_chapter: i32,
    target_verse: i32,
    rel_type: &str,
    source_dataset: &str,
) -> bool {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO bible_cross_refs (id, source_book, source_chapter, source_verse,
                target_book, target_chapter, target_verse, rel_type, source_dataset)
         VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(&id)
    .bind(source_book)
    .bind(source_chapter)
    .bind(source_verse)
    .bind(target_book)
    .bind(target_chapter)
    .bind(target_verse)
    .bind(rel_type)
    .bind(source_dataset)
    .execute(pool)
    .await
    .is_ok()
}

/// Insert a lexicon entry. Upserts on `strongs_id`.
#[allow(clippy::too_many_arguments)]
pub async fn insert_lexicon_entry(
    pool: &PgPool,
    strongs_id: &str,
    original_word: &str,
    transliteration: &str,
    lang: &str,
    root: Option<&str>,
    definition: &str,
    usage_count: i32,
    semantic_range: &[String],
) -> bool {
    sqlx::query(
        "INSERT INTO bible_lexicon (strongs_id, original_word, transliteration, lang, root,
                definition, usage_count, semantic_range)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (strongs_id) DO UPDATE
         SET definition = EXCLUDED.definition,
             usage_count = EXCLUDED.usage_count,
             semantic_range = EXCLUDED.semantic_range",
    )
    .bind(strongs_id)
    .bind(original_word)
    .bind(transliteration)
    .bind(lang)
    .bind(root)
    .bind(definition)
    .bind(usage_count)
    .bind(semantic_range)
    .execute(pool)
    .await
    .is_ok()
}

/// Return counts of all Bible tables for health/status display.
pub async fn bible_stats(pool: &PgPool) -> (i64, i64, i64, i64) {
    let verse_count: i64 = sqlx::query("SELECT COUNT(*) AS cnt FROM bible_verses")
        .fetch_one(pool)
        .await
        .map(|row| row.try_get("cnt").unwrap_or(0))
        .unwrap_or(0);

    let pericope_count: i64 = sqlx::query("SELECT COUNT(*) AS cnt FROM bible_pericopes")
        .fetch_one(pool)
        .await
        .map(|row| row.try_get("cnt").unwrap_or(0))
        .unwrap_or(0);

    let crossref_count: i64 = sqlx::query("SELECT COUNT(*) AS cnt FROM bible_cross_refs")
        .fetch_one(pool)
        .await
        .map(|row| row.try_get("cnt").unwrap_or(0))
        .unwrap_or(0);

    let lexicon_count: i64 = sqlx::query("SELECT COUNT(*) AS cnt FROM bible_lexicon")
        .fetch_one(pool)
        .await
        .map(|row| row.try_get("cnt").unwrap_or(0))
        .unwrap_or(0);

    (verse_count, pericope_count, crossref_count, lexicon_count)
}

// ---------------------------------------------------------------------------
// Circuit breaker helpers
// ---------------------------------------------------------------------------

/// Mark primary model unhealthy (called on 429/402/500/timeout from Director).
pub async fn set_primary_unhealthy(pool: &PgPool) {
    let _ = sqlx::query(
        "UPDATE director_config SET primary_healthy = false, updated_at = now() WHERE id = 1",
    )
    .execute(pool)
    .await;
}

/// Mark fallback model unhealthy.
pub async fn set_fallback_unhealthy(pool: &PgPool) {
    let _ = sqlx::query(
        "UPDATE director_config SET fallback_healthy = false, updated_at = now() WHERE id = 1",
    )
    .execute(pool)
    .await;
}

/// Reset both health flags (called by background health-check task every 5 min).
pub async fn reset_health_flags(pool: &PgPool) {
    let _ = sqlx::query(
        "UPDATE director_config
         SET primary_healthy = true, fallback_healthy = true,
             last_health_check = now(), updated_at = now()
         WHERE id = 1",
    )
    .execute(pool)
    .await;
}
