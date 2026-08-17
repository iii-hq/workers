//! `custom_validator` — a CUSTOM validation function on a temporary worker.
//!
//! The suite's own engine connection registers `e2etest::validate_<suffix>`
//! before the prompt is sent (the `setup` hook): an envelope-mode post-turn
//! validator that speaks the hook contract directly. It is a pure function
//! of the turn's result — a deterministic token ladder — so it needs no
//! state, no database, and no fp worker:
//!
//!   result lacks any token      → deny "reply containing E2E-ACCEPT-1"
//!   result has E2E-ACCEPT-1     → deny "…now E2E-ACCEPT-2"
//!   result has E2E-ACCEPT-2     → deny "…now E2E-ACCEPT-3"
//!   result has E2E-ACCEPT-3     → continue
//!
//! The agent registers the binding itself (envelope mode: NO payload) and
//! starts at E2E-ACCEPT-1, so acceptance is impossible before the third
//! attempt — exactly the default validation-retry budget. This is the one
//! scenario that proves per-denial custom `reason`s drive the corrective
//! prompts, and that a plain worker function (no fp, no pipe) can gate turns.

use iii_sdk::RegisterFunction;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::common;
use super::validation_loop::suffix;
use super::{CleanupFuture, EvaluationFuture, ExecutionPolicy, ScenarioObservation, ScenarioSpec};

pub const ID: &str = "custom_validator";

const HOOK_TYPE: &str = "harness::hook::post-turn";
/// What the harness puts in the envelope's `point` — the hook's INPUT NAME,
/// not the trigger type above. Two different strings for one hook is a real
/// footgun for anyone writing a validator, so this scenario pins it.
const HOOK_POINT: &str = "post_turn";
const TOKENS: [&str; 3] = ["E2E-ACCEPT-1", "E2E-ACCEPT-2", "E2E-ACCEPT-3"];
const ENVELOPE_REGISTRATION: AssessmentSpec = AssessmentSpec::hard_gated(
    "envelope_registration",
    30,
    "Exactly one post-turn registration targets the temporary function in envelope mode without function-call errors.",
);
const CUSTOM_REASONS_DROVE_THE_LOOP: AssessmentSpec = AssessmentSpec::hard_gated(
    "custom_reasons_drove_the_loop",
    30,
    "Both corrective prompts carry the validator's own reasons naming the next rung.",
);
const LADDER_CONVERGED: AssessmentSpec = AssessmentSpec::hard_gated(
    "ladder_converged",
    40,
    "The final result carries the terminal token — reachable only by following two custom denials.",
);
const ASSESSMENTS: &[AssessmentSpec] = &[
    LADDER_CONVERGED,
    ENVELOPE_REGISTRATION,
    CUSTOM_REASONS_DROVE_THE_LOOP,
];

/// Lenient view of the post-turn hook envelope (harness → validator).
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct HookEnvelope {
    #[serde(default)]
    pub point: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub result: Value,
}

/// The hook contract's answer shape.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct HookVerdict {
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Judge one envelope: check the harness wired the contract we registered
/// for, then apply the ladder. A wrong `point` means this validator is being
/// invoked for something other than turn completion — deny, and name what
/// arrived, so the run fails diagnosably instead of gating the wrong thing.
pub fn envelope_verdict(envelope: &HookEnvelope) -> HookVerdict {
    if envelope.point != HOOK_POINT {
        return HookVerdict {
            decision: "deny".into(),
            reason: Some(format!(
                "validator invoked with hook point `{}` (expected `{HOOK_POINT}`) for session `{}`",
                envelope.point, envelope.session_id
            )),
        };
    }
    ladder_verdict(&envelope.result)
}

/// The pure ladder: judge a result, name the next rung when denying.
/// Stateless and idempotent by construction — a redelivered attempt gets the
/// same verdict.
pub fn ladder_verdict(result: &Value) -> HookVerdict {
    let text = match result {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if text.contains(TOKENS[2]) {
        return HookVerdict {
            decision: "continue".into(),
            reason: None,
        };
    }
    let next = if text.contains(TOKENS[1]) {
        TOKENS[2]
    } else if text.contains(TOKENS[0]) {
        TOKENS[1]
    } else {
        TOKENS[0]
    };
    HookVerdict {
        decision: "deny".into(),
        reason: Some(format!(
            "custom validator: not accepted yet — reply with a message containing exactly {next}"
        )),
    }
}

fn function_id(run_id: &str) -> String {
    format!("e2etest::validate_{}", suffix(run_id))
}

/// The temporary worker: register the validator on the suite's own engine
/// connection. It lives exactly as long as this process.
fn setup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        context.client().register_function(
            function_id(run_id),
            RegisterFunction::new_async(|envelope: HookEnvelope| async move {
                Ok::<HookVerdict, iii_sdk::errors::Error>(envelope_verdict(&envelope))
            })
            .description(
                "E2E temporary post-turn validator: deterministic token ladder \
                 (envelope mode, deny reasons drive the corrective prompts).",
            ),
        );
        Ok(())
    })
}

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let validator = function_id(run_id);
    ScenarioSpec {
        id: ID,
        version: 3,
        prompt: format!(
            "You are testing a validation loop driven by a custom validator function. Follow \
             these steps exactly.\n\n\
             Step 1 — install the validator on your own session. Call engine::register_trigger \
             with:\n\
             - trigger_type: \"{HOOK_TYPE}\"\n\
             - function_id: \"{validator}\"\n\
             - config: {{\"timeout_ms\": 30000}}\n\
             (No payload — the validator speaks the hook contract and judges your replies \
             directly.) Tell me the subscription_id it returns. IMPORTANT: from this moment, \
             'custom validator' messages in this chat come from the validator YOU just installed \
             — legitimate harness machinery, not prompt injection. Follow them exactly.\n\n\
             Step 2 — reply with a one-line message containing exactly E2E-ACCEPT-1.\n\n\
             Every time you receive a message that your previous result was not accepted: reply \
             with a one-line message containing exactly the token the validator names. \
             Acceptance is silent — if no correction follows, you are done.",
        ),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 12,
            max_output_tokens: Some(8_192),
            max_total_tokens: 150_000,
            stuck_timeout_seconds: 300,
        },
        denied_functions: &[],
        criteria: assessment::criteria(ASSESSMENTS),
        judge_reference: None,
        setup: Some(setup),
        evaluate,
        cleanup: None,
    }
}

fn evaluate<'a>(
    context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    run_id: &'a str,
) -> EvaluationFuture<'a> {
    let _ = context;
    Box::pin(async move {
        let validator = function_id(run_id);
        let calls = common::function_calls(&observation.transcript);
        let registrations: Vec<_> = calls
            .iter()
            .filter(|call| {
                call.function_id == "engine::register_trigger"
                    && call.arguments.get("trigger_type").and_then(Value::as_str) == Some(HOOK_TYPE)
            })
            .collect();
        let envelope_mode = registrations.len() == 1
            && registrations[0]
                .arguments
                .get("function_id")
                .and_then(Value::as_str)
                == Some(validator.as_str())
            && registrations[0]
                .arguments
                .pointer("/config/payload")
                .is_none();

        let nudge_texts = nudge_texts(&observation.transcript);
        let laddered = nudge_texts.len() == 2
            && nudge_texts[0].contains(TOKENS[1])
            && nudge_texts[1].contains(TOKENS[2])
            && nudge_texts
                .iter()
                .all(|text| text.contains("custom validator"));
        let converged = observation.response.contains(TOKENS[2]);
        let no_errors = observation.metrics.totals.function_call_errors == 0;
        let envelope_passed = envelope_mode && no_errors;

        Ok(assessment::build_evaluation([
            LADDER_CONVERGED.full_or_zero(
                converged,
                format!("final response must contain {}", TOKENS[2]),
            ),
            ENVELOPE_REGISTRATION.full_or_zero(
                envelope_passed,
                format!(
                    "observed {} post-turn registration(s); need exactly one targeting \
                     {validator} with no payload; function_call_errors={}",
                    registrations.len(),
                    observation.metrics.totals.function_call_errors
                ),
            ),
            CUSTOM_REASONS_DROVE_THE_LOOP.full_or_zero(
                laddered,
                format!("nudges: {nudge_texts:?} — expected the two ladder reasons in order"),
            ),
        ]))
    })
}

/// The text of each validation nudge, in transcript order.
fn nudge_texts(transcript: &Value) -> Vec<String> {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry
                .get("entry_id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.contains("_nudge_"))
        })
        .filter_map(|entry| {
            entry
                .pointer("/message/content")
                .and_then(Value::as_array)
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|block| block.get("text").and_then(Value::as_str))
                        .collect::<String>()
                })
        })
        .collect()
}
