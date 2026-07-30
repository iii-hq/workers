use serde_json::{json, Value};

use crate::context::E2eContext;

use super::common;
use super::{
    CleanupFuture, CriterionSpec, EvaluationFuture, ExecutionPolicy, ObjectiveEvaluation,
    ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "persistent_state";
const KEY: &str = "persistent_state";

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let scope = scope(run_id);
    let expected = expected(run_id);
    ScenarioSpec {
        id: ID,
        prompt: format!(
            "Store this JSON value for later use in scope `{scope}` under key `{KEY}`: {}. Confirm briefly after it has been stored.",
            serde_json::to_string(&expected).expect("serialize static scenario value")
        ),
        execution: ExecutionPolicy {
            max_turns: 12,
            max_output_tokens: Some(8_192),
            max_total_tokens: 122_880,
            stuck_timeout_seconds: 240,
        },
        denied_functions: &[],
        threshold: 90,
        criteria: vec![
            CriterionSpec {
                id: "durable_result",
                weight: 60,
                description: "The exact requested JSON is present at the requested state key.",
            },
            CriterionSpec {
                id: "function_discipline",
                weight: 30,
                description: "Exactly one successful write targets the requested scope and key.",
            },
            CriterionSpec {
                id: "confirmation",
                weight: 10,
                description: "The final response briefly confirms completion.",
            },
        ],
        judge_reference: None,
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
        let state_matches = observed == expected;
        let no_errors = observation.metrics.totals.function_call_errors == 0;
        let response = observation.response.as_str();
        let concise_confirmation = !response.trim().is_empty() && response.chars().count() <= 240;

        Ok(ObjectiveEvaluation {
            hard_gates: vec![
                common::gate(
                    "state_persisted",
                    state_matches,
                    format!("expected {expected}, observed {observed}"),
                ),
                common::gate(
                    "single_exact_write",
                    exact_write,
                    format!("observed {} state::set call(s)", writes.len()),
                ),
                common::gate(
                    "no_function_errors",
                    no_errors,
                    format!(
                        "observed {} function-call error(s)",
                        observation.metrics.totals.function_call_errors
                    ),
                ),
            ],
            awards: vec![
                common::award(
                    "durable_result",
                    if state_matches { 60 } else { 0 },
                    "awarded when the durable value exactly matches",
                ),
                common::award(
                    "function_discipline",
                    if exact_write && no_errors { 30 } else { 0 },
                    "awarded for one exact, successful state write",
                ),
                common::award(
                    "confirmation",
                    if concise_confirmation { 10 } else { 0 },
                    "awarded for a non-empty confirmation under 240 characters",
                ),
            ],
        })
    })
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
