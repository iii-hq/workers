//! Scenario execution lifecycle:
//! Allocate → Boot → Probe → Arm → Send → Fault/Release → Await →
//! Collect → Grade → Teardown → Report.
//!
//! Every phase returns [`RunError`]. Classification is derived once after
//! process state has been inspected, so a process exit is combined with the
//! phase failure through [`Classification::combine`] instead of being
//! reclassified by individual early returns.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::artifacts::{write_json, ArtifactSink};
use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;
use crate::expand::Placeholders;
use crate::fixtures::ScenarioFixture;
use crate::grader::{self, Evidence};
use crate::readiness::{self, ReadinessSpec};
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::services::RunServices;
use crate::stack::{
    expected_config_entries, expected_harness_config_entry, EarlyExit, RunPaths, Stack, StackBins,
};
use crate::types::recorder::{RecorderEventKind, RecorderEventV1};
use crate::types::scenario::{
    Classification, CompiledScenarioV1, ExecutionReportV1, IntegrationResultV1, InvariantResultV1,
};
use crate::types::script::{RouterScriptV1, SchemaVersion1};

const SEND_TIMEOUT_MS: u64 = 30_000;
const DISCOVERY_POLL_INTERVAL: Duration = Duration::from_millis(200);
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const TARGET_POLL_INTERVAL: Duration = Duration::from_millis(50);
const LIFECYCLE_GRACE: Duration = Duration::from_secs(3);

pub struct RunOutcome {
    pub result: IntegrationResultV1,
    pub run_id: String,
    pub run_root: PathBuf,
    pub duration_ms: u64,
}

pub async fn run_scenario(
    bins: &StackBins,
    fixture: &ScenarioFixture,
    artifacts_dir: &Path,
    retain_success: bool,
) -> RunOutcome {
    let started_at = now_rfc3339();
    let started = std::time::Instant::now();
    let run_id = format!("cr{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
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
    let execution = ExecutionReportV1 {
        schema_version: SchemaVersion1::V1,
        run_id: run_id.clone(),
        started_at,
        duration_ms,
        result_path: "result.json".to_string(),
    };

    if let Err(error) = runner.write_reports(&result, &execution, artifacts_dir) {
        tracing::error!("reporting failed: {error:#}");
        classification = classification.combine(classify(Some(&error), ProcessState::Running));
        result.classification = classification;

        // A first write may have succeeded before the second failed. Rewrite
        // both reports with the final runner_error classification.
        if let Err(retry_error) = runner.write_reports(&result, &execution, artifacts_dir) {
            tracing::error!("final report retry failed: {retry_error:#}");
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

struct ExpandedFixture {
    scenario: CompiledScenarioV1,
    script: RouterScriptV1,
    expected_prompt: String,
}

struct RunContext {
    scenario: CompiledScenarioV1,
    expected_prompt: String,
    readiness_deadline: Deadline,
    deadline: Option<Deadline>,
    turn_id: Option<String>,
    send_response: Option<Value>,
    final_status: Value,
    transcript: Vec<Value>,
    recorder_events: Vec<RecorderEventV1>,
    timed_out: bool,
}

impl RunContext {
    fn new(scenario: CompiledScenarioV1, expected_prompt: String) -> Self {
        let readiness_deadline =
            Deadline::after(Duration::from_millis(scenario.deadlines.readiness_ms));
        Self {
            scenario,
            expected_prompt,
            readiness_deadline,
            deadline: None,
            turn_id: None,
            send_response: None,
            final_status: Value::Null,
            transcript: Vec::new(),
            recorder_events: Vec::new(),
            timed_out: false,
        }
    }

    fn scenario_deadline(&self, phase: RunPhase) -> Result<Deadline, RunError> {
        self.deadline.ok_or_else(|| {
            RunError::new(
                phase,
                RunErrorKind::Runner,
                "scenario deadline was not initialized by send",
            )
        })
    }
}

struct ScenarioRunner<'a> {
    bins: &'a StackBins,
    fixture: &'a ScenarioFixture,
    run_id: String,
    session_id: String,
    sink: Option<ArtifactSink>,
    invariants: Vec<InvariantResultV1>,
}

impl ScenarioRunner<'_> {
    async fn execute(&mut self, artifacts_dir: &Path) -> Classification {
        let paths = match RunPaths::allocate(artifacts_dir, &self.run_id) {
            Ok(paths) => paths,
            Err(error) => {
                let error = RunError::with_source(
                    RunPhase::Allocate,
                    RunErrorKind::Runner,
                    "allocate run paths",
                    error,
                );
                return self.finish(Err(error), None);
            }
        };
        self.sink = Some(ArtifactSink::new(paths.root.clone()));

        let ExpandedFixture {
            scenario,
            script,
            expected_prompt,
        } = match self.expand_fixture() {
            Ok(expanded) => expanded,
            Err(error) => {
                let error = RunError::with_source(
                    RunPhase::Allocate,
                    RunErrorKind::Runner,
                    "expand compiled fixture",
                    error,
                );
                return self.finish(Err(error), None);
            }
        };
        let teardown_budget = Duration::from_millis(scenario.deadlines.teardown_ms);
        let mut context = RunContext::new(scenario, expected_prompt);

        if let Err(error) = paths.scenario_dir(&context.scenario.id) {
            let error = RunError::with_source(
                RunPhase::Allocate,
                RunErrorKind::Runner,
                "allocate scenario artifact directory",
                error,
            );
            return self.finish(Err(error), None);
        }

        let mut stack = match Stack::boot(self.bins, paths).await {
            Ok(stack) => stack,
            Err(error) => {
                let error = RunError::with_source(
                    RunPhase::Boot,
                    RunErrorKind::Setup,
                    "boot isolated stack",
                    error,
                );
                return self.finish(Err(error), None);
            }
        };
        stack.set_teardown_budget(teardown_budget);

        let mut services = match RunServices::start(
            &stack.ws_url,
            script,
            stack.paths.root.join("recorder.log.jsonl"),
        )
        .await
        {
            Ok(services) => services,
            Err(error) => {
                let error = RunError::with_source(
                    RunPhase::Boot,
                    RunErrorKind::Setup,
                    "start run-scoped support services",
                    error,
                );
                let early_exit = stack.early_exit();
                stack.teardown().await;
                return self.finish(Err(error), early_exit);
            }
        };

        let outcome = self.run_phases(&mut stack, &services, &mut context).await;

        // Inspect process state while the subject is still running. Service
        // shutdown is intentionally before process teardown, and a second
        // inspection catches a child that exits during that boundary.
        let mut early_exit = stack.early_exit();
        let teardown_deadline = Deadline::after(teardown_budget);
        if let Err(error) = teardown_deadline
            .timeout("support service shutdown", services.shutdown())
            .await
        {
            tracing::warn!("support service shutdown exceeded teardown budget: {error}");
        }
        if early_exit.is_none() {
            early_exit = stack.early_exit();
        }
        stack.set_teardown_budget(teardown_deadline.remaining());
        stack.teardown().await;

        self.finish(outcome, early_exit)
    }

    async fn run_phases(
        &mut self,
        stack: &mut Stack,
        services: &RunServices,
        context: &mut RunContext,
    ) -> Result<(), RunError> {
        self.probe(stack, services, context).await?;
        self.arm(stack, services, context).await?;
        self.send(services, context).await?;
        self.fault(stack, services, context).await?;
        self.release(services, context).await?;
        self.r#await(services, context).await?;
        self.collect(services, context).await?;
        self.grade(services, context)
    }

    async fn probe(
        &mut self,
        stack: &mut Stack,
        services: &RunServices,
        context: &RunContext,
    ) -> Result<(), RunError> {
        let spec = ReadinessSpec::pre_harness(expected_config_entries(&stack.paths));
        if let Err(report) =
            readiness::probe(services.client(), &spec, context.readiness_deadline).await
        {
            self.write_artifact(
                &context.scenario.id,
                "readiness-failure.json",
                &json!({ "phase": "pre_harness", "missing": report.missing }),
                RunPhase::Probe,
            )?;
            return Err(RunError::new(
                RunPhase::Probe,
                RunErrorKind::Setup,
                "pre-harness readiness failed",
            ));
        }
        Ok(())
    }

    async fn arm(
        &mut self,
        stack: &mut Stack,
        services: &RunServices,
        context: &RunContext,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Arm;
        let recorder = services.recorder();
        let scenario = &context.scenario;

        let digest = recorder
            .configure(&self.run_id, &scenario.recorder)
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Setup,
                    "configure controlled recorder",
                    error,
                )
            })?;
        let expected_digest = crate::canonical::sha256_of_canonical(&Value::Object(
            scenario.recorder.target.request_schema.clone(),
        ));
        if digest != expected_digest {
            return Err(RunError::new(
                phase,
                RunErrorKind::Setup,
                format!("target schema digest mismatch: {digest} != {expected_digest}"),
            ));
        }

        recorder.reset(&self.run_id).map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Setup,
                "reset controlled recorder",
                error,
            )
        })?;
        if !recorder
            .snapshot(None)
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Setup,
                    "snapshot controlled recorder after reset",
                    error,
                )
            })?
            .is_empty()
        {
            return Err(RunError::new(
                phase,
                RunErrorKind::Setup,
                "recorder snapshot is not empty after reset",
            ));
        }

        let controlled_function_ids: Vec<String> = std::iter::once(&scenario.recorder.target)
            .chain(scenario.recorder.extra_functions.iter())
            .map(|function| function.function_id.clone())
            .collect();
        let discovery_deadline = context.readiness_deadline;
        discovery_deadline
            .poll_until(
                "controlled function discovery",
                DISCOVERY_POLL_INTERVAL,
                || async {
                    let listed = services
                        .client()
                        .call_with_deadline(
                            "engine::functions::list",
                            json!({ "include_internal": true }),
                            discovery_deadline,
                            DEFAULT_CALL_TIMEOUT_MS,
                        )
                        .await;
                    Ok(listed.ok().and_then(|listed| {
                        controlled_function_ids
                            .iter()
                            .all(|id| readiness::has_function(&listed, id))
                            .then_some(())
                    }))
                },
            )
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Setup,
                    "controlled functions did not appear in discovery",
                    error,
                )
            })?;

        stack.spawn_harness(self.bins).map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Setup,
                "spawn harness under test",
                error,
            )
        })?;

        let harness_deadline = context.readiness_deadline;
        let spec = ReadinessSpec::harness_surface(expected_harness_config_entry(&stack.paths));
        if let Err(report) = readiness::probe(services.client(), &spec, harness_deadline).await {
            self.write_artifact(
                &scenario.id,
                "readiness-failure.json",
                &json!({ "phase": "harness", "missing": report.missing }),
                phase,
            )?;
            return Err(RunError::new(
                phase,
                RunErrorKind::Setup,
                "harness readiness failed",
            ));
        }

        recorder
            .bind_lifecycle(
                scenario.recorder.lifecycle.trigger_type.as_str(),
                &self.session_id,
            )
            .await
            .map_err(|error| {
                RunError::with_source(phase, RunErrorKind::Setup, "bind lifecycle recorder", error)
            })?;
        for binding in &scenario.bindings {
            recorder
                .bind(
                    &binding.trigger_type,
                    &binding.function_id,
                    binding.config.clone(),
                )
                .await
                .map_err(|error| {
                    RunError::with_source(
                        phase,
                        RunErrorKind::Setup,
                        format!(
                            "bind trigger {} to {}",
                            binding.trigger_type, binding.function_id
                        ),
                        error,
                    )
                })?;
        }

        let bound_function_ids: Vec<String> =
            std::iter::once("integration-recorder::lifecycle".to_string())
                .chain(
                    scenario
                        .bindings
                        .iter()
                        .map(|binding| binding.function_id.clone()),
                )
                .collect();
        let binding_deadline = context.readiness_deadline;
        binding_deadline
            .poll_until(
                "registered trigger discovery",
                DISCOVERY_POLL_INTERVAL,
                || async {
                    let listed = services
                        .client()
                        .call_with_deadline(
                            "engine::registered-triggers::list",
                            json!({ "include_internal": true }),
                            binding_deadline,
                            DEFAULT_CALL_TIMEOUT_MS,
                        )
                        .await;
                    Ok(listed.ok().and_then(|listed| {
                        bound_function_ids
                            .iter()
                            .all(|id| readiness::has_registered_trigger(&listed, id))
                            .then_some(())
                    }))
                },
            )
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Setup,
                    "trigger bindings did not appear in discovery",
                    error,
                )
            })?;

        self.sink
            .as_mut()
            .expect("artifact sink initialized after allocation")
            .write_scenario_text(
                &scenario.id,
                "expected-system-prompt.txt",
                &context.expected_prompt,
            )
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Runner,
                    "write expected system prompt",
                    error,
                )
            })?;

        Ok(())
    }

    async fn send(
        &mut self,
        services: &RunServices,
        context: &mut RunContext,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Send;
        let deadline = Deadline::after(Duration::from_millis(
            context.scenario.deadlines.scenario_ms,
        ));
        context.deadline = Some(deadline);

        self.write_artifact(
            &context.scenario.id,
            "request.json",
            &context.scenario.send,
            phase,
        )?;
        let response = services
            .client()
            .call_with_deadline(
                "harness::send",
                context.scenario.send.clone(),
                deadline,
                SEND_TIMEOUT_MS,
            )
            .await;
        match response {
            Ok(value) => {
                self.write_artifact(&context.scenario.id, "send-response.json", &value, phase)?;
                context.turn_id = value
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .map(String::from);
                context.send_response = Some(value);
                Ok(())
            }
            Err(error) => {
                let value = json!({ "error": error });
                self.write_artifact(&context.scenario.id, "send-response.json", &value, phase)?;
                Err(rpc_failure(
                    phase,
                    RunErrorKind::Contract,
                    deadline,
                    "harness::send failed",
                    error,
                ))
            }
        }
    }

    async fn fault(
        &mut self,
        stack: &mut Stack,
        services: &RunServices,
        context: &RunContext,
    ) -> Result<(), RunError> {
        let Some(fault) = &context.scenario.fault else {
            return Ok(());
        };
        let phase = RunPhase::Fault;
        let deadline = context.scenario_deadline(phase)?;

        let crate::types::scenario::FaultKind::EngineSigkill = fault.kind;
        deadline
            .poll_until("fault trigger", TARGET_POLL_INTERVAL, || async {
                let events = services.recorder().snapshot(None)?;
                let count = events
                    .iter()
                    .filter(|event| {
                        event.kind == RecorderEventKind::TargetCall
                            && event.function_id == fault.function_id
                    })
                    .count() as u64;
                Ok((count >= fault.after_target_calls).then_some(()))
            })
            .await
            .map_err(|error| {
                let kind = if deadline.is_expired() {
                    RunErrorKind::Timeout
                } else {
                    RunErrorKind::Runner
                };
                RunError::with_source(
                    phase,
                    kind,
                    format!(
                        "fewer than {} target calls observed before fault",
                        fault.after_target_calls
                    ),
                    error,
                )
            })?;

        stack.kill_engine().await.map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Runner,
                "kill engine for fault injection",
                error,
            )
        })?;
        deadline
            .timeout(
                "fault restart delay",
                tokio::time::sleep(Duration::from_millis(fault.restart_delay_ms)),
            )
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Timeout,
                    "fault restart delay exceeded scenario deadline",
                    error,
                )
            })?;
        stack.respawn_engine().map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Runner,
                "respawn engine after fault injection",
                error,
            )
        })?;
        Ok(())
    }

    async fn release(
        &mut self,
        services: &RunServices,
        context: &RunContext,
    ) -> Result<(), RunError> {
        let Some(release) = &context.scenario.release else {
            return Ok(());
        };
        let phase = RunPhase::Release;
        let deadline = context.scenario_deadline(phase)?;

        deadline
            .poll_until(
                format!("pending call {}", release.function_call_id),
                STATUS_POLL_INTERVAL,
                || async {
                    let status = services
                        .client()
                        .call_with_deadline(
                            "harness::status",
                            json!({ "session_id": self.session_id }),
                            deadline,
                            DEFAULT_CALL_TIMEOUT_MS,
                        )
                        .await;
                    Ok(status.ok().and_then(|status| {
                        status
                            .get("pending_function_calls")
                            .and_then(Value::as_array)
                            .is_some_and(|calls| {
                                calls
                                    .iter()
                                    .any(|call| call.as_str() == Some(&release.function_call_id))
                            })
                            .then_some(())
                    }))
                },
            )
            .await
            .map_err(|error| {
                RunError::with_source(
                    phase,
                    RunErrorKind::Timeout,
                    format!(
                        "call {} never appeared as pending",
                        release.function_call_id
                    ),
                    error,
                )
            })?;

        let response = services
            .client()
            .call_with_deadline(
                "harness::function::resolve",
                json!({
                    "session_id": self.session_id,
                    "turn_id": context.turn_id,
                    "function_call_id": release.function_call_id,
                    "action": release.action,
                }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await;
        match response {
            Ok(value) => {
                self.write_artifact(&context.scenario.id, "resolve-response.json", &value, phase)?;
                Ok(())
            }
            Err(error) => {
                let value = json!({ "error": error });
                self.write_artifact(&context.scenario.id, "resolve-response.json", &value, phase)?;
                Err(rpc_failure(
                    phase,
                    RunErrorKind::Contract,
                    deadline,
                    "harness::function::resolve failed",
                    error,
                ))
            }
        }
    }

    async fn r#await(
        &mut self,
        services: &RunServices,
        context: &mut RunContext,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Await;
        let deadline = context.scenario_deadline(phase)?;
        let last_status = Arc::new(Mutex::new(Value::Null));
        let session_id = self.session_id.clone();

        let terminal = deadline
            .poll_until("terminal harness status", STATUS_POLL_INTERVAL, || {
                let last_status = Arc::clone(&last_status);
                let session_id = session_id.clone();
                async move {
                    let response = services
                        .client()
                        .call_with_deadline(
                            "harness::status",
                            json!({ "session_id": session_id }),
                            deadline,
                            DEFAULT_CALL_TIMEOUT_MS,
                        )
                        .await;
                    let Ok(status) = response else {
                        return Ok(None);
                    };
                    *last_status
                        .lock()
                        .map_err(|_| anyhow::anyhow!("last status lock poisoned"))? =
                        status.clone();
                    let terminal = matches!(
                        status.get("status").and_then(Value::as_str),
                        Some("completed") | Some("failed") | Some("cancelled")
                    );
                    Ok(terminal.then_some(status))
                }
            })
            .await;

        match terminal {
            Ok(status) => context.final_status = status,
            Err(error) if deadline.is_expired() => {
                context.timed_out = true;
                context.final_status = last_status
                    .lock()
                    .map_err(|_| {
                        RunError::new(phase, RunErrorKind::Runner, "last status lock poisoned")
                    })?
                    .clone();
                tracing::error!("await timed out: {error:#}");
                return Ok(());
            }
            Err(error) => {
                return Err(RunError::with_source(
                    phase,
                    RunErrorKind::Runner,
                    "poll terminal harness status",
                    error,
                ));
            }
        }

        let grace_expires =
            (tokio::time::Instant::now() + LIFECYCLE_GRACE).min(deadline.expires_at());
        let grace = Deadline::at(grace_expires);
        if !grace.is_expired() {
            let lifecycle = grace
                .poll_until("lifecycle delivery grace", TARGET_POLL_INTERVAL, || async {
                    let events = services.recorder().snapshot(None)?;
                    Ok(events
                        .iter()
                        .any(|event| event.kind == RecorderEventKind::Lifecycle)
                        .then_some(()))
                })
                .await;
            if let Err(error) = lifecycle {
                if !grace.is_expired() {
                    return Err(RunError::with_source(
                        phase,
                        RunErrorKind::Runner,
                        "poll lifecycle delivery",
                        error,
                    ));
                }
            }
        }
        Ok(())
    }

    async fn collect(
        &mut self,
        services: &RunServices,
        context: &mut RunContext,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Collect;
        let deadline = context.scenario_deadline(phase)?;

        if deadline.is_expired() {
            context.timed_out = true;
            context.transcript = Vec::new();
        } else {
            context.transcript = self.collect_transcript(services.client(), deadline).await?;
        }
        self.write_artifact(
            &context.scenario.id,
            "transcript.json",
            &context.transcript,
            phase,
        )?;
        self.write_artifact(
            &context.scenario.id,
            "status.json",
            &context.final_status,
            phase,
        )?;
        self.write_artifact(
            &context.scenario.id,
            "router-calls.json",
            &services.router().evidence(),
            phase,
        )?;

        context.recorder_events = services.recorder().snapshot(None).map_err(|error| {
            RunError::with_source(
                phase,
                RunErrorKind::Runner,
                "snapshot recorder evidence",
                error,
            )
        })?;
        let (target_calls, lifecycle_events): (Vec<_>, Vec<_>) = context
            .recorder_events
            .iter()
            .partition(|event| event.kind == RecorderEventKind::TargetCall);
        self.write_artifact(
            &context.scenario.id,
            "target-calls.json",
            &target_calls,
            phase,
        )?;
        self.write_artifact(
            &context.scenario.id,
            "lifecycle-events.json",
            &lifecycle_events,
            phase,
        )?;
        if deadline.is_expired() {
            context.timed_out = true;
        }
        Ok(())
    }

    fn grade(&mut self, services: &RunServices, context: &RunContext) -> Result<(), RunError> {
        let phase = RunPhase::Grade;
        let evidence = Evidence {
            run_id: self.run_id.clone(),
            session_id: self.session_id.clone(),
            turn_id: context.turn_id.clone(),
            send_response: context.send_response.clone(),
            status: context.final_status.clone(),
            transcript: context.transcript.clone(),
            generations_consumed: services.router().generations_consumed(),
            generations_total: services.router().total_generations(),
            recorder_events: context.recorder_events.clone(),
        };
        self.invariants = grader::grade(&context.scenario.invariants, &evidence);
        let invariants = self.invariants.clone();
        self.write_artifact(&context.scenario.id, "invariants.json", &invariants, phase)?;

        if context.timed_out {
            return Err(RunError::new(
                phase,
                RunErrorKind::Timeout,
                "scenario deadline elapsed before evidence collection completed",
            ));
        }
        if self.invariants.iter().any(|invariant| !invariant.passed)
            || services.router().contract_failed()
        {
            return Err(RunError::new(
                phase,
                RunErrorKind::Contract,
                "one or more scenario contracts failed",
            ));
        }
        Ok(())
    }

    async fn collect_transcript(
        &self,
        client: &Client,
        deadline: Deadline,
    ) -> Result<Vec<Value>, RunError> {
        let phase = RunPhase::Collect;
        let mut messages = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = BTreeSet::new();

        loop {
            let mut payload = json!({
                "session_id": self.session_id,
                "limit": 100,
                "include_custom": true,
            });
            if let Some(cursor) = &cursor {
                payload["cursor"] = json!(cursor);
            }
            let page = client
                .call_with_deadline(
                    "session::messages",
                    payload,
                    deadline,
                    DEFAULT_CALL_TIMEOUT_MS,
                )
                .await
                .map_err(|error| {
                    rpc_failure(
                        phase,
                        RunErrorKind::Runner,
                        deadline,
                        "collect session transcript",
                        error,
                    )
                })?;
            if let Some(items) = page.get("messages").and_then(Value::as_array) {
                messages.extend(items.iter().cloned());
            }
            let Some(next) = page.get("next_cursor").and_then(Value::as_str) else {
                break;
            };
            if !seen_cursors.insert(next.to_string()) {
                return Err(RunError::new(
                    phase,
                    RunErrorKind::Runner,
                    format!("session::messages repeated cursor {next:?}"),
                ));
            }
            cursor = Some(next.to_string());
        }
        Ok(messages)
    }

    fn expand_fixture(&self) -> anyhow::Result<ExpandedFixture> {
        let base = Placeholders::new(&self.run_id, &self.session_id);
        let expected_prompt = base.expand_str(&self.fixture.system_prompt_template)?;
        let digest = crate::canonical::sha256_of_bytes(expected_prompt.as_bytes());
        let placeholders = base.with_system_prompt_sha256(&digest);

        let mut scenario_value = serde_json::to_value(&self.fixture.scenario)?;
        placeholders.expand_value(&mut scenario_value)?;
        let scenario: CompiledScenarioV1 = serde_json::from_value(scenario_value)?;

        let mut script_value = serde_json::to_value(&self.fixture.script)?;
        placeholders.expand_value(&mut script_value)?;
        let script: RouterScriptV1 = serde_json::from_value(script_value)?;

        Ok(ExpandedFixture {
            scenario,
            script,
            expected_prompt,
        })
    }

    fn write_artifact<T>(
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

    fn finish(
        &mut self,
        outcome: Result<(), RunError>,
        early_exit: Option<EarlyExit>,
    ) -> Classification {
        let mut error = outcome.err();
        if let Some(run_error) = &error {
            let error_chain = run_error.chain_string();
            tracing::error!(
                phase = %run_error.phase,
                kind = %run_error.kind,
                "scenario execution failed: {error_chain}"
            );
            if let Err(artifact_error) = self.write_failure_artifact(run_error) {
                tracing::error!("failure artifact could not be written: {artifact_error:#}");
                error = Some(artifact_error);
            }
        }

        let process_state = match early_exit {
            Some(exit) => {
                tracing::error!(
                    process = %exit.name,
                    status = %exit.status,
                    stderr = %exit.stderr_log.display(),
                    "subject process exited before teardown"
                );
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

    fn result(&self, classification: Classification) -> IntegrationResultV1 {
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

    fn write_reports(
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
enum ProcessState {
    Running,
    Crashed,
}

fn classify(error: Option<&RunError>, process_state: ProcessState) -> Classification {
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

fn rpc_failure(
    phase: RunPhase,
    default_kind: RunErrorKind,
    deadline: Deadline,
    message: impl Into<String>,
    error: String,
) -> RunError {
    let kind = if deadline.is_expired() {
        RunErrorKind::Timeout
    } else {
        default_kind
    };
    RunError::with_source(phase, kind, message, anyhow::anyhow!(error))
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_exit_is_combined_with_phase_failure_by_precedence() {
        let timeout = RunError::new(RunPhase::Await, RunErrorKind::Timeout, "deadline");
        assert_eq!(
            classify(Some(&timeout), ProcessState::Crashed),
            Classification::ProcessCrash
        );

        let runner = RunError::new(RunPhase::Collect, RunErrorKind::Runner, "artifact");
        assert_eq!(
            classify(Some(&runner), ProcessState::Crashed),
            Classification::RunnerError
        );
        assert_eq!(
            classify(None, ProcessState::Crashed),
            Classification::ProcessCrash
        );
    }
}
