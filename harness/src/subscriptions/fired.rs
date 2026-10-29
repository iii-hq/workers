//! Durable `observer_fired` bookkeeping entries.
//!
//! Every delivery attempt of an agent observer — plus lifecycle endings that
//! happen without a fire — appends a `kind: "custom"` `observer_fired` entry
//! into the OWNER session's transcript (the chat that created the observer).
//! Custom entries are model-invisible (excluded from default
//! `session::messages` reads and the model context), so this is a pure UI
//! signal with two uses on the console:
//!   * render a turn-less "observer fired" notice in the timeline, and
//!   * keep a fired `once` observer visible in the panel after it is disposed
//!     (the console's list refetch can no longer see it, but this durable
//!     record can).
//!
//! Transcripts written before 0.26 carry the same record under an older
//! custom type ([`STORED_CUSTOM_TYPE`]); readers accept both.
//!
//! Best-effort: a failed append logs and returns; it never blocks the fire's
//! real work (the notification wake or the dispatched call).

use serde::Serialize;
use serde_json::{json, Value};

use crate::bindings::Binding;
use crate::clients::session::SessionClient;
use crate::types::message::AgentMessage;

/// custom_type stamped on the transcript entry (mirrored by the console mapper).
pub const CUSTOM_TYPE: &str = "observer_fired";

/// The custom type transcripts written before 0.26 carry for the same record.
pub const STORED_CUSTOM_TYPE: &str = "trigger_fired";

/// Whether `custom_type` names an observer activity record.
pub fn is_fired_record(custom_type: &str) -> bool {
    custom_type == CUSTOM_TYPE || custom_type == STORED_CUSTOM_TYPE
}

/// What happened when iii delivered an observed call to a binding. The
/// custom type also covers lifecycle-only records, so this field is the
/// authoritative semantic discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FireOutcome {
    Delivered,
    DeliveryFailed,
    Skipped,
    Expired,
    Unregistered,
    Invalidated,
}

/// Why a binding disappeared after this activity. Kept separate from
/// [`FireOutcome`] because a delivered fire can also consume its binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetirementReason {
    OnceConsumed,
    MaxFires,
    Expired,
    Unregistered,
    Invalidated,
    Exhausted,
}

/// The `data` payload of an `observer_fired` custom entry. Carries enough for
/// the console to render both the chat notice and a standalone fired panel
/// row after a reload (label / target / state watch).
#[derive(Debug, Serialize)]
pub struct ObserverFired<'a> {
    /// The binding's observer id (equal to the binding id).
    pub observer_id: &'a str,
    /// The binding's target function id (`harness::send` for a wake, else the
    /// called function). Records written before the delivery hop carry the
    /// legacy values `"notify"` / `"spawn"`.
    pub target: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<&'a str>,
    /// Human-readable event text declared as `action` when the binding was
    /// created. Unlike `label`, this says what happened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<&'a str>,
    /// The observed function and the observer's args and filters. Records of
    /// bindings without a source legitimately omit them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filters: Option<&'a Value>,
    pub outcome: FireOutcome,
    pub once: bool,
    /// The binding's durable fire counter immediately after this activity.
    /// Delivery records use the post-claim count; non-delivery records retain
    /// the count already persisted on the binding.
    pub fires: u64,
    /// This activity retired the binding. The structured reason explains why.
    pub retired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retirement_reason: Option<RetirementReason>,
    /// The state watch: the scope/key a `state::on-changed` event names, or
    /// the observer's `$eq` filters on them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'a str>,
    /// What this fire delivered: the final dispatched payload of a ƒ-call
    /// (post-conditions, post-projection, post-filesystem-stamping — the
    /// attempted payload when the dispatch failed), or the post-conditions
    /// event a wake injected. `None` on skip/gc/expiry records — nothing was
    /// delivered. Uncapped by design; see the 2026-08-05 spec.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<&'a Value>,
    pub fired_at: i64,
}

/// The source fields of a record: the observed function, args and filters.
pub fn source_fields(binding: &Binding) -> (Option<&str>, Option<&Value>, Option<&Value>) {
    binding
        .source
        .as_ref()
        .map_or((None, None, None), |source| {
            (
                Some(source.function_id.as_str()),
                source.args.as_ref(),
                source.filters.as_ref(),
            )
        })
}

/// Build a lifecycle-only record from the teardown's authoritative binding
/// snapshot. Race-prone callers emit it only after winning retirement; expiry,
/// explicit unregistration, and invalidation share this shape so their
/// source/count fields cannot drift.
pub fn retirement_record<'a>(
    binding: &'a Binding,
    outcome: FireOutcome,
    reason: RetirementReason,
    note: Option<&'a str>,
    fired_at: i64,
) -> ObserverFired<'a> {
    let (function_id, args, filters) = source_fields(binding);
    let (scope, key) = binding.state_watch();
    let label = binding
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
        });
    ObserverFired {
        observer_id: &binding.id,
        target: &binding.target.function_id,
        label,
        action: binding.event_action(),
        function_id,
        args,
        filters,
        outcome,
        once: binding.lifecycle.once,
        fires: binding.fires,
        retired: true,
        retirement_reason: Some(reason),
        scope,
        key,
        note,
        payload: None,
        fired_at,
    }
}

/// Append the fired record into the owner session. Best-effort — logs and
/// returns on error so a transcript hiccup never blocks the fire.
///
/// The `payload` field is uncapped by design, so it is the likeliest reason an
/// append ever fails (the transport ceiling). When a record carrying a
/// payload fails to append, retry ONCE with `payload: None` (same entry id —
/// session-manager dedups on it, so a partially-succeeded first attempt is
/// harmless): the bookkeeping the timeline and the fired-panel ghost row
/// actually need (ids, label, the `retired` flag) then survives even when the
/// payload itself cannot be persisted.
pub async fn emit(
    session: &SessionClient,
    owner_session_id: &str,
    entry_id: &str,
    rec: ObserverFired<'_>,
) {
    let had_payload = rec.payload.is_some();
    let data = match serde_json::to_value(&rec) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "observer_fired record serialize failed; dropping");
            return;
        }
    };
    let origin = json!({ "observer_fired": true });
    let Err(e) = session
        .append_custom(owner_session_id, CUSTOM_TYPE, data, entry_id, Some(&origin))
        .await
    else {
        return;
    };
    if !had_payload {
        tracing::warn!(
            error = %e,
            session_id = %owner_session_id,
            entry_id = %entry_id,
            "observer_fired record append failed (non-fatal)"
        );
        return;
    }
    tracing::warn!(
        error = %e,
        session_id = %owner_session_id,
        entry_id = %entry_id,
        "observer_fired append failed with a payload attached; retrying once with the payload dropped"
    );
    let retry_data = match serde_json::to_value(drop_payload(rec)) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "observer_fired retry record serialize failed; dropping");
            return;
        }
    };
    if let Err(e2) = session
        .append_custom(
            owner_session_id,
            CUSTOM_TYPE,
            retry_data,
            entry_id,
            Some(&origin),
        )
        .await
    {
        tracing::warn!(
            error = %e2,
            session_id = %owner_session_id,
            entry_id = %entry_id,
            "observer_fired record append failed (non-fatal)"
        );
    }
}

/// `rec` with its payload dropped — the shape retried when an append carrying
/// a payload fails. Serializing this is exactly equivalent to serializing the
/// original record with `payload: None`, since the field is
/// `skip_serializing_if = "Option::is_none"`.
fn drop_payload(rec: ObserverFired<'_>) -> ObserverFired<'_> {
    ObserverFired {
        payload: None,
        ..rec
    }
}

/// Current wall-clock ms for the record's `fired_at`.
pub fn now_ms() -> i64 {
    AgentMessage::now_ms()
}

/// A `state::on-changed` event carries `{scope?, key}`; other events
/// (cron/stream/turn) carry no watch. Best-effort — returns `(None, None)`
/// when absent.
pub fn event_state_watch(event: &Value) -> (Option<&str>, Option<&str>) {
    (
        event.get("scope").and_then(Value::as_str),
        event.get("key").and_then(Value::as_str),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_accept_both_record_types() {
        assert!(is_fired_record("observer_fired"));
        assert!(is_fired_record("trigger_fired"));
        assert!(!is_fired_record("compaction"));
    }

    #[test]
    fn state_watch_reads_scope_and_key_from_event() {
        let ev = json!({ "scope": "cache-repl-pipeline", "key": "facts", "value": 1 });
        assert_eq!(
            event_state_watch(&ev),
            (Some("cache-repl-pipeline"), Some("facts"))
        );
        // Turn/cron events carry no watch.
        assert_eq!(
            event_state_watch(&json!({ "session_id": "s" })),
            (None, None)
        );
        assert_eq!(event_state_watch(&Value::Null), (None, None));
    }

    #[test]
    fn record_omits_empty_optionals() {
        let rec = ObserverFired {
            observer_id: "sub_1",
            target: "harness::send",
            label: None,
            action: None,
            function_id: None,
            args: None,
            filters: None,
            outcome: FireOutcome::Delivered,
            once: true,
            fires: 1,
            retired: true,
            retirement_reason: Some(RetirementReason::OnceConsumed),
            scope: None,
            key: None,
            note: None,
            payload: None,
            fired_at: 42,
        };
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["observer_id"], "sub_1");
        assert_eq!(v["target"], "harness::send");
        assert_eq!(v["outcome"], "delivered");
        assert_eq!(v["once"], true);
        assert_eq!(v["fires"], 1);
        assert_eq!(v["retired"], true);
        assert_eq!(v["retirement_reason"], "once_consumed");
        assert_eq!(v["fired_at"], 42);
        // Skipped optionals must not appear.
        assert!(v.get("label").is_none());
        assert!(v.get("action").is_none());
        assert!(v.get("function_id").is_none());
        assert!(v.get("args").is_none());
        assert!(v.get("filters").is_none());
        assert!(v.get("payload").is_none());
    }

    #[test]
    fn record_serializes_the_registration_and_exact_enum_names() {
        let args = json!({ "expression": "0 * * * * *" });
        let rec = ObserverFired {
            observer_id: "sub_1",
            target: "state::set",
            label: Some("heartbeat"),
            action: Some("scheduled heartbeat received"),
            function_id: Some("cron::on-tick"),
            args: Some(&args),
            filters: None,
            outcome: FireOutcome::DeliveryFailed,
            once: false,
            fires: 7,
            retired: true,
            retirement_reason: Some(RetirementReason::MaxFires),
            scope: None,
            key: None,
            note: Some("dispatch failed"),
            payload: None,
            fired_at: 42,
        };
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["function_id"], "cron::on-tick");
        assert_eq!(v["action"], "scheduled heartbeat received");
        assert_eq!(v["args"], args);
        assert_eq!(v["outcome"], "delivery_failed");
        assert_eq!(v["fires"], 7);
        assert_eq!(v["retirement_reason"], "max_fires");

        let outcomes = [
            FireOutcome::Delivered,
            FireOutcome::DeliveryFailed,
            FireOutcome::Skipped,
            FireOutcome::Expired,
            FireOutcome::Unregistered,
            FireOutcome::Invalidated,
        ];
        assert_eq!(
            outcomes
                .into_iter()
                .map(|value| serde_json::to_value(value).unwrap())
                .collect::<Vec<_>>(),
            vec![
                json!("delivered"),
                json!("delivery_failed"),
                json!("skipped"),
                json!("expired"),
                json!("unregistered"),
                json!("invalidated"),
            ]
        );
        let reasons = [
            RetirementReason::OnceConsumed,
            RetirementReason::MaxFires,
            RetirementReason::Expired,
            RetirementReason::Unregistered,
            RetirementReason::Invalidated,
            RetirementReason::Exhausted,
        ];
        assert_eq!(
            reasons
                .into_iter()
                .map(|value| serde_json::to_value(value).unwrap())
                .collect::<Vec<_>>(),
            vec![
                json!("once_consumed"),
                json!("max_fires"),
                json!("expired"),
                json!("unregistered"),
                json!("invalidated"),
                json!("exhausted"),
            ]
        );
    }

    #[test]
    fn record_carries_the_delivered_payload_when_present() {
        let payload = json!({ "event": { "db": "primary", "op": "update", "affected_rows": 1 } });
        let rec = ObserverFired {
            observer_id: "sub_1",
            target: "receiving::check_completion",
            label: None,
            action: None,
            function_id: Some("database::on-row-changed"),
            args: None,
            filters: None,
            outcome: FireOutcome::Delivered,
            once: false,
            fires: 3,
            retired: false,
            retirement_reason: None,
            scope: None,
            key: None,
            note: None,
            payload: Some(&payload),
            fired_at: 42,
        };
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["payload"], payload);
        assert_eq!(v["fires"], 3);
    }

    #[test]
    fn drop_payload_clears_only_the_payload_field() {
        let payload = json!({ "returning": ["row1", "row2"] });
        let rec = ObserverFired {
            observer_id: "sub_1",
            target: "receiving::check_completion",
            label: Some("lbl"),
            action: Some("record changed"),
            function_id: Some("state::on-changed"),
            args: None,
            filters: None,
            outcome: FireOutcome::DeliveryFailed,
            once: false,
            fires: 4,
            retired: false,
            retirement_reason: None,
            scope: Some("scope"),
            key: Some("key"),
            note: Some("note"),
            payload: Some(&payload),
            fired_at: 42,
        };
        let stripped = drop_payload(rec);
        assert_eq!(stripped.observer_id, "sub_1");
        assert_eq!(stripped.target, "receiving::check_completion");
        assert_eq!(stripped.label, Some("lbl"));
        assert_eq!(stripped.action, Some("record changed"));
        assert_eq!(stripped.function_id, Some("state::on-changed"));
        assert_eq!(stripped.outcome, FireOutcome::DeliveryFailed);
        assert!(!stripped.once);
        assert_eq!(stripped.fires, 4);
        assert!(!stripped.retired);
        assert_eq!(stripped.retirement_reason, None);
        assert_eq!(stripped.scope, Some("scope"));
        assert_eq!(stripped.key, Some("key"));
        assert_eq!(stripped.note, Some("note"));
        assert!(stripped.payload.is_none());
        assert_eq!(stripped.fired_at, 42);

        // Equivalent to serializing the original with `payload: None`.
        let v = serde_json::to_value(&stripped).unwrap();
        assert!(v.get("payload").is_none());
    }
}
