use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use harness::functions::metrics::SessionMetricsResponseV1;
use harness::types::model::Model;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::scenarios::ExecutionPolicy;

mod summary;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePhase {
    Setup,
    Execute,
    Collect,
    Evaluate,
    Cleanup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureRecord {
    pub phase: FailurePhase,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardGateReport {
    pub id: String,
    pub passed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriterionReport {
    pub id: String,
    pub possible: u8,
    pub awarded: Option<u8>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelUsageReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostReport {
    pub subject_usd: Option<f64>,
    pub judge_usd: Option<f64>,
    pub total_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Passed,
    QualityFailed,
    HardGateFailed,
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

    fn label(self) -> &'static str {
        match self {
            Self::Passed => "PASS",
            Self::QualityFailed => "QUALITY FAIL",
            Self::HardGateFailed => "HARD GATE FAIL",
            Self::SubjectError => "SUBJECT ERROR",
            Self::JudgeError => "JUDGE ERROR",
            Self::ResourceLimit => "RESOURCE LIMIT",
            Self::InfrastructureError => "INFRA ERROR",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryAttemptReport {
    pub run_id: String,
    pub session_id: String,
    pub wall_time_ms: u64,
    pub status: RunStatus,
    pub cost: CostReport,
    pub failures: Vec<FailureRecord>,
}

impl From<&E2eRunReport> for RetryAttemptReport {
    fn from(report: &E2eRunReport) -> Self {
        Self {
            run_id: report.run_id.clone(),
            session_id: report.session_id.clone(),
            wall_time_ms: report.wall_time_ms,
            status: report.status,
            cost: report.cost.clone(),
            failures: report.failures.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_usage: Option<ModelUsageReport>,
    pub cost: CostReport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retry_attempts: Vec<RetryAttemptReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
            judge_usage: None,
            cost: CostReport::default(),
            retry_attempts: Vec::new(),
            failures: Vec::new(),
        }
    }

    pub fn push_failure(
        &mut self,
        status: RunStatus,
        phase: FailurePhase,
        message: impl Into<String>,
    ) {
        let is_primary = self.failures.is_empty();
        self.failures.push(FailureRecord {
            phase,
            message: message.into(),
        });
        if is_primary {
            self.status = status;
        }
    }

    pub fn finish(&mut self, status: RunStatus) {
        self.status = status;
    }

    pub fn update_cost(&mut self, judge_expected: bool) {
        let subject_usd = self
            .metrics
            .as_ref()
            .and_then(|metrics| metrics.totals.cost_usd);
        let judge_skipped = !judge_expected || self.status == RunStatus::HardGateFailed;
        let judge_usd = if judge_skipped {
            Some(0.0)
        } else {
            self.judge_usage.as_ref().and_then(|usage| usage.cost_usd)
        };
        self.cost = CostReport {
            subject_usd,
            judge_usd,
            total_usd: subject_usd
                .zip(judge_usd)
                .map(|(subject, judge)| subject + judge),
        };
    }

    pub fn attach_retry_attempts(&mut self, retry_attempts: Vec<RetryAttemptReport>) {
        if retry_attempts.is_empty() {
            return;
        }
        self.wall_time_ms = retry_attempts
            .iter()
            .fold(self.wall_time_ms, |total, attempt| {
                total.saturating_add(attempt.wall_time_ms)
            });
        self.cost.subject_usd = sum_cost(
            retry_attempts
                .iter()
                .map(|attempt| attempt.cost.subject_usd)
                .chain([self.cost.subject_usd]),
        );
        self.cost.judge_usd = sum_cost(
            retry_attempts
                .iter()
                .map(|attempt| attempt.cost.judge_usd)
                .chain([self.cost.judge_usd]),
        );
        self.cost.total_usd = sum_cost(
            retry_attempts
                .iter()
                .map(|attempt| attempt.cost.total_usd)
                .chain([self.cost.total_usd]),
        );
        self.retry_attempts = retry_attempts;
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScenarioAggregate {
    pub runs: u32,
    pub scored_runs: u32,
    pub passed_runs: u32,
    pub required_passes: u32,
    pub pass_rate: f64,
    pub median_score: Option<f64>,
    pub hard_gate_failures: u32,
    pub technical_failures: u32,
    pub cost: CostReport,
}

fn default_scenario_version() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize)]
pub struct E2eScenarioReport {
    pub scenario_id: String,
    #[serde(default = "default_scenario_version")]
    pub scenario_version: u32,
    pub threshold: u8,
    pub execution_policy: ExecutionPolicy,
    pub aggregate: ScenarioAggregate,
    pub passed: bool,
    pub runs: Vec<E2eRunReport>,
}

impl E2eScenarioReport {
    pub fn aggregate(
        scenario_id: impl Into<String>,
        scenario_version: u32,
        threshold: u8,
        execution_policy: ExecutionPolicy,
        runs: Vec<E2eRunReport>,
    ) -> Self {
        let run_count = runs.len() as u32;
        let scored_runs = runs.iter().filter(|run| run.score.is_some()).count() as u32;
        let passed_runs = runs
            .iter()
            .filter(|run| run.status == RunStatus::Passed)
            .count() as u32;
        let hard_gate_failures = runs
            .iter()
            .filter(|run| run.status == RunStatus::HardGateFailed)
            .count() as u32;
        let technical_failures = runs
            .iter()
            .filter(|run| run.status.is_technical_failure())
            .count() as u32;
        let required_passes = required_passes(run_count);
        let median_score = median(runs.iter().filter_map(|run| run.score));
        let cost = CostReport {
            subject_usd: sum_cost(runs.iter().map(|run| run.cost.subject_usd)),
            judge_usd: sum_cost(runs.iter().map(|run| run.cost.judge_usd)),
            total_usd: sum_cost(runs.iter().map(|run| run.cost.total_usd)),
        };
        let passed = run_count > 0
            && technical_failures == 0
            && passed_runs >= required_passes
            && median_score.is_some_and(|score| score >= f64::from(threshold));
        Self {
            scenario_id: scenario_id.into(),
            scenario_version,
            threshold,
            execution_policy,
            aggregate: ScenarioAggregate {
                runs: run_count,
                scored_runs,
                passed_runs,
                required_passes,
                pass_rate: if run_count == 0 {
                    0.0
                } else {
                    f64::from(passed_runs) / f64::from(run_count)
                },
                median_score,
                hard_gate_failures,
                technical_failures,
                cost,
            },
            passed,
            runs,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
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

    pub fn read_from(input: &Path) -> Result<(Self, PathBuf)> {
        let path = if input.is_dir() {
            input.join("results.json")
        } else {
            input.to_path_buf()
        };
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let report = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode E2E report {}", path.display()))?;
        Ok((report, path))
    }

    pub fn has_ci_blocking_failure(&self) -> bool {
        self.scenarios.is_empty()
            || self.scenarios.iter().any(|scenario| {
                scenario.aggregate.hard_gate_failures > 0
                    || scenario.aggregate.technical_failures > 0
            })
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

fn sum_cost(values: impl IntoIterator<Item = Option<f64>>) -> Option<f64> {
    values
        .into_iter()
        .try_fold(0.0, |total, value| Some(total + value?))
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
            1,
            80,
            ExecutionPolicy {
                max_turns: 1,
                max_output_tokens: Some(1),
                max_total_tokens: 1,
                stuck_timeout_seconds: 1,
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
    fn costs_are_aggregated_without_hiding_unknown_values() {
        let mut first = run(90, true);
        first.cost = CostReport {
            subject_usd: Some(0.1),
            judge_usd: Some(0.02),
            total_usd: Some(0.12),
        };
        let mut second = run(90, true);
        second.cost = CostReport {
            subject_usd: Some(0.2),
            judge_usd: None,
            total_usd: None,
        };
        let report = aggregate(vec![first, second]);
        assert!((report.aggregate.cost.subject_usd.unwrap() - 0.3).abs() < f64::EPSILON);
        assert_eq!(report.aggregate.cost.judge_usd, None);
        assert_eq!(report.aggregate.cost.total_usd, None);
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
    fn cleanup_failure_does_not_hide_the_primary_failure_status() {
        let mut report = E2eRunReport::new("run".into(), "session".into(), "prompt".into());
        report.push_failure(
            RunStatus::SubjectError,
            FailurePhase::Execute,
            "provider unavailable",
        );
        report.push_failure(
            RunStatus::InfrastructureError,
            FailurePhase::Cleanup,
            "cleanup unavailable",
        );

        assert_eq!(report.status, RunStatus::SubjectError);
        assert_eq!(report.failures.len(), 2);
    }

    #[test]
    fn hard_gate_failures_count_as_scored_failed_runs() {
        let mut outvoted = run(45, false);
        outvoted.status = RunStatus::HardGateFailed;
        let report = aggregate(vec![outvoted, run(90, true), run(90, true)]);
        assert!(report.passed);
        assert_eq!(report.aggregate.hard_gate_failures, 1);
        assert_eq!(report.aggregate.median_score, Some(90.0));

        let mut decisive = run(45, false);
        decisive.status = RunStatus::HardGateFailed;
        let report = aggregate(vec![decisive, run(90, true)]);
        assert!(!report.passed);
        assert_eq!(report.aggregate.median_score, Some(67.5));
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
        assert!(value["scenarios"][0]["aggregate"]["cost"].is_object());
    }

    #[test]
    fn retry_attempts_preserve_failures_time_and_cost() {
        let mut failed = E2eRunReport::new("retry".into(), "retry-session".into(), "prompt".into());
        failed.wall_time_ms = 2_000;
        failed.cost = CostReport {
            subject_usd: Some(0.10),
            judge_usd: Some(0.0),
            total_usd: Some(0.10),
        };
        failed.push_failure(
            RunStatus::SubjectError,
            FailurePhase::Execute,
            "stream ended without a terminal frame",
        );

        let mut passed = run(100, true);
        passed.wall_time_ms = 3_000;
        passed.cost = CostReport {
            subject_usd: Some(0.20),
            judge_usd: Some(0.0),
            total_usd: Some(0.20),
        };
        passed.attach_retry_attempts(vec![RetryAttemptReport::from(&failed)]);

        assert_eq!(passed.wall_time_ms, 5_000);
        assert_eq!(passed.retry_attempts.len(), 1);
        assert!((passed.cost.total_usd.unwrap() - 0.30).abs() < f64::EPSILON);
    }

    #[test]
    fn summary_surfaces_the_actionable_failure_details() {
        let mut failed = run(50, false);
        failed.status = RunStatus::HardGateFailed;
        failed.hard_gates.push(HardGateReport {
            id: "durable_effect".into(),
            passed: false,
            reason: "expected row was missing".into(),
        });
        failed.criteria.push(CriterionReport {
            id: "correctness".into(),
            possible: 100,
            awarded: Some(50),
            reason: "only half of the expected result was present".into(),
        });
        let report = E2eReport::new(model(), None, None, None, vec![aggregate(vec![failed])]);

        let summary = report.summary(false);
        assert!(summary.contains("Harness E2E: FAIL"));
        assert!(summary.contains("gate durable_effect: FAIL - expected row was missing"));
        assert!(summary.contains("criterion correctness: 50/100"));
    }

    #[test]
    fn quality_score_failure_does_not_block_ci() {
        let report = E2eReport::new(
            model(),
            None,
            None,
            None,
            vec![aggregate(vec![run(49, false)])],
        );
        assert!(!report.passed);
        assert!(!report.has_ci_blocking_failure());
    }

    #[test]
    fn hard_gate_failure_blocks_ci_even_with_a_score() {
        let mut hard_gate = run(80, true);
        hard_gate.status = RunStatus::HardGateFailed;
        let report = E2eReport::new(model(), None, None, None, vec![aggregate(vec![hard_gate])]);
        assert!(report.has_ci_blocking_failure());
    }

    #[test]
    fn technical_failure_blocks_ci() {
        let mut technical = run(80, true);
        technical.status = RunStatus::InfrastructureError;
        let report = E2eReport::new(model(), None, None, None, vec![aggregate(vec![technical])]);
        assert!(report.has_ci_blocking_failure());
    }

    #[test]
    fn an_empty_report_blocks_ci() {
        let report = E2eReport::new(model(), None, None, None, vec![]);
        assert!(report.has_ci_blocking_failure());
    }

    fn model() -> ModelArtifact {
        ModelArtifact {
            model: "model".into(),
            provider: "provider".into(),
            context_window: 10_000,
            max_output_tokens: 2_000,
            supports_tools: Some(true),
            supports_vision: Some(false),
        }
    }
}
