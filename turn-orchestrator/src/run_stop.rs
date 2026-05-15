//! `run::stop` — cooperative stop entrypoint.
//!
//! Thin wrapper that composes three existing primitives:
//!
//! 1. `router::abort` writes `session/<id>/abort_signal = true`. The
//!    durable state handlers (`states/steering`, `states/assistant`,
//!    `states/functions`) read this flag at every transition and route
//!    the run to `TearingDown`.
//! 2. `approval::sweep_session` marks every pending approval row for the
//!    session as `timed_out` with reason `run_stopped`, so a previously
//!    queued approval can no longer dispatch the inner function after the
//!    user clicked stop.
//! 3. `iii::durable::publish turn::step_requested` wakes the subscriber so
//!    it can pick the new state up immediately instead of waiting for the
//!    next natural tick.
//!
//! The flag write itself is delegated rather than reimplemented so the
//! durable state machine and `router::abort` (called from the legacy
//! HTTP `agent/{session_id}/abort` endpoint) cannot drift.

use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;

use crate::persistence;
use crate::run_start;
use crate::state::TurnState;

pub const FUNCTION_ID: &str = "run::stop";

pub async fn execute(iii: III, payload: Value) -> Result<Value, IIIError> {
    let session_id = required_str(&payload, "session_id")?;

    let prior = persistence::load_record(&iii, &session_id).await;
    let prior_state = prior.as_ref().map(|r| r.state);

    // Idempotency: no record → nothing to stop.
    if prior.is_none() {
        tracing::info!(%session_id, "run::stop: no record");
        return Ok(json!({
            "session_id": session_id,
            "accepted": false,
            "reason": "no_record",
        }));
    }
    if prior_state == Some(TurnState::Stopped) {
        tracing::info!(%session_id, "run::stop: already stopped");
        return Ok(json!({
            "session_id": session_id,
            "accepted": false,
            "reason": "already_stopped",
            "prior_state": "stopped",
        }));
    }

    // 1) Flag write via existing primitive.
    if let Err(e) = iii
        .trigger(TriggerRequest {
            function_id: "router::abort".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(error = %e, %session_id, "run::stop: router::abort failed");
    }

    // 2) Sweep pending approvals so no queued approval can still dispatch
    //    its inner function after the user clicked stop.
    if let Err(e) = iii
        .trigger(TriggerRequest {
            function_id: "approval::sweep_session".into(),
            payload: json!({
                "session_id": session_id,
                "reason": "run_stopped",
            }),
            action: None,
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(error = %e, %session_id, "run::stop: approval::sweep_session failed");
    }

    // 3) Nudge the durable subscriber so the abort branch fires immediately.
    run_start::publish_step(&iii, &session_id).await;

    tracing::info!(
        %session_id,
        prior_state = ?prior_state,
        "run::stop accepted",
    );

    Ok(json!({
        "session_id": session_id,
        "accepted": true,
        "prior_state": prior_state.map(|s| s.as_str()),
    }))
}

pub fn register(iii: &III) {
    let iii_async = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(FUNCTION_ID.to_string()).with_description(
            "Cooperatively stop an in-flight run: raise the abort flag, sweep \
             pending approvals, and nudge the durable subscriber. Idempotent; \
             returns {accepted: false, reason} when the session is unknown or \
             already stopped."
                .to_string(),
        ),
        move |payload: Value| {
            let iii = iii_async.clone();
            async move { execute(iii, payload).await }
        },
    ));
}

fn required_str(payload: &Value, field: &str) -> Result<String, IIIError> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| IIIError::Handler(format!("missing required field: {field}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_str_extracts_or_errors() {
        let p = json!({ "session_id": "abc" });
        assert_eq!(required_str(&p, "session_id").unwrap(), "abc");
        assert!(required_str(&p, "missing").is_err());
    }
}
