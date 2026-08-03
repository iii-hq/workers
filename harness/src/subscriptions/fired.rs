//! Durable `trigger_fired` bookkeeping entries.
//!
//! Every real subscription fire — a wake or a mechanical call — appends a
//! `kind: "custom"` `trigger_fired` entry into the OWNER session's
//! transcript (the chat that registered the trigger). Custom entries are
//! model-invisible (excluded from default `session::messages` reads and the
//! model context), so this is a pure UI signal with two uses on the console:
//!   * render a turn-less "trigger fired" notice in the timeline, and
//!   * keep a fired `once` trigger visible in the panel after the engine
//!     unregisters it (the 5s poll can no longer see it, but this durable
//!     record can).
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
    pub fired_at: i64,
}

/// Append the fired record into the owner session. Best-effort — logs and
/// returns on error so a transcript hiccup never blocks the fire.
pub async fn emit(
    session: &SessionClient,
    owner_session_id: &str,
    entry_id: &str,
    rec: TriggerFired<'_>,
) {
    let data = match serde_json::to_value(&rec) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "trigger_fired record serialize failed; dropping");
            return;
        }
    };
    if let Err(e) = session
        .append_custom(
            owner_session_id,
            CUSTOM_TYPE,
            data,
            entry_id,
            Some(&json!({ "trigger_fired": true })),
        )
        .await
    {
        tracing::warn!(
            error = %e,
            session_id = %owner_session_id,
            entry_id = %entry_id,
            "trigger_fired record append failed (non-fatal)"
        );
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
    }
}
