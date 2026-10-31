//! `harness::observers::list` / `harness::observers::dispose` — the read and
//! teardown surface over the durable binding store (the console's data source,
//! and any owner's introspection).
//!
//! `engine::observers::list` cannot serve this: every agent observer targets
//! the internal delivery hop and the engine never reports its metadata, so it
//! knows neither the owner nor the delivery shape. The store is the authority
//! — these functions expose it as data: the observed function, its args and
//! filters, the delivery target (absent = notify), label/event action,
//! conditions, lifecycle, fire count. Nothing here interprets a source, which
//! is what keeps the console compatible with functions that do not exist yet.
//!
//! Teardown goes through `dispose` rather than `engine::observers::dispose`:
//! the engine-side removal alone would orphan the durable record, leaving
//! `expects_wake` armed forever on a wake that can no longer fire.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bindings::{Binding, ConditionSpec};
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::subscriptions::fired;

pub const OBSERVERS_LIST_ID: &str = "harness::observers::list";
pub const OBSERVERS_LIST_DESC: &str =
    "List the observers a session created with harness::observe: observer id, observed \
     function with its args and filters, target (absent = notifies the owner), label, \
     conditions, lifecycle, and fire count. In-turn calls may omit `session_id`.";

pub const OBSERVERS_DISPOSE_ID: &str = "harness::observers::dispose";
pub const OBSERVERS_DISPOSE_DESC: &str =
    "Remove one observer a session created, by observer id (the engine observer and the \
     durable record); a still-armed wake notifies its parked owner. `session_id` must name \
     the owner; in-turn calls may omit it.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ObserversListRequest {
    /// Owner session whose observers to list; in-turn calls may omit it (the
    /// calling session is injected).
    #[serde(default)]
    pub session_id: Option<String>,
}

/// One binding, as data. `function_id`, `args` and `filters` are what its
/// observer observes.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ObserverRow {
    pub observer_id: String,
    /// The observed function.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    /// Where the observed function is hosted, when the observer names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<Value>,
    /// The function a delivery calls. Absent for a wake — the delivery
    /// notifies the owner session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<ConditionSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Human-readable event text declared as `action`.
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
pub struct ObserversListResponse {
    pub observers: Vec<ObserverRow>,
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
    req: ObserversListRequest,
) -> Result<ObserversListResponse, HarnessError> {
    let session_id = required_session_id(req.session_id.as_deref())?;
    let store = deps.bindings().await;
    let mut bindings = store.list_for_owner(session_id).await?;
    bindings.sort_by_key(|b| b.created_at);
    Ok(ObserversListResponse {
        observers: bindings.iter().map(row_from).collect(),
    })
}

/// Project a stored binding into its wire row. Pure — the mapping IS the
/// contract the console renders from.
fn row_from(binding: &Binding) -> ObserverRow {
    let source = binding.source.as_ref();
    let target = (binding.target.function_id != crate::functions::SEND_ID)
        .then(|| binding.target.function_id.clone());
    let label = binding_label(binding).map(str::to_string);
    ObserverRow {
        observer_id: binding.id.clone(),
        function_id: source.map(|s| s.function_id.clone()),
        namespace: source.and_then(|s| s.namespace.clone()),
        args: source.and_then(|s| s.args.clone()),
        filters: source.and_then(|s| s.filters.clone()),
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
pub struct ObserversDisposeRequest {
    /// The observer to remove (`id` is accepted as an alias).
    // The agent's `harness::unobserve` calls this field `id` and models carry
    // that name over — verify-wake-fix-1 postmortem: the first teardown of
    // the run failed on a raw serde "missing field" for exactly this.
    #[serde(alias = "id")]
    pub observer_id: String,
    /// The observer's owner session; in-turn calls may omit it (the calling
    /// session is injected).
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ObserversDisposeResponse {
    /// False when no record existed (already removed) — honest, not an error.
    pub removed: bool,
}

pub async fn dispose(
    deps: &Deps,
    req: ObserversDisposeRequest,
) -> Result<ObserversDisposeResponse, HarnessError> {
    let session_id = required_session_id(req.session_id.as_deref())?;
    let store = deps.bindings().await;
    let Some(binding) = store.get(&req.observer_id).await? else {
        return Ok(ObserversDisposeResponse { removed: false });
    };
    if binding.owner.session_id != session_id {
        return Err(HarnessError::InvalidRequest(format!(
            "observer {} belongs to session {}, not {}",
            binding.id, binding.owner.session_id, session_id
        )));
    }
    let was_armed_wake = crate::bindings::expiry::is_unfired_wake(&binding);
    if binding.observer_id.is_some() {
        crate::bindings::dispose_observer(deps, &binding.id).await;
    }
    store.delete(&binding.id).await?;
    // Removing a still-armed wake from outside the owner's own turn strands
    // the parked owner silently — tell it, same as an expiry would.
    if was_armed_wake {
        crate::bindings::expiry::notify_wake_lost(
            deps,
            &binding,
            crate::bindings::expiry::WakeLost::Unregistered {
                by: OBSERVERS_DISPOSE_ID,
            },
        )
        .await;
    } else {
        record_disposed(deps, &binding).await;
    }
    Ok(ObserversDisposeResponse { removed: true })
}

/// Persist the explicit removal for bindings that do not warrant waking a
/// parked session. The armed, unfired wake branch above keeps its
/// notification + custom-record pair; every other shape gets only this
/// model-invisible lifecycle record.
async fn record_disposed(deps: &Deps, binding: &Binding) {
    fired::emit(
        &deps.session().await,
        &binding.owner.session_id,
        &format!("e_obsdisposed_{}", binding.id),
        disposed_record(binding, fired::now_ms()),
    )
    .await;
}

/// Pure wire-record builder kept separate so its source/count contract can be
/// pinned without a session-manager test double.
fn disposed_record(binding: &Binding, fired_at: i64) -> fired::ObserverFired<'_> {
    fired::retirement_record(
        binding,
        fired::FireOutcome::Unregistered,
        fired::RetirementReason::Unregistered,
        Some("disposed by harness::observers::dispose"),
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
    use crate::bindings::{BindingTarget, Lifecycle, OwnerScope, SourceSpec};
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
            observer_id: Some(id.into()),
            source: Some(SourceSpec {
                function_id: "state::on-changed".into(),
                namespace: Some("default".into()),
                args: None,
                filters: Some(json!({
                    "input.scope": { "$eq": "run" },
                    "input.key": { "$eq": "done" },
                })),
            }),
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
            dedup_key: Some(json!({
                "function_id": "state::on-changed",
                "label": "dedup-label",
                "action": "run completion received",
            })),
            fires: 3,
            created_at,
        }
    }

    #[test]
    fn a_wake_row_has_no_target_and_reads_what_is_observed() {
        let row = row_from(&binding("sub_a", crate::functions::SEND_ID, 10));
        assert_eq!(row.observer_id, "sub_a");
        assert_eq!(row.function_id.as_deref(), Some("state::on-changed"));
        assert_eq!(row.namespace.as_deref(), Some("default"));
        assert_eq!(
            row.filters,
            Some(json!({ "input.scope": { "$eq": "run" }, "input.key": { "$eq": "done" } }))
        );
        assert!(row.args.is_none());
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
        assert_eq!(row.function_id.as_deref(), Some("state::on-changed"));
        assert!(row.label.is_none());
        assert!(row.action.is_none());
    }

    #[test]
    fn a_wake_label_falls_back_to_the_target_payload() {
        let mut b = binding("sub_c", crate::functions::SEND_ID, 30);
        b.dedup_key = Some(json!({ "function_id": "harness::on-timer" }));
        b.source = Some(SourceSpec {
            function_id: "harness::on-timer".into(),
            namespace: None,
            args: Some(json!({ "at": 1 })),
            filters: None,
        });
        let row = row_from(&b);
        assert_eq!(row.label.as_deref(), Some("wake-label"));
        assert_eq!(row.function_id.as_deref(), Some("harness::on-timer"));
        assert_eq!(row.args, Some(json!({ "at": 1 })));
    }

    #[test]
    fn disposing_a_recurring_call_builds_one_complete_custom_record() {
        let mut b = binding("sub_call", "state::set", 30);
        b.lifecycle.once = false;
        b.lifecycle.max_fires = Some(10);
        b.fires = 4;

        assert!(
            !crate::bindings::expiry::is_unfired_wake(&b),
            "a call must take the custom-only branch"
        );
        let record = serde_json::to_value(disposed_record(&b, 42)).unwrap();
        assert_eq!(record["observer_id"], "sub_call");
        assert_eq!(record["target"], "state::set");
        assert_eq!(record["function_id"], "state::on-changed");
        assert_eq!(record["filters"]["input.key"]["$eq"], "done");
        assert_eq!(record["action"], "run completion received");
        assert_eq!(record["fires"], 4);
        assert_eq!(record["outcome"], "unregistered");
        assert_eq!(record["retirement_reason"], "unregistered");
        assert_eq!(record["retired"], true);
        assert_eq!(record["fired_at"], 42);
        assert!(record.get("payload").is_none());
    }

    /// `id` is the field name `harness::unobserve` uses; both spellings must
    /// deserialize to the same request.
    #[test]
    fn dispose_accepts_id_alias_for_observer_id() {
        let canonical: ObserversDisposeRequest =
            serde_json::from_value(json!({ "observer_id": "sub_a" })).unwrap();
        assert_eq!(canonical.observer_id, "sub_a");
        let aliased: ObserversDisposeRequest =
            serde_json::from_value(json!({ "id": "sub_a" })).unwrap();
        assert_eq!(aliased.observer_id, "sub_a");
    }
}
