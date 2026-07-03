//! Startup GC for durable subscription bindings orphaned across restarts.
//!
//! Both `harness::react` and `harness::notify_agent` bindings are durable
//! engine-side (they survive the registering connection's disconnect), but the
//! session→binding bookkeeping lives only in the in-memory
//! [`SubscriptionRegistry`], which is wiped on every harness restart.
//!
//! - react bindings carry their whole spec in trigger metadata, so they keep
//!   FIRING after a restart; the leak is a deleted owner session leaving them
//!   spawning orphan sub-agents forever. GC rule: unregister when the stamped
//!   owner session ([`OWNER_SESSION_KEY`]) is definitely gone. Never GC on
//!   doubt — no stamp or an unreadable owner keeps the binding.
//! - notify bindings can only deliver through a live registry entry
//!   (`claim_fire` drops unknown subscription ids), so after a restart every
//!   pre-existing one is dead weight firing silent no-ops forever. GC rule:
//!   unregister any whose subscription id the local registry doesn't know. A
//!   racing fresh registration is safe: the interceptor inserts into the
//!   registry BEFORE dispatching the engine registration, so anything
//!   listable engine-side is already visible locally.

use serde_json::{json, Value};

use crate::deps::Deps;
use crate::subscriptions::{NOTIFY_AGENT_ID, OWNER_SESSION_KEY};

/// Whether the owning session is live, provably gone, or unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerState {
    Live,
    Gone,
    Unknown,
}

/// Pure decision: GC a binding only when it carries an owner stamp AND that
/// owner is definitely gone. Everything else is kept.
pub fn should_gc(owner: Option<&str>, state: OwnerState) -> bool {
    matches!((owner, state), (Some(_), OwnerState::Gone))
}

/// `session::get` returns a body with a `meta` field for a live session and a
/// null / meta-less body for a deleted one; a transport error is unknown.
fn owner_state_from_get(result: Result<Value, impl std::fmt::Display>) -> OwnerState {
    match result {
        Ok(v) if v.get("meta").is_some() => OwnerState::Live,
        Ok(_) => OwnerState::Gone,
        Err(_) => OwnerState::Unknown,
    }
}

/// Whether a listed notify binding is dead weight: no parseable subscription
/// id, or an id the local registry doesn't know (its entry died with a
/// restart and can never come back — subscriptions are ephemeral by design).
pub fn notify_should_gc(sub_id: Option<&str>, known_locally: bool) -> bool {
    match sub_id {
        None => true,
        Some(_) => !known_locally,
    }
}

/// Best-effort startup GC of both binding kinds: every step tolerates failure
/// and simply keeps the binding.
pub async fn run(deps: &Deps) {
    reconcile_react(deps).await;
    reconcile_notify(deps).await;
}

/// The engine trigger ids currently bound to `function_id`, or None when the
/// listing itself failed (callers skip the pass rather than guess).
async fn list_binding_ids(deps: &Deps, function_id: &str) -> Option<Vec<String>> {
    let engine = deps.engine().await;
    let list = match engine
        .dispatch(
            "engine::registered-triggers::list",
            json!({ "function_id": function_id }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(error = %e, function_id, "reconcile: list failed; skipping pass");
            return None;
        }
    };
    Some(
        list.get("registered_triggers")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// GC react bindings whose stamped owner session is gone.
async fn reconcile_react(deps: &Deps) {
    let engine = deps.engine().await;
    let Some(ids) = list_binding_ids(deps, crate::functions::react::REACT_ID).await else {
        return;
    };

    let mut gc = 0usize;
    for id in ids {
        let info = match engine
            .dispatch("engine::registered-triggers::info", json!({ "id": id }))
            .await
        {
            Ok(v) => v,
            Err(_) => continue,
        };
        let owner = info
            .get("metadata")
            .and_then(|m| m.get(OWNER_SESSION_KEY))
            .and_then(Value::as_str);
        let Some(owner) = owner else { continue };

        let state = owner_state_from_get(
            engine
                .dispatch("session::get", json!({ "session_id": owner }))
                .await,
        );
        if should_gc(Some(owner), state)
            && engine
                .dispatch("engine::unregister_trigger", json!({ "id": id }))
                .await
                .is_ok()
        {
            gc += 1;
            tracing::info!(
                binding = %id,
                owner_session = %owner,
                "react reconcile: unregistered a binding whose owner session is gone"
            );
        }
    }
    if gc > 0 {
        tracing::info!(count = gc, "react reconcile: GC'd orphaned react bindings on startup");
    }
}

/// The metadata key naming a binding's owning session, per target function:
/// the notify path injects the owner as plain `session_id`; the react path
/// stamps [`OWNER_SESSION_KEY`] (react's own `session_id` field means "spawn
/// into", not ownership — never match on it).
pub fn owner_key(function_id: &str) -> &'static str {
    if function_id == NOTIFY_AGENT_ID {
        "session_id"
    } else {
        OWNER_SESSION_KEY
    }
}

/// Immediately unregister every durable binding owned by `session_id` — the
/// engine-side complement of the registry's `take_session`, which after a
/// harness restart no longer knows the session's bindings. Called on
/// `session::deleted` so deleting a chat is a complete teardown of its wiring
/// without waiting for the next startup reconcile.
pub async fn sweep_owner(deps: &Deps, session_id: &str) -> usize {
    let engine = deps.engine().await;
    let mut swept = 0usize;
    for function_id in [crate::functions::react::REACT_ID, NOTIFY_AGENT_ID] {
        let Some(ids) = list_binding_ids(deps, function_id).await else {
            continue;
        };
        for id in ids {
            let info = match engine
                .dispatch("engine::registered-triggers::info", json!({ "id": id }))
                .await
            {
                Ok(v) => v,
                Err(_) => continue,
            };
            let owned = info
                .pointer(&format!("/metadata/{}", owner_key(function_id)))
                .and_then(Value::as_str)
                == Some(session_id);
            if owned
                && engine
                    .dispatch("engine::unregister_trigger", json!({ "id": id }))
                    .await
                    .is_ok()
            {
                swept += 1;
            }
        }
    }
    if swept > 0 {
        tracing::info!(
            session_id,
            count = swept,
            "session deleted: swept durable bindings by owner stamp"
        );
    }
    swept
}

/// GC notify bindings whose subscription id the local registry doesn't know.
async fn reconcile_notify(deps: &Deps) {
    let engine = deps.engine().await;
    let Some(ids) = list_binding_ids(deps, NOTIFY_AGENT_ID).await else {
        return;
    };

    let mut gc = 0usize;
    for id in ids {
        let info = match engine
            .dispatch("engine::registered-triggers::info", json!({ "id": id }))
            .await
        {
            Ok(v) => v,
            Err(_) => continue,
        };
        let sub_id = info
            .pointer("/metadata/subscription_id")
            .and_then(Value::as_str);
        let known = sub_id
            .map(|s| deps.subscriptions.session_of(s).is_some())
            .unwrap_or(false);
        if notify_should_gc(sub_id, known)
            && engine
                .dispatch("engine::unregister_trigger", json!({ "id": id }))
                .await
                .is_ok()
        {
            gc += 1;
            tracing::info!(
                binding = %id,
                subscription = sub_id.unwrap_or("<none>"),
                "notify reconcile: unregistered a binding with no live registry entry"
            );
        }
    }
    if gc > 0 {
        tracing::info!(count = gc, "notify reconcile: GC'd dead notify bindings on startup");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc_only_a_stamped_owner_that_is_gone() {
        assert!(should_gc(Some("s1"), OwnerState::Gone));
        // Live owner, unknown lookup, and no stamp are all kept.
        assert!(!should_gc(Some("s1"), OwnerState::Live));
        assert!(!should_gc(Some("s1"), OwnerState::Unknown));
        assert!(!should_gc(None, OwnerState::Gone));
        assert!(!should_gc(None, OwnerState::Live));
    }

    #[test]
    fn owner_key_is_per_target_function() {
        use crate::subscriptions::NOTIFY_AGENT_ID;
        assert_eq!(owner_key(NOTIFY_AGENT_ID), "session_id");
        // react (and anything else) matches only the explicit owner stamp —
        // react's own `session_id` field means "spawn into", not ownership.
        assert_eq!(owner_key(crate::functions::react::REACT_ID), OWNER_SESSION_KEY);
    }

    #[test]
    fn notify_gc_spares_only_locally_known_subscriptions() {
        // Known locally → live subscription, keep.
        assert!(!notify_should_gc(Some("sub_1"), true));
        // Unknown locally → registry entry died with a restart, dead weight.
        assert!(notify_should_gc(Some("sub_1"), false));
        // No parseable subscription id → can never deliver.
        assert!(notify_should_gc(None, false));
        assert!(notify_should_gc(None, true));
    }

    #[test]
    fn owner_state_reads_meta_presence() {
        assert_eq!(
            owner_state_from_get(Ok::<_, String>(json!({ "meta": { "created_at": 1 } }))),
            OwnerState::Live
        );
        assert_eq!(
            owner_state_from_get(Ok::<_, String>(Value::Null)),
            OwnerState::Gone
        );
        assert_eq!(
            owner_state_from_get(Ok::<_, String>(json!({}))),
            OwnerState::Gone
        );
        assert_eq!(
            owner_state_from_get(Err::<Value, _>("timeout".to_string())),
            OwnerState::Unknown
        );
    }
}
