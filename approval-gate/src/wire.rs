//! Wire-format types for the approval gate.
//!
//! Pure data shapes and small wire-shape helpers — no I/O, no `iii_sdk`
//! deps, no async. Anything a downstream worker would need to
//! understand the approval-gate protocol lives here:
//!
//! - [`Denial`] — structured deny payload (`kind` + `detail`) carried on
//!   hook replies, persisted records, and `approval_resolved` events.
//! - [`Decision`] — internal allow/deny choice; pairs `Deny` with its
//!   [`Denial`] so the type system rules out structureless deny.
//! - [`WireDecision`] — coarse `"allow"` / `"deny"` enum used at the
//!   `approval::resolve` RPC boundary, where the UI / orchestrator
//!   doesn't yet know the full [`Denial`].
//! - [`IncomingCall`] — parsed `agent::before_function_call` envelope.
//! - [`pending_key`], [`extract_call`], [`block_reply_for`] — pure
//!   helpers for going to / from the wire.
//!
//! The handler crate re-exports the public items from [`crate`] so
//! existing call sites don't need to import the module directly.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Structured deny payload carried on wire replies, persisted records, and
/// `approval_resolved` stream events. Replaces the legacy free-form
/// `decision_reason` / `reason` strings so consumers (turn-orchestrator
/// stitching, UIs, the LLM) can branch on `kind` instead of parsing prose.
///
/// Wire shape (serde tag=kind, content=detail, snake_case):
///   `{ "kind": "policy", "detail": { "classifier_reason": "...", "classifier_fn": "..." } }`
///   `{ "kind": "user_rejected", "detail": null }`
///   `{ "kind": "user_corrected", "detail": { "feedback": "..." } }`
///   `{ "kind": "state_error",   "detail": { "phase": "...", "error": "..." } }`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum Denial {
    Policy {
        classifier_reason: String,
        classifier_fn: String,
    },
    UserRejected,
    UserCorrected {
        feedback: String,
    },
    StateError {
        phase: String,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct IncomingCall {
    pub session_id: String,
    pub function_call_id: String,
    pub function_id: String,
    pub args: Value,
    pub approval_required: Vec<String>,
    pub event_id: String,
    pub reply_stream: String,
}

impl IncomingCall {
    pub fn requires_approval(&self) -> bool {
        self.approval_required
            .iter()
            .any(|n| n == &self.function_id)
    }
}

/// Internal allow/deny choice. Paired with a structured [`Denial`] on
/// the `Deny` arm so callers that emit a wire reply can't accidentally
/// drop the deny reason on the floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(Denial),
}

/// Wire-format decision string used by `approval::resolve` and stored
/// as the `status` field of resolved approval records.
///
/// Serializes / deserializes as `"allow"` or `"deny"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WireDecision {
    Allow,
    Deny,
}

/// Build the state-store key for a pending approval entry.
///
/// `session_id` and `function_call_id` must not contain `/`. They are caller-controlled
/// IDs minted by turn-orchestrator; today neither format uses the separator.
pub fn pending_key(session_id: &str, function_call_id: &str) -> String {
    debug_assert!(!session_id.contains('/'), "session_id must not contain '/'");
    debug_assert!(
        !function_call_id.contains('/'),
        "function_call_id must not contain '/'"
    );
    format!("{session_id}/{function_call_id}")
}

/// Parse the `agent::before_function_call` envelope into the
/// [`IncomingCall`] the gate's intercept logic operates on. Accepts both
/// the modern `function_call` shape and the legacy `tool_call` alias so
/// older sessions in-flight at upgrade time keep working.
///
/// Returns `None` if any required field is missing — handlers treat that
/// as "not our concern" and pass the envelope through.
pub fn extract_call(envelope: &Value) -> Option<IncomingCall> {
    let event_id = envelope
        .get("event_id")
        .and_then(Value::as_str)?
        .to_string();
    let reply_stream = envelope
        .get("reply_stream")
        .and_then(Value::as_str)?
        .to_string();
    let inner = envelope.get("payload").unwrap_or(envelope);
    let session_id = inner.get("session_id").and_then(Value::as_str)?.to_string();
    let fc = inner
        .get("function_call")
        .or_else(|| inner.get("tool_call"))?;
    let function_id = fc
        .get("function_id")
        .or_else(|| fc.get("name"))
        .and_then(Value::as_str)?
        .to_string();
    Some(IncomingCall {
        session_id,
        function_call_id: fc.get("id").and_then(Value::as_str)?.to_string(),
        function_id,
        args: fc.get("arguments").cloned().unwrap_or_else(|| json!({})),
        approval_required: inner
            .get("approval_required")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        event_id,
        reply_stream,
    })
}

/// Build the hook block reply for a [`Decision`]. Deny replies carry the
/// structured [`Denial`] under `denial`; consumers (turn-orchestrator
/// stitching, UIs, the LLM) branch on `denial.kind` rather than parsing a
/// free-form `reason` string.
pub fn block_reply_for(decision: &Decision) -> Value {
    match decision {
        Decision::Allow => json!({ "block": false }),
        Decision::Deny(denial) => json!({
            "block": true,
            "denial": denial,
        }),
    }
}
