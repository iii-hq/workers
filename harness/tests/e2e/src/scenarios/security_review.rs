use serde_json::json;

use super::common;
use super::{CriterionSpec, ExecutionPolicy, ScenarioSpec, JUDGE_BACKED_PASS_THRESHOLD};

pub const ID: &str = "security_review";

pub fn scenario(_run_id: &str) -> ScenarioSpec {
    ScenarioSpec {
        id: ID,
        version: 1,
        prompt: r#"Review these independent snippets for security vulnerabilities:

1. `const API_KEY = "sk-live-example";`
2. `db.execute("SELECT * FROM users WHERE id = " + request.id);`
3. `response.redirect(request.query.next);`

For each snippet, identify the vulnerability, explain its impact, and recommend a practical remediation. Keep the review concise and do not perform any external action."#
            .into(),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 2,
            max_output_tokens: Some(4_096),
            max_total_tokens: 49_152,
            stuck_timeout_seconds: 120,
        },
        denied_functions: &[],
        threshold: JUDGE_BACKED_PASS_THRESHOLD,
        criteria: vec![
            CriterionSpec {
                id: "coverage",
                weight: 40,
                description: "Full credit: names the hardcoded secret, SQL injection, and open \
redirect, each on its own snippet. Half: two of the three. Zero: one or none, or \
findings mapped to the wrong snippets.",
            },
            CriterionSpec {
                id: "accuracy",
                weight: 30,
                description: "Full credit: every stated impact is technically correct with no \
invented vulnerabilities. Half: minor imprecision or one speculative extra finding. \
Zero: materially wrong impacts or fabricated issues dominate.",
            },
            CriterionSpec {
                id: "remediation",
                weight: 20,
                description: "Full credit: a concrete standard fix per finding (rotate plus \
secret manager; parameterized queries; allowlisted or relative-path redirects). Half: \
fixes for only some findings or vague advice. Zero: missing or incorrect fixes.",
            },
            CriterionSpec {
                id: "clarity",
                weight: 10,
                description: "Full credit: concise snippet-by-snippet structure, findings \
trivially mapped. Half: readable but disorganized or padded. Zero: unclear which \
finding belongs to which snippet.",
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
        setup: None,
        evaluate: common::evaluate_text_response,
        cleanup: None,
    }
}
