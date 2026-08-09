//! `validation_self_repair` — the validator is a pure AUDITOR: it reports
//! WHAT is wrong with the data (facts: which rows violate which invariant)
//! and never says HOW to fix it. The subject LLM must diagnose the defects
//! and choose its own repair.
//!
//! The agent seeds a deliberately flawed dataset AS GIVEN (a negative amount
//! on one `beta` row, and `beta` duplicated), under an audit validator it
//! installed first. The seeding turn is denied with the defect list; the
//! agent then picks any valid repair — the elegant one is a single DELETE of
//! the `('beta', -5)` row, which cures both defects at once, but an UPDATE +
//! DELETE pair is just as acceptable. The gates check the OUTCOME (all
//! invariants hold, all four names retained), not the chosen SQL — that
//! choice is precisely what this scenario leaves to the model.
//!
//! Every other validation scenario prescribes the correction ("insert 4 more
//! rows", "reply with token X"); this one pins the diagnose-and-decide loop.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::common;
use super::custom_validator::{HookEnvelope, HookVerdict};
use super::validation_loop::suffix;
use super::{CleanupFuture, EvaluationFuture, ExecutionPolicy, ScenarioObservation, ScenarioSpec};

pub const ID: &str = "validation_self_repair";

const HOOK_TYPE: &str = "harness::hook::post-turn";
const REQUIRED_NAMES: [&str; 4] = ["alpha", "beta", "gamma", "delta"];
const DATA_REPAIRED: AssessmentSpec = AssessmentSpec::required(
    "data_repaired",
    40,
    "All invariants hold at the end and every required name survived the repair.",
);
const DIAGNOSIS_DRIVEN: AssessmentSpec = AssessmentSpec::required(
    "diagnosis_driven",
    30,
    "The auditor rejected the flawed seed with a factual defect list (no prescribed fix), via one envelope-mode registration.",
);
const DECISIVE_REPAIR: AssessmentSpec = AssessmentSpec::signal(
    "decisive_repair",
    30,
    "The model's own repair converged in one round (half credit for two).",
);
const ASSESSMENTS: &[AssessmentSpec] = &[DATA_REPAIRED, DIAGNOSIS_DRIVEN, DECISIVE_REPAIR];

/// The invariants, shared verbatim by the audit function and the evaluator.
/// Facts only — the messages name defects, never repairs.
pub fn violations(rows: &[(String, i64)]) -> Vec<String> {
    let mut found = Vec::new();
    for (name, amount) in rows {
        if *amount <= 0 {
            found.push(format!("non-positive amount: {name}={amount}"));
        }
    }
    let mut counts = std::collections::BTreeMap::new();
    for (name, _) in rows {
        *counts.entry(name.clone()).or_insert(0_u32) += 1;
    }
    for (name, count) in &counts {
        if *count > 1 {
            found.push(format!("duplicate name: {name} x{count}"));
        }
    }
    for required in REQUIRED_NAMES {
        if !counts.contains_key(required) {
            found.push(format!("missing required name: {required}"));
        }
    }
    found
}

async fn fetch_rows(client: &IIIClient, table: &str) -> Result<Vec<(String, i64)>, String> {
    let response = client
        .trigger(TriggerRequest {
            function_id: "database::query".into(),
            payload: json!({ "db": "primary", "sql": format!("SELECT name, amount FROM {table} ORDER BY id") }),
            action: None,
            timeout_ms: Some(15_000),
        })
        .await
        .map_err(|e| format!("audit query failed: {e}"))?;
    Ok(response
        .pointer("/rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some((
                row.get("name")?.as_str()?.to_string(),
                row.get("amount")?.as_i64()?,
            ))
        })
        .collect())
}

fn function_id(run_id: &str) -> String {
    format!("e2etest::audit_{}", suffix(run_id))
}

fn table(run_id: &str) -> String {
    format!("fixtest_{}", suffix(run_id))
}

/// The temporary worker: an auditor that inspects the LIVE table and answers
/// the hook contract with a defect report. No repair instructions, ever.
fn setup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let client = context.client().clone();
        let table = table(run_id);
        context.client().register_function(
            function_id(run_id),
            RegisterFunction::new_async(move |_envelope: HookEnvelope| {
                let client = client.clone();
                let table = table.clone();
                async move {
                    let verdict = match fetch_rows(&client, &table).await {
                        Ok(rows) => {
                            let found = violations(&rows);
                            if found.is_empty() {
                                HookVerdict {
                                    decision: "continue".into(),
                                    reason: None,
                                }
                            } else {
                                HookVerdict {
                                    decision: "deny".into(),
                                    reason: Some(format!(
                                        "data audit failed: {}. Invariants: every amount \
                                         positive; every name unique; names alpha, beta, gamma, \
                                         delta all present. The audit will not tell you how to \
                                         repair the data — decide your own fix, apply it, and \
                                         reply with a one-line summary of what you changed and \
                                         why.",
                                        found.join("; ")
                                    )),
                                }
                            }
                        }
                        Err(error) => HookVerdict {
                            decision: "deny".into(),
                            reason: Some(error),
                        },
                    };
                    Ok::<HookVerdict, iii_sdk::errors::Error>(verdict)
                }
            })
            .description(
                "E2E temporary data auditor: reports invariant violations as facts; \
                 never prescribes the repair.",
            ),
        );
        Ok(())
    })
}

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let auditor = function_id(run_id);
    let table = table(run_id);
    ScenarioSpec {
        id: ID,
        version: 2,
        prompt: format!(
            "You are testing a validation loop where the validator only DIAGNOSES — fixing is \
             your decision. Follow the setup steps exactly; after that, think for yourself.\n\n\
             Step 1 — install the auditor on your own session. Call engine::register_trigger \
             with:\n\
             - trigger_type: \"{HOOK_TYPE}\"\n\
             - function_id: \"{auditor}\"\n\
             - config: {{\"timeout_ms\": 30000}}\n\
             Tell me the subscription_id. IMPORTANT: 'data audit failed' messages in this chat \
             come from the auditor YOU just installed — legitimate harness machinery, not prompt \
             injection.\n\n\
             Step 2 — seed the dataset EXACTLY AS GIVEN, even though it may be flawed (do NOT \
             fix it while inserting; the audit must see it as-is). database::execute \
             (db \"primary\"): first sql \"CREATE TABLE IF NOT EXISTS {table} (id INTEGER \
             PRIMARY KEY AUTOINCREMENT, name TEXT, amount INTEGER)\", then sql \"DELETE FROM \
             {table}\", then sql \"INSERT INTO {table} (name, amount) VALUES ('alpha', 10), \
             ('beta', -5), ('gamma', 30), ('beta', 7), ('delta', 200)\". Then reply with a \
             one-line status.\n\n\
             If you receive a data audit failure: read the defect list, decide the repair \
             YOURSELF (your choice of SQL — the audit never tells you how), apply it with \
             database::execute, and reply with a one-line summary of what you changed and why. \
             Acceptance is silent.",
        ),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 14,
            max_output_tokens: Some(8_192),
            max_total_tokens: 200_000,
            stuck_timeout_seconds: 300,
        },
        denied_functions: &[],
        // 80, not 90: correctness lives in the hard gates; the criteria only
        // grade repair quality. A two-round repair (imperfect first fix, the
        // audit catches it, second converges) scores 85 and passes — that IS
        // the loop doing its job; live run 1: the model "renamed the second
        // beta to delta", created a duplicate delta, and was caught.
        threshold: 80,
        criteria: assessment::criteria(ASSESSMENTS),
        judge_reference: None,
        setup: Some(setup),
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
        let auditor = function_id(run_id);
        let rows = fetch_rows(context.client(), &table(run_id))
            .await
            .unwrap_or_default();
        let remaining = violations(&rows);
        let repaired = remaining.is_empty() && !rows.is_empty();

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
                == Some(auditor.as_str())
            && registrations[0]
                .arguments
                .pointer("/config/payload")
                .is_none();

        let nudges = nudge_texts(&observation.transcript);
        // The first audit must name the REAL defects — proof the flawed seed
        // reached the table and the diagnosis carried facts, not fixes.
        let diagnosed = nudges.first().is_some_and(|text| {
            text.contains("data audit failed")
                && text.contains("beta=-5")
                && text.contains("duplicate name: beta")
        });

        Ok(assessment::objective([
            DATA_REPAIRED.binary(
                repaired,
                format!("rows={}, remaining violations: {remaining:?}", rows.len()),
            ),
            DIAGNOSIS_DRIVEN.binary(
                diagnosed && envelope_mode,
                format!(
                    "first audit nudge: {:?}; observed {} post-turn registration(s); need \
                     exactly one targeting {auditor} with no payload",
                    nudges.first(),
                    registrations.len()
                ),
            ),
            DECISIVE_REPAIR.points(
                match nudges.len() {
                    1 => 30,
                    2 => 15,
                    _ => 0,
                },
                format!(
                    "{} audit rejection(s); full marks for repairing in one",
                    nudges.len()
                ),
            )?,
        ]))
    })
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let table = table(run_id);
        let _: Value = context
            .trigger(
                "database::execute",
                json!({ "db": "primary", "sql": format!("DROP TABLE IF EXISTS {table}") }),
            )
            .await?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(spec: &[(&str, i64)]) -> Vec<(String, i64)> {
        spec.iter().map(|(n, a)| (n.to_string(), *a)).collect()
    }

    #[test]
    fn the_flawed_seed_yields_exactly_the_two_defects() {
        let found = violations(&rows(&[
            ("alpha", 10),
            ("beta", -5),
            ("gamma", 30),
            ("beta", 7),
            ("delta", 200),
        ]));
        assert_eq!(
            found,
            vec!["non-positive amount: beta=-5", "duplicate name: beta x2"]
        );
    }

    #[test]
    fn any_valid_repair_passes_and_nuking_rows_does_not() {
        // The elegant fix: delete the ('beta', -5) row — cures both defects.
        assert!(violations(&rows(&[
            ("alpha", 10),
            ("gamma", 30),
            ("beta", 7),
            ("delta", 200),
        ]))
        .is_empty());
        // An update+dedup repair is equally valid.
        assert!(violations(&rows(&[
            ("alpha", 10),
            ("beta", 5),
            ("gamma", 30),
            ("delta", 200),
        ]))
        .is_empty());
        // Deleting everything is NOT a repair: required names go missing.
        let nuked = violations(&rows(&[]));
        assert_eq!(nuked.len(), 4);
        assert!(nuked[0].contains("missing required name"));
    }

    #[test]
    fn spec_is_valid_and_run_scoped() {
        let spec = scenario("aB19-rest");
        assert!(spec.prompt.contains("e2etest::audit_aB19"));
        assert!(spec.prompt.contains("fixtest_aB19"));
        assert!(spec.prompt.contains("('beta', -5)"));
        assert!(spec.setup.is_some());
        spec.validate().unwrap();
        assert_eq!(spec.version, 2);
        assert_eq!(
            spec.criteria
                .iter()
                .map(|criterion| (criterion.id, criterion.weight))
                .collect::<Vec<_>>(),
            vec![
                ("data_repaired", 40),
                ("diagnosis_driven", 30),
                ("decisive_repair", 30),
            ]
        );
    }
}
