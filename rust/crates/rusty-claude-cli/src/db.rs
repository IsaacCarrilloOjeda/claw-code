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
