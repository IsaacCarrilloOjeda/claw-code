//! Shared HTTP client for GHOST.
//!
//! A single `reqwest::Client` is lazily initialized and reused across all
//! modules. This avoids per-request TLS handshakes and enables HTTP connection
//! pooling.
//!
//! For endpoints that need shorter timeouts (embeddings, search, SMS), use
//! `reqwest::RequestBuilder::timeout()` on the individual request rather than
//! creating a new client.

use std::sync::OnceLock;
use std::time::Duration;

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

/// Default timeout applied to all requests. Individual call sites can override
/// with `RequestBuilder::timeout()`.
const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Return a reference to the global HTTP client. Initialized on first call.
pub fn shared_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .pool_max_idle_per_host(4)
            .build()
            .expect("failed to build shared HTTP client")
    })
}
