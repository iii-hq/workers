//! Resolve the active `sandbox_id` for a shell call.

use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

use crate::errors::ShellError;

const STATE_SCOPE: &str = "agent";

pub fn parse_sandbox_id_from_args(args: &Value) -> Option<String> {
    args.get("sandbox_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

pub fn parse_session_id_from_args(args: &Value) -> Option<String> {
    args.get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn sandbox_id_state_key(session_id: &str) -> String {
    format!("session/{session_id}/sandbox_id")
}

/// Read the `sandbox_id` for `session_id` from `agent` state.
///
/// Returns `None` if the key is absent, the value is empty, or the
/// `state::get` trigger fails (the failure is logged via `tracing::warn!`).
///
/// Response-shape parsing is exercised by the gated `replay-test`
/// integration suite; no in-process unit test is feasible until iii-sdk
/// ships an in-process engine helper.
pub async fn load_sandbox_id_from_state(iii: &III, session_id: &str) -> Option<String> {
    let payload = json!({
        "scope": STATE_SCOPE,
        "key": sandbox_id_state_key(session_id),
    });
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload,
            action: None,
            timeout_ms: None,
        })
        .await;
    match resp {
        Ok(value) => value
            .get("value")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
        Err(e) => {
            tracing::warn!(error = %e, %session_id, "state::get for sandbox_id failed");
            None
        }
    }
}

/// Resolve the active `sandbox_id` for a shell call.
///
/// Precedence: explicit `args.sandbox_id` first; if absent, look up
/// `args.session_id` against `agent` state at `session/<sid>/sandbox_id`;
/// if still absent, return `ShellError::missing_sandbox()`.
///
/// The state-lookup branch is exercised by the gated `replay-test`
/// integration suite (`workers/replay-test/tests/end_to_end.rs`); no
/// in-process unit test is feasible until iii-sdk ships an in-process
/// engine helper.
pub async fn resolve_sandbox_id(iii: &III, args: &Value) -> Result<String, ShellError> {
    if let Some(id) = parse_sandbox_id_from_args(args) {
        return Ok(id);
    }
    if let Some(session_id) = parse_session_id_from_args(args) {
        if let Some(id) = load_sandbox_id_from_state(iii, &session_id).await {
            return Ok(id);
        }
    }
    Err(ShellError::missing_sandbox())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sandbox_id_from_args_finds_string() {
        let args = json!({ "sandbox_id": "abc-123" });
        assert_eq!(
            parse_sandbox_id_from_args(&args).as_deref(),
            Some("abc-123")
        );
    }

    #[test]
    fn parse_sandbox_id_from_args_rejects_empty() {
        assert!(parse_sandbox_id_from_args(&json!({ "sandbox_id": "" })).is_none());
        assert!(parse_sandbox_id_from_args(&json!({})).is_none());
    }

    #[test]
    fn key_includes_session_id() {
        assert_eq!(sandbox_id_state_key("sid-1"), "session/sid-1/sandbox_id");
    }

    #[test]
    fn parse_args_prefer_explicit_sandbox_id_over_session() {
        // `resolve_sandbox_id` cannot run without an `&III`, but its first
        // branch is reachable via `parse_sandbox_id_from_args`. This pins
        // the precedence: when `sandbox_id` is present, the resolver never
        // needs to consult `session_id` or `agent` state.
        let args = json!({ "sandbox_id": "explicit-id", "session_id": "fallback" });
        assert_eq!(
            parse_sandbox_id_from_args(&args).as_deref(),
            Some("explicit-id"),
        );
        assert_eq!(
            parse_session_id_from_args(&args).as_deref(),
            Some("fallback"),
        );
    }

    #[test]
    fn parse_args_falls_through_to_session_when_sandbox_id_missing() {
        // If `sandbox_id` is absent or empty, the resolver must fall back
        // to `session_id`. Anything past that needs `&III` and is covered
        // by the gated `replay-test` integration suite.
        let args = json!({ "session_id": "sid-only" });
        assert!(parse_sandbox_id_from_args(&args).is_none());
        assert_eq!(
            parse_session_id_from_args(&args).as_deref(),
            Some("sid-only"),
        );

        let empty_sb = json!({ "sandbox_id": "", "session_id": "sid-only" });
        assert!(parse_sandbox_id_from_args(&empty_sb).is_none());
        assert_eq!(
            parse_session_id_from_args(&empty_sb).as_deref(),
            Some("sid-only"),
        );
    }

    #[test]
    fn parse_args_with_neither_field_yields_none() {
        // The terminal `Err(ShellError::missing_sandbox())` branch of
        // `resolve_sandbox_id` is reached when both parsers return `None`.
        let args = json!({});
        assert!(parse_sandbox_id_from_args(&args).is_none());
        assert!(parse_session_id_from_args(&args).is_none());
    }
}
