//! Catch-up for one-shot state bindings whose key was written while no
//! engine trigger could see the write.
//!
//! State events do not replay: the state worker's fan-out delivers only to
//! triggers that are ACTIVE at write time, and activation is asynchronous —
//! `IIIClient::register_trigger` queues the registration over the channel and
//! returns before the provider installs it. A one-shot binding on
//! `(scope, key)` therefore has three windows in which the write it waits for
//! lands unseen:
//!
//! * BEFORE the registration — the key was already written. This is the
//!   Linkly `link-build-e2e` run (`0491ffe3…`): the parent read `null`, the
//!   sub-agent wrote `b`, the parent registered a wake on `b`, and parked
//!   forever on `fires: 0` while `harness::metrics` reported
//!   `complete: false` for 900 s;
//! * BETWEEN the registration and the provider's activation;
//! * while no trigger existed at all — an engine or harness restart.
//!
//! All three end the same way: the key holds exactly the value the binding
//! was waiting for and the binding stays armed forever, `session_expects_wake`
//! keeps the session non-terminal, and nothing reports it.
//!
//! The reconciliation is ONE rule applied from three places: if an unfired
//! one-shot keyed state binding's key currently holds a value, deliver that
//! value NOW as a synthetic `state:updated` event (`replayed: true`) through
//! the ordinary delivery hop ([`crate::functions::trigger_deliver`]). The
//! hop's claim CAS keeps a racing live fire and this catch-up exactly-once —
//! whoever loses the claim records nothing and wakes nobody — and the
//! binding's lifecycle retires it. Biased deliberately toward firing: a
//! spurious wake costs one extra turn once; a lost one strands the owner
//! forever. Callers:
//!
//! * [`crate::functions::subscribe`] — right after the trigger is registered,
//!   so the registration response can say what happened;
//! * [`super::expiry::sweep`] — every sweep, for still-armed bindings, which
//!   closes the activation window with a bounded delay;
//! * [`super::gc`] — the startup replay, for restarts.
//!
//! Standing bindings (`once: false`) are NOT reconciled: "every future write"
//! does not include a value that predates the registration.

use std::future::Future;

use serde_json::{json, Value};

use super::Binding;
use crate::deps::Deps;
use crate::functions::send::LockHeld;
use crate::functions::trigger_deliver::DeliverResult;

/// What the catch-up read found and did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatchUp {
    /// Not an unfired one-shot keyed state binding: nothing to reconcile.
    NotApplicable,
    /// The key holds no value; the live trigger owns the delivery.
    Absent,
    /// The current value was delivered as the binding's event. A one-shot
    /// binding is retired by that delivery.
    Delivered,
    /// The value was offered to the delivery hop, which stopped it: a
    /// declared condition answered `skip` (the binding stays armed, the
    /// arrival counted), or a concurrent live fire won the claim (that fire
    /// delivered it).
    Stopped { gate: String, reason: String },
    /// The key could not be read, or the hop errored. The next sweep retries.
    Failed(String),
}

impl CatchUp {
    pub fn delivered(&self) -> bool {
        matches!(self, CatchUp::Delivered)
    }
}

/// The `(scope, key)` an unfired one-shot state binding watches — the only
/// shape whose missed write is recoverable from the key's current value.
pub fn watched_key(binding: &Binding) -> Option<(&str, &str)> {
    if binding.fires != 0 || !binding.lifecycle.once {
        return None;
    }
    let (trigger_type, config) = binding.trigger_watch()?;
    if trigger_type != "state" {
        return None;
    }
    let scope = config.get("scope")?.as_str()?;
    let key = config.get("key")?.as_str()?;
    Some((scope, key))
}

/// The synthetic event standing in for the missed fire. Shaped like the state
/// worker's own `state:updated` payload so conditions and the wake text read
/// it the same way, plus `replayed: true` so the owner can tell a catch-up
/// from a live write. `old_value` is unknowable after the fact.
pub fn replay_event(scope: &str, key: &str, value: Value) -> Value {
    json!({
        "type": "state",
        "event_type": "state:updated",
        "scope": scope,
        "key": key,
        "old_value": null,
        "new_value": value,
        "replayed": true,
    })
}

/// Read the watched key and, when it holds a value, deliver it through the
/// hop. Every side effect goes through `deliver` — never a direct inject —
/// so the claim CAS, the declared conditions, the `trigger_fired` record and
/// the lifecycle retirement are the live fire's.
pub async fn catch_up(deps: &Deps, binding: &Binding) -> CatchUp {
    catch_up_holding(deps, binding, LockHeld::default()).await
}

/// [`catch_up`] from inside a turn that already holds a session's lock — the
/// registering turn reconciling its own binding. The hop threads it to every
/// injection it can make, so the wake parks as a queued row or merges into
/// the running turn instead of re-acquiring the non-reentrant lock.
pub async fn catch_up_holding(deps: &Deps, binding: &Binding, held: LockHeld<'_>) -> CatchUp {
    let timeout_ms = deps.cfg().await.session_timeout_ms;
    let outcome = reconcile(
        binding,
        |scope, key| async move {
            crate::state::state_get(&deps.iii, &scope, &key, timeout_ms)
                .await
                .map_err(|e| e.to_string())
        },
        |event, metadata| {
            // On its OWN task, awaited. The registering turn's tool phase is
            // already a deep poll chain (step → call → intercept → handle),
            // and the hop adds conditions → claim → dispatch → inject →
            // deliver on top; nested inline that overflowed the SDK
            // connection thread's 2 MiB stack in a debug build (INT-030,
            // first run). A spawned task polls from the scheduler's shallow
            // frame; awaiting its handle keeps the ordering and the lock
            // context exactly as before.
            let deps = deps.clone();
            let held = held.0.map(str::to_string);
            async move {
                tokio::spawn(async move {
                    let held = LockHeld(held.as_deref());
                    crate::functions::trigger_deliver::handle_holding(
                        &deps,
                        event,
                        Some(metadata),
                        held,
                    )
                    .await
                    .map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| format!("delivery task aborted: {e}"))?
            }
        },
    )
    .await;
    match &outcome {
        CatchUp::Delivered => {
            tracing::info!(binding = %binding.id, "delivered a state wake from the key's current value");
        }
        CatchUp::Stopped { gate, reason } => {
            tracing::info!(binding = %binding.id, gate, reason, "state wake catch-up stopped by the delivery hop");
        }
        CatchUp::Failed(error) => {
            tracing::warn!(binding = %binding.id, error, "state wake catch-up failed; the sweep retries");
        }
        CatchUp::NotApplicable | CatchUp::Absent => {}
    }
    outcome
}

/// The decision, factored over its two I/O steps so it is testable without
/// an engine: `read` returns the key's current value (`null` when absent),
/// `deliver` is the delivery hop taking `(event, metadata)`.
pub(crate) async fn reconcile<R, RF, D, DF>(binding: &Binding, read: R, deliver: D) -> CatchUp
where
    R: FnOnce(String, String) -> RF,
    RF: Future<Output = Result<Value, String>>,
    D: FnOnce(Value, Value) -> DF,
    DF: Future<Output = Result<DeliverResult, String>>,
{
    let Some((scope, key)) = watched_key(binding) else {
        return CatchUp::NotApplicable;
    };
    let value = match read(scope.to_string(), key.to_string()).await {
        Ok(value) if value.is_null() => return CatchUp::Absent,
        Ok(value) => value,
        Err(error) => return CatchUp::Failed(format!("state {scope}/{key} unreadable: {error}")),
    };
    let event = replay_event(scope, key, value);
    let metadata = json!({ "__binding": binding.id });
    match deliver(event, metadata).await {
        Ok(result) if result.delivered => CatchUp::Delivered,
        Ok(result) => CatchUp::Stopped {
            gate: result.gate.unwrap_or_else(|| "unknown".into()),
            reason: result.note.unwrap_or_default(),
        },
        Err(error) => CatchUp::Failed(format!("delivery hop: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{BindingTarget, Lifecycle, OwnerScope};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn wake(dedup: Value) -> Binding {
        Binding {
            id: "sub_r".into(),
            trigger_id: Some("sdk:sub_r".into()),
            owner: OwnerScope {
                session_id: "s1".into(),
                root_session_id: None,
            },
            target: BindingTarget::new(crate::functions::SEND_ID),
            conditions: Vec::new(),
            lifecycle: Lifecycle {
                once: true,
                max_fires: None,
                expires_at: None,
            },
            capability: None,
            causation: Default::default(),
            dedup_key: Some(dedup),
            fires: 0,
            created_at: 0,
        }
    }

    fn keyed_wake() -> Binding {
        wake(json!({
            "trigger_type": "state",
            "config": { "scope": "link-build-e2e", "key": "b" },
        }))
    }

    fn delivered() -> DeliverResult {
        DeliverResult {
            delivered: true,
            gate: None,
            note: None,
        }
    }

    fn stopped(gate: &str, note: &str) -> DeliverResult {
        DeliverResult {
            delivered: false,
            gate: Some(gate.into()),
            note: Some(note.into()),
        }
    }

    /// A recording delivery hop: how many times it was called and with what.
    #[derive(Default)]
    struct Hop {
        calls: AtomicUsize,
        last: Mutex<Option<(Value, Value)>>,
    }

    impl Hop {
        fn deliver(
            self: &Arc<Self>,
            answer: DeliverResult,
        ) -> impl FnOnce(Value, Value) -> std::future::Ready<Result<DeliverResult, String>>
        {
            let hop = self.clone();
            move |event, metadata| {
                hop.calls.fetch_add(1, Ordering::SeqCst);
                *hop.last.lock().unwrap() = Some((event, metadata));
                std::future::ready(Ok(answer))
            }
        }
    }

    /// The Linkly shape, and the shape of a write that lands between the
    /// preflight read and the trigger's activation — the harness cannot tell
    /// them apart, and must not: the key holds a value when the post-
    /// registration read runs, so the value is delivered.
    #[tokio::test]
    async fn a_prewritten_key_is_delivered_once_through_the_hop() {
        let binding = keyed_wake();
        let hop = Arc::new(Hop::default());
        let reads = Arc::new(AtomicUsize::new(0));
        let outcome = reconcile(
            &binding,
            |scope, key| {
                reads.fetch_add(1, Ordering::SeqCst);
                assert_eq!((scope.as_str(), key.as_str()), ("link-build-e2e", "b"));
                std::future::ready(Ok(json!({ "status": "ok" })))
            },
            hop.deliver(delivered()),
        )
        .await;
        assert_eq!(outcome, CatchUp::Delivered);
        assert!(outcome.delivered());
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(hop.calls.load(Ordering::SeqCst), 1, "exactly one delivery");
        let (event, metadata) = hop.last.lock().unwrap().clone().unwrap();
        // The pointer the hop resolves the record from — nothing else.
        assert_eq!(metadata, json!({ "__binding": "sub_r" }));
        assert_eq!(event["type"], "state");
        assert_eq!(event["event_type"], "state:updated");
        assert_eq!(event["scope"], "link-build-e2e");
        assert_eq!(event["key"], "b");
        assert_eq!(event["new_value"], json!({ "status": "ok" }));
        assert_eq!(event["old_value"], Value::Null);
        assert_eq!(
            event["replayed"], true,
            "the owner must be able to tell a catch-up from a live write"
        );
    }

    /// The normal path: the key is still absent at registration, so nothing
    /// is delivered and the live trigger owns the later write.
    #[tokio::test]
    async fn an_absent_key_delivers_nothing() {
        let binding = keyed_wake();
        let hop = Arc::new(Hop::default());
        let outcome = reconcile(
            &binding,
            |_, _| std::future::ready(Ok(Value::Null)),
            hop.deliver(delivered()),
        )
        .await;
        assert_eq!(outcome, CatchUp::Absent);
        assert_eq!(hop.calls.load(Ordering::SeqCst), 0);
    }

    /// A live fire and the catch-up both offer the value; the hop's claim CAS
    /// admits one. The loser must report the lost claim and must NOT retry,
    /// inject, or touch the record itself — the winner already woke the owner.
    #[tokio::test]
    async fn a_lost_claim_is_reported_not_retried() {
        let binding = keyed_wake();
        let hop = Arc::new(Hop::default());
        let outcome = reconcile(
            &binding,
            |_, _| std::future::ready(Ok(json!(1))),
            hop.deliver(stopped(
                "lifecycle",
                "another fire spent the last of the budget",
            )),
        )
        .await;
        assert_eq!(
            outcome,
            CatchUp::Stopped {
                gate: "lifecycle".into(),
                reason: "another fire spent the last of the budget".into(),
            }
        );
        assert!(!outcome.delivered());
        assert_eq!(hop.calls.load(Ordering::SeqCst), 1, "one attempt, no retry");
    }

    /// An unreadable key is not "absent": the sweep must get another chance,
    /// and nothing may be delivered on a guess.
    #[tokio::test]
    async fn an_unreadable_key_fails_open_without_delivering() {
        let binding = keyed_wake();
        let hop = Arc::new(Hop::default());
        let outcome = reconcile(
            &binding,
            |_, _| std::future::ready(Err("state worker down".to_string())),
            hop.deliver(delivered()),
        )
        .await;
        assert!(matches!(outcome, CatchUp::Failed(ref e) if e.contains("state worker down")));
        assert_eq!(hop.calls.load(Ordering::SeqCst), 0);

        let outcome = reconcile(
            &binding,
            |_, _| std::future::ready(Ok(json!(1))),
            |_, _| std::future::ready(Err("hop panicked".to_string())),
        )
        .await;
        assert!(matches!(outcome, CatchUp::Failed(ref e) if e.contains("hop panicked")));
    }

    /// Only the console-04e02cb7 / Linkly shape qualifies: a one-shot `state`
    /// binding on a scope AND key, never fired. Everything else reads nothing.
    #[tokio::test]
    async fn only_unfired_one_shot_keyed_state_bindings_are_reconciled() {
        assert_eq!(watched_key(&keyed_wake()), Some(("link-build-e2e", "b")));

        let fired = Binding {
            fires: 1,
            ..keyed_wake()
        };
        assert_eq!(watched_key(&fired), None);

        let standing = Binding {
            lifecycle: Lifecycle {
                once: false,
                max_fires: Some(1),
                expires_at: None,
            },
            ..keyed_wake()
        };
        assert_eq!(watched_key(&standing), None, "every FUTURE write only");

        let keyless = wake(json!({ "trigger_type": "state", "config": { "scope": "s" } }));
        assert_eq!(watched_key(&keyless), None);

        let cron =
            wake(json!({ "trigger_type": "cron", "config": { "expression": "0 * * * * *" } }));
        assert_eq!(watched_key(&cron), None);

        let legacy = Binding {
            dedup_key: None,
            ..keyed_wake()
        };
        assert_eq!(watched_key(&legacy), None, "pre-dedup-key record: no watch");

        // A one-shot CALL binding on a key qualifies too, same as the startup
        // replay: the missed write is just as lost for it.
        let call = Binding {
            target: BindingTarget::new("state::set"),
            ..keyed_wake()
        };
        assert_eq!(watched_key(&call), Some(("link-build-e2e", "b")));

        for binding in [fired, standing, keyless, cron, legacy] {
            let hop = Arc::new(Hop::default());
            let reads = Arc::new(AtomicUsize::new(0));
            let outcome = reconcile(
                &binding,
                |_, _| {
                    reads.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(json!(1)))
                },
                hop.deliver(delivered()),
            )
            .await;
            assert_eq!(outcome, CatchUp::NotApplicable, "{binding:?}");
            assert_eq!(reads.load(Ordering::SeqCst), 0, "no read for {binding:?}");
            assert_eq!(hop.calls.load(Ordering::SeqCst), 0);
        }
    }

    /// The whole point of the catch-up: once the hop delivers and the
    /// one-shot retires, the owner no longer expects a wake, so its terminal
    /// turn is terminal for `harness::metrics` (`complete: true`) and for
    /// `harness::turn-completed`. The Linkly run sat at `complete: false`
    /// because this never happened.
    #[test]
    fn a_caught_up_one_shot_stops_parking_its_session() {
        let armed = keyed_wake();
        assert!(crate::bindings::is_armed_wake(&armed, 0));
        // The hop claims fire 1 and, because `once`, retires the record.
        assert!(armed.retires_after_fire(1));
        let claimed = Binding {
            fires: 1,
            ..armed.clone()
        };
        assert!(
            !crate::bindings::is_armed_wake(&claimed, 0),
            "a delivered wake parks nobody"
        );
        assert_eq!(watched_key(&claimed), None, "and is never caught up twice");
    }
}
