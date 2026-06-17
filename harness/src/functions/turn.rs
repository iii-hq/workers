//! `harness::turn` — the internal durable loop step (enqueued onto the
//! `default` queue). Consumers never call it directly. An unexpected
//! step error finalises the turn as failed rather than retrying forever.

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::turn_loop::{self, TurnStepPayload, TurnStepResult};

pub async fn handle(deps: &Deps, payload: TurnStepPayload) -> Result<TurnStepResult, HarnessError> {
    let (session_id, turn_id) = (payload.session_id.clone(), payload.turn_id.clone());
    match turn_loop::run_step(deps, payload).await {
        Ok(result) => Ok(result),
        Err(e) => {
            tracing::error!(session_id = %session_id, turn_id = %turn_id, error = %e, "turn step failed; finalising turn as failed");
            Ok(turn_loop::fail_turn(deps, &session_id, &turn_id, &e.to_string()).await)
        }
    }
}
