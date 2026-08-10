//! The fleet watcher: ONE server-side observer of `sandbox::list` for the
//! whole deployment, pushing change-only snapshots onto the bus as
//! `sandbox-code-runner::event { kind: "fleet" }`.
//!
//! The daemon emits no lifecycle events of its own, so SOMETHING has to
//! observe it — this centralizes that into a single reader (instead of one
//! per open console tab) and turns everything downstream into push. The
//! whole module is deleted the day `sandbox::*` emits its own events.
//!
//! Cadence: 2s while the daemon answers, 10s while it is absent. Emission
//! is change-only — a quiet fleet costs subscribers nothing. Transient read
//! errors (timeouts, engine hiccups) emit nothing: stale-but-truthful beats
//! flapping between real and empty snapshots.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::engine::Engine;
use crate::events::{Emitter, Event};

const WATCH_INTERVAL: Duration = Duration::from_millis(2_000);
const ABSENT_RECHECK: Duration = Duration::from_millis(10_000);
const LIST_TIMEOUT_MS: u64 = 10_000;

/// What the watcher last told subscribers, so it only speaks on change.
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
            let next = Reported::Fleet(fleet.clone());
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

pub fn spawn(engine: Arc<dyn Engine>, emitter: Emitter) {
    tokio::spawn(async move {
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
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventDeliverer, SubscriberSet};
    use serde_json::json;
    use std::sync::Mutex;

    struct Recorder {
        seen: Mutex<Vec<Value>>,
    }

    impl EventDeliverer for Recorder {
        fn deliver(&self, _function_id: &str, payload: Value) {
            self.seen.lock().unwrap().push(payload);
        }
    }

    fn harness() -> (Emitter, Arc<Recorder>) {
        let recorder = Arc::new(Recorder {
            seen: Mutex::new(vec![]),
        });
        let subscribers = SubscriberSet::new();
        subscribers.insert("t1".into(), "sub::fn".into());
        (Emitter::new(subscribers, recorder.clone()), recorder)
    }

    fn fleet_of(ids: &[&str]) -> Value {
        json!({
            "sandboxes": ids
                .iter()
                .map(|id| json!({ "sandbox_id": id, "stopped": false }))
                .collect::<Vec<_>>()
        })
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
