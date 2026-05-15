//! Typed approval record schema.
//!
//! Replaces the ad-hoc `serde_json::Value` blobs that handlers used to
//! pass around. The Rust shape mirrors the persisted JSON exactly via
//! serde — wire compatibility is the contract, this is just the in-process
//! representation.
//!
//! ## Lifecycle
//!
//! ```text
//! intercept → Pending ──user allow──> Approved ──invoke──> Executed
//!                  │                         └invoke-err──> Failed
//!                  ├──user deny──> Denied
//!                  └──expires──> TimedOut
//! ```
//!
//! Outcome data (result / error / denial) is required on the terminal
//! status it belongs to and meaningless elsewhere. [`Next`] enforces this
//! at the type level so transitions can't be miscalled.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Denial;

/// Lifecycle status of an approval record. Wire format is snake_case so
/// it stays human-readable in iii-state dumps and audit logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Approved,
    Executed,
    Failed,
    Denied,
    TimedOut,
}

impl Status {
    /// `true` for statuses that represent a final outcome — anything
    /// stitchable into the LLM's next turn. `Pending` and `Approved` are
    /// intermediate; the rest are terminal.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Status::Executed | Status::Failed | Status::Denied | Status::TimedOut
        )
    }
}

/// Persisted approval record. Wire-compatible with the historical
/// JSON shape — every field uses the same key/type the previous
/// `serde_json::Value`-based code emitted, so existing iii-state
/// rows deserialize cleanly.
///
/// Optional fields are scoped to particular statuses (e.g. `result`
/// only when `status == Executed`); the type itself does not enforce
/// that pairing — [`Next`] does, at the transition boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub function_call_id: String,
    pub function_id: String,
    pub args: Value,
    pub status: Status,
    pub expires_at: u64,

    /// Stamped by `handle_intercept` after the pending record is built so
    /// the timeout sweeper can address the right session stream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// Unix ms of the first transition into a terminal status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<u64>,

    /// Function output. Present iff `status == Executed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    /// Function error string. Present iff `status == Failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Structured deny payload. Present iff `status == Denied`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denial: Option<Denial>,

    /// Set when `approval::ack_delivered` stamps the record with the turn
    /// id that surfaced it to the LLM. Records without this stamp surface
    /// again on subsequent `approval::list_undelivered` calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_in_turn_id: Option<String>,
}

impl Record {
    /// Construct a fresh pending record. `session_id` is unset here —
    /// `handle_intercept` stamps it before persisting. `expires_at` is
    /// `now_ms + timeout_ms`, saturating on overflow so a malicious
    /// or buggy caller can't underflow the deadline.
    pub fn new_pending(
        function_call_id: String,
        function_id: String,
        args: Value,
        now_ms: u64,
        timeout_ms: u64,
    ) -> Self {
        Self {
            function_call_id,
            function_id,
            args,
            status: Status::Pending,
            expires_at: now_ms.saturating_add(timeout_ms),
            session_id: None,
            resolved_at: None,
            result: None,
            error: None,
            denial: None,
            delivered_in_turn_id: None,
        }
    }

    /// Serialize to the wire JSON shape. Infallible — `serde_json::to_value`
    /// on a struct with only serializable fields cannot fail at runtime.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("Record is always serializable")
    }

    /// Deserialize from the wire JSON shape. Returns `None` if the value
    /// doesn't match the schema (missing required fields, bad status enum,
    /// etc.) — handlers treat that as "skip this record" rather than
    /// crashing on corrupt state.
    pub fn from_value(v: Value) -> Option<Self> {
        serde_json::from_value(v).ok()
    }
}

/// What `transition_record` should change. Each variant pairs the target
/// [`Status`] with whatever outcome data that status carries. The type
/// system makes invalid combinations unrepresentable: you can't ask for
/// `Executed` without providing a result, or for `Denied` without a
/// `Denial`. `Approved` is an intermediate status carrying no outcome —
/// it exists so the bus can observe the post-allow / pre-invoke state.
#[derive(Debug, Clone)]
pub enum Next {
    Approved,
    Executed { result: Value },
    Failed { error: String },
    Denied { denial: Denial },
    TimedOut,
}

impl Next {
    /// The target status this transition moves the record to.
    pub fn status(&self) -> Status {
        match self {
            Next::Approved => Status::Approved,
            Next::Executed { .. } => Status::Executed,
            Next::Failed { .. } => Status::Failed,
            Next::Denied { .. } => Status::Denied,
            Next::TimedOut => Status::TimedOut,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_is_terminal_matches_lifecycle() {
        assert!(!Status::Pending.is_terminal());
        assert!(!Status::Approved.is_terminal());
        assert!(Status::Executed.is_terminal());
        assert!(Status::Failed.is_terminal());
        assert!(Status::Denied.is_terminal());
        assert!(Status::TimedOut.is_terminal());
    }

    #[test]
    fn status_serializes_as_snake_case_string() {
        assert_eq!(serde_json::to_value(Status::Pending).unwrap(), json!("pending"));
        assert_eq!(serde_json::to_value(Status::Approved).unwrap(), json!("approved"));
        assert_eq!(serde_json::to_value(Status::Executed).unwrap(), json!("executed"));
        assert_eq!(serde_json::to_value(Status::Failed).unwrap(), json!("failed"));
        assert_eq!(serde_json::to_value(Status::Denied).unwrap(), json!("denied"));
        assert_eq!(serde_json::to_value(Status::TimedOut).unwrap(), json!("timed_out"));
    }

    #[test]
    fn status_round_trips_via_json() {
        for s in [
            Status::Pending,
            Status::Approved,
            Status::Executed,
            Status::Failed,
            Status::Denied,
            Status::TimedOut,
        ] {
            let v = serde_json::to_value(s).unwrap();
            let back: Status = serde_json::from_value(v).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn record_pending_round_trips() {
        let rec = Record::new_pending(
            "c1".into(),
            "shell::exec".into(),
            json!({"command": "ls"}),
            1_000,
            60_000,
        );
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["function_call_id"], "c1");
        assert_eq!(v["function_id"], "shell::exec");
        assert_eq!(v["status"], "pending");
        assert_eq!(v["expires_at"], 61_000);
        // Optional fields are omitted when None.
        assert!(v.as_object().unwrap().get("session_id").is_none());
        assert!(v.as_object().unwrap().get("denial").is_none());
        let back: Record = serde_json::from_value(v).unwrap();
        assert_eq!(back, rec);
    }

    #[test]
    fn record_with_optional_fields_round_trips() {
        let rec = Record {
            function_call_id: "c1".into(),
            function_id: "shell::exec".into(),
            args: json!({}),
            status: Status::Executed,
            expires_at: 60_000,
            session_id: Some("s1".into()),
            resolved_at: Some(5_000),
            result: Some(json!({"ok": true})),
            error: None,
            denial: None,
            delivered_in_turn_id: Some("turn-X".into()),
        };
        let v = serde_json::to_value(&rec).unwrap();
        let back: Record = serde_json::from_value(v).unwrap();
        assert_eq!(back, rec);
    }

    #[test]
    fn record_pending_expires_at_saturates_on_overflow() {
        let rec = Record::new_pending(
            "c1".into(),
            "f".into(),
            json!({}),
            u64::MAX - 5,
            100,
        );
        assert_eq!(rec.expires_at, u64::MAX);
    }

    #[test]
    fn record_deserializes_from_wire_with_unknown_extra_fields() {
        // Forward-compat: unknown fields are silently ignored so a worker
        // can read a record written by a newer worker version without
        // crashing on schema additions it doesn't know about yet.
        let v = json!({
            "function_call_id": "c1",
            "function_id": "f",
            "args": {},
            "status": "pending",
            "expires_at": 1000,
            "future_field": "some new thing",
        });
        let rec: Record = serde_json::from_value(v).unwrap();
        assert_eq!(rec.status, Status::Pending);
    }

    #[test]
    fn next_status_pairing_is_correct() {
        assert_eq!(Next::Approved.status(), Status::Approved);
        assert_eq!(
            Next::Executed { result: json!({"ok": true}) }.status(),
            Status::Executed
        );
        assert_eq!(
            Next::Failed { error: "EACCES".into() }.status(),
            Status::Failed
        );
        assert_eq!(
            Next::Denied { denial: Denial::UserRejected }.status(),
            Status::Denied
        );
        assert_eq!(Next::TimedOut.status(), Status::TimedOut);
    }
}
