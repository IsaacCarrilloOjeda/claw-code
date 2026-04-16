//! Brave Search API integration for GHOST internet access.
//!
//! Set `BRAVE_API_KEY` to enable. When unset, `web_search` returns `Ok(vec![])` silently.
//!
//! Brave Search API docs: <https://api.search.brave.com/app/documentation/web-search/get-started>
//! Free tier: 2 000 queries / month.

use std::time::Duration;

const BRAVE_SEARCH_URL: &str = "https://api.search.brave.com/res/v1/web/search";
const SEARCH_TIMEOUT_SECS: u64 = 8;
const DEFAULT_RESULT_COUNT: u8 = 5;

pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
}

/// Search the web via Brave Search API.
///
/// Returns up to `DEFAULT_RESULT_COUNT` results, or an empty vec when
/// `BRAVE_API_KEY` is unset. Returns `Err` only on network / parse failure.
pub async fn web_search(query: &str) -> Result<Vec<SearchResult>, String> {
    let api_key = match std::env::var("BRAVE_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => return Ok(vec![]),
    };

    let client = crate::http_client::shared_client();

    let resp = client
        .get(BRAVE_SEARCH_URL)
        .timeout(Duration::from_secs(SEARCH_TIMEOUT_SECS))
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", api_key.trim())
        .query(&[
            ("q", query),
            ("count", &DEFAULT_RESULT_COUNT.to_string()),
            ("safesearch", "moderate"),
        ])
        .send()
        .await
        .map_err(|e| format!("Brave Search request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Brave Search error {status}: {body}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Brave response: {e}"))?;

    let results = json["web"]["results"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let title = item["title"].as_str()?.to_string();
                    let url = item["url"].as_str()?.to_string();
                    let description = item["description"].as_str().unwrap_or("").to_string();
                    Some(SearchResult {
                        title,
                        url,
                        description,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(results)
}

/// Format search results into a compact block for system prompt injection.
pub fn format_results(results: &[SearchResult]) -> String {
    results
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{}. {}\n   {}\n   {}", i + 1, r.title, r.url, r.description))
        .collect::<Vec<_>>()
        .join("\n\n")
}
