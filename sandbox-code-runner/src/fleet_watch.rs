//! The fleet watcher: ONE server-side reader of `sandbox::list` for the
//! whole deployment, pushing change-only snapshots onto the bus as calls of
//! `sandbox-code-runner::on-event { kind: "fleet" }`.
//!
//! The daemon emits no lifecycle events of its own, so SOMETHING has to
//! read it — this centralizes that into a single reader (instead of one per
//! open console tab) and turns everything downstream into push. The whole
//! module is deleted the day `sandbox::*` emits its own events.
//!
//! It runs only while `sandbox-code-runner::on-event` has observers: the
//! function's added hook retains it (the first observer starts the loop) and
//! its removed hook releases it (the last one stops it), so an unwatched
//! deployment never polls the daemon.
//!
//! Cadence: 2s while the daemon answers, 10s while it is absent. Emission
//! is change-only — a quiet fleet costs observers nothing. Transient read
//! errors (timeouts, engine hiccups) emit nothing: stale-but-truthful beats
//! flapping between real and empty snapshots.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::{IIIClient, ObserveRejection, Observer};
use serde_json::Value;
use tokio::task::JoinHandle;

use crate::engine::Engine;
use crate::events::{Emitter, Event, ON_EVENT_FN};

const WATCH_INTERVAL: Duration = Duration::from_millis(2_000);
const ABSENT_RECHECK: Duration = Duration::from_millis(10_000);
const LIST_TIMEOUT_MS: u64 = 10_000;

/// What the watcher last told observers, so it only speaks on change.
#[derive(PartialEq, Clone)]
enum Reported {
    Nothing,
    Absent,
    Fleet(Value),
}

/// Extract and order the daemon's sandbox list so equality means "the
/// fleet genuinely changed", not "the daemon enumerated differently".
fn normalize(list_response: &Value) -> Option<Value> {
    let mut sandboxes: Vec<Value> = list_response.get("sandboxes")?.as_array()?.clone();
    sandboxes.sort_by(|a, b| {
        let id = |v: &Value| {
            v.get("sandbox_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        id(a).cmp(&id(b))
    });
    Some(Value::Array(sandboxes))
}

/// The comparison copy: the daemon derives `age_secs` from an Instant, so
/// it grows on EVERY read — left in, consecutive snapshots always differ
/// and change-only emission degrades to a 2s firehose. Equality runs on
/// this age-stripped copy; the emitted payload keeps the full rows so
/// observers still get fresh ages.
fn strip_ages(fleet: &Value) -> Value {
    let Some(rows) = fleet.as_array() else {
        return fleet.clone();
    };
    Value::Array(
        rows.iter()
            .map(|row| {
                let mut row = row.clone();
                if let Some(obj) = row.as_object_mut() {
                    obj.remove("age_secs");
                }
                row
            })
            .collect(),
    )
}

fn is_function_not_found(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("function_not_found") || m.contains("not found")
}

/// One observation: read, compare, maybe emit. Returns the new state and
/// the delay before the next observation.
fn step(
    previous: &Reported,
    read: Result<Value, String>,
    emitter: &Emitter,
) -> (Reported, Duration) {
    match read {
        Ok(response) => {
            let Some(fleet) = normalize(&response) else {
                return (previous.clone(), WATCH_INTERVAL);
            };
            let next = Reported::Fleet(strip_ages(&fleet));
            if *previous != next {
                emitter.emit(Event::fleet(false, fleet));
            }
            (next, WATCH_INTERVAL)
        }
        Err(message) if is_function_not_found(&message) => {
            if *previous != Reported::Absent {
                emitter.emit(Event::fleet(true, Value::Array(vec![])));
            }
            (Reported::Absent, ABSENT_RECHECK)
        }
        Err(_) => (previous.clone(), WATCH_INTERVAL),
    }
}

/// The watch loop: read, compare, maybe emit, sleep — until aborted. Starts
/// from `Reported::Nothing`, so its first read always emits the fleet.
async fn watch(engine: Arc<dyn Engine>, emitter: Emitter) {
    let mut reported = Reported::Nothing;
    loop {
        let read = engine
            .call(
                "sandbox::list".to_string(),
                serde_json::json!({}),
                LIST_TIMEOUT_MS,
            )
            .await;
        let (next, delay) = step(&reported, read, &emitter);
        reported = next;
        tokio::time::sleep(delay).await;
    }
}

/// The watch loop, run while `sandbox-code-runner::on-event` has observers.
pub struct FleetWatch {
    engine: Arc<dyn Engine>,
    emitter: Emitter,
    state: Mutex<Watching>,
}

#[derive(Default)]
struct Watching {
    observers: usize,
    task: Option<JoinHandle<()>>,
}

impl FleetWatch {
    pub fn new(engine: Arc<dyn Engine>, emitter: Emitter) -> Arc<Self> {
        Arc::new(Self {
            engine,
            emitter,
            state: Mutex::new(Watching::default()),
        })
    }

    /// One more observer. The first starts the loop, whose opening read
    /// waits for `active` (that observer accepted), so the observer receives
    /// the opening snapshot.
    pub fn retain(&self, active: impl Future<Output = ()> + Send + 'static) {
        let mut watching = self.lock();
        watching.observers += 1;
        if watching.observers == 1 {
            let (engine, emitter) = (self.engine.clone(), self.emitter.clone());
            watching.task = Some(tokio::spawn(async move {
                active.await;
                watch(engine, emitter).await;
            }));
            tracing::info!("fleet watcher started: {ON_EVENT_FN} is observed");
        }
    }

    /// One observer fewer. The last stops the loop.
    pub fn release(&self) {
        let mut watching = self.lock();
        watching.observers = watching.observers.saturating_sub(1);
        if watching.observers == 0 {
            if let Some(task) = watching.task.take() {
                task.abort();
                tracing::info!("fleet watcher stopped: {ON_EVENT_FN} has no observers");
            }
        }
    }

    /// Whether the loop is running (tests).
    pub fn is_running(&self) -> bool {
        self.lock().task.is_some()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Watching> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Set `sandbox-code-runner::on-event`'s added and removed hooks by id, before
/// the function registers: each accepted observer retains the watcher, and
/// its removal releases it.
pub fn run_while_observed(iii: &IIIClient, engine: Arc<dyn Engine>, emitter: Emitter) {
    let fleet = FleetWatch::new(engine, emitter);
    let on_added = fleet.clone();
    iii.on_observer_added_for(ON_EVENT_FN, move |obs: Observer| {
        let fleet = on_added.clone();
        async move {
            fleet.retain(async move {
                let _ = obs.accepted().await;
            });
            Ok::<(), ObserveRejection>(())
        }
    });
    iii.on_observer_removed_for(ON_EVENT_FN, move |_obs: Observer| {
        let fleet = fleet.clone();
        async move { fleet.release() }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventDeliverer;
    use serde_json::json;

    struct Recorder {
        seen: Mutex<Vec<Value>>,
    }

    impl EventDeliverer for Recorder {
        fn is_observed(&self, _function_id: &str) -> bool {
            true
        }

        fn deliver(&self, _function_id: &str, payload: Value) {
            self.seen.lock().unwrap().push(payload);
        }
    }

    fn harness() -> (Emitter, Arc<Recorder>) {
        let recorder = Arc::new(Recorder {
            seen: Mutex::new(vec![]),
        });
        (Emitter::new(recorder.clone()), recorder)
    }

    fn fleet_of(ids: &[&str]) -> Value {
        json!({
            "sandboxes": ids
                .iter()
                .map(|id| json!({ "sandbox_id": id, "stopped": false }))
                .collect::<Vec<_>>()
        })
    }

    fn fleet_rows(rows: &[(&str, u64, u64, bool)]) -> Value {
        json!({
            "sandboxes": rows
                .iter()
                .map(|(id, age, in_flight, stopped)| json!({
                    "sandbox_id": id,
                    "age_secs": age,
                    "exec_in_flight": in_flight,
                    "stopped": stopped,
                }))
                .collect::<Vec<_>>()
        })
    }

    #[tokio::test]
    async fn the_first_observer_starts_the_watcher_and_the_last_stops_it() {
        let (emitter, recorder) = harness();
        let fake = crate::engine::FakeEngine::new();
        fake.with_response("sandbox::list", Ok(fleet_of(&["a"])));
        let fleet = FleetWatch::new(fake, emitter);
        assert!(!fleet.is_running());

        fleet.retain(std::future::ready(()));
        fleet.retain(std::future::ready(()));
        assert!(fleet.is_running());
        // The opening read emits the current fleet.
        for _ in 0..50 {
            if !recorder.seen.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(recorder.seen.lock().unwrap()[0]["kind"], "fleet");

        fleet.release();
        assert!(fleet.is_running(), "one observer is left");
        fleet.release();
        assert!(!fleet.is_running(), "the last observer stops the watcher");
        // A release with no observer left changes nothing.
        fleet.release();
        assert!(!fleet.is_running());
    }

    #[tokio::test]
    async fn emits_on_first_read_and_then_only_on_change() {
        let (emitter, recorder) = harness();
        let mut state = Reported::Nothing;

        let (next, _) = step(&state, Ok(fleet_of(&["a"])), &emitter);
        state = next;
        let (next, _) = step(&state, Ok(fleet_of(&["a"])), &emitter);
        state = next;
        let (next, _) = step(&state, Ok(fleet_of(&["a", "b"])), &emitter);
        state = next;
        let _ = state;

        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["kind"], "fleet");
        assert_eq!(seen[0]["daemon_absent"], false);
        assert_eq!(seen[0]["sandboxes"].as_array().unwrap().len(), 1);
        assert_eq!(seen[1]["sandboxes"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn an_age_only_change_does_not_emit() {
        let (emitter, recorder) = harness();
        let (state, _) = step(
            &Reported::Nothing,
            Ok(fleet_rows(&[("a", 5, 0, false)])),
            &emitter,
        );
        let (_, _) = step(&state, Ok(fleet_rows(&[("a", 7, 0, false)])), &emitter);
        assert_eq!(recorder.seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn real_changes_emit_and_carry_current_ages() {
        let (emitter, recorder) = harness();
        let (state, _) = step(
            &Reported::Nothing,
            Ok(fleet_rows(&[("a", 5, 0, false)])),
            &emitter,
        );
        // A new id, an exec_in_flight change, and a stopped flip each emit.
        let (state, _) = step(
            &state,
            Ok(fleet_rows(&[("a", 7, 0, false), ("b", 1, 0, false)])),
            &emitter,
        );
        let (state, _) = step(
            &state,
            Ok(fleet_rows(&[("a", 9, 1, false), ("b", 3, 0, false)])),
            &emitter,
        );
        let (_, _) = step(
            &state,
            Ok(fleet_rows(&[("a", 11, 0, true), ("b", 5, 0, false)])),
            &emitter,
        );
        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        // The emitted rows keep FRESH ages even though comparison strips them.
        assert_eq!(seen[1]["sandboxes"][0]["age_secs"], 7);
        assert_eq!(seen[1]["sandboxes"][1]["age_secs"], 1);
        assert_eq!(seen[2]["sandboxes"][0]["exec_in_flight"], 1);
        assert_eq!(seen[2]["sandboxes"][0]["age_secs"], 9);
        assert_eq!(seen[3]["sandboxes"][0]["stopped"], true);
        assert_eq!(seen[3]["sandboxes"][0]["age_secs"], 11);
    }

    #[tokio::test]
    async fn enumeration_order_is_not_a_change() {
        let (emitter, recorder) = harness();
        let (state, _) = step(&Reported::Nothing, Ok(fleet_of(&["a", "b"])), &emitter);
        let (_, _) = step(&state, Ok(fleet_of(&["b", "a"])), &emitter);
        assert_eq!(recorder.seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn daemon_absence_emits_once_and_rechecks_slowly() {
        let (emitter, recorder) = harness();
        let (state, delay) = step(
            &Reported::Nothing,
            Err("Function sandbox::list not found".into()),
            &emitter,
        );
        assert_eq!(delay, ABSENT_RECHECK);
        let (_, _) = step(
            &state,
            Err("Function sandbox::list not found".into()),
            &emitter,
        );
        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["daemon_absent"], true);
    }

    #[tokio::test]
    async fn a_transient_error_keeps_quiet_and_keeps_the_state() {
        let (emitter, recorder) = harness();
        let (state, _) = step(&Reported::Nothing, Ok(fleet_of(&["a"])), &emitter);
        let (after_err, delay) = step(&state, Err("timeout".into()), &emitter);
        assert_eq!(delay, WATCH_INTERVAL);
        assert!(after_err == state);
        // The unchanged fleet after recovery does not re-emit either.
        let (_, _) = step(&after_err, Ok(fleet_of(&["a"])), &emitter);
        assert_eq!(recorder.seen.lock().unwrap().len(), 1);
    }
}
