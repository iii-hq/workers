//! `steering_check` handler: drain steering queue, follow-up queue, abort flag.

use harness_types::{AgentEvent, AgentMessage, AssistantMessage, ErrorKind, StopReason};
use iii_sdk::{TriggerRequest, III};
use serde_json::json;

use crate::events;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};

/// Pure routing decision for `steering_check`. Lifted out of `handle` so
/// every branch is unit-testable without an `III` instance.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SteeringRoute {
    /// Abort flag was set: synthesize an aborted assistant message and tear down.
    Abort,
    /// External `steering` inbox had messages: append them and run another assistant turn.
    Steering,
    /// External `followup` inbox had messages: append them and run another assistant turn.
    Followup,
    /// No external messages but a function-call batch just finished — feed
    /// the function results back to the assistant for a follow-up turn.
    /// **This is the branch that was missing pre-fix.**
    ContinueAfterFunction,
    /// Nothing more to do — emit `TurnEnd` and tear down.
    EndTurn,
}

// Four bools is one over the clippy default. Each represents an
// independent precondition checked in priority order; collapsing them
// into a struct or bitflag obscures the call site, where naming each
// argument at the boundary is the readability win.
#[allow(clippy::fn_params_excessive_bools)]
pub(crate) fn route(
    abort: bool,
    has_steering: bool,
    has_followup: bool,
    has_function_results: bool,
) -> SteeringRoute {
    if abort {
        SteeringRoute::Abort
    } else if has_steering {
        SteeringRoute::Steering
    } else if has_followup {
        SteeringRoute::Followup
    } else if has_function_results {
        SteeringRoute::ContinueAfterFunction
    } else {
        SteeringRoute::EndTurn
    }
}

pub async fn handle(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let abort = crate::abort::is_set(iii, &record.session_id).await;
    let steering = if abort {
        Vec::new()
    } else {
        drain_queue(iii, "steering", &record.session_id).await
    };
    let followup = if abort || !steering.is_empty() {
        Vec::new()
    } else {
        drain_queue(iii, "followup", &record.session_id).await
    };

    match route(
        abort,
        !steering.is_empty(),
        !followup.is_empty(),
        !record.function_results.is_empty(),
    ) {
        SteeringRoute::Abort => {
            // Abort: build a legacy-shaped aborted message, persist it onto
            // the transcript, then emit TurnEnd carrying it. Mirror of
            // `provider-router/src/loop_state.rs:139-148` and
            // `loop_state.rs:321-332`.
            let aborted = aborted_message();
            let mut messages = persistence::load_messages(iii, &record.session_id).await;
            messages.push(AgentMessage::Assistant(aborted.clone()));
            persistence::save_messages(iii, &record.session_id, &messages).await;
            record.last_assistant = Some(aborted.clone());
            if !record.turn_end_emitted {
                events::emit(
                    iii,
                    &record.session_id,
                    &AgentEvent::TurnEnd {
                        message: AgentMessage::Assistant(aborted),
                        function_results: Vec::new(),
                    },
                )
                .await;
                record.turn_end_emitted = true;
            }
            record.transition_to(TurnState::TearingDown);
        }
        SteeringRoute::Steering => {
            emit_turn_end_once(iii, record).await;
            let mut messages = persistence::load_messages(iii, &record.session_id).await;
            messages.extend(steering);
            persistence::save_messages(iii, &record.session_id, &messages).await;
            // Function results (if any) are already in `messages`; clear the
            // transient signal so the next SteeringCheck doesn't re-loop.
            record.function_results.clear();
            record.transition_to(TurnState::AwaitingAssistant);
        }
        SteeringRoute::Followup => {
            emit_turn_end_once(iii, record).await;
            let mut messages = persistence::load_messages(iii, &record.session_id).await;
            messages.extend(followup);
            persistence::save_messages(iii, &record.session_id, &messages).await;
            record.function_results.clear();
            record.transition_to(TurnState::AwaitingAssistant);
        }
        SteeringRoute::ContinueAfterFunction => {
            // The previous turn already emitted TurnEnd in `function_finalize`
            // (see `states/functions.rs::handle_finalize` — `turn_end_emitted = true`).
            // The next turn will emit its own TurnStart in `handle_awaiting`.
            // Function results are already persisted into `messages` by
            // finalize, so the next assistant turn picks them up via
            // `persistence::load_messages`. Just clear the transient signal.
            record.function_results.clear();
            record.transition_to(TurnState::AwaitingAssistant);
        }
        SteeringRoute::EndTurn => {
            emit_turn_end_once(iii, record).await;
            record.transition_to(TurnState::TearingDown);
        }
    }
    Ok(())
}

/// Emit `TurnEnd` only when the current turn hasn't already emitted one
/// (`functions::handle_finalize` and `assistant::handle_awaiting`/`handle_finished`'s
/// terminating branches set `turn_end_emitted = true`). For no-tool turns
/// reaching `steering_check` from `handle_finished` directly, this is the
/// single emission point. Mirrors legacy `loop_state.rs:194-200`.
async fn emit_turn_end_once(iii: &III, record: &mut TurnStateRecord) {
    if record.turn_end_emitted {
        return;
    }
    let message = AgentMessage::Assistant(record.last_assistant.clone().unwrap_or_else(|| {
        AssistantMessage {
            content: Vec::new(),
            stop_reason: StopReason::End,
            error_message: None,
            error_kind: None,
            usage: None,
            model: String::new(),
            provider: String::new(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }));
    events::emit(
        iii,
        &record.session_id,
        &AgentEvent::TurnEnd {
            message,
            function_results: Vec::new(),
        },
    )
    .await;
    record.turn_end_emitted = true;
}

/// Legacy-shaped aborted assistant message — mirror of
/// `provider-router/src/loop_state.rs:321-332`. Kept private to the
/// steering handler since this is the only place an abort produces a
/// synthetic message in the durable path.
fn aborted_message() -> AssistantMessage {
    AssistantMessage {
        content: Vec::new(),
        stop_reason: StopReason::Aborted,
        error_message: Some("aborted".into()),
        error_kind: Some(ErrorKind::Transient),
        usage: None,
        model: "harness".into(),
        provider: "harness".into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    }
}

async fn drain_queue(iii: &III, name: &str, session_id: &str) -> Vec<AgentMessage> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "session-inbox::drain".into(),
            payload: json!({ "name": name, "session_id": session_id }),
            action: None,
            timeout_ms: None,
        })
        .await;
    let value = match resp {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    value
        .get("items")
        .cloned()
        .map(serde_json::from_value::<Vec<AgentMessage>>)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aborted_message_matches_legacy_shape() {
        let m = aborted_message();
        assert_eq!(m.stop_reason, StopReason::Aborted);
        assert_eq!(m.model, "harness");
        assert_eq!(m.provider, "harness");
        assert!(matches!(m.error_kind, Some(ErrorKind::Transient)));
        assert_eq!(m.error_message.as_deref(), Some("aborted"));
        assert!(m.content.is_empty());
    }

    // ── route() coverage ──────────────────────────────────────────────
    // Pins every branch of the steering-check routing decision so a
    // future refactor can't silently drop a transition. Writing these as
    // pure-fn assertions instead of integration tests keeps them fast
    // and removes the III dependency.

    #[test]
    fn route_abort_wins_over_everything() {
        // Abort flag dominates even when queues and function_results are present.
        assert_eq!(route(true, true, true, true), SteeringRoute::Abort);
        assert_eq!(route(true, false, false, false), SteeringRoute::Abort);
    }

    #[test]
    fn route_steering_takes_precedence_over_followup_and_function_results() {
        assert_eq!(
            route(false, true, true, true),
            SteeringRoute::Steering,
            "steering inbox messages must be drained before followup or function-results continuation"
        );
    }

    #[test]
    fn route_followup_takes_precedence_over_function_results() {
        assert_eq!(
            route(false, false, true, true),
            SteeringRoute::Followup,
            "followup inbox messages must be drained before function-results continuation"
        );
    }

    /// **Regression pin: the bug that caused the agent to stop after a function call.**
    ///
    /// Before the fix, `SteeringCheck` reached from `FunctionFinalize` with
    /// no external messages fell through to `EndTurn`, so the model never
    /// saw the function result. This test asserts the new branch routes
    /// the loop back to `AwaitingAssistant` instead.
    #[test]
    fn route_continues_to_assistant_when_function_results_present() {
        assert_eq!(
            route(false, false, false, true),
            SteeringRoute::ContinueAfterFunction,
            "post-function SteeringCheck must continue to AwaitingAssistant so the model sees the result"
        );
    }

    #[test]
    fn route_ends_turn_when_nothing_pending() {
        // No function call, no inbox messages, no abort — this is a clean
        // text-only assistant response: the turn legitimately ends here.
        assert_eq!(route(false, false, false, false), SteeringRoute::EndTurn);
    }
}
