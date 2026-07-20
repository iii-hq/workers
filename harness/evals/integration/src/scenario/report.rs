use std::path::Path;

use serde::Serialize;
use serde_json::json;

use crate::artifacts::write_json;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::stack::EarlyExit;
use crate::types::scenario::{Classification, ExecutionReportV1, IntegrationResultV1};
use crate::types::script::SchemaVersion1;

use super::runner::ScenarioRunner;

impl ScenarioRunner<'_> {
    pub(super) fn write_run_artifact<T>(
        &mut self,
        name: &str,
        value: &T,
        phase: RunPhase,
    ) -> Result<(), RunError>
    where
        T: Serialize + ?Sized,
    {
        self.sink
            .as_mut()
            .ok_or_else(|| {
                RunError::new(
                    phase,
                    RunErrorKind::Runner,
                    "artifact sink is not initialized",
                )
            })?
            .write_json(name, value)
            .map(|_| ())
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Runner,
                    format!("write run artifact {name}"),
                    error,
                )
            })
    }

    pub(super) fn write_artifact<T>(
        &mut self,
        scenario_id: &str,
        name: &str,
        value: &T,
        phase: RunPhase,
    ) -> Result<(), RunError>
    where
        T: Serialize + ?Sized,
    {
        self.sink
            .as_mut()
            .ok_or_else(|| {
                RunError::new(
                    phase,
                    RunErrorKind::Runner,
                    "artifact sink is not initialized",
                )
            })?
            .write_scenario_json(scenario_id, name, value)
            .map(|_| ())
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Runner,
                    format!("write scenario artifact {name}"),
                    error,
                )
            })
    }

    pub(super) fn finish(
        &mut self,
        outcome: Result<(), RunError>,
        early_exit: Option<EarlyExit>,
    ) -> Classification {
        let mut error = outcome.err();
        if let Some(run_error) = &error {
            let error_chain = run_error.chain_string();
            tracing::error!(
                target: "harness_integration::scenario",
                phase = %run_error.phase,
                kind = %run_error.kind,
                "scenario execution failed: {error_chain}"
            );
            if let Err(artifact_error) = self.write_failure_artifact(run_error) {
                tracing::error!(
                    target: "harness_integration::scenario",
                    "failure artifact could not be written: {artifact_error:#}"
                );
                error = Some(artifact_error);
            }
        }

        let process_state = match early_exit {
            Some(exit) => {
                tracing::error!(
                    target: "harness_integration::scenario",
                    process = %exit.name,
                    status = %exit.status,
                    stderr = %exit.stderr_log.display(),
                    "subject process exited before teardown"
                );
                if let Err(artifact_error) =
                    self.write_run_artifact("process-exit.json", &exit, RunPhase::Teardown)
                {
                    tracing::error!(
                        target: "harness_integration::scenario",
                        "process-exit artifact could not be written: {artifact_error:#}"
                    );
                    error = Some(artifact_error);
                }
                ProcessState::Crashed
            }
            None => ProcessState::Running,
        };
        classify(error.as_ref(), process_state)
    }

    fn write_failure_artifact(&mut self, error: &RunError) -> Result<(), RunError> {
        let scenario_id = self.fixture.scenario.id.clone();
        self.write_artifact(
            &scenario_id,
            "failure.json",
            &json!({
                "phase": error.phase.to_string(),
                "kind": error.kind.to_string(),
                "message": error.message,
                "error": error.chain_string(),
            }),
            error.phase,
        )
    }

    pub(super) fn result(&self, classification: Classification) -> IntegrationResultV1 {
        IntegrationResultV1 {
            schema_version: SchemaVersion1::V1,
            scenario_id: self.fixture.scenario.id.clone(),
            classification,
            invariants: self.invariants.clone(),
            artifacts: self
                .sink
                .as_ref()
                .map(|sink| sink.paths().to_vec())
                .unwrap_or_default(),
        }
    }

    pub(super) fn write_reports(
        &self,
        result: &IntegrationResultV1,
        execution: &ExecutionReportV1,
        artifacts_dir: &Path,
    ) -> Result<(), RunError> {
        let run_root = self
            .sink
            .as_ref()
            .map(|sink| sink.run_root().to_path_buf())
            .unwrap_or_else(|| artifacts_dir.join(&self.run_id));
        write_json(&run_root, &run_root.join("result.json"), result).map_err(|error| {
            RunError::with_source(
                RunPhase::Report,
                RunErrorKind::Runner,
                "write stable result.json",
                error,
            )
        })?;
        write_json(&run_root, &run_root.join("execution.json"), execution).map_err(|error| {
            RunError::with_source(
                RunPhase::Report,
                RunErrorKind::Runner,
                "write volatile execution.json",
                error,
            )
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ProcessState {
    Running,
    Crashed,
}

pub(super) fn classify(error: Option<&RunError>, process_state: ProcessState) -> Classification {
    let phase_classification = error
        .map(RunError::classification)
        .unwrap_or(Classification::Pass);
    let process_error = match process_state {
        ProcessState::Running => None,
        ProcessState::Crashed => Some(RunError::new(
            RunPhase::Teardown,
            RunErrorKind::ProcessCrash,
            "subject process exited before teardown",
        )),
    };
    let process_classification = process_error
        .as_ref()
        .map(RunError::classification)
        .unwrap_or(Classification::Pass);
    phase_classification.combine(process_classification)
}

pub(super) fn rpc_failure(
    phase: RunPhase,
    kind: RunErrorKind,
    message: impl Into<String>,
    error: String,
) -> RunError {
    RunError::with_source(phase, kind, message, anyhow::anyhow!(error))
}

pub(super) fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}
