//! Durable `trigger_fired` bookkeeping entries.
//!
//! Every real subscription fire — a wake or a mechanical call — appends a
//! `kind: "custom"` `trigger_fired` entry into the OWNER session's
//! transcript (the chat that registered the trigger). Custom entries are
//! model-invisible (excluded from default `session::messages` reads and the
//! model context), so this is a pure UI signal with two uses on the console:
//!   * render a turn-less "trigger fired" notice in the timeline, and
//!   * keep a fired `once` trigger visible in the panel after the engine
//!     unregisters it (the console's list refetch can no longer see it, but
//!     this durable record can).
//!
//! Best-effort: a failed append logs and returns; it never blocks the fire's
//! real work (the notification wake or the dispatched call).

use serde::Serialize;
use serde_json::{json, Value};

use crate::clients::session::SessionClient;
use crate::types::message::AgentMessage;

/// custom_type stamped on the transcript entry (mirrored by the console mapper).
pub const CUSTOM_TYPE: &str = "trigger_fired";

/// The `data` payload of a `trigger_fired` custom entry. Carries enough for the
/// console to render both the chat notice and a standalone fired panel row after
/// a reload (label / target / state watch).
#[derive(Debug, Serialize)]
pub struct TriggerFired<'a> {
    pub subscription_id: &'a str,
    /// Engine trigger id — lets the console dedup against a still-registered
    /// (recurring) panel row. Absent when the local slot no longer maps one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_id: Option<&'a str>,
    /// The binding's target function id (`harness::send` for a wake, else the
    /// called function). Records written before the delivery hop carry the
    /// legacy values `"notify"` / `"spawn"`.
    pub target: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<&'a str>,
    pub once: bool,
    /// This fire unregistered the binding (once teardown).
    pub retired: bool,
    /// state-trigger watch, extracted from the fired event when present.
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
    rec: TriggerFired<'_>,
) {
    let had_payload = rec.payload.is_some();
    let data = match serde_json::to_value(&rec) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "trigger_fired record serialize failed; dropping");
            return;
        }
    };
    let origin = json!({ "trigger_fired": true });
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
            "trigger_fired record append failed (non-fatal)"
        );
        return;
    }
    tracing::warn!(
        error = %e,
        session_id = %owner_session_id,
        entry_id = %entry_id,
        "trigger_fired append failed with a payload attached; retrying once with the payload dropped"
    );
    let retry_data = match serde_json::to_value(drop_payload(rec)) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "trigger_fired retry record serialize failed; dropping");
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
            "trigger_fired record append failed (non-fatal)"
        );
    }
}

/// `rec` with its payload dropped — the shape retried when an append carrying
/// a payload fails. Serializing this is exactly equivalent to serializing the
/// original record with `payload: None`, since the field is
/// `skip_serializing_if = "Option::is_none"`.
fn drop_payload(rec: TriggerFired<'_>) -> TriggerFired<'_> {
    TriggerFired {
        payload: None,
        ..rec
    }
}

/// Current wall-clock ms for the record's `fired_at`.
pub fn now_ms() -> i64 {
    AgentMessage::now_ms()
}

/// A state fire delivers `{scope?, key}` in its event; other trigger types
/// (cron/stream/turn) carry no watch. Best-effort — returns `(None, None)`
/// when absent.
pub fn event_state_watch(event: &Value) -> (Option<&str>, Option<&str>) {
    (
        event.get("scope").and_then(Value::as_str),
        event.get("key").and_then(Value::as_str),
    )
}

/// `e_notify_…` → `e_trigfired_…`, reusing the notify fire's monotonic suffix so
/// a redelivered engine fire dedups on the same entry id.
pub fn entry_id_from_notify(notify_entry_id: &str) -> String {
    notify_entry_id.replacen("e_notify_", "e_trigfired_", 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_id_swaps_only_the_notify_prefix() {
        assert_eq!(entry_id_from_notify("e_notify_sub_1"), "e_trigfired_sub_1");
        assert_eq!(
            entry_id_from_notify("e_notify_sub_1_7"),
            "e_trigfired_sub_1_7"
        );
        // Only the leading occurrence is swapped.
        assert_eq!(
            entry_id_from_notify("e_notify_e_notify_x"),
            "e_trigfired_e_notify_x"
        );
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
        let rec = TriggerFired {
            subscription_id: "sub_1",
            trigger_id: None,
            target: "harness::send",
            label: None,
            once: true,
            retired: true,
            scope: None,
            key: None,
            note: None,
            payload: None,
            fired_at: 42,
        };
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["subscription_id"], "sub_1");
        assert_eq!(v["target"], "harness::send");
        assert_eq!(v["once"], true);
        assert_eq!(v["retired"], true);
        assert_eq!(v["fired_at"], 42);
        // Skipped optionals must not appear.
        assert!(v.get("trigger_id").is_none());
        assert!(v.get("label").is_none());
        assert!(v.get("payload").is_none());
    }

    #[test]
    fn record_carries_the_delivered_payload_when_present() {
        let payload = json!({ "event": { "db": "primary", "op": "update", "affected_rows": 1 } });
        let rec = TriggerFired {
            subscription_id: "sub_1",
            trigger_id: None,
            target: "receiving::check_completion",
            label: None,
            once: false,
            retired: false,
            scope: None,
            key: None,
            note: None,
            payload: Some(&payload),
            fired_at: 42,
        };
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["payload"], payload);
    }

    #[test]
    fn drop_payload_clears_only_the_payload_field() {
        let payload = json!({ "returning": ["row1", "row2"] });
        let rec = TriggerFired {
            subscription_id: "sub_1",
            trigger_id: Some("trig_1"),
            target: "receiving::check_completion",
            label: Some("lbl"),
            once: false,
            retired: false,
            scope: Some("scope"),
            key: Some("key"),
            note: Some("note"),
            payload: Some(&payload),
            fired_at: 42,
        };
        let stripped = drop_payload(rec);
        assert_eq!(stripped.subscription_id, "sub_1");
        assert_eq!(stripped.trigger_id, Some("trig_1"));
        assert_eq!(stripped.target, "receiving::check_completion");
        assert_eq!(stripped.label, Some("lbl"));
        assert!(!stripped.once);
        assert!(!stripped.retired);
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
