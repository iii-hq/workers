//! Register `guardrails::*` functions on the iii bus.

use iii_sdk::{FunctionRef, IIIError, RegisterFunctionMessage, Value, III};
use serde_json::json;

use crate::rules::{KEY_PATTERNS, PII_PATTERNS};
use crate::scan::{run_checks, scan_regex_for_test, toxicity_score, Rules};

pub struct GuardrailsFunctionRefs {
    pub check_input: FunctionRef,
    pub check_output: FunctionRef,
    pub classify: FunctionRef,
}

impl GuardrailsFunctionRefs {
    pub fn unregister_all(self) {
        for r in [self.check_input, self.check_output, self.classify] {
            r.unregister();
        }
    }
}

pub async fn register_with_iii(iii: &III) -> anyhow::Result<GuardrailsFunctionRefs> {
    let check_input = make_check(iii, "check_input", "Run input-side guardrails.");
    let check_output = make_check(
        iii,
        "check_output",
        "Run output-side guardrails (same ruleset).",
    );

    let iii_for_classify = iii.clone();
    let classify = iii.register_function((
        RegisterFunctionMessage::with_id("guardrails::classify".into())
            .with_description("Boolean + score classifier per category.".into()),
        move |payload: Value| {
            let _iii = iii_for_classify.clone();
            async move {
                let text = payload.get("text").and_then(Value::as_str).unwrap_or("");
                let pii = !scan_regex_for_test(text, &PII_PATTERNS).is_empty();
                let keys_leaked = !scan_regex_for_test(text, &KEY_PATTERNS).is_empty();
                let jailbreak = run_checks(text, None)
                    .reasons
                    .iter()
                    .any(|s| s.contains("jailbreak"));
                let tox = toxicity_score(text);
                Ok(json!({
                    "pii": pii,
                    "keys_leaked": keys_leaked,
                    "jailbreak": jailbreak,
                    "toxicity": tox,
                }))
            }
        },
    ));

    Ok(GuardrailsFunctionRefs {
        check_input,
        check_output,
        classify,
    })
}

fn make_check(iii: &III, name: &'static str, desc: &'static str) -> FunctionRef {
    let iii_for = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(format!("guardrails::{name}"))
            .with_description(desc.into()),
        move |payload: Value| {
            let _iii = iii_for.clone();
            async move {
                let text = payload.get("text").and_then(Value::as_str).unwrap_or("");
                let rules: Option<Rules> = payload
                    .get("rules")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok());
                let result = run_checks(text, rules.as_ref());
                serde_json::to_value(result).map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ))
}
