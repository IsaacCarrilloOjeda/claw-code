//! Process-wide token-event broadcast channel.
//!
//! The dispatcher publishes a `TokenEvent` after every successful agent call.
//! SSE subscribers at `GET /stream/tokens/:job_id` filter by `job_id` and
//! forward matching events as `data: {json}\n\n` frames. Clients that only
//! care about the final tally can also read `tokens` off the plain JSON
//! response from `/chat` or `/code/chat`.
//!
//! Channel capacity is intentionally small — slow subscribers drop frames
//! rather than stall the hot path. Every event lives for a single broadcast;
//! there is no history buffer.

use std::sync::OnceLock;

use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

const CHANNEL_CAPACITY: usize = 256;

#[derive(Debug, Clone, Serialize)]
pub struct TokenEvent {
    #[serde(serialize_with = "serialize_uuid")]
    pub job_id: Uuid,
    pub agent: String,
    pub tier: String,
    pub input: u32,
    pub output: u32,
    pub cache_read: u32,
    pub cost_cents: i32,
}

fn serialize_uuid<S: serde::Serializer>(uuid: &Uuid, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&uuid.to_string())
}

static SENDER: OnceLock<broadcast::Sender<TokenEvent>> = OnceLock::new();

fn sender() -> &'static broadcast::Sender<TokenEvent> {
    SENDER.get_or_init(|| broadcast::channel(CHANNEL_CAPACITY).0)
}

/// Subscribe to the token-event stream. Each subscriber gets its own
/// lagging-tolerant receiver.
pub fn subscribe() -> broadcast::Receiver<TokenEvent> {
    sender().subscribe()
}

/// Publish a token event. No-op (returns Ok silently) when there are no
/// subscribers — which is the common case.
pub fn publish(event: TokenEvent) {
    // broadcast::send returns Err only when there are no active receivers;
    // the event is a pure fanout, so that's fine to drop.
    let _ = sender().send(event);
}
