//! OAuth refresh-token storage + access-token exchange.
//!
//! Shared by Calendar (Wave 3), Docs (Wave 4), and Gmail (later). Isaac
//! completes the one-time consent flow manually and stores the `refresh_token`
//! via `save_refresh`. From then on `valid_access_token` hands out a working
//! bearer string, hitting Google's token endpoint only when the current
//! `access_token` is missing or within 60 seconds of expiry.
//!
//! Required env vars (read lazily — not at daemon startup):
//!
//! - `GOOGLE_OAUTH_CLIENT_ID`
//! - `GOOGLE_OAUTH_CLIENT_SECRET`

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sqlx::PgPool;

use crate::db;
use crate::http_client::shared_client;

const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REFRESH_SAFETY_WINDOW_SECS: i64 = 60;

#[derive(Debug, Clone)]
pub struct OAuthToken {
    pub provider: String,
    pub account_label: String,
    pub refresh_token: String,
    pub access_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
}

impl From<db::OAuthTokenRow> for OAuthToken {
    fn from(r: db::OAuthTokenRow) -> Self {
        Self {
            provider: r.provider,
            account_label: r.account_label,
            refresh_token: r.refresh_token,
            access_token: r.access_token,
            expires_at: r.expires_at,
            scopes: r.scopes,
        }
    }
}

/// Fetch the stored token row for a (provider, account). Returns None if
/// nothing has been saved yet.
pub async fn load(
    pool: &PgPool,
    provider: &str,
    account: &str,
) -> Result<Option<OAuthToken>, sqlx::Error> {
    Ok(db::load_oauth_token(pool, provider, account)
        .await?
        .map(OAuthToken::from))
}

/// Upsert a `refresh_token` + its granted scopes. Called once after the
/// one-time OAuth consent flow delivers a `refresh_token` out-of-band. Clears
/// any stale `access_token` so the next call refreshes.
pub async fn save_refresh(
    pool: &PgPool,
    provider: &str,
    account: &str,
    refresh_token: &str,
    scopes: &[String],
) -> Result<(), sqlx::Error> {
    db::upsert_oauth_token(pool, provider, account, refresh_token, scopes).await
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    expires_in: i64,
    #[serde(default)]
    #[allow(dead_code)]
    scope: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    token_type: Option<String>,
}

/// Parse the JSON body returned by Google's token endpoint. Split out so we
/// can unit-test the decoding without touching the network.
fn parse_token_response(body: &str) -> Result<(String, i64), String> {
    let parsed: GoogleTokenResponse = serde_json::from_str(body)
        .map_err(|e| format!("google token response parse error: {e}"))?;
    Ok((parsed.access_token, parsed.expires_in))
}

fn client_credentials() -> Result<(String, String), String> {
    let id = std::env::var("GOOGLE_OAUTH_CLIENT_ID")
        .map_err(|_| "GOOGLE_OAUTH_CLIENT_ID is not set".to_string())?;
    let secret = std::env::var("GOOGLE_OAUTH_CLIENT_SECRET")
        .map_err(|_| "GOOGLE_OAUTH_CLIENT_SECRET is not set".to_string())?;
    if id.is_empty() || secret.is_empty() {
        return Err("GOOGLE_OAUTH_CLIENT_ID/SECRET must be non-empty".into());
    }
    Ok((id, secret))
}

/// Exchange a stored `refresh_token` for a new `access_token` against Google's
/// token endpoint. Writes the result back so `valid_access_token` can reuse
/// it until expiry. Returns the `access_token` string on success.
pub async fn refresh_access(
    pool: &PgPool,
    provider: &str,
    account: &str,
) -> Result<String, String> {
    let row = load(pool, provider, account)
        .await
        .map_err(|e| format!("load oauth token: {e}"))?
        .ok_or_else(|| format!("no stored oauth token for ({provider}, {account})"))?;

    let (client_id, client_secret) = client_credentials()?;

    let form = [
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("refresh_token", row.refresh_token.as_str()),
        ("grant_type", "refresh_token"),
    ];

    let resp = shared_client()
        .post(GOOGLE_TOKEN_URL)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("google token request failed: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("read google token body: {e}"))?;

    if !status.is_success() {
        return Err(format!("google token endpoint returned {status}: {body}"));
    }

    let (access_token, expires_in) = parse_token_response(&body)?;
    let expires_at = Utc::now() + Duration::seconds(expires_in);

    db::update_oauth_access_token(pool, provider, account, &access_token, expires_at)
        .await
        .map_err(|e| format!("persist access token: {e}"))?;

    Ok(access_token)
}

/// Return a valid `access_token`, refreshing it if the stored one is absent
/// or within the safety window of expiring.
pub async fn valid_access_token(
    pool: &PgPool,
    provider: &str,
    account: &str,
) -> Result<String, String> {
    let row = load(pool, provider, account)
        .await
        .map_err(|e| format!("load oauth token: {e}"))?
        .ok_or_else(|| format!("no stored oauth token for ({provider}, {account})"))?;

    let needs_refresh = match (row.access_token.as_ref(), row.expires_at) {
        (Some(token), Some(expiry)) => {
            if token.is_empty() {
                true
            } else {
                let now = Utc::now();
                let cushion = Duration::seconds(REFRESH_SAFETY_WINDOW_SECS);
                expiry <= now + cushion
            }
        }
        _ => true,
    };

    if needs_refresh {
        refresh_access(pool, provider, account).await
    } else {
        Ok(row.access_token.expect("checked above"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_token_response_ok() {
        let body = r#"{
            "access_token": "ya29.a0AfB_abc",
            "expires_in": 3599,
            "scope": "https://www.googleapis.com/auth/calendar",
            "token_type": "Bearer"
        }"#;
        let (tok, expires_in) = parse_token_response(body).expect("parse");
        assert_eq!(tok, "ya29.a0AfB_abc");
        assert_eq!(expires_in, 3599);
    }

    #[test]
    fn parse_token_response_missing_optional_fields() {
        // scope + token_type are optional per our parser.
        let body = r#"{"access_token":"abc","expires_in":60}"#;
        let (tok, expires_in) = parse_token_response(body).expect("parse");
        assert_eq!(tok, "abc");
        assert_eq!(expires_in, 60);
    }

    #[test]
    fn parse_token_response_rejects_garbage() {
        assert!(parse_token_response("not json").is_err());
        assert!(parse_token_response(r#"{"expires_in":60}"#).is_err());
    }
}
