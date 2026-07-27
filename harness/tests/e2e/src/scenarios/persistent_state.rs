use harness::types::turn::FunctionPolicy;
use serde_json::{json, Value};

use crate::context::E2eContext;
use crate::report::CriterionSource;

use super::common;
use super::{
    CleanupFuture, CriterionSpec, EvaluationFuture, ExecutionPolicy, ModelRequirements,
    ObjectiveEvaluation, ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "persistent_state";

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let scope = format!("e2e:{run_id}");
    let key = "persistent_state";
    let expected = json!({
        "owner": "quality-suite",
        "run_id": run_id,
        "status": "stored"
    });
    ScenarioSpec {
        id: ID,
        prompt: format!(
            "Store this JSON value for later use in scope `{scope}` under key `{key}`: {}. Confirm briefly after it has been stored.",
            serde_json::to_string(&expected).expect("serialize static scenario value")
        ),
        evaluation_context: json!({
            "scope": scope,
            "key": key,
            "expected": expected,
        }),
        functions: FunctionPolicy {
            allow: vec![
                "engine::functions::list".into(),
                "engine::functions::info".into(),
                "state::get".into(),
                "state::set".into(),
            ],
            ..FunctionPolicy::default()
        },
        requirements: ModelRequirements {
            tools: true,
            minimum_context_window: 32_768,
            minimum_output_tokens: 2_048,
            ..ModelRequirements::default()
        },
        execution: ExecutionPolicy {
            max_turns: 12,
            max_output_tokens: 8_192,
            max_total_tokens: 122_880,
            timeout_seconds: 240,
            thinking_level: None,
        },
        threshold: 90,
        criteria: vec![
            CriterionSpec {
                id: "durable_result",
                source: CriterionSource::Objective,
                weight: 60,
                description: "The exact requested JSON is present at the requested state key.",
            },
            CriterionSpec {
                id: "function_discipline",
                source: CriterionSource::Objective,
                weight: 30,
                description: "Exactly one successful write targets the requested scope and key.",
            },
            CriterionSpec {
                id: "confirmation",
                source: CriterionSource::Objective,
                weight: 10,
                description: "The final response briefly confirms completion.",
            },
        ],
        judge_reference: None,
        evaluate,
        cleanup,
    }
}

fn evaluate<'a>(
    context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    evaluation_context: &'a Value,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let scope = string_field(evaluation_context, "scope")?;
        let key = string_field(evaluation_context, "key")?;
        let expected = &evaluation_context["expected"];
        let observed = common::state_value(
            context
                .trigger("state::get", json!({ "scope": scope, "key": key }))
                .await?,
        );
        let calls = common::function_calls(&observation.transcript);
        let writes: Vec<_> = calls
            .iter()
            .filter(|call| call.function_id == "state::set")
            .collect();
        let exact_write = writes.len() == 1
            && writes[0].arguments == json!({ "scope": scope, "key": key, "value": expected });
        let state_matches = observed == *expected;
        let no_errors = observation.metrics.totals.function_call_errors == 0;
        let response = common::final_response(&observation.output);
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
            evidence: json!({
                "expected": expected,
                "actual": observed,
                "state_writes": writes.iter().map(|call| &call.arguments).collect::<Vec<_>>(),
                "final_response": response,
            }),
        })
    })
}

fn cleanup<'a>(context: &'a E2eContext, evaluation_context: &'a Value) -> CleanupFuture<'a> {
    Box::pin(async move {
        let scope = string_field(evaluation_context, "scope")?;
        let key = string_field(evaluation_context, "key")?;
        let _: Value = context
            .trigger("state::delete", json!({ "scope": scope, "key": key }))
            .await?;
        Ok(())
    })
}

fn string_field<'a>(value: &'a Value, field: &str) -> anyhow::Result<&'a str> {
    value[field]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing evaluation field {field}"))
}
