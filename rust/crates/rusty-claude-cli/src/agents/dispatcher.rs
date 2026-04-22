use super::calendar::Calendar;
use super::chief_of_staff::ChiefOfStaff;
use super::docs::Docs;
use super::dreamer::Dreamer;
use super::intent::{self, Intent};
use super::research::Research;
use super::{Agent, AgentRequest, AgentResponse, ModelTier};
use crate::infra::budget;
use crate::infra::events::{self, NewEvent, Outcome};
use sqlx::PgPool;

pub struct Dispatcher {
    // Wave 3: registered agents live here.
}

impl Dispatcher {
    pub fn new() -> Self {
        Self {}
    }

    /// Single entry point for every inbound message. Classifies the routing
    /// prefix via `intent::classify`, strips it, then delegates to the
    /// appropriate backend (`chat_dispatcher`, `director`, …).
    ///
    /// Wave 3: wraps the call with a per-agent daily budget check and writes
    /// a `ghost_events` row for every outcome (success / refused / error).
    /// Budget / event writes never fail the user's request — they log a
    /// warning via `eprintln!` and continue.
    #[allow(clippy::too_many_lines)]
    pub async fn dispatch(
        &self,
        req: AgentRequest,
        pool: &PgPool,
    ) -> Result<AgentResponse, String> {
        let (intent, stripped) = intent::classify(&req.message);
        let req = AgentRequest {
            message: stripped,
            ..req
        };

        // Ignore short-circuits before any budget / event work.
        if matches!(intent, Intent::Ignore) {
            return Ok(AgentResponse::empty_for(ModelTier::Fast));
        }

        let (agent_name, tier) = match intent {
            Intent::Chat => ("chat_dispatcher", ModelTier::Fast),
            Intent::Director => ("director", ModelTier::Mid),
            Intent::Research => ("research", ModelTier::Fast),
            Intent::Scheduled => ("scheduled", ModelTier::Fast),
            Intent::Calendar => ("calendar", ModelTier::Fast),
            Intent::ChiefOfStaff => ("chief_of_staff", ModelTier::Mid),
            Intent::Docs => ("docs", ModelTier::Fast),
            Intent::Dreamer => ("dreamer", ModelTier::Mid),
            Intent::Ignore => unreachable!("ignore handled above"),
        };

        let job_uuid = parse_uuid(&req.job_id);

        // Budget gate. A blown cap writes a `refused` event and returns 402-ish.
        let status = budget::check(pool, agent_name)
            .await
            .map_err(|e| format!("budget check failed: {e}"))?;
        if status.is_blown() {
            record_event(
                pool,
                NewEvent {
                    job_id: job_uuid,
                    agent: agent_name,
                    tier: tier.as_str(),
                    input: Some(&req.message),
                    output: None,
                    tokens_in: 0,
                    tokens_out: 0,
                    cost_cents: 0,
                    outcome: Outcome::Refused,
                },
            )
            .await;
            return Err(format!(
                "budget cap hit for {agent_name} today ({}¢ of {}¢)",
                status.spent_cents, status.cap_cents
            ));
        }

        // Call the backing handler.
        let call_result = match intent {
            Intent::Chat => {
                crate::chat_dispatcher::dispatch(
                    &req.message,
                    &req.history,
                    &req.job_id,
                    Some(pool),
                    req.sender_phone.as_deref(),
                )
                .await
            }
            Intent::Director => {
                crate::director::sonnet_reply(&req.message, &req.job_id, Some(pool)).await
            }
            Intent::Research => Research::new()
                .handle(req.clone(), pool)
                .await
                .map(|resp| (resp.text, resp.usage)),
            Intent::Scheduled => Err("scheduled task execution not yet implemented".into()),
            Intent::Calendar => Calendar::new()
                .handle(req.clone(), pool)
                .await
                .map(|resp| (resp.text, resp.usage)),
            Intent::ChiefOfStaff => ChiefOfStaff::new()
                .handle(req.clone(), pool)
                .await
                .map(|resp| (resp.text, resp.usage)),
            Intent::Docs => Docs::new()
                .handle(req.clone(), pool)
                .await
                .map(|resp| (resp.text, resp.usage)),
            Intent::Dreamer => Dreamer::new()
                .handle(req.clone(), pool)
                .await
                .map(|resp| (resp.text, resp.usage)),
            Intent::Ignore => unreachable!("handled above"),
        };

        match call_result {
            Ok((text, usage)) => {
                let tokens_in = i64::from(usage.tokens_in);
                let tokens_out = i64::from(usage.tokens_out);
                let cost = budget::cost_cents(tier, tokens_in, tokens_out);

                if let Err(e) = budget::debit(pool, agent_name, tokens_in, tokens_out, cost).await {
                    eprintln!("[ghost budget] debit failed for {agent_name}: {e}");
                }

                record_event(
                    pool,
                    NewEvent {
                        job_id: job_uuid,
                        agent: agent_name,
                        tier: tier.as_str(),
                        input: Some(&req.message),
                        output: Some(&text),
                        tokens_in: clamp_i32(tokens_in),
                        tokens_out: clamp_i32(tokens_out),
                        cost_cents: clamp_i32(cost),
                        outcome: Outcome::Success,
                    },
                )
                .await;

                if let Some(jid) = job_uuid {
                    crate::infra::token_stream::publish(crate::infra::token_stream::TokenEvent {
                        job_id: jid,
                        agent: agent_name.to_string(),
                        tier: tier.as_str().to_string(),
                        input: usage.tokens_in,
                        output: usage.tokens_out,
                        cache_read: 0,
                        cost_cents: clamp_i32(cost),
                    });
                }

                Ok(AgentResponse { text, usage, tier })
            }
            Err(e) => {
                record_event(
                    pool,
                    NewEvent {
                        job_id: job_uuid,
                        agent: agent_name,
                        tier: tier.as_str(),
                        input: Some(&req.message),
                        output: Some(&e),
                        tokens_in: 0,
                        tokens_out: 0,
                        cost_cents: 0,
                        outcome: Outcome::Error,
                    },
                )
                .await;
                Err(e)
            }
        }
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_uuid(s: &str) -> Option<uuid::Uuid> {
    uuid::Uuid::parse_str(s).ok()
}

fn clamp_i32(v: i64) -> i32 {
    i32::try_from(v).unwrap_or(if v > 0 { i32::MAX } else { i32::MIN })
}

async fn record_event(pool: &PgPool, event: NewEvent<'_>) {
    if let Err(e) = events::record(pool, event).await {
        eprintln!("[ghost events] record failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Source;

    fn req(msg: &str) -> AgentRequest {
        AgentRequest {
            message: msg.to_string(),
            history: Vec::new(),
            source: Source::Dashboard,
            job_id: "test-job".to_string(),
            sender_phone: None,
        }
    }

    /// The `.` prefix must short-circuit into an empty-text response without
    /// touching the network or the database. A null/unusable pool proves it.
    #[tokio::test]
    async fn dot_prefix_returns_empty_without_backend() {
        // Build a PgPool that would panic on use. We rely on the Ignore branch
        // returning before any backend is invoked.
        let options: sqlx::postgres::PgPoolOptions = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .test_before_acquire(false);
        // Connect lazily — a real connection is never opened because the
        // Ignore branch doesn't query.
        let pool = options
            .connect_lazy("postgres://invalid:invalid@127.0.0.1:1/invalid")
            .expect("lazy connect should not fail");

        let dispatcher = Dispatcher::new();
        let resp = dispatcher
            .dispatch(req(".silent"), &pool)
            .await
            .expect("ignore branch must succeed");

        assert_eq!(resp.text, "");
        assert_eq!(resp.usage.tokens_in, 0);
        assert_eq!(resp.usage.tokens_out, 0);
    }

    #[test]
    fn clamp_i32_caps_at_bounds() {
        assert_eq!(clamp_i32(0), 0);
        assert_eq!(clamp_i32(i64::from(i32::MAX)), i32::MAX);
        assert_eq!(clamp_i32(i64::from(i32::MAX) + 1), i32::MAX);
        assert_eq!(clamp_i32(i64::from(i32::MIN) - 1), i32::MIN);
    }

    #[test]
    fn parse_uuid_accepts_valid_only() {
        assert!(parse_uuid("not-a-uuid").is_none());
        let valid = uuid::Uuid::new_v4();
        assert_eq!(parse_uuid(&valid.to_string()), Some(valid));
    }
}
