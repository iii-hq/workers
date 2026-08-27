//! `harness::triggers::list` / `harness::triggers::unregister` — the read and
//! teardown surface over the durable binding store (the console's data source,
//! and any owner's introspection).
//!
//! The engine's own `registered-triggers::list` cannot serve this: every
//! agent binding registers as the internal delivery hop with metadata that is
//! only a `__binding` pointer, so the engine knows neither the owner nor the
//! shape. The store is the authority — these functions expose it as data:
//! trigger type, config, delivery target (absent = notify), label/event action,
//! conditions, lifecycle, fire count. Nothing here interprets a source type, which is
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
use crate::subscriptions::fired;

pub const TRIGGERS_LIST_ID: &str = "harness::triggers::list";
pub const TRIGGERS_LIST_DESC: &str =
    "Read-only: the trigger bindings a session owns (durable records) — subscription id, \
     trigger type/config, delivery target (absent = notifies the owner), label/event \
     action, conditions, lifecycle, and fire count. In-turn agent calls may omit \
     `session_id` (defaults to the calling session).";

pub const TRIGGERS_UNREGISTER_ID: &str = "harness::triggers::unregister";
pub const TRIGGERS_UNREGISTER_DESC: &str =
    "Tear down one trigger binding by subscription id: unregister the engine trigger AND \
     delete the durable record (an engine-side unregister alone strands the owner's armed \
     wake). `session_id` must name the binding's owner; in-turn agent calls may omit it \
     (defaults to the calling session). A still-armed wake torn down this way notifies \
     its parked owner.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TriggersListRequest {
    /// The owning session whose bindings to list. In-turn agent calls may
    /// omit it — the harness injects the calling session. External callers
    /// (console, CLI) must name the owner explicitly.
    #[serde(default)]
    pub session_id: Option<String>,
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
    /// Human-readable event text declared as `metadata.action`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
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

/// In-turn calls arrive with the caller's session injected at the dispatch
/// chokepoint; a still-absent id means an external caller that must name the
/// owner itself.
fn required_session_id(session_id: Option<&str>) -> Result<&str, HarnessError> {
    session_id.ok_or_else(|| {
        HarnessError::InvalidRequest(
            "session_id is required outside a turn — name the owning session".into(),
        )
    })
}

pub async fn list(
    deps: &Deps,
    req: TriggersListRequest,
) -> Result<TriggersListResponse, HarnessError> {
    let session_id = required_session_id(req.session_id.as_deref())?;
    let store = deps.bindings().await;
    let mut bindings = store.list_for_owner(session_id).await?;
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
    let label = binding_label(binding).map(str::to_string);
    TriggerRow {
        subscription_id: binding.id.clone(),
        trigger_id: binding.trigger_id.clone(),
        trigger_type,
        config,
        target,
        conditions: binding.conditions.clone(),
        label,
        action: binding.event_action().map(str::to_string),
        once: binding.lifecycle.once,
        max_fires: binding.lifecycle.max_fires,
        expires_at: binding.lifecycle.expires_at,
        fires: binding.fires,
        created_at: binding.created_at,
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TriggersUnregisterRequest {
    /// `id` accepted as an alias: the sibling teardown contract
    /// (`engine::unregister_trigger`) calls this field `id`, and models carry
    /// that name over — verify-wake-fix-1 postmortem: the first unregister of
    /// the run failed on a raw serde "missing field" for exactly this.
    #[serde(alias = "id")]
    pub subscription_id: String,
    /// The binding's owner session — a correctness handshake, checked against
    /// the record. In-turn agent calls may omit it (the harness injects the
    /// calling session). The console's privileged path supplies the owner it
    /// just listed.
    #[serde(default)]
    pub session_id: Option<String>,
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
    let session_id = required_session_id(req.session_id.as_deref())?;
    let store = deps.bindings().await;
    let Some(binding) = store.get(&req.subscription_id).await? else {
        return Ok(TriggersUnregisterResponse { removed: false });
    };
    if binding.owner.session_id != session_id {
        return Err(HarnessError::InvalidRequest(format!(
            "subscription {} belongs to session {}, not {}",
            binding.id, binding.owner.session_id, session_id
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
    } else {
        record_unregistered(deps, &binding).await;
    }
    Ok(TriggersUnregisterResponse { removed: true })
}

/// Persist the explicit retirement for bindings that do not warrant waking a
/// parked session. The armed, unfired wake branch above keeps its existing
/// notification + custom-record pair; every other shape gets only this
/// model-invisible lifecycle record.
async fn record_unregistered(deps: &Deps, binding: &Binding) {
    fired::emit(
        &deps.session().await,
        &binding.owner.session_id,
        &format!("e_trigunregistered_{}", binding.id),
        unregistered_record(binding, fired::now_ms()),
    )
    .await;
}

/// Pure wire-record builder kept separate so its source/config/count contract
/// can be pinned without a session-manager test double.
fn unregistered_record(binding: &Binding, fired_at: i64) -> fired::TriggerFired<'_> {
    fired::retirement_record(
        binding,
        fired::TriggerOutcome::Unregistered,
        fired::RetirementReason::Unregistered,
        Some("unregistered by harness::triggers::unregister"),
        fired_at,
    )
}

fn binding_label(binding: &Binding) -> Option<&str> {
    binding
        .dedup_key
        .as_ref()
        .and_then(|key| key.get("label"))
        .and_then(Value::as_str)
        .or_else(|| {
            binding
                .target
                .payload
                .as_ref()
                .and_then(|payload| payload.get("label"))
                .and_then(Value::as_str)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{BindingTarget, Causation, Lifecycle, OwnerScope};
    use serde_json::json;

    #[test]
    fn absent_session_id_errors_with_the_external_caller_message() {
        let err = required_session_id(None).unwrap_err();
        assert!(err.to_string().contains("required outside a turn"));
        assert_eq!(required_session_id(Some("s1")).unwrap(), "s1");
    }

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
                "metadata": { "action": "run completion received" },
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
        assert_eq!(row.action.as_deref(), Some("run completion received"));
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
        assert!(row.action.is_none());
    }

    #[test]
    fn a_wake_label_falls_back_to_the_target_payload() {
        let mut b = binding("sub_c", crate::functions::SEND_ID, 30);
        b.dedup_key = Some(json!({ "trigger_type": "timer", "config": { "at": 1 } }));
        let row = row_from(&b);
        assert_eq!(row.label.as_deref(), Some("wake-label"));
        assert_eq!(row.trigger_type.as_deref(), Some("timer"));
    }

    #[test]
    fn recurring_call_unregistration_builds_one_complete_custom_record() {
        let mut b = binding("sub_call", "state::set", 30);
        b.lifecycle.once = false;
        b.lifecycle.max_fires = Some(10);
        b.fires = 4;

        assert!(
            !crate::bindings::expiry::is_unfired_wake(&b),
            "a call must take the custom-only branch"
        );
        let record = serde_json::to_value(unregistered_record(&b, 42)).unwrap();
        assert_eq!(record["subscription_id"], "sub_call");
        assert_eq!(record["trigger_id"], "trg_sub_call");
        assert_eq!(record["target"], "state::set");
        assert_eq!(record["trigger_type"], "state");
        assert_eq!(record["config"], json!({ "scope": "run", "key": "done" }));
        assert_eq!(record["action"], "run completion received");
        assert_eq!(record["fires"], 4);
        assert_eq!(record["outcome"], "unregistered");
        assert_eq!(record["retirement_reason"], "unregistered");
        assert_eq!(record["retired"], true);
        assert_eq!(record["fired_at"], 42);
        assert!(record.get("payload").is_none());
    }

    /// `id` is the field name the sibling engine contract uses; both spellings
    /// must deserialize to the same request.
    #[test]
    fn unregister_accepts_id_alias_for_subscription_id() {
        let canonical: TriggersUnregisterRequest =
            serde_json::from_value(json!({ "subscription_id": "sub_a" })).unwrap();
        assert_eq!(canonical.subscription_id, "sub_a");
        let aliased: TriggersUnregisterRequest =
            serde_json::from_value(json!({ "id": "sub_a" })).unwrap();
        assert_eq!(aliased.subscription_id, "sub_a");
    }
}
