use std::env;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use super::store::load_runs;
use super::{JobStatus, RunMetadata};
use crate::report::{E2eReport, E2eRunReport, E2eScenarioReport};

pub(super) const MAX_EXECUTIONS: usize = 100;

pub(super) fn load_execution_summaries(runs_dir: &Path) -> Result<Vec<Value>> {
    let mut values = Vec::new();
    for run in load_runs(runs_dir)? {
        values.push(execution_summary(&run.metadata, run.report.as_ref())?);
    }
    values.sort_by(|left, right| {
        right
            .get("started_at")
            .and_then(Value::as_str)
            .cmp(&left.get("started_at").and_then(Value::as_str))
    });
    values.truncate(MAX_EXECUTIONS);
    Ok(values)
}

pub(super) fn execution_summary(
    metadata: &RunMetadata,
    report: Option<&E2eReport>,
) -> Result<Value> {
    let generated_at = if metadata.completed_at.is_empty() {
        &metadata.started_at
    } else {
        &metadata.completed_at
    };
    let execution = execution_identity(metadata);
    let Some(report) = report else {
        let status = match metadata.status {
            JobStatus::Cancelled => "cancelled",
            JobStatus::Running | JobStatus::Cancelling => "running",
            JobStatus::Completed => "incomplete",
            JobStatus::Failed => "infra_failed",
        };
        return Ok(json!({
            "id": metadata.id,
            "label": metadata.label,
            "run_id": metadata.id,
            "attempt": 1,
            "workflow_name": "Harness E2E Local",
            "workflow_url": "",
            "event": "local",
            "actor": actor(),
            "started_at": metadata.started_at,
            "completed_at": metadata.completed_at,
            "conclusion": if metadata.status == JobStatus::Failed { "failure" } else { "" },
            "status": status,
            "availability": "unavailable",
            "detail_path": null,
            "generated_at": generated_at,
            "lane": "local",
            "execution": execution,
            "requested_runs": metadata.request.runs,
            "subjects": [],
            "scenario_metrics": [],
            "totals": {},
            "first_failure": if metadata.error.is_empty() { Value::Null } else { json!({"kind":"runner", "message": metadata.error}) },
        }));
    };

    let subject_id = slug(&format!(
        "{}-{}",
        report.subject.provider, report.subject.model
    ));
    let scenarios: Vec<_> = report.scenarios.iter().map(scenario_summary).collect();
    let hard_gate_failures: u32 = report
        .scenarios
        .iter()
        .map(|value| value.aggregate.hard_gate_failures)
        .sum();
    let technical_failures: u32 = report
        .scenarios
        .iter()
        .map(|value| value.aggregate.technical_failures)
        .sum();
    let retries: usize = report
        .scenarios
        .iter()
        .flat_map(|scenario| &scenario.runs)
        .map(|run| run.retry_attempts.len())
        .sum();
    let wall_time_seconds = report
        .scenarios
        .iter()
        .flat_map(|scenario| &scenario.runs)
        .map(|run| run.wall_time_ms as f64 / 1000.0)
        .sum::<f64>();
    let costs: Vec<_> = report
        .scenarios
        .iter()
        .map(|scenario| scenario.aggregate.cost.total_usd)
        .collect();
    let total_cost_usd = sum_complete(&costs);
    let scores: Vec<_> = report
        .scenarios
        .iter()
        .filter_map(|scenario| scenario.aggregate.median_score)
        .collect();
    let average_score = mean(&scores);
    let expected = report.scenarios.len();
    let passed = report
        .scenarios
        .iter()
        .filter(|scenario| scenario.passed)
        .count();
    let status = semantic_status(report.passed, hard_gate_failures, technical_failures);
    let totals = efficiency_totals(report);
    let subject = json!({
        "id": subject_id,
        "model": report.subject.model,
        "provider": report.subject.provider,
        "judge": report.judge,
        "engine_revision": report.engine_revision,
        "passed": report.passed,
        "expected_reports": expected,
        "received_reports": expected,
        "scenario_pass_rate": if expected == 0 { 0.0 } else { passed as f64 / expected as f64 },
        "report_coverage": 1.0,
        "hard_gate_failures": hard_gate_failures,
        "technical_failures": technical_failures,
        "infra_failures": 0,
        "retry_attempts": retries,
        "total_cost_usd": total_cost_usd,
        "wall_time_seconds": wall_time_seconds,
        "scenarios": scenarios,
    });
    Ok(json!({
        "id": metadata.id,
        "label": metadata.label,
        "run_id": metadata.id,
        "attempt": 1,
        "workflow_name": "Harness E2E Local",
        "workflow_url": "",
        "event": "local",
        "actor": actor(),
        "started_at": metadata.started_at,
        "completed_at": metadata.completed_at,
        "conclusion": if hard_gate_failures > 0 || technical_failures > 0 { "failure" } else { "success" },
        "status": status,
        "availability": "full",
        "detail_path": format!("runs/{}.json", metadata.id),
        "generated_at": generated_at,
        "lane": "local",
        "execution": execution,
        "release": { "tag":"", "worker":"", "version":"", "url":"", "registry_tag":"local" },
        "source": { "sha":"", "ref":"local", "repository":"" },
        "requested_runs": metadata.request.runs,
        "subjects": [subject],
        "scenario_metrics": scenario_metrics(&subject_id, report),
        "totals": {
            "expected_reports": expected,
            "received_reports": expected,
            "report_coverage": 100.0,
            "passed_scenarios": passed,
            "scenario_pass_rate": if expected == 0 { 0.0 } else { passed as f64 / expected as f64 * 100.0 },
            "average_score": average_score,
            "total_cost_usd": total_cost_usd,
            "wall_time_seconds": wall_time_seconds,
            "hard_gate_failures": hard_gate_failures,
            "technical_failures": technical_failures,
            "missing_reports": 0,
            "retries": retries,
            "total_tokens": totals.0,
            "function_calls": totals.1,
        },
        "workflow_duration_seconds": wall_time_seconds,
        "first_failure": first_failure(report),
    }))
}

pub(super) fn execution_detail_value(metadata: &RunMetadata, report: &E2eReport) -> Result<Value> {
    let summary = execution_summary(metadata, Some(report))?;
    let subject_id = slug(&format!(
        "{}-{}",
        report.subject.provider, report.subject.model
    ));
    let base = serde_json::to_value(report)?;
    let reports: Vec<_> = report
        .scenarios
        .iter()
        .map(|scenario| {
            let mut value = base.clone();
            value["passed"] = json!(scenario.passed);
            value["scenarios"] = json!([scenario]);
            json!({
                "subject_id": subject_id,
                "scenario_id": scenario.scenario_id,
                "available": true,
                "report": value,
            })
        })
        .collect();
    let mut detail = summary;
    detail["reports"] = json!(reports);
    Ok(detail)
}

fn scenario_summary(scenario: &E2eScenarioReport) -> Value {
    let wall_time_seconds = scenario
        .runs
        .iter()
        .map(|run| run.wall_time_ms as f64 / 1000.0)
        .sum::<f64>();
    let retries: usize = scenario
        .runs
        .iter()
        .map(|run| run.retry_attempts.len())
        .sum();
    json!({
        "id": scenario.scenario_id,
        "status": semantic_status(scenario.passed, scenario.aggregate.hard_gate_failures, scenario.aggregate.technical_failures),
        "passed": scenario.passed,
        "threshold": scenario.threshold,
        "runs": scenario.aggregate.runs,
        "median_score": scenario.aggregate.median_score,
        "pass_rate": scenario.aggregate.pass_rate,
        "hard_gate_failures": scenario.aggregate.hard_gate_failures,
        "technical_failures": scenario.aggregate.technical_failures,
        "infra_failures": 0,
        "retries": retries,
        "total_cost_usd": scenario.aggregate.cost.total_usd,
        "wall_time_seconds": wall_time_seconds,
    })
}

fn semantic_status(passed: bool, hard_gates: u32, technical: u32) -> &'static str {
    if technical > 0 {
        "technical_failed"
    } else if hard_gates > 0 {
        "hard_gate_failed"
    } else if passed {
        "passed"
    } else {
        "quality_advisory"
    }
}

fn scenario_metrics(subject_id: &str, report: &E2eReport) -> Vec<Value> {
    report
        .scenarios
        .iter()
        .map(|scenario| {
            let metric = |run: &E2eRunReport, name: &str| -> Option<f64> {
                match name {
                    "tokens" => run.metrics.as_ref().and_then(|value| {
                        value
                            .totals
                            .input_tokens
                            .zip(value.totals.output_tokens)
                            .map(|(input, output)| (input + output) as f64)
                    }),
                    "duration_seconds" => Some(run.wall_time_ms as f64 / 1000.0),
                    "cost_usd" => run.cost.total_usd,
                    "function_calls" => run
                        .metrics
                        .as_ref()
                        .map(|value| value.totals.function_calls as f64),
                    "function_call_errors" => run
                        .metrics
                        .as_ref()
                        .map(|value| value.totals.function_call_errors as f64),
                    "sessions" => run
                        .metrics
                        .as_ref()
                        .map(|value| value.totals.sessions as f64),
                    "turns" => run.metrics.as_ref().map(|value| value.totals.turns as f64),
                    _ => None,
                }
            };
            let mut averages = serde_json::Map::new();
            let mut samples = serde_json::Map::new();
            for name in [
                "tokens",
                "duration_seconds",
                "cost_usd",
                "function_calls",
                "function_call_errors",
                "sessions",
                "turns",
            ] {
                let values: Vec<_> = scenario
                    .runs
                    .iter()
                    .filter_map(|run| metric(run, name))
                    .collect();
                averages.insert(name.into(), json!(mean(&values)));
                samples.insert(name.into(), json!(values.len()));
            }
            let contract = json!({
                "execution_policy": scenario.execution_policy,
                "scenario_id": scenario.scenario_id,
                "scenario_version": scenario.scenario_version,
                "threshold": scenario.threshold,
            });
            json!({
                "subject_id": subject_id,
                "scenario_id": scenario.scenario_id,
                "scenario_version": scenario.scenario_version,
                "contract_fingerprint": contract_fingerprint(&contract),
                "run_count": scenario.runs.len(),
                "averages": averages,
                "samples": samples,
            })
        })
        .collect()
}

pub(super) fn contract_fingerprint(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).expect("serialize scenario contract");
    let hash = bytes.into_iter().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    });
    format!("fnv1a32:{hash:08x}")
}

fn efficiency_totals(report: &E2eReport) -> (Option<f64>, Option<f64>) {
    let mut tokens = Vec::new();
    let mut calls = Vec::new();
    for run in report.scenarios.iter().flat_map(|scenario| &scenario.runs) {
        if let Some(metrics) = &run.metrics {
            if let Some((input, output)) = metrics
                .totals
                .input_tokens
                .zip(metrics.totals.output_tokens)
            {
                tokens.push((input + output) as f64);
            }
            calls.push(metrics.totals.function_calls as f64);
        }
    }
    (sum_available(&tokens), sum_available(&calls))
}

fn first_failure(report: &E2eReport) -> Value {
    for scenario in &report.scenarios {
        for run in &scenario.runs {
            if let Some(failure) = run.failures.first() {
                return json!({
                    "kind": "run_failure",
                    "scenario_id": scenario.scenario_id,
                    "phase": failure.phase,
                    "message": failure.message,
                });
            }
            if let Some(gate) = run.hard_gates.iter().find(|gate| !gate.passed) {
                return json!({
                    "kind": "hard_gate",
                    "scenario_id": scenario.scenario_id,
                    "message": format!("{}: {}", gate.id, gate.reason),
                });
            }
        }
    }
    Value::Null
}

fn execution_identity(metadata: &RunMetadata) -> Value {
    json!({
        "id": metadata.id,
        "run_id": metadata.id,
        "attempt": 1,
        "event": "local",
        "actor": actor(),
        "workflow_name": "Harness E2E Local",
        "workflow_url": "",
        "label": metadata.label,
        "started_at": metadata.started_at,
        "completed_at": metadata.completed_at,
        "conclusion": if metadata.status == JobStatus::Failed { "failure" } else { "success" },
        "head_sha": "",
        "head_branch": "local",
        "repository": "",
    })
}

pub(super) fn validate_execution_id(value: &str) -> std::result::Result<(), String> {
    if value.starts_with("local-")
        && value.len() <= 80
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        Ok(())
    } else {
        Err("invalid execution id".into())
    }
}

fn sum_complete(values: &[Option<f64>]) -> Option<f64> {
    values
        .iter()
        .copied()
        .try_fold(0.0, |total, value| Some(total + value?))
}

fn sum_available(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum())
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn actor() -> String {
    env::var("USER").unwrap_or_else(|_| "local".into())
}

pub(super) fn repository_url() -> String {
    "https://github.com/iii-hq/workers".into()
}
