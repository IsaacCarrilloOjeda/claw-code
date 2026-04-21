//! Approval module (Wave 2).
//!
//! Recognises `y` / `yes` / `y <token>` approval responses over SMS, finds the
//! pending job they refer to, and marks it approved. This wave contains the
//! logic only — no daemon wiring yet. Wave 3 will route inbound SMS through
//! here once real agents start requesting approval.

use sqlx::PgPool;

use crate::db;

/// What kind of approval message this is, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalKind {
    /// Plain "y" / "yes" — resolves the single most recent pending job for the sender.
    Plain,
    /// "y <token>" — resolves the specific pending job by token.
    Tokened(String),
}

/// Returns `Some(ApprovalKind)` if the message is an approval response, `None`
/// otherwise. Accepts `y`, `Y`, `yes`, `Yes`, `YES`, `y <token>`, with any
/// surrounding whitespace. Suffix after `y`/`yes` (if non-empty after trimming)
/// is treated as a token — the caller decides whether it matches any real job.
pub fn is_approval_message(text: &str) -> Option<ApprovalKind> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();

    let mut parts = lower.splitn(2, char::is_whitespace);
    let head = parts.next()?;
    let rest = parts.next().map(str::trim).filter(|s| !s.is_empty());

    if head != "y" && head != "yes" {
        return None;
    }

    match rest {
        None => Some(ApprovalKind::Plain),
        Some(token) => Some(ApprovalKind::Tokened(token.to_string())),
    }
}

/// A pending job awaiting approval — the minimum fields the caller needs.
#[derive(Debug, Clone)]
pub struct PendingJob {
    pub id: uuid::Uuid,
    pub input: String,
    pub agent: String,
    pub confirmation_token: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::PendingJobRow> for PendingJob {
    fn from(row: db::PendingJobRow) -> Self {
        Self {
            id: row.id,
            input: row.input,
            agent: row.agent,
            confirmation_token: row.confirmation_token,
            created_at: row.created_at,
        }
    }
}

/// Find the most recent pending job for this phone number. `None` if no
/// pending jobs exist for the contact.
pub async fn find_pending_for_contact(
    pool: &PgPool,
    phone: &str,
) -> Result<Option<PendingJob>, sqlx::Error> {
    Ok(db::find_pending_job_by_phone(pool, phone)
        .await?
        .map(PendingJob::from))
}

/// Resolve a pending job by its confirmation token, scoped to the phone so one
/// contact's approval can't resolve another's job.
pub async fn resolve_by_token(
    pool: &PgPool,
    phone: &str,
    token: &str,
) -> Result<Option<PendingJob>, sqlx::Error> {
    Ok(db::find_pending_job_by_token(pool, phone, token)
        .await?
        .map(PendingJob::from))
}

/// Mark a job approved: `status = 'done'`, `completed_at = now()`. Returns
/// `Err(sqlx::Error::RowNotFound)` if the job isn't in `waiting_confirmation`
/// status (defence against double-approve / stale-token races).
pub async fn mark_job_approved(pool: &PgPool, job_id: uuid::Uuid) -> Result<(), sqlx::Error> {
    let affected = db::mark_waiting_job_done(pool, job_id).await?;
    if affected == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lowercase_y() {
        assert_eq!(is_approval_message("y"), Some(ApprovalKind::Plain));
    }

    #[test]
    fn plain_uppercase_y() {
        assert_eq!(is_approval_message("Y"), Some(ApprovalKind::Plain));
    }

    #[test]
    fn plain_yes() {
        assert_eq!(is_approval_message("yes"), Some(ApprovalKind::Plain));
    }

    #[test]
    fn plain_yes_padded() {
        assert_eq!(is_approval_message("  yes  "), Some(ApprovalKind::Plain));
    }

    #[test]
    fn tokened_y() {
        assert_eq!(
            is_approval_message("y abc123"),
            Some(ApprovalKind::Tokened("abc123".to_string()))
        );
    }

    #[test]
    fn tokened_yes() {
        assert_eq!(
            is_approval_message("yes abc123"),
            Some(ApprovalKind::Tokened("abc123".to_string()))
        );
    }

    #[test]
    fn yeah_is_not_approval() {
        assert_eq!(is_approval_message("yeah"), None);
    }

    #[test]
    fn yes_please_is_tokened() {
        // Intentional: "please" is treated as a token; the caller's lookup
        // decides whether it matches any real job.
        assert_eq!(
            is_approval_message("yes please"),
            Some(ApprovalKind::Tokened("please".to_string()))
        );
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(is_approval_message(""), None);
        assert_eq!(is_approval_message("   "), None);
    }

    #[test]
    fn no_is_not_approval() {
        assert_eq!(is_approval_message("no"), None);
    }

    #[test]
    fn mixed_case_yes_token() {
        assert_eq!(
            is_approval_message("Yes ABC"),
            Some(ApprovalKind::Tokened("abc".to_string()))
        );
    }

    #[test]
    fn extra_whitespace_between_head_and_token() {
        assert_eq!(
            is_approval_message("y   abc"),
            Some(ApprovalKind::Tokened("abc".to_string()))
        );
    }
}
