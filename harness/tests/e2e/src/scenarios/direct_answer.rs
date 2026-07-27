use serde_json::json;

use crate::context::E2eContext;
use crate::report::CriterionSource;

use super::common;
use super::{
    CriterionSpec, EvaluationFuture, ExecutionPolicy, ModelRequirements, ObjectiveEvaluation,
    ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "direct_answer";

pub fn scenario(_run_id: &str) -> ScenarioSpec {
    ScenarioSpec {
        id: ID,
        prompt: "Explain to a non-technical reader, in at most two sentences, the difference between authentication and authorization. Do not perform any external action.".into(),
        requirements: ModelRequirements {
            minimum_context_window: 8_192,
            minimum_output_tokens: 1_024,
            ..ModelRequirements::default()
        },
        execution: ExecutionPolicy {
            max_turns: 2,
            max_output_tokens: 2_048,
            max_total_tokens: 32_768,
            timeout_seconds: 120,
            thinking_level: None,
        },
        threshold: 80,
        criteria: vec![
            CriterionSpec {
                id: "correctness",
                source: CriterionSource::Judge,
                weight: 50,
                description: "Correctly distinguishes proving identity from deciding permissions.",
            },
            CriterionSpec {
                id: "clarity",
                source: CriterionSource::Judge,
                weight: 30,
                description: "Uses language a non-technical reader can understand.",
            },
            CriterionSpec {
                id: "instruction_adherence",
                source: CriterionSource::Judge,
                weight: 20,
                description: "Answers directly in no more than two sentences.",
            },
        ],
        judge_reference: Some(json!({
            "authentication": "verifies who a user or system is",
            "authorization": "decides what an authenticated identity may access or do",
            "format": "at most two sentences for a non-technical reader"
        })),
        evaluate,
        cleanup: None,
    }
}

fn evaluate<'a>(
    _context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    _run_id: &'a str,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let calls = common::function_calls(&observation.transcript);
        let response = common::final_response(&observation.output);
        Ok(ObjectiveEvaluation {
            hard_gates: vec![
                common::gate(
                    "response_present",
                    !response.trim().is_empty(),
                    if response.trim().is_empty() {
                        "the assistant returned no text"
                    } else {
                        "the assistant returned a textual answer"
                    },
                ),
                common::gate(
                    "no_function_calls",
                    calls.is_empty() && observation.metrics.totals.function_calls == 0,
                    format!("observed {} function call(s)", calls.len()),
                ),
                common::gate(
                    "single_turn",
                    observation.metrics.totals.turns == 1,
                    format!(
                        "observed {} turn(s), expected exactly one",
                        observation.metrics.totals.turns
                    ),
                ),
            ],
            awards: Vec::new(),
            evidence: json!({ "final_response": response }),
        })
    })
}
