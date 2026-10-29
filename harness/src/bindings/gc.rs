//! Binding garbage collection and the boot replay.
//!
//! The durable store survives harness restarts; the engine's observers do
//! not — they belong to the harness connection. Startup therefore rewrites
//! records from before 0.26 ([`super::migrate`]), re-creates every surviving
//! binding's observer from its record, and runs the ordinary lifecycle
//! sweeps below:
//!
//! * **Owner gone.** A deleted session leaves bindings that would keep firing
//!   — waking a session that no longer exists.
//! * **Stale spawn targets.** Records whose target is `harness::spawn` predate
//!   the spawn-target removal: delivery never creates an agent, so they are
//!   retired LOUDLY — the owner gets a notification naming the migration
//!   (observe for a wake, spawn directly) — never adapted.
//! * **Orphan observers.** An observer aimed at the delivery hop whose
//!   binding record is gone (an in-memory state adapter restarted, a record
//!   deleted while the observer could not be disposed) is disposed.

use std::collections::HashSet;
use std::time::Duration;

use serde_json::{json, Value};

use crate::deps::Deps;

/// How long a replay waits to learn that its observer id is still held by
/// the previous harness connection.
const REPLAY_READY_WAIT: Duration = Duration::from_secs(5);

/// Whether the owning session is live, provably gone, or unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerState {
    Live,
    Gone,
    Unknown,
}

/// Never GC on doubt: an unreadable session lookup keeps the binding. A
/// wrongly-kept binding fires once too often and is visible; a wrongly-dropped
/// one strands a run silently, and nothing reports it.
pub fn should_gc(state: OwnerState) -> bool {
    state == OwnerState::Gone
}

/// Startup sweep: rewrite records from before 0.26, drop bindings whose owner
/// is provably gone, loudly retire records that still target
/// `harness::spawn`, dispose observers whose record disappeared — then
/// re-create every surviving binding's observer from the durable record and
/// deliver one-shot state wakes whose key was written while no observer
/// existed for it.
pub async fn run(deps: &Deps) {
    let migrated = super::migrate::run(deps).await;
    let store = deps.bindings().await;
    let bindings = match store.list().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "binding sweep skipped: store unreadable");
            return;
        }
    };

    let orphans = reconcile_orphan_observers(deps, &bindings).await;

    let mut dropped = 0usize;
    let mut stale_spawn = 0usize;
    let mut survivors: Vec<crate::bindings::Binding> = Vec::new();
    for binding in bindings {
        let owner_gone = should_gc(owner_state(deps, &binding.owner.session_id).await);
        if owner_gone {
            if binding.observer_id.is_some() {
                super::dispose_observer(deps, &binding.id).await;
            }
            if store.delete(&binding.id).await.is_ok() {
                dropped += 1;
            }
            continue;
        }
        if binding.target.function_id == crate::functions::SPAWN_ID {
            retire_stale_spawn_binding(deps, &binding).await;
            stale_spawn += 1;
            continue;
        }
        survivors.push(binding);
    }

    let (rearmed, caught_up) = replay_observers(deps, survivors).await;
    if migrated.rewritten > 0
        || migrated.retired > 0
        || dropped > 0
        || stale_spawn > 0
        || orphans > 0
        || rearmed > 0
        || caught_up > 0
    {
        tracing::info!(
            rewritten = migrated.rewritten,
            untranslated = migrated.retired,
            owner_gone = dropped,
            stale_spawn = stale_spawn,
            orphan_observers = orphans,
            rearmed = rearmed,
            caught_up = caught_up,
            "swept observer bindings at startup"
        );
    }
}

/// Re-create the observer of every surviving binding.
///
/// Observers are connection-scoped: an engine restart or a harness restart
/// loses them while the durable record stays armed — before this replay the
/// owner then waited forever on a watch that no longer existed
/// (console-04e02cb7 postmortem). The store is the authority; the observer is
/// re-derived at every boot, and reconnects within one process are covered by
/// the SDK's own observer replay.
async fn replay_observers(deps: &Deps, bindings: Vec<crate::bindings::Binding>) -> (usize, usize) {
    let store = deps.bindings().await;
    let now_ms = crate::subscriptions::fired::now_ms();
    let mut rearmed = 0usize;
    let mut caught_up = 0usize;
    for mut binding in bindings {
        // This process already observes it (created after boot).
        if deps.observer_handles.contains(&binding.id) {
            continue;
        }
        // Deletion owns cleanup. Do not resurrect its observers during
        // concurrent startup recovery, or deliver a stale catch-up wake.
        let _topology = deps.topology.lock().await;
        match crate::functions::delete_session_tree::guard_owner(deps, &binding.owner.session_id)
            .await
        {
            Ok(None) => {}
            Ok(Some(_)) => continue,
            Err(error) => {
                tracing::warn!(%error, "binding rearm blocked by unreadable deletion guard");
                continue;
            }
        }
        if binding.is_exhausted(now_ms) {
            // Expiry (bindings::expiry) owns retirement and its owner notice.
            continue;
        }
        let Some(handle) = super::observe_binding(deps, &binding) else {
            // A record without a source: nothing to observe.
            continue;
        };
        take_over_from_previous_connection(deps, &binding, handle);
        match store.rearm_observer_id(&binding, &binding.id).await {
            Ok(crate::bindings::AttachOutcome::Attached(current)) => {
                binding = *current;
                rearmed += 1;
            }
            Ok(crate::bindings::AttachOutcome::Gone) => {
                super::dispose_observer(deps, &binding.id).await;
                continue;
            }
            Err(e) => {
                tracing::warn!(binding = %binding.id, error = %e, "observer id re-arm failed");
                // The observer itself is live and delivers by binding id;
                // only the recorded marker is stale. Keep going.
            }
        }
        super::compose::schedule(deps, &binding);
        drop(_topology);
        caught_up += usize::from(catch_up_state_wake(deps, &binding).await);
    }
    (rearmed, caught_up)
}

/// A harness that restarted within the engine's suspension grace finds its
/// observer ids still held by its previous connection
/// (`OBSERVER_ID_CONFLICT`). Dispose the stale one through the engine and
/// observe again, in the background so the replay never waits on it.
fn take_over_from_previous_connection(
    deps: &Deps,
    binding: &crate::bindings::Binding,
    handle: iii_sdk::ObserverHandle,
) {
    let deps = deps.clone();
    let binding = binding.clone();
    tokio::spawn(async move {
        let Ok(Err(_)) = tokio::time::timeout(REPLAY_READY_WAIT, handle.ready()).await else {
            return;
        };
        if handle.reason().map(|reason| reason.code).as_deref() != Some("OBSERVER_ID_CONFLICT") {
            return;
        }
        deps.observer_handles.remove(&binding.id);
        let _ = handle.dispose().await;
        if !super::dispose_observer(&deps, &binding.id).await {
            return;
        }
        if super::observe_binding(&deps, &binding).is_some() {
            tracing::info!(binding = %binding.id, "observer taken over from the previous connection");
        }
    });
}

/// A one-shot state wake that has never fired watches exactly one key; that
/// key may have been written while no observer existed for it. When this
/// binding qualifies for the replay catch-up read, name the watched
/// `(scope, key)`.
fn state_wake_catchup(binding: &crate::bindings::Binding) -> Option<(String, String)> {
    if binding.fires != 0 || !binding.lifecycle.once {
        return None;
    }
    match binding.state_watch() {
        (Some(scope), Some(key)) => Some((scope.to_string(), key.to_string())),
        _ => None,
    }
}

/// Deliver the wake a lost observer missed: the watched key already holds a
/// value, so the value read NOW stands in for the missed event (the
/// `state::on-changed` event shape). Biased deliberately toward firing — a
/// spurious wake costs one extra turn once, a lost one strands the owner
/// forever — and the delivery claim keeps it exactly-once against a racing
/// live fire.
async fn catch_up_state_wake(deps: &Deps, binding: &crate::bindings::Binding) -> bool {
    let Some((scope, key)) = state_wake_catchup(binding) else {
        return false;
    };
    let timeout_ms = deps.cfg().await.session_timeout_ms;
    let value = match crate::state::state_get(&deps.iii, &scope, &key, timeout_ms).await {
        Ok(v) if !v.is_null() => v,
        _ => return false,
    };
    let event = json!({
        "type": "state",
        "event_type": "state:updated",
        "scope": scope,
        "key": key,
        "old_value": null,
        "new_value": value,
        "replayed": true,
    });
    let metadata = json!({ "__binding": binding.id });
    match crate::functions::observer_deliver::handle(deps, event, Some(metadata)).await {
        Ok(result) => {
            tracing::info!(
                binding = %binding.id,
                scope = %scope,
                key = %key,
                result = ?result,
                "replayed missed state wake"
            );
            result.delivered
        }
        Err(e) => {
            tracing::warn!(binding = %binding.id, error = %e, "state wake catch-up failed");
            false
        }
    }
}

/// Tear down a stored binding that still targets `harness::spawn` — a record
/// from before the spawn-target removal. Loud on the owner side: a silent
/// dispose would leave the owner believing in an armed reaction it no longer
/// has. Called from the startup sweep (mass-retire) and from delivery resolve
/// (the race backstop for a fire already in flight).
pub async fn retire_stale_spawn_binding(deps: &Deps, binding: &crate::bindings::Binding) {
    if binding.observer_id.is_some() {
        super::dispose_observer(deps, &binding.id).await;
    }
    if let Err(e) = deps.bindings().await.delete(&binding.id).await {
        tracing::warn!(binding = %binding.id, error = %e, "stale spawn-binding delete failed");
    }
    let watch = binding
        .source_text()
        .map(|source| format!(" (was observing {source})"))
        .unwrap_or_default();
    let notice = format!(
        "[notification] binding {}{watch} was RETIRED: `harness::spawn` is not a binding \
         target — delivery never creates an agent. Observe the same function as a wake (omit \
         `target`) and spawn workers directly from the woken turn.",
        binding.id
    );
    let message = crate::types::message::AgentMessage::user_text(notice);
    if let Err(e) = crate::functions::send::inject(
        deps,
        &binding.owner.session_id,
        message,
        Some(&format!("e_stalespawn_{}", binding.id)),
        Some(&json!({ "notification": true, "binding": binding.id })),
    )
    .await
    {
        tracing::warn!(binding = %binding.id, error = %e, "stale spawn-binding notice failed");
    }
    crate::subscriptions::fired::emit(
        &deps.session().await,
        &binding.owner.session_id,
        &format!("e_obsstale_{}", binding.id),
        crate::subscriptions::fired::retirement_record(
            binding,
            crate::subscriptions::fired::FireOutcome::Invalidated,
            crate::subscriptions::fired::RetirementReason::Invalidated,
            Some("harness::spawn is not a binding target; the binding was retired"),
            crate::subscriptions::fired::now_ms(),
        ),
    )
    .await;
}

/// The state adapter may lose records the engine's observers outlive (an
/// in-memory adapter restarting while the harness stays connected). Startup
/// and every binding pass (each engine worker connect/disconnect/announce —
/// the state worker coming back among them) dispose observers aimed at the
/// delivery hop whose binding record is gone, so recurring sources do not
/// call a permanent no-op forever. The observer id IS the binding id, which
/// is what makes this possible: the engine's listing never returns metadata.
pub async fn reconcile_orphan_observers(
    deps: &Deps,
    bindings: &[crate::bindings::Binding],
) -> usize {
    let stored: HashSet<String> = bindings.iter().map(|binding| binding.id.clone()).collect();
    let Some(ids) = delivery_observer_ids(deps).await else {
        return 0;
    };
    let store = deps.bindings().await;
    let mut retired = 0usize;
    for id in ids {
        if !is_orphan(&id, &stored) {
            continue;
        }
        // The listing is a snapshot. A registration may insert its record
        // after that snapshot and then create its observer before this loop
        // reaches it; a fresh read prevents us from disposing that valid
        // newcomer.
        match store.get(&id).await {
            Ok(Some(_)) | Err(_) => continue,
            Ok(None) => {}
        }
        if super::dispose_observer(deps, &id).await {
            retired += 1;
            tracing::info!(observer = %id, "disposed a delivery observer with no binding record");
        }
    }
    retired
}

fn is_orphan(observer_id: &str, stored: &HashSet<String>) -> bool {
    !stored.contains(observer_id)
}

/// Every session's bindings, dropped when the session is deleted.
pub async fn sweep_owner(deps: &Deps, session_id: &str) -> usize {
    let store = deps.bindings().await;
    let Ok(bindings) = store.list_for_owner(session_id).await else {
        return 0;
    };
    let mut dropped = 0usize;
    for binding in bindings {
        if binding.observer_id.is_some() {
            super::dispose_observer(deps, &binding.id).await;
        }
        if store.delete(&binding.id).await.is_ok() {
            dropped += 1;
        }
    }
    dropped
}

async fn owner_state(deps: &Deps, session_id: &str) -> OwnerState {
    let resp = deps
        .iii
        .call(iii_sdk::protocol::CallRequest {
            function_id: "session::get".to_string(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(deps.cfg().await.session_timeout_ms),
        })
        .await;
    match resp {
        Ok(v) if v.get("meta").is_some() => OwnerState::Live,
        Ok(_) => OwnerState::Gone,
        Err(_) => OwnerState::Unknown,
    }
}

/// The engine builtin listing observers.
const OBSERVERS_LIST_ID: &str = "engine::observers::list";

/// Ids of the observers aimed at the delivery hop that a harness created.
/// `None` when the listing failed: reconciliation then leaves them alone.
async fn delivery_observer_ids(deps: &Deps) -> Option<Vec<String>> {
    let resp = deps
        .iii
        .call(iii_sdk::protocol::CallRequest {
            function_id: OBSERVERS_LIST_ID.to_string(),
            payload: json!({ "worker": "harness" }),
            action: None,
            timeout_ms: Some(deps.cfg().await.dispatch_timeout_ms),
        })
        .await
        .ok()?;
    Some(delivery_targets(&resp))
}

/// The ids of a listing's observers whose target is the delivery hop.
fn delivery_targets(listing: &Value) -> Vec<String> {
    listing
        .get("observers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|observer| {
            observer
                .get("target")
                .and_then(|target| target.get("function_id"))
                .and_then(Value::as_str)
                == Some(crate::functions::observer_deliver::DELIVER_ID)
        })
        .filter_map(|observer| {
            observer
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_provably_gone_owner_is_swept() {
        assert!(should_gc(OwnerState::Gone));
        assert!(!should_gc(OwnerState::Live));
        // The one that matters: a lookup that failed must NOT drop a binding.
        assert!(!should_gc(OwnerState::Unknown));
    }

    #[test]
    fn only_delivery_observers_without_a_record_are_orphans() {
        let stored = HashSet::from(["sub_live".to_string()]);
        assert!(!is_orphan("sub_live", &stored));
        assert!(is_orphan("sub_missing", &stored));
    }

    #[test]
    fn the_orphan_scan_reads_only_observers_of_the_delivery_hop() {
        let listing = json!({ "observers": [
            { "id": "sub_a", "target": { "function_id": crate::functions::observer_deliver::DELIVER_ID, "namespace": "default" } },
            { "id": "own", "target": { "function_id": "harness::on-engine-change", "namespace": "default" } },
            { "id": "sub_b", "target": { "function_id": crate::functions::observer_deliver::DELIVER_ID, "namespace": "default" } },
        ]});
        assert_eq!(delivery_targets(&listing), vec!["sub_a", "sub_b"]);
        assert!(delivery_targets(&json!({})).is_empty());
    }

    fn wake_binding(filters: Option<Value>) -> crate::bindings::Binding {
        crate::bindings::Binding {
            id: "sub_r".into(),
            observer_id: Some("sub_r".into()),
            source: Some(crate::bindings::SourceSpec {
                function_id: crate::bindings::STATE_ON_CHANGED_FN.into(),
                namespace: None,
                args: None,
                filters,
            }),
            owner: crate::bindings::OwnerScope {
                session_id: "s1".into(),
                root_session_id: None,
            },
            target: crate::bindings::BindingTarget::new(crate::functions::SEND_ID),
            conditions: Vec::new(),
            lifecycle: crate::bindings::Lifecycle {
                once: true,
                max_fires: None,
                expires_at: None,
            },
            capability: None,
            dedup_key: None,
            fires: 0,
            created_at: 0,
        }
    }

    fn scope_key(scope: &str, key: Option<&str>) -> Option<Value> {
        let mut filters = json!({ "input.scope": { "$eq": scope } });
        if let Some(key) = key {
            filters["input.key"] = json!({ "$eq": key });
        }
        Some(filters)
    }

    /// The replay catch-up reads exactly the console-04e02cb7 shape: a
    /// one-shot `state::on-changed` wake on a scope/key, never fired.
    #[test]
    fn catchup_names_the_watched_key_for_unfired_one_shot_state_wakes() {
        let b = wake_binding(scope_key("calc-delivery", Some("plan")));
        assert_eq!(
            state_wake_catchup(&b),
            Some(("calc-delivery".to_string(), "plan".to_string()))
        );
    }

    #[test]
    fn catchup_skips_fired_standing_keyless_and_non_state_bindings() {
        let fired = crate::bindings::Binding {
            fires: 1,
            ..wake_binding(scope_key("s", Some("k")))
        };
        assert_eq!(state_wake_catchup(&fired), None);

        let standing = crate::bindings::Binding {
            lifecycle: crate::bindings::Lifecycle {
                once: false,
                max_fires: None,
                expires_at: None,
            },
            ..wake_binding(scope_key("s", Some("k")))
        };
        assert_eq!(state_wake_catchup(&standing), None);

        let keyless = wake_binding(scope_key("s", None));
        assert_eq!(state_wake_catchup(&keyless), None);

        let mut cron = wake_binding(None);
        cron.source = Some(crate::bindings::SourceSpec {
            function_id: "cron::on-tick".into(),
            namespace: None,
            args: Some(json!({ "expression": "0 * * * * *" })),
            filters: None,
        });
        assert_eq!(state_wake_catchup(&cron), None);
    }
}
