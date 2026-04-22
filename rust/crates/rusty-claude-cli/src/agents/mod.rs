pub mod calendar;
pub mod chief_of_staff;
pub mod dispatcher;
pub mod docs;
pub mod dreamer;
pub mod intent;
pub mod oauth;
pub mod research;
pub mod summarizer;

use async_trait::async_trait;
use sqlx::PgPool;

#[derive(Debug, Clone, Copy)]
pub enum Source {
    Sms,
    Dashboard,
    Scheduled,
}

#[derive(Debug, Clone, Copy)]
pub enum ModelTier {
    Fast,
    Code,
    Mid,
    Full,
}

impl ModelTier {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelTier::Fast => "fast",
            ModelTier::Code => "code",
            ModelTier::Mid => "mid",
            ModelTier::Full => "full",
        }
    }
}

/// Token usage pulled from an LLM response. Populated by the backend
/// (`chat_dispatcher::dispatch`, `director::sonnet_reply`) and used by the
/// dispatcher to cost + debit the per-agent budget.
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub tokens_in: u32,
    pub tokens_out: u32,
}

#[derive(Debug, Clone)]
pub struct AgentRequest {
    pub message: String,
    pub history: Vec<serde_json::Value>,
    pub source: Source,
    pub job_id: String,
    pub sender_phone: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentResponse {
    pub text: String,
    pub usage: Usage,
    pub tier: ModelTier,
}

impl AgentResponse {
    /// Empty response with zero usage — used by the `Ignore` branch and by
    /// tests that don't care about tier.
    pub fn empty_for(tier: ModelTier) -> Self {
        Self {
            text: String::new(),
            usage: Usage::default(),
            tier,
        }
    }
}

#[async_trait]
pub trait Agent: Send + Sync {
    fn name(&self) -> &'static str;
    fn declared_tier(&self) -> ModelTier;
    fn requires_approval(&self, req: &AgentRequest) -> bool;
    async fn handle(&self, req: AgentRequest, pool: &PgPool) -> Result<AgentResponse, String>;
}
