use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::artifacts::ArtifactSink;
use crate::deadline::Deadline;
use crate::expand::{expand_compiled_fixture, ExpandedFixtureV1};
use crate::fixtures::ScenarioFixture;
use crate::process::TeardownReport;
use crate::runtime::{RunError, RunPhase};
use crate::services::RunServices;
use crate::stack::{EarlyExit, RunPaths, Stack, StackBins};
use crate::types::scenario::{
    Classification, ExecutionReportV1, IntegrationResultV1, InvariantResultV1,
};
use crate::types::script::SchemaVersion1;

use super::report::{classify, now_rfc3339, ProcessState};
use super::state::PreparedRun;

pub struct RunOutcome {
    pub result: IntegrationResultV1,
    pub run_id: String,
    pub run_root: PathBuf,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize)]
struct SupportServicesTeardown {
    deadline_exceeded: bool,
    error: Option<String>,
}

impl SupportServicesTeardown {
    fn clean() -> Self {
        Self {
            deadline_exceeded: false,
            error: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct StackTeardownReport {
    support_services: SupportServicesTeardown,
    processes: TeardownReport,
}

impl StackTeardownReport {
    fn processes_only(processes: TeardownReport) -> Self {
        Self {
            support_services: SupportServicesTeardown::clean(),
            processes,
        }
    }

    fn complete(&self) -> bool {
        !self.support_services.deadline_exceeded
            && self.support_services.error.is_none()
            && self.processes.complete()
    }
}

pub async fn run_scenario(
    bins: &StackBins,
    fixture: &ScenarioFixture,
    artifacts_dir: &Path,
    retain_success: bool,
) -> RunOutcome {
    let started_at = now_rfc3339();
    let started = std::time::Instant::now();
    let run_id = format!("ir{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let run_root = artifacts_dir.join(&run_id);
    let session_id = format!("s_{}", uuid::Uuid::new_v4().simple());

    let mut runner = ScenarioRunner {
        bins,
        fixture,
        run_id: run_id.clone(),
        session_id,
        sink: None,
        invariants: Vec::new(),
    };

    let mut classification = runner.execute(artifacts_dir).await;
    let mut result = runner.result(classification);
    let duration_ms = started.elapsed().as_millis() as u64;
    let mut execution = ExecutionReportV1 {
        schema_version: SchemaVersion1::V1,
        run_id: run_id.clone(),
        scenario_id: result.scenario_id.clone(),
        started_at,
        duration_ms,
        result_path: "result.json".to_string(),
        result_sha256: result_digest(&result),
    };

    if let Err(error) = runner.write_reports(&result, &execution, artifacts_dir) {
        tracing::error!(target: "harness_integration::scenario", "reporting failed: {error:#}");
        classification = classification.combine(classify(Some(&error), ProcessState::Running));
        result.classification = classification;
        execution.result_sha256 = result_digest(&result);

        // A first write may have succeeded before the second failed. Rewrite
        // both reports with the final runner_error classification.
        if let Err(retry_error) = runner.write_reports(&result, &execution, artifacts_dir) {
            tracing::error!(
                target: "harness_integration::scenario",
                "final report retry failed: {retry_error:#}"
            );
        }
    }

    if classification == Classification::Pass && !retain_success {
        if let Some(sink) = &runner.sink {
            sink.trim_passing_run();
        }
    }

    RunOutcome {
        result,
        run_id,
        run_root,
        duration_ms,
    }
}

pub(super) fn result_digest(result: &IntegrationResultV1) -> String {
    let value = serde_json::to_value(result).expect("integration result serializes");
    let rendered = crate::canonical::canonical_json_pretty(&value);
    crate::canonical::sha256_of_bytes(rendered.as_bytes())
}

pub(super) struct ScenarioRunner<'a> {
    pub(super) bins: &'a StackBins,
    pub(super) fixture: &'a ScenarioFixture,
    pub(super) run_id: String,
    pub(super) session_id: String,
    pub(super) sink: Option<ArtifactSink>,
    pub(super) invariants: Vec<InvariantResultV1>,
}

impl ScenarioRunner<'_> {
    async fn execute(&mut self, artifacts_dir: &Path) -> Classification {
        let paths = match RunPaths::allocate(artifacts_dir, &self.run_id) {
            Ok(paths) => paths,
            Err(error) => {
                let error = RunError::runner(RunPhase::Allocate, "allocate run paths", error);
                return self.finish(Err(error), None);
            }
        };
        self.sink = Some(ArtifactSink::new(paths.root.clone()));

        let ExpandedFixtureV1 {
            scenario,
            script,
            system_prompt: expected_prompt,
        } = match expand_compiled_fixture(&self.fixture.compiled(), &self.run_id, &self.session_id)
        {
            Ok(expanded) => expanded,
            Err(error) => {
                let error = RunError::runner(RunPhase::Allocate, "expand compiled fixture", error);
                return self.finish_without_stack(error);
            }
        };
        let teardown_budget = Duration::from_millis(scenario.deadlines.teardown_ms);
        let prepared = PreparedRun::new(scenario, expected_prompt);

        if let Err(error) = paths.scenario_dir(&prepared.scenario.id) {
            let error = RunError::runner(
                RunPhase::Allocate,
                "allocate scenario artifact directory",
                error,
            );
            return self.finish_without_stack(error);
        }

        let mut stack = match Stack::boot(self.bins, paths).await {
            Ok(stack) => stack,
            Err(failure) => {
                let error = RunError::setup(RunPhase::Boot, "boot isolated stack", failure.error);
                return self.fail_before_services(error, failure.teardown, None);
            }
        };
        stack.set_teardown_budget(teardown_budget);

        let services = match RunServices::start(
            &stack.ws_url,
            script,
            stack.paths.root.join("recorder.log.jsonl"),
        )
        .await
        {
            Ok(services) => services,
            Err(error) => {
                let error =
                    RunError::setup(RunPhase::Boot, "start run-scoped support services", error);
                let early_exit = stack.early_exit();
                let processes = stack.teardown().await;
                return self.fail_before_services(error, processes, early_exit);
            }
        };

        let outcome = self.run_phases(&mut stack, &services, &prepared).await;
        self.finalize(stack, services, teardown_budget, outcome)
            .await
    }

    async fn run_phases(
        &mut self,
        stack: &mut Stack,
        services: &RunServices,
        prepared: &PreparedRun,
    ) -> Result<(), RunError> {
        self.probe(stack, services, prepared).await?;
        self.arm(stack, services, prepared).await?;
        let mut active = self.send(services, prepared).await?;
        self.fault(stack, services, prepared, &active).await?;
        self.release(services, prepared, &active).await?;
        self.r#await(services, &mut active).await?;
        self.collect(services, prepared, &mut active).await?;
        self.grade(services, prepared, &active)
    }

    fn finish_without_stack(&mut self, error: RunError) -> Classification {
        self.fail_before_services(error, TeardownReport::default(), None)
    }

    /// Write `teardown.json` and fold the failure into a classification for
    /// a run that ended before support services started.
    pub(super) fn fail_before_services(
        &mut self,
        error: RunError,
        processes: TeardownReport,
        early_exit: Option<EarlyExit>,
    ) -> Classification {
        let teardown = StackTeardownReport::processes_only(processes);
        let teardown_complete = teardown.complete();
        let artifact = self.write_run_artifact("teardown.json", &teardown, RunPhase::Teardown);
        let classification = self.finish(Err(error), early_exit);
        combine_teardown(classification, teardown_complete, artifact)
    }

    /// Shared teardown tail for run and serve. Inspects process state while
    /// the subject is still running; service shutdown is intentionally
    /// before process teardown, and a second inspection catches a child
    /// that exits during that boundary.
    pub(super) async fn finalize(
        &mut self,
        mut stack: Stack,
        mut services: RunServices,
        teardown_budget: Duration,
        outcome: Result<(), RunError>,
    ) -> Classification {
        let mut early_exit = stack.early_exit();
        let teardown_deadline = Deadline::after(teardown_budget);
        let support_services = match teardown_deadline
            .timeout("support service shutdown", services.shutdown())
            .await
        {
            Ok(()) => SupportServicesTeardown::clean(),
            Err(error) => {
                tracing::warn!(
                    target: "harness_integration::scenario",
                    "support service shutdown exceeded teardown budget: {error}"
                );
                SupportServicesTeardown {
                    deadline_exceeded: true,
                    error: Some(error.to_string()),
                }
            }
        };
        if early_exit.is_none() {
            early_exit = stack.early_exit();
        }
        stack.set_teardown_budget(teardown_deadline.remaining());
        let processes = stack.teardown().await;
        let teardown = StackTeardownReport {
            support_services,
            processes,
        };
        let teardown_complete = teardown.complete();
        let artifact = self.write_run_artifact("teardown.json", &teardown, RunPhase::Teardown);
        let classification = self.finish(outcome, early_exit);
        combine_teardown(classification, teardown_complete, artifact)
    }
}

pub(super) fn combine_teardown(
    classification: Classification,
    teardown_complete: bool,
    artifact: Result<(), RunError>,
) -> Classification {
    if let Err(error) = artifact {
        tracing::error!(
            target: "harness_integration::scenario",
            "teardown artifact could not be written: {error:#}"
        );
        return classification.combine(Classification::RunnerError);
    }
    if !teardown_complete {
        tracing::error!(
            target: "harness_integration::scenario",
            "teardown was incomplete; see teardown.json"
        );
        return classification.combine(Classification::RunnerError);
    }
    classification
}
