use serde_json::{json, Value};

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::common;
use super::{
    CleanupFuture, EvaluationFuture, ExecutionPolicy, ObjectiveEvaluation, ScenarioObservation,
    ScenarioSpec,
};

pub const ID: &str = "persistent_state";
const KEY: &str = "persistent_state";
const DURABLE_RESULT: AssessmentSpec = AssessmentSpec::hard_gated(
    "durable_result",
    60,
    "The exact requested JSON is present at the requested state key.",
);
const FUNCTION_DISCIPLINE: AssessmentSpec = AssessmentSpec::hard_gated(
    "function_discipline",
    30,
    "Exactly one successful write targets the requested scope and key.",
);
const CONFIRMATION: AssessmentSpec = AssessmentSpec::score_only(
    "confirmation",
    10,
    "The final response briefly confirms completion.",
);
const ASSESSMENTS: &[AssessmentSpec] = &[DURABLE_RESULT, FUNCTION_DISCIPLINE, CONFIRMATION];

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let scope = scope(run_id);
    let expected = expected(run_id);
    ScenarioSpec {
        id: ID,
        version: 3,
        prompt: format!(
            "Store this JSON value for later use in scope `{scope}` under key `{KEY}`: {}. Confirm briefly after it has been stored.",
            serde_json::to_string(&expected).expect("serialize static scenario value")
        ),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 12,
            max_output_tokens: Some(8_192),
            max_total_tokens: 122_880,
            stuck_timeout_seconds: 240,
        },
        denied_functions: &[],
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
        let scope = scope(run_id);
        let expected = expected(run_id);
        let observed = common::state_value(
            context
                .trigger("state::get", json!({ "scope": scope, "key": KEY }))
                .await?,
        );
        let calls = common::function_calls(&observation.transcript);
        let writes: Vec<_> = calls
            .iter()
            .filter(|call| call.function_id == "state::set")
            .collect();
        let exact_write = writes.len() == 1
            && writes[0].arguments == json!({ "scope": scope, "key": KEY, "value": expected });
        assess(
            &expected,
            &observed,
            exact_write,
            writes.len(),
            observation.metrics.totals.function_call_errors,
            observation.response.as_str(),
        )
    })
}

fn assess(
    expected: &Value,
    observed: &Value,
    exact_write: bool,
    state_set_calls: usize,
    function_call_errors: u64,
    response: &str,
) -> anyhow::Result<ObjectiveEvaluation> {
    let state_matches = observed == expected;
    let function_discipline = exact_write && function_call_errors == 0;
    let response_present = !response.trim().is_empty();
    let response_chars = response.chars().count();
    let concise_confirmation = response_present && response_chars <= 240;
    let confirmation_points = if concise_confirmation {
        CONFIRMATION.weight()
    } else {
        0
    };

    Ok(assessment::build_evaluation([
        DURABLE_RESULT.full_or_zero(
            state_matches,
            format!("expected {expected}, observed {observed}"),
        ),
        FUNCTION_DISCIPLINE.full_or_zero(
            function_discipline,
            format!(
                "exact_write={exact_write}; observed {state_set_calls} state::set call(s); observed {function_call_errors} function-call error(s)"
            ),
        ),
        CONFIRMATION.award(
            confirmation_points,
            format!(
                "response_present={response_present}; observed {response_chars} character(s); limit 240"
            ),
        )?,
    ]))
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let scope = scope(run_id);
        let _: Value = context
            .trigger("state::delete", json!({ "scope": scope, "key": KEY }))
            .await?;
        Ok(())
    })
}

fn scope(run_id: &str) -> String {
    format!("e2e:{run_id}")
}

fn expected(run_id: &str) -> Value {
    json!({
        "owner": "quality-suite",
        "run_id": run_id,
        "status": "stored"
    })
}
