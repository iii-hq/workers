use serde_json::json;

use super::common;
use super::{CriterionSpec, ExecutionPolicy, ScenarioSpec, JUDGE_BACKED_PASS_THRESHOLD};

pub const ID: &str = "direct_answer";

pub fn scenario(_run_id: &str) -> ScenarioSpec {
    ScenarioSpec {
        id: ID,
        version: 1,
        prompt: "Explain to a non-technical reader, in at most two sentences, the difference between authentication and authorization. Do not perform any external action.".into(),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 2,
            max_output_tokens: Some(2_048),
            max_total_tokens: 32_768,
            stuck_timeout_seconds: 120,
        },
        denied_functions: &[],
        threshold: JUDGE_BACKED_PASS_THRESHOLD,
        criteria: vec![
            CriterionSpec {
                id: "correctness",
                weight: 50,
                description: "Full credit: authentication framed as proving who you are and \
authorization as what you may do, with no conflation. Half: both defined but the \
contrast is muddled or partially wrong. Zero: definitions swapped, merged, or \
incorrect.",
            },
            CriterionSpec {
                id: "clarity",
                weight: 30,
                description: "Full credit: plain everyday language a non-technical reader \
follows, any technical term immediately explained. Half: mostly clear but leans on \
unexplained jargon. Zero: jargon-heavy or confusing.",
            },
            CriterionSpec {
                id: "instruction_adherence",
                weight: 20,
                description: "Full credit: a direct answer in one or two sentences with no \
preamble or lists. Half: correct content but three sentences or noticeable padding. \
Zero: far over length or the answer is buried.",
            },
        ],
        judge_reference: Some(json!({
            "authentication": "verifies who a user or system is",
            "authorization": "decides what an authenticated identity may access or do",
            "format": "at most two sentences for a non-technical reader"
        })),
        setup: None,
        evaluate: common::evaluate_text_response,
        cleanup: None,
    }
}
