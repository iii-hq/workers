use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use harness::functions::metrics::SessionMetricsResponseV1;
use harness::types::model::Model;
use serde::Serialize;
use serde_json::Value;

use crate::scenarios::{ExecutionPolicy, ModelRequirements};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePhase {
    Setup,
    Execute,
    Collect,
    Evaluate,
    Cleanup,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailureRecord {
    pub phase: FailurePhase,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HardGateReport {
    pub id: String,
    pub passed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CriterionReport {
    pub id: String,
    pub possible: u8,
    pub awarded: Option<u8>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Passed,
    QualityFailed,
    HardGateFailed,
    Unsupported,
    SubjectError,
    JudgeError,
    ResourceLimit,
    InfrastructureError,
}

impl RunStatus {
    pub fn is_technical_failure(self) -> bool {
        matches!(
            self,
            Self::SubjectError | Self::JudgeError | Self::ResourceLimit | Self::InfrastructureError
        )
    }
}

#[derive(Debug, Serialize)]
pub struct E2eRunReport {
    pub run_id: String,
    pub session_id: String,
    pub prompt: String,
    pub wall_time_ms: u64,
    pub score: Option<u8>,
    pub status: RunStatus,
    pub hard_gates: Vec<HardGateReport>,
    pub criteria: Vec<CriterionReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<SessionMetricsResponseV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_attempts: Option<u8>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<FailureRecord>,
}

impl E2eRunReport {
    pub fn new(run_id: String, session_id: String, prompt: String) -> Self {
        Self {
            run_id,
            session_id,
            prompt,
            wall_time_ms: 0,
            score: None,
            status: RunStatus::InfrastructureError,
            hard_gates: Vec::new(),
            criteria: Vec::new(),
            transcript: None,
            metrics: None,
            judge_attempts: None,
            failures: Vec::new(),
        }
    }

    pub fn push_failure(
        &mut self,
        status: RunStatus,
        phase: FailurePhase,
        message: impl Into<String>,
    ) {
        self.failures.push(FailureRecord {
            phase,
            message: message.into(),
        });
        self.status = status;
    }

    pub fn finish(&mut self, status: RunStatus) {
        self.status = status;
    }
}

#[derive(Debug, Serialize)]
pub struct ScenarioAggregate {
    pub runs: u32,
    pub eligible_runs: u32,
    pub scored_runs: u32,
    pub passed_runs: u32,
    pub required_passes: u32,
    pub pass_rate: f64,
    pub median_score: Option<f64>,
    pub technical_failures: u32,
}

#[derive(Debug, Serialize)]
pub struct E2eScenarioReport {
    pub scenario_id: String,
    pub threshold: u8,
    pub requirements: ModelRequirements,
    pub execution_policy: ExecutionPolicy,
    pub aggregate: ScenarioAggregate,
    pub passed: bool,
    pub runs: Vec<E2eRunReport>,
}

impl E2eScenarioReport {
    pub fn aggregate(
        scenario_id: impl Into<String>,
        threshold: u8,
        requirements: ModelRequirements,
        execution_policy: ExecutionPolicy,
        runs: Vec<E2eRunReport>,
    ) -> Self {
        let run_count = runs.len() as u32;
        let eligible_runs = runs
            .iter()
            .filter(|run| run.status != RunStatus::Unsupported)
            .count() as u32;
        let scored_runs = runs.iter().filter(|run| run.score.is_some()).count() as u32;
        let passed_runs = runs
            .iter()
            .filter(|run| run.status == RunStatus::Passed)
            .count() as u32;
        let technical_failures = runs
            .iter()
            .filter(|run| run.status.is_technical_failure())
            .count() as u32;
        let required_passes = required_passes(eligible_runs);
        let median_score = median(runs.iter().filter_map(|run| run.score));
        let passed = eligible_runs > 0
            && technical_failures == 0
            && passed_runs >= required_passes
            && median_score.is_some_and(|score| score >= f64::from(threshold));
        Self {
            scenario_id: scenario_id.into(),
            threshold,
            requirements,
            execution_policy,
            aggregate: ScenarioAggregate {
                runs: run_count,
                eligible_runs,
                scored_runs,
                passed_runs,
                required_passes,
                pass_rate: if eligible_runs == 0 {
                    0.0
                } else {
                    f64::from(passed_runs) / f64::from(eligible_runs)
                },
                median_score,
                technical_failures,
            },
            passed,
            runs,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelArtifact {
    pub model: String,
    pub provider: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    pub supports_tools: Option<bool>,
    pub supports_vision: Option<bool>,
}

impl From<Model> for ModelArtifact {
    fn from(model: Model) -> Self {
        Self {
            model: model.id,
            provider: model.provider,
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            supports_tools: model.supports_tools,
            supports_vision: model.supports_vision,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct E2eReport {
    pub subject: ModelArtifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge: Option<ModelArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_revision: Option<String>,
    pub passed: bool,
    pub scenarios: Vec<E2eScenarioReport>,
}

impl E2eReport {
    pub fn new(
        subject: ModelArtifact,
        judge: Option<ModelArtifact>,
        judge_protocol: Option<String>,
        engine_revision: Option<String>,
        scenarios: Vec<E2eScenarioReport>,
    ) -> Self {
        let passed = !scenarios.is_empty() && scenarios.iter().all(|scenario| scenario.passed);
        Self {
            subject,
            judge,
            judge_protocol,
            engine_revision,
            passed,
            scenarios,
        }
    }

    pub fn write_to(&self, output: &Path) -> Result<PathBuf> {
        fs::create_dir_all(output)
            .with_context(|| format!("create report directory {}", output.display()))?;
        let path = output.join("results.json");
        let mut bytes = serde_json::to_vec_pretty(self)
            .with_context(|| format!("serialize {}", path.display()))?;
        bytes.push(b'\n');
        fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }
}

fn required_passes(runs: u32) -> u32 {
    runs.saturating_mul(2).saturating_add(2) / 3
}

fn median(values: impl IntoIterator<Item = u8>) -> Option<f64> {
    let mut values: Vec<_> = values.into_iter().collect();
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(f64::from(values[middle]))
    } else {
        Some((f64::from(values[middle - 1]) + f64::from(values[middle])) / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(score: u8, passed: bool) -> E2eRunReport {
        let mut report = E2eRunReport::new("run".into(), "session".into(), "prompt".into());
        report.score = Some(score);
        report.status = if passed {
            RunStatus::Passed
        } else {
            RunStatus::QualityFailed
        };
        report
    }

    fn aggregate(runs: Vec<E2eRunReport>) -> E2eScenarioReport {
        E2eScenarioReport::aggregate(
            "case",
            80,
            ModelRequirements::default(),
            ExecutionPolicy {
                max_turns: 1,
                max_output_tokens: 1,
                max_total_tokens: 1,
                timeout_seconds: 1,
            },
            runs,
        )
    }

    #[test]
    fn one_run_requires_that_run_to_pass() {
        assert!(aggregate(vec![run(80, true)]).passed);
        assert!(!aggregate(vec![run(100, false)]).passed);
    }

    #[test]
    fn three_runs_require_two_passes_and_threshold_median() {
        let report = aggregate(vec![run(79, false), run(80, true), run(90, true)]);
        assert!(report.passed);
        assert_eq!(report.aggregate.required_passes, 2);
        assert_eq!(report.aggregate.pass_rate, 2.0 / 3.0);
        assert_eq!(report.aggregate.median_score, Some(80.0));

        let low = aggregate(vec![run(70, true), run(79, true), run(100, false)]);
        assert!(!low.passed);
    }

    #[test]
    fn technical_errors_are_not_quality_scores_and_fail_the_aggregate() {
        let mut error = E2eRunReport::new("run".into(), "session".into(), "prompt".into());
        error.push_failure(
            RunStatus::JudgeError,
            FailurePhase::Evaluate,
            "judge unavailable",
        );
        let report = aggregate(vec![run(90, true), run(90, true), error]);
        assert!(!report.passed);
        assert_eq!(report.aggregate.scored_runs, 2);
        assert_eq!(report.aggregate.technical_failures, 1);
        assert_eq!(report.aggregate.median_score, Some(90.0));
    }

    #[test]
    fn report_contains_current_execution_shape() {
        let report = E2eReport::new(
            ModelArtifact {
                model: "model".into(),
                provider: "provider".into(),
                context_window: 10_000,
                max_output_tokens: 2_000,
                supports_tools: Some(true),
                supports_vision: Some(false),
            },
            None,
            None,
            None,
            vec![aggregate(vec![run(90, true)])],
        );
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["scenarios"][0]["aggregate"]["median_score"], 90.0);
        assert_eq!(value["scenarios"][0]["runs"][0]["status"], "passed");
    }
}
