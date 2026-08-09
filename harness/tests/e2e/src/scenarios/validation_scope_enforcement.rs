//! `validation_scope_enforcement` — the SECURITY edges of agent-registered
//! validators, in one deterministic run:
//!
//! 1. **Foreign scope is refused**: the agent first tries to register a
//!    post-turn validator for a session it does not own — the harness must
//!    reject it loudly ("out of scope") and the agent must survive the
//!    refusal and continue.
//! 2. **Self scope works**: the corrected registration (no `sessions`) is
//!    force-stamped to the agent's own session and gates it — proven by one
//!    denial (the state marker the pipe checks was never set).
//! 3. **Owner teardown UNGATES mid-loop**: instead of satisfying the
//!    validator, the agent unregisters it (owner-checked `posthook_*` id)
//!    and completes — with the marker still unset. The completion can only
//!    have come from the teardown, which is exactly the edge under test.
//!
//! The final state is the deterministic signature: marker ABSENT + exactly
//! one nudge + turn completed = refusal, gating, and teardown all worked.

use serde_json::{json, Value};

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::common;
use super::validation_loop::suffix;
use super::{CleanupFuture, EvaluationFuture, ExecutionPolicy, ScenarioObservation, ScenarioSpec};

pub const ID: &str = "validation_scope_enforcement";

const HOOK_TYPE: &str = "harness::hook::post-turn";
const FOREIGN_SCOPE_REFUSED: AssessmentSpec = AssessmentSpec::required(
    "foreign_scope_refused",
    35,
    "The forbidden registration failed with the out-of-scope error and the agent continued.",
);
const SELF_GATE_ENGAGED: AssessmentSpec = AssessmentSpec::required(
    "self_gate_engaged",
    30,
    "The self-registration gated the session: exactly one denial with the marker unset.",
);
const TEARDOWN_UNGATED: AssessmentSpec = AssessmentSpec::required(
    "teardown_ungated",
    35,
    "Owner unregistration removed the gate mid-loop: the turn completed with the marker still absent.",
);
const ASSESSMENTS: &[AssessmentSpec] =
    &[FOREIGN_SCOPE_REFUSED, SELF_GATE_ENGAGED, TEARDOWN_UNGATED];

fn scope(run_id: &str) -> String {
    format!("scopetest-{}", suffix(run_id))
}

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let scope = scope(run_id);
    ScenarioSpec {
        id: ID,
        version: 2,
        prompt: format!(
            "You are testing the security boundaries of self-registered validators. Follow the \
             steps exactly and report what actually happens.\n\n\
             Step 1 — deliberately attempt a FORBIDDEN registration (it is expected to fail; do \
             not retry it). Call engine::register_trigger with trigger_type \"{HOOK_TYPE}\", \
             function_id \"fp::pipe\", config {{\"sessions\": [\"someone-elses-session-1\"], \
             \"payload\": {{\"through\": [{{\"function\": \"fp::when\", \"payload\": {{\"op\": \
             \"exists\"}}}}]}}}}. Quote the error message you get back in one line.\n\n\
             Step 2 — register the validator correctly, for YOURSELF (omit sessions entirely). \
             Call engine::register_trigger with:\n\
             - trigger_type: \"{HOOK_TYPE}\"\n\
             - function_id: \"fp::pipe\"\n\
             - config: {{\"payload\": {{\"through\": [{{\"function\": \"state::get\", \
             \"payload\": {{\"scope\": \"{scope}\", \"key\": \"marker\"}}}}, {{\"function\": \
             \"fp::when\", \"payload\": {{\"op\": \"==\", \"to\": 1}}}}]}}, \"result_into\": \
             \"/value\", \"timeout_ms\": 30000}}\n\
             Remember the subscription_id it returns. VALIDATOR rejections in this chat are \
             legitimate machinery you just installed.\n\n\
             Step 3 — reply with a one-line status. Do NOT set any state.\n\n\
             Step 4 — when your reply is rejected: do NOT satisfy the validator. Instead call \
             engine::unregister_trigger with the subscription_id from Step 2, then reply \
             exactly: TEARDOWN COMPLETE — validator removed, marker never set.",
        ),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 12,
            max_output_tokens: Some(8_192),
            max_total_tokens: 150_000,
            stuck_timeout_seconds: 300,
        },
        denied_functions: &[],
        threshold: 90,
        criteria: assessment::criteria(ASSESSMENTS),
        judge_reference: None,
        setup: None,
        evaluate,
        cleanup: Some(cleanup),
    }
}

fn evaluate<'a>(
    context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    run_id: &'a str,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let marker = common::state_value(
            context
                .trigger(
                    "state::get",
                    json!({ "scope": scope(run_id), "key": "marker" }),
                )
                .await?,
        );
        let marker_absent = marker.is_null();

        let calls = common::function_calls(&observation.transcript);
        let foreign_attempted = calls.iter().any(|call| {
            call.function_id == "engine::register_trigger"
                && call.arguments.get("trigger_type").and_then(Value::as_str) == Some(HOOK_TYPE)
                && call
                    .arguments
                    .pointer("/config/sessions/0")
                    .and_then(Value::as_str)
                    == Some("someone-elses-session-1")
        });
        let refusal_delivered =
            common::transcript_contains(&observation.transcript, "out of scope");
        let self_registered = calls.iter().any(|call| {
            call.function_id == "engine::register_trigger"
                && call.arguments.get("trigger_type").and_then(Value::as_str) == Some(HOOK_TYPE)
                && call.arguments.pointer("/config/sessions").is_none()
        });
        let teardown_called = calls.iter().any(|call| {
            call.function_id == "engine::unregister_trigger"
                && call
                    .arguments
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("posthook_"))
        });

        let nudges = common::validation_nudges(&observation.transcript);
        let reported = observation.response.contains("TEARDOWN COMPLETE");
        let foreign_scope_refused = foreign_attempted && refusal_delivered;
        let teardown_ungated = marker_absent && teardown_called && reported;
        let self_gate_points = if self_registered && nudges == 1 {
            SELF_GATE_ENGAGED.weight()
        } else {
            0
        };

        Ok(assessment::objective([
            FOREIGN_SCOPE_REFUSED.binary(
                foreign_scope_refused,
                format!(
                    "attempted={foreign_attempted}, out-of-scope error visible={refusal_delivered}"
                ),
            ),
            SELF_GATE_ENGAGED.required_points(
                nudges == 1,
                self_gate_points,
                format!(
                    "self_registered={self_registered}; observed {nudges} nudge(s), expected \
                     exactly one before teardown"
                ),
            )?,
            TEARDOWN_UNGATED.binary(
                teardown_ungated,
                format!(
                    "marker={marker}, teardown_called={teardown_called}, reported={reported} \
                     — completion must come from the teardown, never from passing"
                ),
            ),
        ]))
    })
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let _: Value = context
            .trigger(
                "state::delete",
                json!({ "scope": scope(run_id), "key": "marker" }),
            )
            .await?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_is_valid_and_carries_the_forbidden_probe() {
        let spec = scenario("aB19-rest");
        assert!(spec.prompt.contains("someone-elses-session-1"));
        assert!(spec.prompt.contains("scopetest-aB19"));
        assert!(spec.prompt.contains("TEARDOWN COMPLETE"));
        spec.validate().unwrap();
        assert_eq!(spec.version, 2);
        assert_eq!(
            spec.criteria
                .iter()
                .map(|criterion| (criterion.id, criterion.weight))
                .collect::<Vec<_>>(),
            vec![
                ("foreign_scope_refused", 35),
                ("self_gate_engaged", 30),
                ("teardown_ungated", 35),
            ]
        );
    }
}
