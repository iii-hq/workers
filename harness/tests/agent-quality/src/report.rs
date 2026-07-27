use std::fs;
use std::path::{Path, PathBuf};

use harness::functions::metrics::SessionMetricsResponseV1;
use serde::Serialize;
use serde_json::Value;

use crate::error::{EvalError, FailureRecord};
use crate::limits::AgentQualityLimitsV1;
use crate::subject::SubjectArtifactV1;

pub use eval::report::EvalBenchmarkV1 as AgentQualityBenchmarkV1;

#[derive(Debug, Serialize)]
pub struct ScenarioObservationV1 {
    pub metrics: SessionMetricsResponseV1,
    pub transcript: Value,
}

#[derive(Debug, Serialize)]
pub struct AgentQualityScenarioReportV1 {
    pub scenario_id: &'static str,
    pub prompt: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub wall_time_ms: u64,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation: Option<ScenarioObservationV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<FailureRecord>,
}

impl AgentQualityScenarioReportV1 {
    pub fn new(scenario_id: &'static str, session_id: String, prompt: String) -> Self {
        Self {
            scenario_id,
            prompt,
            session_id,
            turn_id: None,
            wall_time_ms: 0,
            passed: false,
            observation: None,
            evaluation: None,
            failures: Vec::new(),
        }
    }

    pub fn push_failure(&mut self, error: EvalError) {
        self.failures.push(error.record);
        self.passed = false;
    }

    pub fn finish(&mut self, wall_time_ms: u64) {
        self.wall_time_ms = wall_time_ms;
        self.passed = self.failures.is_empty() && self.evaluation.is_some();
    }

    pub fn benchmark(&self) -> Option<AgentQualityBenchmarkV1> {
        let metrics = &self.observation.as_ref()?.metrics;
        AgentQualityBenchmarkV1::from_metrics(metrics, self.wall_time_ms)
    }
}

#[derive(Debug, Serialize)]
pub struct AgentQualityRunReportV1 {
    pub schema_version: &'static str,
    pub run_id: String,
    pub subject: SubjectArtifactV1,
    pub limits: AgentQualityLimitsV1,
    pub scenarios: Vec<AgentQualityScenarioReportV1>,
    pub passed: bool,
}

impl AgentQualityRunReportV1 {
    pub fn new(
        run_id: String,
        subject: SubjectArtifactV1,
        limits: AgentQualityLimitsV1,
        scenarios: Vec<AgentQualityScenarioReportV1>,
    ) -> Self {
        let passed = scenarios.iter().all(|scenario| scenario.passed);
        Self {
            schema_version: "1",
            run_id,
            subject,
            limits,
            scenarios,
            passed,
        }
    }

    pub fn write_to(&self, output_root: &Path) -> Result<PathBuf, EvalError> {
        let run_dir = output_root.join(&self.run_id);
        fs::create_dir_all(&run_dir).map_err(|error| {
            EvalError::setup(format!(
                "create report directory {}: {error}",
                run_dir.display()
            ))
        })?;
        let path = run_dir.join("results.json");
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| EvalError::setup(format!("serialize {}: {error}", path.display())))?;
        bytes.push(b'\n');
        fs::write(&path, bytes)
            .map_err(|error| EvalError::setup(format!("write {}: {error}", path.display())))?;
        Ok(run_dir)
    }
}

#[cfg(test)]
mod tests {
    use harness::prompt::SystemPromptStrategy;
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    fn subject() -> SubjectArtifactV1 {
        SubjectArtifactV1 {
            schema_version: "1",
            subject_id: "subject".into(),
            subject_sha256: "a".repeat(64),
            system_prompt_sha256: "b".repeat(64),
            model: "model".into(),
            provider: "provider".into(),
            system_prompt_strategy: SystemPromptStrategy::Override,
            thinking_level: None,
            provider_options: None,
        }
    }

    #[test]
    fn writes_one_self_contained_run_report() {
        let mut scenario =
            AgentQualityScenarioReportV1::new("example", "session-1".into(), "prompt".into());
        scenario.push_failure(EvalError::assertion("wrong result"));
        scenario.finish(12);
        let report = AgentQualityRunReportV1::new(
            "run-1".into(),
            subject(),
            AgentQualityLimitsV1::default(),
            vec![scenario],
        );
        let dir = tempdir().unwrap();
        let run = report.write_to(dir.path()).unwrap();
        let results: Value =
            serde_json::from_slice(&fs::read(run.join("results.json")).unwrap()).unwrap();
        assert_eq!(results["schema_version"], "1");
        assert_eq!(results["passed"], false);
        assert_eq!(results["limits"]["execution"]["max_turns"], 20);
        assert_eq!(results["scenarios"][0]["prompt"], "prompt");
        assert_eq!(
            results["scenarios"][0]["failures"][0]["message"],
            json!("wrong result")
        );
        assert_eq!(fs::read_dir(run).unwrap().count(), 1);
    }
}
