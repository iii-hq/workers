//! Per-session abort flag shared across the state machine.
//!
//! The flag is `session/<id>/abort_signal` under scope `agent`. It is the
//! single coordination primitive between `run::stop` (and the legacy
//! `router::abort`) and the durable handlers in `states/*`.
//!
//! Handlers call [`is_set`] at every state transition that precedes a
//! costly operation; [`clear`] is called once per turn at the top of
//! `handle_provisioning`, which is the only safe re-entry point that
//! cannot race with a still-running `run::stop`.

use iii_sdk::{TriggerRequest, III};
use serde_json::{json, Value};

const STATE_SCOPE: &str = "agent";

pub fn abort_signal_key(session_id: &str) -> String {
    format!("session/{session_id}/abort_signal")
}

pub async fn is_set(iii: &III, session_id: &str) -> bool {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({
            "scope": STATE_SCOPE,
            "key": abort_signal_key(session_id),
        }),
        action: None,
        timeout_ms: None,
    })
    .await
    .ok()
    .and_then(|v| v.as_bool())
    .unwrap_or(false)
}

pub async fn clear(iii: &III, session_id: &str) {
    if let Err(e) = iii
        .trigger(TriggerRequest {
            function_id: "state::set".into(),
            payload: json!({
                "scope": STATE_SCOPE,
                "key": abort_signal_key(session_id),
                "value": Value::Bool(false),
            }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(error = %e, %session_id, "abort::clear: state::set failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_uses_session_namespace() {
        assert_eq!(abort_signal_key("abc"), "session/abc/abort_signal");
    }
}
