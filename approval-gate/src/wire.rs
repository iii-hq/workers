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

use crate::rules::RuleMatch;

/// Structured deny payload carried on wire replies, persisted records, and
/// `approval_resolved` stream events. Consumers (turn-orchestrator
/// stitching, UIs, the LLM) branch on `kind` instead of parsing prose.
///
/// Wire shape (serde tag=kind, content=detail, snake_case):
///   `{ "kind": "approval_rule_denied", "detail": { "rule": { ... } } }`
///   `{ "kind": "user_rejected", "detail": null }`
///   `{ "kind": "user_corrected", "detail": { "feedback": "..." } }`
///   `{ "kind": "state_error",   "detail": { "phase": "...", "error": "..." } }`
///
/// `Policy` names the matching rule from the layered ruleset
/// (`approval-gate/src/rules.rs`). The old `classifier_reason` /
/// `classifier_fn` shape went away when the classifier surface was
/// deleted in favor of pure rules-based decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum Denial {
    ApprovalRuleDenied {
        rule: RuleMatch,
    },
    /// Legacy compatibility for records/events produced before explicit
    /// rule metadata shipped.
    Policy {
        rule_permission: String,
        rule_pattern: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractCallError {
    pub phase: &'static str,
    pub error: String,
    pub event_id: Option<String>,
    pub reply_stream: Option<String>,
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
/// Returns a typed error if required fields are missing so the subscriber
/// can fail closed when it has enough routing data to send a reply.
pub fn parse_call(envelope: &Value) -> Result<IncomingCall, ExtractCallError> {
    let route_event_id = envelope
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let route_reply_stream = envelope
        .get("reply_stream")
        .and_then(Value::as_str)
        .map(str::to_string);
    let err = |error: &str| ExtractCallError {
        phase: "extract_call",
        error: error.to_string(),
        event_id: route_event_id.clone(),
        reply_stream: route_reply_stream.clone(),
    };
    let event_id = route_event_id
        .clone()
        .ok_or_else(|| err("missing event_id"))?;
    let reply_stream = route_reply_stream
        .clone()
        .ok_or_else(|| err("missing reply_stream"))?;
    let inner = envelope.get("payload").unwrap_or(envelope);
    let session_id = inner
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| err("missing session_id"))?
        .to_string();
    let fc = inner
        .get("function_call")
        .or_else(|| inner.get("tool_call"))
        .ok_or_else(|| err("missing function_call"))?;
    let function_id = fc
        .get("function_id")
        .or_else(|| fc.get("name"))
        .and_then(Value::as_str)
        .ok_or_else(|| err("missing function_id"))?
        .to_string();
    Ok(IncomingCall {
        session_id,
        function_call_id: fc
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| err("missing function_call.id"))?
            .to_string(),
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

pub fn extract_call(envelope: &Value) -> Option<IncomingCall> {
    parse_call(envelope).ok()
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

pub fn state_error_reply(phase: &str, error: &str) -> Value {
    block_reply_for(&Decision::Deny(Denial::StateError {
        phase: phase.to_string(),
        error: error.to_string(),
    }))
}
