//! Binding garbage collection.
//!
//! The durable store survives harness restarts, but the configured state
//! adapter may not. Startup therefore reconciles engine delivery triggers
//! against their binding records, plus the ordinary lifecycle sweeps below:
//!
//! * **Owner gone.** A deleted session leaves bindings that would keep firing
//!   — waking a session that no longer exists.
//! * **Stale spawn targets.** Records whose target is `harness::spawn` predate
//!   the spawn-target removal: trigger delivery never creates an agent now, so
//!   they are retired LOUDLY — the owner gets a notification naming the
//!   migration (register a wake, spawn directly) — never adapted.
//! * **Legacy targets.** Engine bindings registered before the delivery hop
//!   point at `harness::notify_agent` / `harness::trigger-call` (or straight at
//!   `harness::spawn`). Those paths are gone, so the bindings can only fire
//!   into nothing. They are unregistered rather than adapted: a shim would
//!   have to trust the very metadata the new model exists to stop reading.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::deps::Deps;

/// The engine-side targets that no longer have a handler. A binding pointing
/// at one of these predates the delivery hop.
const LEGACY_TARGETS: [&str; 2] = ["harness::notify_agent", "harness::trigger-call"];

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

/// Startup sweep: drop bindings whose owner is provably gone, loudly retire
/// records that still target `harness::spawn`, unregister delivery triggers
/// whose record disappeared, and remove bindings aimed at old handlers.
pub async fn run(deps: &Deps) {
    let store = deps.bindings().await;
    let bindings = match store.list().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "binding sweep skipped: store unreadable");
            return;
        }
    };

    let orphans = reconcile_orphan_delivery_triggers(deps, &bindings).await;

    let mut dropped = 0usize;
    let mut stale_spawn = 0usize;
    for binding in bindings {
        let owner_gone = should_gc(owner_state(deps, &binding.owner.session_id).await);
        if owner_gone {
            if let Some(trigger_id) = binding.trigger_id.as_deref() {
                crate::functions::subscribe::unregister_engine_trigger(deps, trigger_id).await;
            }
            if store.delete(&binding.id).await.is_ok() {
                dropped += 1;
            }
            continue;
        }
        if binding.target.function_id == crate::functions::SPAWN_ID {
            retire_stale_spawn_binding(deps, &binding).await;
            stale_spawn += 1;
        }
    }

    let legacy = retire_legacy(deps).await + retire_engine_spawn_triggers(deps).await;
    if dropped > 0 || stale_spawn > 0 || legacy > 0 || orphans > 0 {
        tracing::info!(
            owner_gone = dropped,
            stale_spawn = stale_spawn,
            legacy = legacy,
            orphan_engine = orphans,
            "swept trigger bindings at startup"
        );
    }
}

/// Tear down a stored binding that still targets `harness::spawn` — a record
/// from before the spawn-target removal. Loud on the owner side: a silent
/// unregister would leave the owner believing in an armed reaction it no
/// longer has. Called from the startup sweep (mass-retire) and from delivery
/// resolve (the race backstop for a fire already in flight).
pub async fn retire_stale_spawn_binding(deps: &Deps, binding: &crate::bindings::Binding) {
    if let Some(trigger_id) = binding.trigger_id.as_deref() {
        crate::functions::subscribe::unregister_engine_trigger(deps, trigger_id).await;
    }
    if let Err(e) = deps.bindings().await.delete(&binding.id).await {
        tracing::warn!(binding = %binding.id, error = %e, "stale spawn-binding delete failed");
    }
    let watch = binding
        .trigger_watch()
        .map(|(ty, cfg)| format!(" (was watching `{ty}` {cfg})"))
        .unwrap_or_default();
    let notice = format!(
        "[notification] binding {}{watch} was RETIRED: `harness::spawn` is no longer a binding \
         target — trigger delivery never creates an agent. Re-register the same trigger as a \
         wake (omit `function_id`) and spawn workers directly from the woken turn.",
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
        &format!("e_trigstale_{}", binding.id),
        crate::subscriptions::fired::TriggerFired {
            subscription_id: &binding.id,
            trigger_id: binding.trigger_id.as_deref(),
            target: &binding.target.function_id,
            label: None,
            once: binding.lifecycle.once,
            retired: true,
            scope: None,
            key: None,
            note: Some("harness::spawn is no longer a binding target; the binding was retired"),
            payload: None,
            fired_at: crate::subscriptions::fired::now_ms(),
        },
    )
    .await;
}

/// Unregister engine triggers aimed DIRECTLY at `harness::spawn` — worker
/// registrations from before the delivery hop (agent registrations always
/// pointed at the hop). No owner is knowable engine-side, so this one is
/// log-loud only.
async fn retire_engine_spawn_triggers(deps: &Deps) -> usize {
    let mut retired = 0usize;
    for id in list_binding_ids(deps, crate::functions::SPAWN_ID)
        .await
        .unwrap_or_default()
    {
        if crate::functions::subscribe::unregister_engine_trigger(deps, &id).await {
            retired += 1;
            tracing::info!(
                trigger_id = %id,
                "dropped an engine trigger aimed at harness::spawn — trigger delivery never \
                 creates an agent"
            );
        }
    }
    retired
}

/// Unregister every engine binding still aimed at a handler this harness no
/// longer serves. Best-effort and idempotent — a failed unregister is retried
/// on the next startup.
async fn retire_legacy(deps: &Deps) -> usize {
    let mut retired = 0usize;
    for target in LEGACY_TARGETS {
        for id in list_binding_ids(deps, target).await.unwrap_or_default() {
            if crate::functions::subscribe::unregister_engine_trigger(deps, &id).await {
                retired += 1;
                tracing::info!(
                    trigger_id = %id,
                    target = %target,
                    "dropped a pre-delivery-hop binding — its handler no longer exists"
                );
            }
        }
    }
    retired
}

/// Engine providers outlive an in-memory state adapter restart. Startup and
/// the periodic expiry pass both remove delivery triggers whose trusted
/// `__binding` pointer no longer resolves, so recurring providers do not
/// invoke a permanent no-op forever.
pub async fn reconcile_orphan_delivery_triggers(
    deps: &Deps,
    bindings: &[crate::bindings::Binding],
) -> usize {
    let stored: HashSet<String> = bindings.iter().map(|binding| binding.id.clone()).collect();
    let Some(ids) = list_binding_ids(deps, crate::functions::trigger_deliver::DELIVER_ID).await
    else {
        return 0;
    };
    let store = deps.bindings().await;
    let mut retired = 0usize;
    for id in ids {
        let Some(binding_id) = delivery_binding_id(deps, &id).await else {
            // An unreadable detail response is not proof of orphanhood.
            continue;
        };
        if !is_orphan(binding_id.as_deref(), &stored) {
            continue;
        }
        // The listing is a snapshot. A registration may insert its record
        // after that snapshot and then arm the engine trigger before this
        // loop reaches it; a fresh read prevents us from deleting that valid
        // newcomer.
        if let Some(binding_id) = binding_id.as_deref() {
            match store.get(binding_id).await {
                Ok(Some(_)) | Err(_) => continue,
                Ok(None) => {}
            }
        }
        if crate::functions::subscribe::unregister_engine_trigger(deps, &id).await {
            retired += 1;
            tracing::info!(
                trigger_id = %id,
                binding = binding_id.as_deref().unwrap_or("<missing>"),
                "dropped delivery trigger with no binding record"
            );
        }
    }
    retired
}

fn is_orphan(binding_id: Option<&str>, stored: &HashSet<String>) -> bool {
    binding_id.is_none_or(|id| !stored.contains(id))
}

/// Every session's bindings, dropped when the session is deleted.
pub async fn sweep_owner(deps: &Deps, session_id: &str) -> usize {
    let store = deps.bindings().await;
    let Ok(bindings) = store.list_for_owner(session_id).await else {
        return 0;
    };
    let mut dropped = 0usize;
    for binding in bindings {
        if let Some(trigger_id) = binding.trigger_id.as_deref() {
            crate::functions::subscribe::unregister_engine_trigger(deps, trigger_id).await;
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
        .trigger(iii_sdk::protocol::TriggerRequest {
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

/// Engine-side binding ids currently aimed at a function.
async fn list_binding_ids(deps: &Deps, function_id: &str) -> Option<Vec<String>> {
    let resp = deps
        .iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "engine::registered-triggers::list".to_string(),
            payload: json!({ "function_id": function_id, "include_internal": true }),
            action: None,
            timeout_ms: Some(deps.cfg().await.dispatch_timeout_ms),
        })
        .await
        .ok()?;
    Some(
        resp.get("registered_triggers")?
            .as_array()?
            .iter()
            .filter_map(|t| t.get("id").and_then(Value::as_str).map(str::to_string))
            .collect(),
    )
}

/// `list` returns summaries without metadata; fetch the detail to read the
/// trusted pointer. `Some(None)` means the detail was readable but had no
/// pointer. `None` means lookup failed, so reconciliation leaves it alone.
async fn delivery_binding_id(deps: &Deps, trigger_id: &str) -> Option<Option<String>> {
    let resp = deps
        .iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "engine::registered-triggers::info".to_string(),
            payload: json!({ "id": trigger_id, "include_internal": true }),
            action: None,
            timeout_ms: Some(deps.cfg().await.dispatch_timeout_ms),
        })
        .await
        .ok()?;
    let detail = resp.get("registered_trigger").unwrap_or(&resp);
    Some(
        detail
            .get("metadata")
            .and_then(|metadata| metadata.get("__binding"))
            .and_then(Value::as_str)
            .map(str::to_string),
    )
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
    fn legacy_targets_name_the_removed_handlers() {
        assert!(LEGACY_TARGETS.contains(&"harness::notify_agent"));
        assert!(LEGACY_TARGETS.contains(&"harness::trigger-call"));
        // The live hop must never be swept as legacy.
        assert!(!LEGACY_TARGETS.contains(&crate::functions::trigger_deliver::DELIVER_ID));
    }

    #[test]
    fn only_delivery_pointers_without_a_record_are_orphans() {
        let stored = HashSet::from(["sub_live".to_string()]);
        assert!(!is_orphan(Some("sub_live"), &stored));
        assert!(is_orphan(Some("sub_missing"), &stored));
        assert!(is_orphan(None, &stored));
    }
}
