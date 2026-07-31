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
use crate::stack::{EarlyExit, RunLayout, Stack, StackBins};
use crate::types::scenario::{Classification, ExecutionReportV1, IntegrationResultV1};
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

    let mut runner = ScenarioRunner::new(bins, fixture, run_id.clone(), session_id);

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
    /// First floor or verify failure, scrubbed of run-scoped ids.
    pub(super) failure: Option<String>,
    /// Raw serialized [`crate::evidence_data::RunEvidence`], published in
    /// playground results for Playwright; JSON null until collected.
    pub(super) evidence: serde_json::Value,
}

/// Allocate + expand result shared by Direct and Playground drivers.
pub(super) struct ExpandedRun {
    pub paths: RunLayout,
    pub expanded: ExpandedFixtureV1,
}

/// Booted stack + services ready for scenario phases.
pub(super) struct BootedRun {
    pub stack: Stack,
    pub services: RunServices,
    pub prepared: PreparedRun,
    pub teardown_budget: Duration,
}

impl<'a> ScenarioRunner<'a> {
    pub(super) fn new(
        bins: &'a StackBins,
        fixture: &'a ScenarioFixture,
        run_id: String,
        session_id: String,
    ) -> Self {
        Self {
            bins,
            fixture,
            run_id,
            session_id,
            sink: None,
            failure: None,
            evidence: serde_json::Value::Null,
        }
    }

    /// Allocate run layout, open the artifact sink, and expand the fixture.
    pub(super) fn expand_for_run(
        &mut self,
        artifacts_dir: &Path,
    ) -> Result<ExpandedRun, Classification> {
        let paths = match RunLayout::allocate(artifacts_dir, &self.run_id) {
            Ok(paths) => paths,
            Err(error) => {
                let error = RunError::runner(RunPhase::Allocate, "allocate run paths", error);
                return Err(self.finish(Err(error), None));
            }
        };
        self.sink = Some(ArtifactSink::new(paths.root.clone()));

        let expanded =
            match expand_compiled_fixture(&self.fixture.compiled(), &self.run_id, &self.session_id)
            {
                Ok(expanded) => expanded,
                Err(error) => {
                    let error =
                        RunError::runner(RunPhase::Allocate, "expand compiled fixture", error);
                    return Err(self.finish_without_stack(error));
                }
            };
        Ok(ExpandedRun { paths, expanded })
    }

    /// Create scenario dirs, boot the stack, and start support services.
    pub(super) async fn boot_prepared(
        &mut self,
        paths: RunLayout,
        expanded: ExpandedFixtureV1,
    ) -> Result<BootedRun, Classification> {
        let ExpandedFixtureV1 {
            scenario,
            script,
            system_prompt: expected_prompt,
        } = expanded;
        let teardown_budget = Duration::from_millis(scenario.deadlines.teardown_ms);
        let prepared = PreparedRun::new(scenario, expected_prompt);

        if let Err(error) = paths.scenario_dir(&prepared.scenario.id) {
            let error = RunError::runner(
                RunPhase::Allocate,
                "allocate scenario artifact directory",
                error,
            );
            return Err(self.finish_without_stack(error));
        }

        let mut stack = match Stack::boot(self.bins, paths).await {
            Ok(stack) => stack,
            Err(failure) => {
                let error = RunError::setup(RunPhase::Boot, "boot isolated stack", failure.error);
                return Err(self.fail_before_services(error, failure.teardown, None));
            }
        };
        stack.set_teardown_budget(teardown_budget);

        let services = match RunServices::start(&stack.ws_url, script).await {
            Ok(services) => services,
            Err(error) => {
                let error =
                    RunError::setup(RunPhase::Boot, "start run-scoped support services", error);
                let early_exit = stack.early_exit();
                let processes = stack.teardown().await;
                return Err(self.fail_before_services(error, processes, early_exit));
            }
        };

        Ok(BootedRun {
            stack,
            services,
            prepared,
            teardown_budget,
        })
    }

    /// Arm a booted stack (shared by Direct and Playground drivers).
    pub(super) async fn arm_booted(&mut self, booted: &mut BootedRun) -> Result<(), RunError> {
        self.arm(&mut booted.stack, &booted.services, &booted.prepared)
            .await
    }

    async fn execute(&mut self, artifacts_dir: &Path) -> Classification {
        let ExpandedRun { paths, expanded } = match self.expand_for_run(artifacts_dir) {
            Ok(expanded) => expanded,
            Err(classification) => return classification,
        };
        let mut booted = match self.boot_prepared(paths, expanded).await {
            Ok(booted) => booted,
            Err(classification) => return classification,
        };

        let outcome = async {
            self.arm_booted(&mut booted).await?;
            self.run_phases_after_arm(&mut booted.stack, &booted.services, &booted.prepared)
                .await
        }
        .await;
        self.finalize(
            booted.stack,
            booted.services,
            booted.teardown_budget,
            outcome,
        )
        .await
    }

    async fn run_phases_after_arm(
        &mut self,
        stack: &mut Stack,
        services: &RunServices,
        prepared: &PreparedRun,
    ) -> Result<(), RunError> {
        let mut active = self.send(services, prepared).await?;
        self.fault(stack, services, prepared, &active).await?;
        self.r#await(services, &mut active).await?;
        self.collect(services, prepared, &mut active).await?;
        let evidence = self.build_evidence(services, &active, Some(active.send_response.clone()));
        self.verify_evidence(services, &evidence, active.timed_out)
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

    /// Shared teardown tail for run and playground. Inspects process state while
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
