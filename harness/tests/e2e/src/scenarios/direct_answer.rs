use serde_json::json;

use super::common;
use super::{CriterionSpec, ExecutionPolicy, ScenarioSpec};

pub const ID: &str = "direct_answer";

pub fn scenario(_run_id: &str) -> ScenarioSpec {
    ScenarioSpec {
        id: ID,
        prompt: "Explain to a non-technical reader, in at most two sentences, the difference between authentication and authorization. Do not perform any external action.".into(),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 2,
            max_output_tokens: Some(2_048),
            max_total_tokens: 32_768,
            stuck_timeout_seconds: 120,
        },
        threshold: 80,
        criteria: vec![
            CriterionSpec {
                id: "correctness",
                weight: 50,
                description: "Correctly distinguishes proving identity from deciding permissions.",
            },
            CriterionSpec {
                id: "clarity",
                weight: 30,
                description: "Uses language a non-technical reader can understand.",
            },
            CriterionSpec {
                id: "instruction_adherence",
                weight: 20,
                description: "Answers directly in no more than two sentences.",
            },
        ],
        judge_reference: Some(json!({
            "authentication": "verifies who a user or system is",
            "authorization": "decides what an authenticated identity may access or do",
            "format": "at most two sentences for a non-technical reader"
        })),
        evaluate: common::evaluate_text_response,
        cleanup: None,
    }
}
