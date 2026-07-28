//! `harness::triggers::list` / `harness::triggers::unregister` — the read and
//! teardown surface over the durable binding store (the console's data source,
//! and any owner's introspection).
//!
//! The engine's own `registered-triggers::list` cannot serve this: every
//! agent binding registers as the internal delivery hop with metadata that is
//! only a `__binding` pointer, so the engine knows neither the owner nor the
//! shape. The store is the authority — these functions expose it as data:
//! trigger type, config, delivery target (absent = notify), conditions,
//! lifecycle, fire count. Nothing here interprets a source type, which is
//! what keeps the console compatible with trigger sources that do not exist
//! yet.
//!
//! Teardown goes through `unregister` rather than a raw engine
//! `unregister_trigger`: the engine-side removal alone would orphan the
//! durable record, leaving `expects_wake` armed forever on a wake that can no
//! longer fire.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bindings::{Binding, ConditionSpec};
use crate::deps::Deps;
use crate::error::HarnessError;

pub const TRIGGERS_LIST_ID: &str = "harness::triggers::list";
pub const TRIGGERS_LIST_DESC: &str =
    "Read-only: the trigger bindings a session owns (durable records) — subscription id, \
     trigger type/config, delivery target (absent = notifies the owner), conditions, \
     lifecycle, and fire count.";

pub const TRIGGERS_UNREGISTER_ID: &str = "harness::triggers::unregister";
pub const TRIGGERS_UNREGISTER_DESC: &str =
    "Tear down one trigger binding by subscription id: unregister the engine trigger AND \
     delete the durable record (an engine-side unregister alone strands the owner's armed \
     wake). `session_id` must name the binding's owner. A still-armed wake torn down this \
     way notifies its parked owner.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TriggersListRequest {
    /// The owning session whose bindings to list.
    pub session_id: String,
}

/// One binding, as data. `trigger_type`/`config` are read from the
/// canonicalised registration request and absent on records that predate it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TriggerRow {
    pub subscription_id: String,
    /// The engine's own trigger id (absent only in the brief window before
    /// the engine acknowledged the registration).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    /// The function a fire calls. Absent for a wake — the fire notifies the
    /// owner session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<ConditionSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub once: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fires: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    pub fires: u64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TriggersListResponse {
    pub subscriptions: Vec<TriggerRow>,
}

pub async fn list(
    deps: &Deps,
    req: TriggersListRequest,
) -> Result<TriggersListResponse, HarnessError> {
    let store = deps.bindings().await;
    let mut bindings = store.list_for_owner(&req.session_id).await?;
    bindings.sort_by_key(|b| b.created_at);
    Ok(TriggersListResponse {
        subscriptions: bindings.iter().map(row_from).collect(),
    })
}

/// Project a stored binding into its wire row. Pure — the mapping IS the
/// contract the console renders from.
fn row_from(binding: &Binding) -> TriggerRow {
    let (trigger_type, config) = binding
        .trigger_watch()
        .map(|(ty, cfg)| (Some(ty.to_string()), Some(cfg.clone())))
        .unwrap_or((None, None));
    let target = (binding.target.function_id != crate::functions::SEND_ID)
        .then(|| binding.target.function_id.clone());
    let label = binding
        .dedup_key
        .as_ref()
        .and_then(|k| k.get("label"))
        .and_then(Value::as_str)
        .or_else(|| {
            binding
                .target
                .payload
                .as_ref()
                .and_then(|p| p.get("label"))
                .and_then(Value::as_str)
        })
        .map(str::to_string);
    TriggerRow {
        subscription_id: binding.id.clone(),
        trigger_id: binding.trigger_id.clone(),
        trigger_type,
        config,
        target,
        conditions: binding.conditions.clone(),
        label,
        once: binding.lifecycle.once,
        max_fires: binding.lifecycle.max_fires,
        expires_at: binding.lifecycle.expires_at,
        fires: binding.fires,
        created_at: binding.created_at,
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TriggersUnregisterRequest {
    pub subscription_id: String,
    /// The binding's owner session — a correctness handshake, checked against
    /// the record. (Deployment permissions keep this function off the
    /// unattended agent surface; the console's privileged path supplies the
    /// owner it just listed.)
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TriggersUnregisterResponse {
    /// False when no record existed (already retired) — honest, not an error.
    pub removed: bool,
}

pub async fn unregister(
    deps: &Deps,
    req: TriggersUnregisterRequest,
) -> Result<TriggersUnregisterResponse, HarnessError> {
    let store = deps.bindings().await;
    let Some(binding) = store.get(&req.subscription_id).await? else {
        return Ok(TriggersUnregisterResponse { removed: false });
    };
    if binding.owner.session_id != req.session_id {
        return Err(HarnessError::InvalidRequest(format!(
            "subscription {} belongs to session {}, not {}",
            binding.id, binding.owner.session_id, req.session_id
        )));
    }
    let was_armed_wake = crate::bindings::expiry::is_unfired_wake(&binding);
    if let Some(trigger_id) = binding.trigger_id.as_deref() {
        crate::functions::subscribe::unregister_engine_trigger(deps, trigger_id).await;
    }
    store.delete(&binding.id).await?;
    // Tearing down a still-armed wake from outside the owner's own turn
    // strands the parked owner silently — tell it, same as an expiry would.
    if was_armed_wake {
        crate::bindings::expiry::notify_wake_lost(
            deps,
            &binding,
            crate::bindings::expiry::WakeLost::Unregistered {
                by: "harness::triggers::unregister",
            },
        )
        .await;
    }
    Ok(TriggersUnregisterResponse { removed: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{BindingTarget, Causation, Lifecycle, OwnerScope};
    use serde_json::json;

    fn binding(id: &str, target_fn: &str, created_at: i64) -> Binding {
        let mut target = BindingTarget::new(target_fn);
        if target_fn == crate::functions::SEND_ID {
            target.payload = Some(json!({ "session_id": "s_owner", "label": "wake-label" }));
        }
        Binding {
            id: id.into(),
            trigger_id: Some(format!("trg_{id}")),
            owner: OwnerScope {
                session_id: "s_owner".into(),
                root_session_id: None,
            },
            target,
            conditions: vec![],
            lifecycle: Lifecycle {
                once: true,
                max_fires: None,
                expires_at: Some(1_800_000_000_000),
            },
            capability: None,
            causation: Causation::default(),
            dedup_key: Some(json!({
                "trigger_type": "state",
                "config": { "scope": "run", "key": "done" },
                "label": "dedup-label",
            })),
            fires: 3,
            created_at,
        }
    }

    #[test]
    fn a_wake_row_has_no_target_and_reads_the_registration_shape() {
        let row = row_from(&binding("sub_a", crate::functions::SEND_ID, 10));
        assert_eq!(row.subscription_id, "sub_a");
        assert_eq!(row.trigger_id.as_deref(), Some("trg_sub_a"));
        assert_eq!(row.trigger_type.as_deref(), Some("state"));
        assert_eq!(row.config, Some(json!({ "scope": "run", "key": "done" })));
        assert!(
            row.target.is_none(),
            "a wake delivers to nobody but the owner"
        );
        assert_eq!(row.label.as_deref(), Some("dedup-label"));
        assert!(row.once);
        assert_eq!(row.expires_at, Some(1_800_000_000_000));
        assert_eq!(row.fires, 3);
    }

    #[test]
    fn a_call_row_names_its_target_and_tolerates_a_missing_dedup_key() {
        let mut b = binding("sub_b", "state::set", 20);
        b.dedup_key = None;
        let row = row_from(&b);
        assert_eq!(row.target.as_deref(), Some("state::set"));
        // Pre-dedup-key records still render — just without the watch shape.
        assert!(row.trigger_type.is_none());
        assert!(row.config.is_none());
        assert!(row.label.is_none());
    }

    #[test]
    fn a_wake_label_falls_back_to_the_target_payload() {
        let mut b = binding("sub_c", crate::functions::SEND_ID, 30);
        b.dedup_key = Some(json!({ "trigger_type": "timer", "config": { "at": 1 } }));
        let row = row_from(&b);
        assert_eq!(row.label.as_deref(), Some("wake-label"));
        assert_eq!(row.trigger_type.as_deref(), Some("timer"));
    }
}
