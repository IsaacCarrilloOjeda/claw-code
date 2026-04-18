//! SMS send helper.
//!
//! Primary path: Android SMS Gateway (`GHOST_SMS_GATEWAY_URL`).
//! Fallback: Twilio (`TWILIO_ACCOUNT_SID`, `TWILIO_AUTH_TOKEN`, `TWILIO_FROM_NUMBER`).
//!
//! Response length rule:
//!   - < 500 chars → send as-is
//!   - ≥ 500 chars → truncate to 497 chars + "..." + newline + job URL

use std::time::Duration;

const SMS_TIMEOUT_SECS: u64 = 10;
const RESPONSE_CHAR_LIMIT: usize = 500;
const TRUNCATE_AT: usize = 497;

/// Send a brief acknowledgment SMS. Best-effort — failures are logged but
/// don't affect the main pipeline.
pub async fn send_ack(to: &str) {
    let body = "Got it -- one sec.";
    if let Err(e) = send_via_gateway(to, body).await {
        if let Err(e2) = send_via_twilio(to, body).await {
            eprintln!("[ghost sms] ack failed (gateway: {e}, twilio: {e2})");
        }
    }
}

/// Format + send a response SMS. Returns `Ok(())` on delivery, `Err(reason)`
/// if both Gateway and Twilio fail.
pub async fn send_response(to: &str, text: &str, job_id: &str) -> Result<(), String> {
    let body = format_body(text, job_id);
    match send_via_gateway(to, &body).await {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("[ghost sms] gateway failed ({e}), trying Twilio");
            send_via_twilio(to, &body).await
        }
    }
}

fn format_body(text: &str, job_id: &str) -> String {
    if text.chars().count() < RESPONSE_CHAR_LIMIT {
        return text.to_string();
    }
    let truncated: String = text.chars().take(TRUNCATE_AT).collect();
    let base_url =
        std::env::var("GHOST_BASE_URL").unwrap_or_else(|_| "https://ghost.app".to_string());
    format!("{truncated}...\n{base_url}/jobs/{job_id}")
}

async fn send_via_gateway(to: &str, body: &str) -> Result<(), String> {
    let url = std::env::var("GHOST_SMS_GATEWAY_URL")
        .map_err(|_| "GHOST_SMS_GATEWAY_URL not set".to_string())?;

    let payload = serde_json::json!({
        "message": body,
        "phoneNumbers": [to],
    });

    let client = crate::http_client::shared_client();

    let mut req = client
        .post(&url)
        .timeout(Duration::from_secs(SMS_TIMEOUT_SECS))
        .json(&payload);

    // Cloud API requires basic auth (GHOST_SMS_GATEWAY_USER / GHOST_SMS_GATEWAY_PASS).
    if let (Ok(user), Ok(pass)) = (
        std::env::var("GHOST_SMS_GATEWAY_USER"),
        std::env::var("GHOST_SMS_GATEWAY_PASS"),
    ) {
        req = req.basic_auth(&user, Some(&pass));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Gateway request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("Gateway returned HTTP {}", resp.status()))
    }
}

async fn send_via_twilio(to: &str, body: &str) -> Result<(), String> {
    let sid = std::env::var("TWILIO_ACCOUNT_SID")
        .map_err(|_| "TWILIO_ACCOUNT_SID not set".to_string())?;
    let auth =
        std::env::var("TWILIO_AUTH_TOKEN").map_err(|_| "TWILIO_AUTH_TOKEN not set".to_string())?;
    let from = std::env::var("TWILIO_FROM_NUMBER")
        .map_err(|_| "TWILIO_FROM_NUMBER not set".to_string())?;

    let url = format!("https://api.twilio.com/2010-04-01/Accounts/{sid}/Messages.json");
    let encoded = format!(
        "To={}&From={}&Body={}",
        percent_encode(to),
        percent_encode(&from),
        percent_encode(body),
    );

    let client = crate::http_client::shared_client();

    let resp = client
        .post(&url)
        .timeout(Duration::from_secs(SMS_TIMEOUT_SECS))
        .basic_auth(&sid, Some(&auth))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(encoded)
        .send()
        .await
        .map_err(|e| format!("Twilio request failed: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("Twilio error {status}: {text}"))
    }
}

/// Percent-encode a string for `application/x-www-form-urlencoded`.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            b' ' => out.push('+'),
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}
