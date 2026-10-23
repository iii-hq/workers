//! The evidence of a baseline and a candidate E2E execution: per-run evidence
//! read from the reports. The outcome is computed in code by
//! [`crate::decision`], never by a model.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::contract::*;
use crate::decision::judge;
use crate::diagnostics;

/// Every run of `scenario_id` in an `execution-get` bundle. Detectors run on
/// each run's own transcript; a run that did not complete is kept and
/// identified, never dropped.
pub fn runs_of(bundle: &Value, scenario_id: &str, patterns: &[String]) -> Vec<EvidenceRunV1> {
    let text = |value: &Value| value.as_str().map(str::to_string);
    let reports = bundle["detail"]["reports"].as_array().into_iter().flatten();
    let scenarios = reports
        .flat_map(|report| {
            report["report"]["scenarios"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter(|scenario| scenario["scenario_id"] == scenario_id);
    scenarios
        .flat_map(|scenario| scenario["runs"].as_array().into_iter().flatten())
        .map(|run| {
            let (completion, technical) = (text(&run["completion"]), text(&run["technical"]));
            let signals = run["transcript"]["messages"].as_array().map(|messages| {
                let mut counts: BTreeMap<String, u32> = patterns
                    .iter()
                    .map(|pattern| (pattern.clone(), 0))
                    .collect();
                let session = run["session_id"].as_str().unwrap_or_default();
                for diagnostic in diagnostics::detect(session, messages).diagnostics {
                    if let Some(count) = counts.get_mut(&diagnostic.pattern()) {
                        *count += 1;
                    }
                }
                counts
            });
            EvidenceRunV1 {
                run_id: run["run_id"].as_str().unwrap_or_default().into(),
                completed: completion.as_deref() == Some("completed")
                    && technical.as_deref() == Some("valid"),
                completion,
                technical,
                status: text(&run["status"]),
                cost_usd: run["cost"]["total_usd"].as_f64(),
                wall_time_ms: run["wall_time_ms"].as_f64(),
                total_tokens: run["efficiency"]["total_tokens"].as_f64(),
                function_calls: run["efficiency"]["function_calls"].as_f64(),
                signals,
            }
        })
        .collect()
}

/// The pair's evidence for `scenario_id`: one `(execution id, bundle)` per
/// side, with `patterns` counted per run, judged by `criterion`.
pub fn evidence(
    patterns: &[String],
    criterion: Option<&CriterionV1>,
    scenario_id: &str,
    baseline: (&str, &Value),
    candidate: (&str, &Value),
    now: i64,
) -> EvidenceV1 {
    let side = |(execution_id, bundle): (&str, &Value)| EvidenceSideV1 {
        execution_id: execution_id.into(),
        runs: runs_of(bundle, scenario_id, patterns),
        n: 0,
        mean: None,
    };
    let mut evidence = EvidenceV1 {
        scenario_id: scenario_id.into(),
        computed_at: now,
        baseline: side(baseline),
        candidate: side(candidate),
        computed_outcome: ValidationOutcomeV1::Inconclusive,
        reason: String::new(),
    };
    judge(&mut evidence, criterion);
    evidence
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A transcript the way the turn loop stores it: a contract fetched
    /// twice with nothing changed in between is the detectors'
    /// `repeated_contract_discovery`.
    fn transcript(repeats: bool) -> Vec<Value> {
        use harness::types::content::ContentBlock;
        use harness::types::event::StopReason;
        use harness::types::message::{
            empty_assistant, AgentMessage, FunctionResultMessage, FunctionResultRoleTag,
        };
        let info = "engine::functions::info";
        let contract = json!({"function_id": "crm::profile", "request_schema": {"type": "object"},
            "response_schema": {"type": "object"}});
        let call = |step: u32, id: &str| {
            let mut message = empty_assistant("p", "m");
            message.stop_reason = StopReason::FunctionCall;
            message.content = vec![ContentBlock::FunctionCall {
                id: id.into(),
                function_id: "agent_trigger".into(),
                arguments: json!({"function": info, "description": "d",
                    "payload": {"function_id": "crm::profile"}}),
            }];
            json!({"entry_id": format!("e_t_a_{step}_assistant"),
                "message": AgentMessage::Assistant(message)})
        };
        let result = |id: &str, shown: &Value| {
            json!({"entry_id": format!("e_t_a_{id}"), "message": AgentMessage::FunctionResult(
            FunctionResultMessage {
                role: FunctionResultRoleTag::FunctionResult,
                function_call_id: id.into(),
                function_id: info.into(),
                content: vec![ContentBlock::text(shown.to_string())],
                details: contract.clone(),
                is_error: false,
                timestamp: 1,
            })})
        };
        let mut entries = vec![call(0, "c1"), result("c1", &contract)];
        if repeats {
            entries.push(call(1, "c2"));
            entries.push(result(
                "c2",
                &json!({"function_id": "crm::profile",
                "contract_status": "unchanged_in_context", "source_function_call_id": "c1"}),
            ));
        }
        entries
    }

    fn bundle() -> Value {
        json!({"detail": {"reports": [
            {"scenario_id": "s", "report": {"scenarios": [
                {"scenario_id": "s", "runs": [
                    {"run_id": "r1", "session_id": "e2e_1", "status": "passed",
                     "completion": "completed", "technical": "valid", "wall_time_ms": 1500,
                     "cost": {"total_usd": 0.25},
                     "efficiency": {"total_tokens": 900, "function_calls": 4},
                     "transcript": {"messages": transcript(true)}},
                    {"run_id": "r2", "session_id": "e2e_2", "status": "passed",
                     "completion": "completed", "technical": "valid",
                     "transcript": {"messages": transcript(false)}},
                    {"run_id": "r3", "session_id": "e2e_3", "status": "failed",
                     "completion": "completed", "technical": "infrastructure_error"}]},
                {"scenario_id": "other", "runs": [{"run_id": "x"}]}]}}]}})
    }

    #[test]
    fn runs_come_from_the_report_with_the_detectors_counts_per_run() {
        let patterns = ["repeated_contract_discovery:engine::functions::info".to_string()];
        let runs = runs_of(&bundle(), "s", &patterns);
        assert_eq!(runs.len(), 3, "only the scenario's runs");
        let counted = |at: usize| {
            runs[at]
                .signals
                .as_ref()
                .map(|signals| signals[&patterns[0]])
        };
        assert_eq!(
            (counted(0), counted(1), counted(2)),
            (Some(1), Some(0), None)
        );
        assert_eq!(
            runs.iter().map(|run| run.completed).collect::<Vec<_>>(),
            [true, true, false],
            "a technical failure is identified, not counted"
        );
        assert_eq!(runs[2].technical.as_deref(), Some("infrastructure_error"));
        assert_eq!(
            (
                runs[0].cost_usd,
                runs[0].wall_time_ms,
                runs[0].total_tokens,
                runs[0].function_calls
            ),
            (Some(0.25), Some(1500.0), Some(900.0), Some(4.0))
        );
        assert_eq!(
            (runs[1].cost_usd, runs[1].total_tokens),
            (None, None),
            "an unreported value is never zero"
        );
        assert!(runs_of(&bundle(), "absent", &patterns).is_empty());
        assert!(runs_of(&json!({}), "s", &patterns).is_empty());
    }
}
