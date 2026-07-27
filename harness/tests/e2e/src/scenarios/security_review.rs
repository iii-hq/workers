use serde_json::json;

use crate::context::E2eContext;
use crate::report::CriterionSource;

use super::common;
use super::{
    CriterionSpec, EvaluationFuture, ExecutionPolicy, ModelRequirements, ObjectiveEvaluation,
    ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "security_review";

pub fn scenario(_run_id: &str) -> ScenarioSpec {
    ScenarioSpec {
        id: ID,
        prompt: r#"Review these independent snippets for security vulnerabilities:

1. `const API_KEY = "sk-live-example";`
2. `db.execute("SELECT * FROM users WHERE id = " + request.id);`
3. `response.redirect(request.query.next);`

For each snippet, identify the vulnerability, explain its impact, and recommend a practical remediation. Keep the review concise and do not perform any external action."#
            .into(),
        requirements: ModelRequirements {
            minimum_context_window: 16_384,
            minimum_output_tokens: 2_048,
            ..ModelRequirements::default()
        },
        execution: ExecutionPolicy {
            max_turns: 2,
            max_output_tokens: 4_096,
            max_total_tokens: 49_152,
            timeout_seconds: 120,
            thinking_level: None,
        },
        threshold: 80,
        criteria: vec![
            CriterionSpec {
                id: "coverage",
                source: CriterionSource::Judge,
                weight: 40,
                description: "Identifies the relevant vulnerability in all three snippets.",
            },
            CriterionSpec {
                id: "accuracy",
                source: CriterionSource::Judge,
                weight: 30,
                description: "Explains each risk accurately without invented findings.",
            },
            CriterionSpec {
                id: "remediation",
                source: CriterionSource::Judge,
                weight: 20,
                description: "Provides a practical mitigation for every finding.",
            },
            CriterionSpec {
                id: "clarity",
                source: CriterionSource::Judge,
                weight: 10,
                description: "Presents a concise, easy-to-map review.",
            },
        ],
        judge_reference: Some(json!({
            "snippet_1": {
                "finding": "hardcoded credential or secret exposure",
                "impact": "credential leakage and unauthorized API access",
                "remediation": "remove and rotate the secret; load it from a secret manager or environment"
            },
            "snippet_2": {
                "finding": "SQL injection",
                "impact": "attacker-controlled query execution or data access",
                "remediation": "use parameterized queries or prepared statements"
            },
            "snippet_3": {
                "finding": "open redirect",
                "impact": "phishing or redirecting users to attacker-controlled destinations",
                "remediation": "allowlist destinations or accept only validated relative paths"
            }
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
                        "the assistant returned no review"
                    } else {
                        "the assistant returned a review"
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
