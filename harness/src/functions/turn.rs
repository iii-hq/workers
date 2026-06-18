//! `harness::turn` — the internal durable loop step (enqueued onto the
//! `default` queue). Consumers never call it directly. An unexpected
//! step error finalises the turn as failed rather than retrying forever.

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::turn_loop::{self, TurnStepPayload, TurnStepResult};

pub async fn handle(deps: &Deps, payload: TurnStepPayload) -> Result<TurnStepResult, HarnessError> {
    // Stamp this turn's identity into OTel baggage for the whole step. Every
    // span the step spawns — session::*, router::chat, and (via the
    // spawn-context propagation in the router/provider clients)
    // provider::*::stream — then carries `iii.session.id`/`iii.message.id`
    // (the BaggageSpanProcessor allowlist), so the turn's spans are
    // attributable and groupable by session/message. `iii.message.id` is the
    // turn id: one assistant turn is one grouped "message".
    let session_id = payload.session_id.clone();
    let turn_id = payload.turn_id.clone();
    let baggage = [
        ("iii.session.id", session_id.as_str()),
        ("iii.message.id", turn_id.as_str()),
    ];
    iii_observability::run_with_baggage(&baggage, run(deps, payload)).await
}

async fn run(deps: &Deps, payload: TurnStepPayload) -> Result<TurnStepResult, HarnessError> {
    let (session_id, turn_id) = (payload.session_id.clone(), payload.turn_id.clone());
    match turn_loop::run_step(deps, payload).await {
        Ok(result) => Ok(result),
        Err(e) => {
            tracing::error!(session_id = %session_id, turn_id = %turn_id, error = %e, "turn step failed; finalising turn as failed");
            Ok(turn_loop::fail_turn(deps, &session_id, &turn_id, &e.to_string()).await)
        }
    }
}
