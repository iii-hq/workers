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
use std::sync::atomic::{AtomicU64, Ordering};
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

/// Keyed one-shot deadlines: each id holds at most one pending task that
/// sleeps until an absolute epoch-ms instant, then runs its work once.
///
/// The harness's one timer facility — the `timer` trigger provider below and
/// the binding-expiry deadlines (`bindings::expiry`) both arm through it, so
/// nothing in the worker needs a periodic "is anything due?" scan. Re-arming
/// an id replaces (aborts) its pending task; [`OneShots::cancel`] aborts it.
/// A task that reaches its deadline DETACHES itself before running its work,
/// so a cancel issued by that work (expiry deletes the binding, which cancels
/// the binding's timer) never aborts the work mid-flight.
#[derive(Clone, Default)]
pub struct OneShots {
    armed: Arc<Mutex<HashMap<String, Armed>>>,
    next_generation: Arc<AtomicU64>,
}

struct Armed {
    generation: u64,
    abort: tokio::task::AbortHandle,
}

impl OneShots {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `work` once at `at_ms` (epoch ms; a past instant runs now),
    /// replacing whatever this id had pending.
    pub fn arm<F>(&self, id: impl Into<String>, at_ms: i64, work: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let id = id.into();
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let armed = self.armed.clone();
        let task_id = id.clone();
        // Held across spawn + insert: a zero-wait task must not look for its
        // entry before the entry exists.
        let mut map = self.armed.lock().unwrap_or_else(|p| p.into_inner());
        let handle = tokio::spawn(async move {
            let wait = (at_ms - AgentMessage::now_ms()).max(0) as u64;
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            {
                let mut map = armed.lock().unwrap_or_else(|p| p.into_inner());
                if !map
                    .get(&task_id)
                    .is_some_and(|entry| entry.generation == generation)
                {
                    // Replaced or cancelled while waking; the newer arming
                    // (if any) owns this id now.
                    return;
                }
                map.remove(&task_id);
            }
            work.await;
        });
        let replaced = map.insert(
            id,
            Armed {
                generation,
                abort: handle.abort_handle(),
            },
        );
        drop(map);
        if let Some(old) = replaced {
            old.abort.abort();
        }
    }

    /// Abort the id's pending task. `true` when one was pending.
    pub fn cancel(&self, id: &str) -> bool {
        let removed = self
            .armed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
        match removed {
            Some(entry) => {
                entry.abort.abort();
                true
            }
            None => false,
        }
    }

    pub fn is_armed(&self, id: &str) -> bool {
        self.armed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.armed.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Armed timers, keyed by engine trigger-instance id. Re-registration on the
/// same id replaces (idempotent under the engine's replay); unregistration
/// aborts the pending task.
#[derive(Clone)]
pub struct TimerBus {
    iii: Arc<IIIClient>,
    dispatch_timeout_ms: u64,
    timers: OneShots,
}

impl TimerBus {
    pub fn new(iii: Arc<IIIClient>, dispatch_timeout_ms: u64) -> Self {
        Self {
            iii,
            dispatch_timeout_ms,
            timers: OneShots::new(),
        }
    }

    pub fn arm(
        &self,
        id: String,
        function_id: String,
        namespace: Option<String>,
        metadata: Option<Value>,
        at: i64,
    ) {
        let bus = self.clone();
        let task_id = id.clone();
        self.timers.arm(id, at, async move {
            bus.fire(&task_id, &function_id, namespace, metadata, at)
                .await;
        });
    }

    pub fn cancel(&self, id: &str) {
        self.timers.cancel(id);
    }

    pub fn armed_count(&self) -> usize {
        self.timers.len()
    }

    /// One fire, then done. Awaited (not void) so a failed dispatch is loggable
    /// with its reason; the stored metadata rides along — for harness-managed
    /// bindings it is the `__binding` pointer the delivery hop resolves.
    async fn fire(
        &self,
        id: &str,
        function_id: &str,
        namespace: Option<String>,
        metadata: Option<Value>,
        at: i64,
    ) {
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
        let request: iii_sdk::protocol::TriggerRequestWithMetadata = match metadata {
            Some(metadata) => request.metadata(metadata),
            None => request.into(),
        };
        let namespace = namespace.as_deref().unwrap_or("default");
        let res = self.iii.trigger(request.namespace(namespace)).await;
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
            config.namespace.clone(),
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
            None,
            AgentMessage::now_ms() + 60_000,
        );
        bus.arm(
            "t2".into(),
            "noop::fn".into(),
            None,
            None,
            AgentMessage::now_ms() + 60_000,
        );
        assert_eq!(bus.armed_count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_replaced_arming_never_runs_and_the_replacement_does() {
        let timers = OneShots::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let deadline = AgentMessage::now_ms() + 60_000;
        let first = tx.clone();
        timers.arm("same", deadline, async move {
            first.send("first").unwrap();
        });
        let second = tx.clone();
        timers.arm("same", deadline, async move {
            second.send("second").unwrap();
        });
        assert_eq!(
            timers.len(),
            1,
            "re-arming replaces rather than duplicating"
        );
        tokio::time::advance(std::time::Duration::from_millis(60_001)).await;
        assert_eq!(rx.recv().await, Some("second"));
        tokio::task::yield_now().await;
        assert!(rx.try_recv().is_err(), "the replaced arming must never run");
        assert!(timers.is_empty(), "a fired timer detaches itself");
    }

    #[tokio::test(start_paused = true)]
    async fn work_that_cancels_its_own_id_still_runs_to_completion() {
        // Expiry's work deletes the binding, and the delete cancels the
        // binding's timer — which must not abort the work in flight.
        let timers = OneShots::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let inner = timers.clone();
        timers.arm("self", AgentMessage::now_ms(), async move {
            assert!(!inner.cancel("self"), "the task detached before its work");
            tokio::task::yield_now().await;
            tx.send("finished").unwrap();
        });
        assert_eq!(rx.recv().await, Some("finished"));
    }
}
