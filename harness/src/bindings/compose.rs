//! Recover one-shot Compose wakes whose terminal event preceded provider
//! activation. The canonical watch and fire count already live in the binding.
//!
//! SDK registration only queues a request, and engine discovery can show the
//! trigger before its provider installs it. An immediate read is therefore not
//! a readiness barrier. The existing expiry sweep retries still-armed watches;
//! every replay enters the ordinary delivery hop and its atomic once claim.

use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::Binding;
use crate::deps::Deps;
use crate::types::message::AgentMessage;

const MAX_RECOVERIES: usize = 16;
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
static RECOVERIES: LazyLock<Recoveries> = LazyLock::new(Recoveries::default);

/// Scheduling only: the durable binding and delivery claim remain authoritative.
#[derive(Clone)]
struct Recoveries {
    active: Arc<Mutex<HashSet<String>>>,
    slots: Arc<tokio::sync::Semaphore>,
}

impl Default for Recoveries {
    fn default() -> Self {
        Self {
            active: Arc::default(),
            slots: Arc::new(tokio::sync::Semaphore::new(MAX_RECOVERIES)),
        }
    }
}

impl Recoveries {
    fn spawn(&self, id: &str, work: impl Future<Output = ()> + Send + 'static) -> bool {
        let mut active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        if !active.insert(id.to_string()) {
            return false;
        }
        let slot = RecoverySlot {
            active: self.clone(),
            id: id.to_string(),
        };
        let slots = self.slots.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let Ok(_permit) = slots.acquire_owned().await else {
                return;
            };
            work.await;
        });
        true
    }
}

struct RecoverySlot {
    active: Recoveries,
    id: String,
}

impl Drop for RecoverySlot {
    fn drop(&mut self) {
        self.active
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.id);
    }
}

/// Never await recovery under the registering turn's session lock, or delay
/// expiry of unrelated bindings behind an unavailable Compose diagnostic.
pub(crate) fn schedule(deps: &Deps, binding: &Binding) {
    let Some(operation_id) = operation_id(binding, AgentMessage::now_ms()) else {
        return;
    };
    let deps = deps.clone();
    let binding_id = binding.id.clone();
    let operation_id = operation_id.to_string();
    RECOVERIES.spawn(&binding.id, async move {
        let snapshot = tokio::time::timeout(SNAPSHOT_TIMEOUT, async {
            deps.engine()
                .await
                .dispatch(
                    "compose::operation",
                    json!({ "operation_id": operation_id }),
                )
                .await
        })
        .await;
        let Ok(Ok(snapshot)) = snapshot else {
            // Diagnostics never invalidate registration; a later sweep retries.
            return;
        };
        let Some(event) = terminal_event(&operation_id, &snapshot) else {
            return;
        };
        // The live event and this persisted copy share conditions, lifecycle
        // CAS, and retirement. Never inject directly or increment fires here.
        if let Err(error) = crate::functions::trigger_deliver::handle(
            &deps,
            event.clone(),
            Some(json!({ "__binding": binding_id })),
        )
        .await
        {
            tracing::warn!(binding = %binding_id, %error, "Compose wake recovery failed");
        }
    });
}

fn operation_id(binding: &Binding, now_ms: i64) -> Option<&str> {
    if !super::is_armed_wake(binding, now_ms)
        || binding.is_exhausted(now_ms)
        || binding.trigger_id.is_none()
    {
        return None;
    }
    let (trigger_type, config) = binding.trigger_watch()?;
    if trigger_type != "compose-operation" {
        return None;
    }
    config
        .get("operation_id")?
        .as_str()
        .filter(|id| !id.is_empty())
}

fn terminal_event<'a>(operation_id: &str, snapshot: &'a Value) -> Option<&'a Value> {
    if snapshot.get("operation_id")?.as_str()? != operation_id
        || !matches!(
            snapshot.get("status")?.as_str()?,
            "succeeded" | "failed" | "cancelled"
        )
    {
        return None;
    }
    let event = snapshot.get("last_event")?;
    let sequence = event.get("sequence")?.as_u64()?;
    (event.get("operation_id")?.as_str()? == operation_id
        && event.get("terminal")?.as_bool()?
        && sequence > 0
        && snapshot.get("last_sequence")?.as_u64()? == sequence)
        .then_some(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{BindingTarget, Lifecycle, OwnerScope};
    use serde_json::json;

    fn wake() -> Binding {
        Binding {
            id: "sub_compose".into(),
            trigger_id: Some("sdk:sub_compose".into()),
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
            dedup_key: Some(json!({
                "trigger_type": "compose-operation",
                "config": { "operation_id": "compose:watched", "terminal_only": true },
            })),
            fires: 0,
            created_at: 0,
        }
    }

    fn settled_snapshot() -> Value {
        json!({
            "operation_id": "compose:watched",
            "status": "succeeded",
            "last_sequence": 3,
            "last_event": {
                "operation_id": "compose:watched",
                "sequence": 3,
                "phase": "complete",
                "detail": "all workers ready",
                "current": 2,
                "total": 2,
                "elapsed_ms": 100,
                "terminal": true,
            },
        })
    }

    #[test]
    fn a_later_snapshot_recovers_completion_after_the_first_probe() {
        let binding = wake();
        let id = operation_id(&binding, 1).expect("an unfired Compose wake needs catch-up");
        let running = json!({
            "operation_id": id,
            "status": "running",
            "last_sequence": 2,
            "last_event": { "operation_id": id, "sequence": 2, "terminal": false },
        });
        assert!(terminal_event(id, &running).is_none());

        // The operation finishes before the provider installs its binding.
        // A later sweep must recover the provider's exact persisted event.
        let settled = settled_snapshot();
        let recovered = terminal_event(id, &settled).expect("the missed event must be recoverable");
        assert_eq!(recovered, &settled["last_event"]);
        assert_eq!(recovered["detail"], "all workers ready");
        assert_eq!(recovered["sequence"], 3);
    }

    #[test]
    fn only_live_unfired_one_shot_wakes_with_an_operation_id_are_recovered() {
        let original = wake();
        assert_eq!(operation_id(&original, 1), Some("compose:watched"));
        let mut variants = Vec::new();
        let mut claimed = original.clone();
        claimed.fires = 1;
        variants.push(claimed);
        let mut standing = original.clone();
        standing.lifecycle.once = false;
        variants.push(standing);
        let mut call = original.clone();
        call.target.function_id = "state::set".into();
        variants.push(call);
        let mut expired = original.clone();
        expired.lifecycle.expires_at = Some(1);
        variants.push(expired);
        let mut registering = original.clone();
        registering.trigger_id = None;
        variants.push(registering);
        for config in [
            json!({}),
            json!({ "operation_id": null }),
            json!({ "operation_id": "" }),
        ] {
            let mut wildcard = original.clone();
            wildcard.dedup_key.as_mut().unwrap()["config"] = config;
            variants.push(wildcard);
        }
        let mut other_type = original;
        other_type.dedup_key.as_mut().unwrap()["trigger_type"] = json!("state");
        variants.push(other_type);
        for binding in variants {
            assert!(operation_id(&binding, 1).is_none(), "{binding:?}");
        }
    }

    #[test]
    fn recovery_refuses_foreign_stale_nonterminal_and_unknown_snapshots() {
        for (pointer, replacement) in [
            ("/operation_id", json!("compose:other")),
            ("/last_event/operation_id", json!("compose:other")),
            ("/last_event/sequence", json!(2)),
            ("/last_sequence", Value::Null),
            ("/last_event/terminal", json!(false)),
            ("/last_event", Value::Null),
            ("/status", json!("running")),
            ("/status", json!("unknown-future-status")),
        ] {
            let mut snapshot = settled_snapshot();
            *snapshot.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                terminal_event("compose:watched", &snapshot).is_none(),
                "{snapshot}"
            );
        }
        for status in ["succeeded", "failed", "cancelled"] {
            let mut snapshot = settled_snapshot();
            snapshot["status"] = json!(status);
            assert!(terminal_event("compose:watched", &snapshot).is_some());
        }
    }

    #[tokio::test]
    async fn recovery_limits_concurrency_without_losing_a_queued_watch() {
        let recoveries = Recoveries::default();
        let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
        let mut releases = Vec::new();
        for index in 0..=MAX_RECOVERIES {
            let (release, done) = tokio::sync::oneshot::channel::<()>();
            releases.push(Some(release));
            let started = started.clone();
            assert!(
                recoveries.spawn(&format!("binding_{index}"), async move {
                    started.send(index).unwrap();
                    let _ = done.await;
                }),
                "every watch must retain its recovery opportunity"
            );
        }
        assert!(!recoveries.spawn("binding_0", async {
            panic!("the same binding cannot have overlapping recoveries");
        }));
        let mut active = HashSet::new();
        for _ in 0..MAX_RECOVERIES {
            let index = tokio::time::timeout(Duration::from_secs(2), starts.recv())
                .await
                .expect("a recovery must start")
                .expect("the start channel remains open");
            active.insert(index);
        }
        assert!(
            starts.try_recv().is_err(),
            "queued work must respect the concurrency bound"
        );

        // Free one slot: the extra watch must run without needing the first
        // MAX_RECOVERIES operations to all finish or a new sweep to select it.
        let queued = (0..=MAX_RECOVERIES)
            .find(|id| !active.contains(id))
            .unwrap();
        let running = *active.iter().next().unwrap();
        releases[running].take().unwrap().send(()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), starts.recv())
                .await
                .expect("the queued recovery must start when a slot is free"),
            Some(queued)
        );
        drop(releases);
    }
}
