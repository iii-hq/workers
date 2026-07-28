//! `timer` — the one-shot deadline as a first-class trigger type.
//!
//! Every live run that needed "tell me at T that it didn't happen" reached
//! for cron and fumbled it: boundary expressions fire early (`0 */10` is
//! 0–10 minutes away, whatever the clock says), the recurring default turns a
//! deadline into a forever-loop, and discovery run 3 re-rolled the same
//! unbounded cron three times rather than apply `once: true`. A deadline is
//! not a schedule; it deserves its own primitive.
//!
//! The provider lives in the harness worker but registers the type with the
//! ENGINE, so it appears in `engine::triggers::list` for discovery-blind
//! agents and rides the engine's registration replay: on a harness restart
//! the engine re-sends every live registration and the timers re-arm at the
//! SAME absolute instant — which is why the harness's registration intercept
//! resolves `{ in_ms }` to `{ at }` before the engine ever stores it. A
//! deadline that passed while the worker was down fires immediately on
//! replay; the delivery hop's claim makes any double-fire a no-op.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::types::message::AgentMessage;

pub const TIMER_TYPE: &str = "timer";
pub const TIMER_DESC: &str = "One-shot deadline: fires exactly once at `at` (epoch ms). Register \
    with { \"in_ms\": <relative ms> } — resolved to an absolute `at` at registration — or \
    { \"at\": <epoch ms> }. The natural second leg of any armed wake or fan-in gate: 'wake me \
    when X happens, or tell me at T that it did not'. Fires once and retires; for recurrence \
    use `cron`.";

/// Registered timers stay armable for at most this far out — far beyond any
/// deadline, and short enough that an epoch-SECONDS timestamp (off by 1000×)
/// cannot masquerade as a valid future instant.
pub const MAX_TIMER_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// The engine-stored config. Raw engine-side registrants may still send
/// `in_ms`; it resolves at (re)registration — durable across restarts only
/// as `at`, which is what the harness intercept always produces.
#[derive(Debug, Deserialize)]
pub struct TimerTriggerConfig {
    #[serde(default)]
    pub at: Option<i64>,
    #[serde(default)]
    pub in_ms: Option<i64>,
}

struct Armed {
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for Armed {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Armed timers, keyed by engine trigger-instance id. Re-registration on the
/// same id replaces (idempotent under the engine's replay); unregistration
/// aborts the pending task.
#[derive(Clone)]
pub struct TimerBus {
    iii: Arc<IIIClient>,
    dispatch_timeout_ms: u64,
    armed: Arc<Mutex<HashMap<String, Armed>>>,
}

impl TimerBus {
    pub fn new(iii: Arc<IIIClient>, dispatch_timeout_ms: u64) -> Self {
        Self {
            iii,
            dispatch_timeout_ms,
            armed: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn arm(&self, id: String, function_id: String, metadata: Option<Value>, at: i64) {
        let bus = self.clone();
        let task_id = id.clone();
        let handle = tokio::spawn(async move {
            let now = AgentMessage::now_ms();
            let wait = (at - now).max(0) as u64;
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            bus.fire(&task_id, &function_id, metadata, at).await;
            bus.armed
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&task_id);
        });
        // Replacing an existing entry drops it, which aborts the old task.
        self.armed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, Armed { handle });
    }

    pub fn cancel(&self, id: &str) {
        self.armed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }

    pub fn armed_count(&self) -> usize {
        self.armed.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// One fire, then done. Awaited (not void) so a failed dispatch is loggable
    /// with its reason; the stored metadata rides along — for harness-managed
    /// bindings it is the `__binding` pointer the delivery hop resolves.
    async fn fire(&self, id: &str, function_id: &str, metadata: Option<Value>, at: i64) {
        let event = json!({
            "trigger": "timer",
            "scheduled_at": at,
            "actual_at": AgentMessage::now_ms(),
        });
        let request = TriggerRequest {
            function_id: function_id.to_string(),
            payload: event,
            action: None,
            timeout_ms: Some(self.dispatch_timeout_ms),
        };
        let res = match metadata {
            Some(m) => self.iii.trigger(request.metadata(m)).await,
            None => self.iii.trigger(request).await,
        };
        if let Err(e) = res {
            tracing::warn!(timer = %id, function_id, error = %e, "timer dispatch failed");
        } else {
            tracing::info!(timer = %id, function_id, "timer fired");
        }
    }
}

/// When this registration should fire, from a raw engine-side config. The
/// harness intercept validates strictly for agents; the provider stays
/// LENIENT — a replayed registration whose `at` already passed must fire now,
/// not error into a parked binding.
pub fn resolve_fire_at(cfg: &TimerTriggerConfig, now_ms: i64) -> Result<i64, String> {
    match (cfg.at, cfg.in_ms) {
        (Some(at), None) => Ok(at),
        (None, Some(in_ms)) if in_ms > 0 => Ok(now_ms + in_ms),
        (None, Some(_)) => Err("`in_ms` must be positive".into()),
        (None, None) => Err("timer config needs `at` (epoch ms) or `in_ms`".into()),
        (Some(_), Some(_)) => Err("pass `at` OR `in_ms`, not both".into()),
    }
}

pub struct TimerHandler {
    pub bus: TimerBus,
}

fn config_error(message: String) -> Error {
    Error::Handler(json!({ "code": "CONFIG_ERROR", "message": message }).to_string())
}

#[async_trait]
impl TriggerHandler for TimerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let cfg: TimerTriggerConfig = serde_json::from_value(config.config.clone())
            .map_err(|e| config_error(format!("timer config: {e}")))?;
        let at = resolve_fire_at(&cfg, AgentMessage::now_ms()).map_err(config_error)?;
        self.bus.arm(
            config.id.clone(),
            config.function_id.clone(),
            config.metadata.clone(),
            at,
        );
        tracing::info!(instance = %config.id, function = %config.function_id, at, "timer armed");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.bus.cancel(&config.id);
        tracing::info!(instance = %config.id, "timer unregistered");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_at_resolves_relative_and_absolute() {
        let now = 1_000_000;
        let rel = TimerTriggerConfig {
            at: None,
            in_ms: Some(600_000),
        };
        assert_eq!(resolve_fire_at(&rel, now).unwrap(), 1_600_000);
        let abs = TimerTriggerConfig {
            at: Some(2_000_000),
            in_ms: None,
        };
        assert_eq!(resolve_fire_at(&abs, now).unwrap(), 2_000_000);
        // The provider is lenient about the past — a replay after downtime
        // must fire immediately, not error into a parked binding.
        let past = TimerTriggerConfig {
            at: Some(1),
            in_ms: None,
        };
        assert_eq!(resolve_fire_at(&past, now).unwrap(), 1);
    }

    #[test]
    fn fire_at_refuses_ambiguous_and_empty_configs() {
        let both = TimerTriggerConfig {
            at: Some(1),
            in_ms: Some(1),
        };
        assert!(resolve_fire_at(&both, 0).unwrap_err().contains("not both"));
        let neither = TimerTriggerConfig {
            at: None,
            in_ms: None,
        };
        assert!(resolve_fire_at(&neither, 0).unwrap_err().contains("`at`"));
        let negative = TimerTriggerConfig {
            at: None,
            in_ms: Some(-5),
        };
        assert!(resolve_fire_at(&negative, 0).is_err());
    }

    #[tokio::test]
    async fn cancel_aborts_an_armed_timer() {
        // No engine: the task would only ever reach the dispatch after its
        // sleep; cancelling within the sleep window proves the abort path.
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:0"));
        let bus = TimerBus::new(iii, 1_000);
        bus.arm(
            "t1".into(),
            "noop::fn".into(),
            None,
            AgentMessage::now_ms() + 60_000,
        );
        assert_eq!(bus.armed_count(), 1);
        bus.cancel("t1");
        assert_eq!(bus.armed_count(), 0);
        // Re-arming the same id replaces rather than duplicating.
        bus.arm(
            "t2".into(),
            "noop::fn".into(),
            None,
            AgentMessage::now_ms() + 60_000,
        );
        bus.arm(
            "t2".into(),
            "noop::fn".into(),
            None,
            AgentMessage::now_ms() + 60_000,
        );
        assert_eq!(bus.armed_count(), 1);
    }
}
