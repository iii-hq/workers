//! `harness::on-timer` — the one-shot deadline as an observable function.
//!
//! Every live run that needed "tell me at T that it didn't happen" reached
//! for cron and fumbled it: boundary expressions fire early (`0 */10` is
//! 0–10 minutes away, whatever the clock says), the recurring default turns a
//! deadline into a forever-loop, and discovery run 3 re-rolled the same
//! unbounded cron three times rather than apply `once: true`. A deadline is
//! not a schedule; it deserves its own primitive.
//!
//! The function is private and its handler returns the event it is called
//! with. Its observers pass args `{ at }` (epoch ms): the added hook arms one
//! one-shot per distinct instant, the removed hook cancels it with that
//! instant's last observer, and the match hook gives each call to the
//! observers of its own instant. Observer args replay unchanged, which is why
//! the `harness::observe` intercept resolves `{ in_ms }` to `{ at }` before it
//! observes. The observers of an instant that already passed (a replay after
//! downtime) share one call, made once the observers of that instant added
//! together are active. An observer of that instant added after the call
//! starts another one, which reaches every observer of the instant still
//! present; the delivery hop's claim makes a second fire of a `once` binding
//! a no-op.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{
    AddedHook, Call, CallAction, CallRequest, IIIClient, Keys, Match, ObserveRejection, Observer,
    RegisterFunction, RemovedHook,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::types::message::AgentMessage;

pub const ON_TIMER_FN: &str = "harness::on-timer";
pub const ON_TIMER_DESC: &str = "One-shot deadline: called once at `at` (epoch ms) with \
    { scheduled_at, actual_at }. Observe it with args { at }; an observer receives the call for \
    its own `at` only. An `at` more than 7 days out is refused (CONFIG_ERROR); an `at` already \
    past is called once as soon as its observers are active. The natural second leg of any armed \
    wake or fan-in gate: 'wake me when X happens, or tell me at T that it did not'. For \
    recurrence observe cron::on-tick.";

/// How long a late call waits for one observer of its instant to become
/// active; a rejected observer resolves at once, a removed one may never.
const ACTIVE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a late call waits for more observers of its instant once every
/// pending one is active: a replay adds an instant's observers in one burst.
const COALESCE: std::time::Duration = std::time::Duration::from_millis(50);

/// Timers stay armable for at most this far out — far beyond any deadline,
/// and short enough that an epoch-SECONDS timestamp (off by 1000×) cannot
/// masquerade as a valid future instant.
pub const MAX_TIMER_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// An observer's args: the instant it wants the call at.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimerArgs {
    /// Epoch milliseconds.
    pub at: i64,
}

/// What the function is called with, and what its observers observe.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TimerEvent {
    /// The observers' `at`.
    pub scheduled_at: i64,
    /// When the call was made.
    pub actual_at: i64,
}

/// The agent-facing deadline shape `harness::observe` accepts for
/// `harness::on-timer`: an absolute `at`, or a relative `in_ms` it resolves
/// to `at` before observing.
#[derive(Debug, Deserialize)]
pub struct TimerRequestArgs {
    #[serde(default)]
    pub at: Option<i64>,
    #[serde(default)]
    pub in_ms: Option<i64>,
}

/// Keyed one-shot deadlines: each id holds at most one pending task that
/// sleeps until an absolute epoch-ms instant, then runs its work once.
///
/// The harness's one timer facility — the `harness::on-timer` provider below
/// and the binding-expiry deadlines (`bindings::expiry`) both arm through it, so
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

/// The `harness::on-timer` provider: one armed one-shot per distinct `at`,
/// shared by the observers of that instant.
#[derive(Clone)]
pub struct TimerBus {
    iii: Arc<IIIClient>,
    timers: OneShots,
    /// Observers per instant.
    observers: Arc<Mutex<HashMap<i64, usize>>>,
    /// Per past instant with a late call pending: its observers not yet
    /// known to be active.
    due: Arc<Mutex<HashMap<i64, Vec<Observer>>>>,
}

impl TimerBus {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            iii,
            timers: OneShots::new(),
            observers: Arc::new(Mutex::new(HashMap::new())),
            due: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Set the function's hooks, then register it.
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let bus = Self::new(iii.clone());
        iii.on_observer_added_for(ON_TIMER_FN, bus.clone());
        iii.on_observer_removed_for(ON_TIMER_FN, bus.clone());
        iii.on_observer_match_for(ON_TIMER_FN, Match::keys(observer_instant, call_instant));
        iii.register_function(
            ON_TIMER_FN,
            RegisterFunction::new(|event: TimerEvent| Ok::<TimerEvent, Error>(event))
                .description(ON_TIMER_DESC)
                .args_format::<TimerArgs>()
                .private(),
        );
        bus
    }

    /// Count one more observer of `at`; `true` for its first.
    fn retain(&self, at: i64) -> bool {
        let mut observers = self.observers.lock().unwrap_or_else(|p| p.into_inner());
        let count = observers.entry(at).or_insert(0);
        *count += 1;
        *count == 1
    }

    /// Count one observer of `at` less; `true` for its last.
    fn release(&self, at: i64) -> bool {
        let mut observers = self.observers.lock().unwrap_or_else(|p| p.into_inner());
        match observers.get_mut(&at) {
            Some(count) if *count > 1 => {
                *count -= 1;
                false
            }
            Some(_) => {
                observers.remove(&at);
                true
            }
            None => false,
        }
    }

    fn arm(&self, at: i64) {
        let bus = self.clone();
        self.timers
            .arm(at.to_string(), at, async move { bus.fire(at).await });
    }

    pub fn armed_count(&self) -> usize {
        self.timers.len()
    }

    /// Call the function once for the past instant `at` when `obs`, and every
    /// other observer of `at` added meanwhile, is active. An observer added
    /// while a late call for its instant is pending joins it.
    fn fire_late(&self, at: i64, obs: Observer) {
        {
            let mut due = self.due.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(waiting) = due.get_mut(&at) {
                waiting.push(obs);
                return;
            }
            due.insert(at, vec![obs]);
        }
        let bus = self.clone();
        tokio::spawn(async move {
            loop {
                let next = bus
                    .due
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get_mut(&at)
                    .and_then(Vec::pop);
                if let Some(obs) = next {
                    let _ = tokio::time::timeout(ACTIVE_WAIT, obs.accepted()).await;
                    continue;
                }
                tokio::time::sleep(COALESCE).await;
                let mut due = bus.due.lock().unwrap_or_else(|p| p.into_inner());
                if due.get(&at).is_some_and(|waiting| !waiting.is_empty()) {
                    continue;
                }
                due.remove(&at);
                break;
            }
            bus.fire(at).await;
        });
    }

    /// Call the function once for `at`; the match hook hands the call to the
    /// observers of that instant.
    async fn fire(&self, at: i64) {
        let event = json!({
            "scheduled_at": at,
            "actual_at": AgentMessage::now_ms(),
        });
        let call = Call {
            input: event.clone(),
            ..Call::default()
        };
        if !self.iii.is_observed_for(ON_TIMER_FN, &call) {
            return;
        }
        let request = CallRequest {
            function_id: ON_TIMER_FN.to_string(),
            payload: event,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        match self.iii.call(request).await {
            Ok(_) => tracing::info!(at, "timer fired"),
            Err(error) => tracing::warn!(at, %error, "timer call failed"),
        }
    }
}

#[async_trait]
impl AddedHook for TimerBus {
    async fn on_added(&self, obs: Observer) -> Result<(), ObserveRejection> {
        let args: TimerArgs = obs.args()?;
        let now = AgentMessage::now_ms();
        if args.at - now > MAX_TIMER_MS {
            return Err(config_error(format!(
                "timer `at` ({}) is more than 7 days out — pass epoch MILLISECONDS",
                args.at
            )));
        }
        let first = self.retain(args.at);
        tracing::info!(observer = %obs.id(), at = args.at, "timer armed");
        if args.at > now {
            if first {
                self.arm(args.at);
            }
        } else {
            // Already due (a replay after downtime): call once the observers
            // are active, or the call would reach nobody.
            self.fire_late(args.at, obs);
        }
        Ok(())
    }
}

#[async_trait]
impl RemovedHook for TimerBus {
    async fn on_removed(&self, obs: Observer) {
        let Ok(args) = obs.args::<TimerArgs>() else {
            return;
        };
        if self.release(args.at) {
            self.timers.cancel(&args.at.to_string());
        }
        if let Some(waiting) = self
            .due
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&args.at)
        {
            waiting.retain(|pending| pending.id() != obs.id());
        }
        tracing::info!(observer = %obs.id(), at = args.at, "timer observer removed");
    }
}

/// The instant an observer wants: its `at`.
fn observer_instant(obs: &Observer) -> Keys<'_> {
    Keys::from(instant(obs.args_value(), "at"))
}

/// The instant a call is for: its `scheduled_at`.
fn call_instant(call: &Call) -> Keys<'_> {
    Keys::from(instant(&call.input, "scheduled_at"))
}

/// The epoch-ms instant at `field`, as the key both sides file under.
fn instant(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_i64)
        .map(|at| at.to_string())
}

/// When a `harness::observe` timer should fire, from the agent's args.
pub fn resolve_fire_at(args: &TimerRequestArgs, now_ms: i64) -> Result<i64, String> {
    match (args.at, args.in_ms) {
        (Some(at), None) => Ok(at),
        (None, Some(in_ms)) if in_ms > 0 => Ok(now_ms + in_ms),
        (None, Some(_)) => Err("`in_ms` must be positive".into()),
        (None, None) => Err("timer args need `at` (epoch ms) or `in_ms`".into()),
        (Some(_), Some(_)) => Err("pass `at` OR `in_ms`, not both".into()),
    }
}

fn config_error(message: String) -> ObserveRejection {
    ObserveRejection::new("CONFIG_ERROR", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fire_at_resolves_relative_and_absolute() {
        let now = 1_000_000;
        let rel = TimerRequestArgs {
            at: None,
            in_ms: Some(600_000),
        };
        assert_eq!(resolve_fire_at(&rel, now).unwrap(), 1_600_000);
        let abs = TimerRequestArgs {
            at: Some(2_000_000),
            in_ms: None,
        };
        assert_eq!(resolve_fire_at(&abs, now).unwrap(), 2_000_000);
    }

    #[test]
    fn fire_at_refuses_ambiguous_and_empty_args() {
        let both = TimerRequestArgs {
            at: Some(1),
            in_ms: Some(1),
        };
        assert!(resolve_fire_at(&both, 0).unwrap_err().contains("not both"));
        let neither = TimerRequestArgs {
            at: None,
            in_ms: None,
        };
        assert!(resolve_fire_at(&neither, 0).unwrap_err().contains("`at`"));
        let negative = TimerRequestArgs {
            at: None,
            in_ms: Some(-5),
        };
        assert!(resolve_fire_at(&negative, 0).is_err());
    }

    /// The observers of one instant share one armed one-shot; it is cancelled
    /// with the instant's last observer, never earlier.
    #[test]
    fn observers_of_one_instant_share_its_one_shot() {
        let bus = TimerBus::new(Arc::new(IIIClient::new("ws://127.0.0.1:0")));
        assert!(
            bus.retain(5_000),
            "the first observer of an instant arms it"
        );
        assert!(!bus.retain(5_000), "the second shares it");
        assert!(bus.retain(9_000), "another instant is armed on its own");
        assert!(!bus.release(5_000), "one observer of 5000 remains");
        assert!(bus.release(5_000), "the last observer of 5000 cancels it");
        assert!(!bus.release(5_000), "an unknown instant releases nothing");
        assert!(bus.release(9_000));
    }

    #[tokio::test]
    async fn cancel_aborts_an_armed_timer() {
        // No engine: the task would only ever reach the call after its
        // sleep; cancelling within the sleep window proves the abort path.
        let bus = TimerBus::new(Arc::new(IIIClient::new("ws://127.0.0.1:0")));
        let at = AgentMessage::now_ms() + 60_000;
        bus.arm(at);
        assert_eq!(bus.armed_count(), 1);
        bus.timers.cancel(&at.to_string());
        assert_eq!(bus.armed_count(), 0);
        // Re-arming the same instant replaces rather than duplicating.
        bus.arm(at);
        bus.arm(at);
        assert_eq!(bus.armed_count(), 1);
    }

    /// An observer's key is its `at`, a call's its `scheduled_at`: the two
    /// meet, so the call reaches exactly that instant's observers.
    #[test]
    fn a_call_and_its_observers_file_under_the_same_instant() {
        let at = 1_700_000_000_000_i64;
        assert_eq!(
            instant(&json!({ "at": at }), "at"),
            instant(
                &json!({ "scheduled_at": at, "actual_at": 1 }),
                "scheduled_at"
            )
        );
        assert_eq!(instant(&json!({}), "scheduled_at"), None);
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
