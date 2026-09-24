//! `harness::function::cancel` — interrupt ONE in-flight function call without
//! cancelling the turn (harness.md § `harness::function::cancel`). The console's
//! per-card stop button. Fires the per-call signal the tool phase races the
//! target invocation against; the loop then records a `cancelled` `is_error`
//! result for the call and carries on, so the model sees the interruption and
//! decides what to do next. Contrast `harness::stop`, which ends the turn.
//!
//! Lock-free by design: the tool phase holds the per-session lock across the
//! whole invocation, so a handler that took the lock here would wait for the
//! very call it is meant to interrupt. The signal is level-triggered, so a
//! cancel that lands before the loop subscribes is still observed.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::turn::{CallState, TurnStatus};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct FunctionCancelRequest {
    pub session_id: String,
    /// Omit to target the session's current turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// The iii `function_call_id` of the call to interrupt (the id the
    /// console's card carries as `functionTriggerId`).
    pub function_call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FunctionCancelResponse {
    /// True when the cancel signal was fired for a call this turn is still
    /// running (or about to run). False when there is nothing to interrupt:
    /// no turn, a different/terminal turn, a call already settled, or a call
    /// parked for approval — a parked call is cancelled by denying it
    /// (`harness::function::resolve`), not here.
    pub cancelling: bool,
}

/// Whether a turn in `status` still has a call in `call_state` to interrupt.
/// `Triggered` is the in-flight checkpoint; an unknown call (`None`) is
/// accepted too because the turn may not have checkpointed it yet when the
/// click lands (the signal is level-triggered, so it is observed when the call
/// does start). `Done` and `Pending` have nothing to interrupt.
pub(crate) fn can_cancel(status: TurnStatus, call_state: Option<CallState>) -> bool {
    if status.is_terminal() {
        return false;
    }
    match call_state {
        None | Some(CallState::Triggered) => true,
        Some(CallState::Done) | Some(CallState::Pending) => false,
    }
}

pub async fn handle(
    deps: &Deps,
    req: FunctionCancelRequest,
) -> Result<FunctionCancelResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let Some(record) =
        crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    else {
        return Ok(FunctionCancelResponse { cancelling: false });
    };
    if let Some(tid) = &req.turn_id {
        if &record.turn_id != tid {
            return Ok(FunctionCancelResponse { cancelling: false });
        }
    }
    let call_state = record.calls.get(&req.function_call_id).map(|c| c.state);
    if !can_cancel(record.status, call_state) {
        return Ok(FunctionCancelResponse { cancelling: false });
    }
    // Idempotent: repeat clicks re-set an already-true signal.
    deps.call_cancels.fire(&crate::locks::call_cancel_key(
        &record.turn_id,
        &req.function_call_id,
    ));
    Ok(FunctionCancelResponse { cancelling: true })
}

#[cfg(test)]
mod tests {
    use super::can_cancel;
    use crate::types::turn::{CallState, TurnStatus};

    #[test]
    fn an_in_flight_call_can_be_cancelled() {
        assert!(can_cancel(TurnStatus::Running, Some(CallState::Triggered)));
    }

    /// The click can land before the loop checkpoints the call (the console
    /// learns the call id from the streamed assistant message). The signal is
    /// level-triggered, so accepting the id here is what makes that click
    /// count once the call starts.
    #[test]
    fn a_call_the_turn_has_not_checkpointed_yet_can_be_cancelled() {
        assert!(can_cancel(TurnStatus::Running, None));
    }

    #[test]
    fn a_settled_call_cannot_be_cancelled() {
        assert!(!can_cancel(TurnStatus::Running, Some(CallState::Done)));
    }

    /// A parked call has no invocation to interrupt; the approval row's deny
    /// is the cancel for that state.
    #[test]
    fn a_pending_call_is_not_cancelled_here() {
        assert!(!can_cancel(
            TurnStatus::AwaitingFunctions,
            Some(CallState::Pending)
        ));
    }

    #[test]
    fn a_terminal_turn_has_nothing_to_cancel() {
        for status in [
            TurnStatus::Completed,
            TurnStatus::Cancelled,
            TurnStatus::Failed,
        ] {
            assert!(
                !can_cancel(status, Some(CallState::Triggered)),
                "{status:?}"
            );
            assert!(!can_cancel(status, None), "{status:?}");
        }
    }
}
