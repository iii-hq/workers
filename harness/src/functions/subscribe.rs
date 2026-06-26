//! `harness::subscribe` — register an ephemeral iii trigger and be notified
//! when it fires, instead of polling (harness.md § Subscriptions). See
//! [`crate::subscriptions`] for the routing / injection design.

use std::sync::Arc;

use iii_sdk::RegisterTriggerInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::subscriptions::{self, handler, registry::SubEntry};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SubscribeRequest {
    /// The owning session. Injected by the harness from the calling turn; a
    /// model-supplied value is overridden (you cannot subscribe for another
    /// session).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The iii trigger type to listen on: `cron`, `state`, `stream`, or another
    /// worker's custom trigger type (e.g. `approval::pending-resolved`). For an
    /// ad-hoc signal, subscribe to `state` on a key and have the signaller call
    /// `state::set` on it (no dedicated emit needed — the engine fans the trigger
    /// out to every subscriber).
    pub trigger_type: String,
    /// The trigger config, passed verbatim to the engine — e.g.
    /// `{ "expression": "0 */5 * * * *" }` for cron, or a `state` scope/key.
    #[serde(default)]
    pub config: Value,
    /// A short human label echoed back in the notification text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Auto-unsubscribe after the first delivered notification. Defaults to true
    /// for one-shot-ish types (state / stream / custom trigger types), false for
    /// recurring `cron`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeResponse {
    pub subscription_id: String,
    /// The effective `once` flag applied (after the per-type default).
    pub once: bool,
}

/// Whether a trigger type recurs by nature (so `once` defaults to false).
fn defaults_recurring(trigger_type: &str) -> bool {
    trigger_type == "cron"
}

pub async fn handle(deps: &Deps, req: SubscribeRequest) -> Result<SubscribeResponse, HarnessError> {
    let session_id = req.session_id.clone().ok_or_else(|| {
        HarnessError::InvalidRequest(
            "harness::subscribe requires an owning session (the harness injects it from the \
             calling turn; it cannot be called out-of-band)"
                .to_string(),
        )
    })?;

    if subscriptions::is_forbidden_trigger_type(&req.trigger_type) {
        return Err(HarnessError::InvalidRequest(format!(
            "cannot bind harness-internal trigger type `{}` (self-notification guard)",
            req.trigger_type
        )));
    }

    let active = deps.subscriptions.count_for(&session_id);
    if active >= subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION {
        return Err(HarnessError::InvalidRequest(format!(
            "subscription cap reached ({} active for this session); unsubscribe first",
            subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION
        )));
    }

    let once = req.once.unwrap_or(!defaults_recurring(&req.trigger_type));

    let sub_id = format!("sub_{}", uuid::Uuid::new_v4().simple());
    let fn_id = subscriptions::internal_fn_id(&sub_id);

    // The handler closure outlives this call, so it owns its own `Deps` (cheap,
    // Arc-backed clone — shares the same engine, config cell, and registry).
    let deps_arc = Arc::new(deps.clone());
    let function = handler::register_subscription_handler(deps_arc, sub_id.clone(), &fn_id);

    // Insert BEFORE binding the trigger so an immediate fire still finds the
    // entry (the trigger handle is attached right after).
    let entry = Arc::new(SubEntry {
        id: sub_id.clone(),
        session_id: session_id.clone(),
        trigger_type: req.trigger_type.clone(),
        label: req.label.clone(),
        once,
        fire_count: Default::default(),
        function,
        trigger: std::sync::Mutex::new(None),
    });
    deps.subscriptions.insert(entry);

    match deps.iii.register_trigger(RegisterTriggerInput {
        trigger_type: req.trigger_type.clone(),
        function_id: fn_id,
        config: req.config.clone(),
        metadata: Some(json!({
            "subscription_id": sub_id,
            "session_id": session_id,
            "label": req.label,
        })),
    }) {
        Ok(trigger) => deps.subscriptions.set_trigger(&sub_id, trigger),
        Err(e) => {
            // Unwind: remove() unregisters the function we just created.
            deps.subscriptions.remove(&sub_id);
            return Err(HarnessError::Dependency(format!(
                "register_trigger `{}`: {e}",
                req.trigger_type
            )));
        }
    }

    Ok(SubscribeResponse {
        subscription_id: sub_id,
        once,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_defaults_one_shot_for_non_recurring() {
        assert!(!defaults_recurring("state"));
        assert!(!defaults_recurring("approval::pending-resolved"));
        assert!(defaults_recurring("cron"));
    }

    #[test]
    fn forbids_binding_harness_internal_trigger_types() {
        assert!(subscriptions::is_forbidden_trigger_type(
            "harness::turn-completed"
        ));
        assert!(!subscriptions::is_forbidden_trigger_type("state"));
        assert!(!subscriptions::is_forbidden_trigger_type("cron"));
    }
}
