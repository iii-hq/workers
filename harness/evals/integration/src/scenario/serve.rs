use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::artifacts::{write_json, ArtifactSink};
use crate::client::DEFAULT_CALL_TIMEOUT_MS;
use crate::deadline::Deadline;
use crate::expand::{expand_compiled_fixture, ExpandedFixtureV1};
use crate::fixtures::ScenarioFixture;
use crate::grader::{self, Evidence};
use crate::process::TeardownReport;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::scenarios::ScenarioDriver;
use crate::services::RunServices;
use crate::stack::{free_loopback_port, RunPaths, Stack, StackBins};
use crate::types::scenario::{
    Classification, CompiledSendV1, ExecutionReportV1, InvariantResultV1,
};
use crate::types::script::SchemaVersion1;

use super::report::now_rfc3339;
use super::runner::{combine_teardown, result_digest, ScenarioRunner};
use super::state::{ActiveTurn, PreparedRun};

const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(100);
const FINAL_STATUS_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServeReadyV1 {
    pub schema_version: SchemaVersion1,
    pub run_id: String,
    pub scenario_id: String,
    pub scenario_slug: String,
    pub driver: ScenarioDriver,
    pub run_root: PathBuf,
    pub result_path: PathBuf,
    pub console_url: String,
    pub engine_url: String,
    pub session: ServeSessionV1,
    pub model: ServeModelV1,
    pub message: String,
    pub functions: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send: Option<CompiledSendV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServeSessionV1 {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServeModelV1 {
    pub id: String,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServeResultV1 {
    pub schema_version: SchemaVersion1,
    pub scenario_id: String,
    pub classification: Classification,
    pub invariants: Vec<InvariantResultV1>,
    pub skipped_invariants: Vec<String>,
    pub artifacts: Vec<String>,
}

pub struct ServeOutcome {
    pub result: ServeResultV1,
    pub run_id: String,
    pub run_root: PathBuf,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize)]
struct SupportServicesTeardown {
    deadline_exceeded: bool,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ServeTeardownReport {
    support_services: SupportServicesTeardown,
    processes: TeardownReport,
}

impl ServeTeardownReport {
    fn complete(&self) -> bool {
        !self.support_services.deadline_exceeded
            && self.support_services.error.is_none()
            && self.processes.complete()
    }
}

pub async fn serve_scenario(
    bins: &StackBins,
    fixture: &ScenarioFixture,
    artifacts_dir: &Path,
    ready_file: &Path,
) -> ServeOutcome {
    let started_at = now_rfc3339();
    let started = std::time::Instant::now();
    let run_id = format!("cu{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
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

    let mut classification = execute_serve(&mut runner, artifacts_dir, ready_file).await;
    let stable = runner.result(classification);
    let duration_ms = started.elapsed().as_millis() as u64;
    let mut execution = ExecutionReportV1 {
        schema_version: SchemaVersion1::V1,
        run_id: run_id.clone(),
        scenario_id: stable.scenario_id.clone(),
        started_at,
        duration_ms,
        result_path: "result.json".to_string(),
        result_sha256: result_digest(&stable),
    };

    if let Err(error) = runner.write_reports(&stable, &execution, artifacts_dir) {
        tracing::error!(target: "harness_integration::scenario", "serve reporting failed: {error:#}");
        classification = classification.combine(Classification::RunnerError);
        let mut retry = runner.result(classification);
        execution.result_sha256 = result_digest(&retry);
        if let Err(retry_error) = runner.write_reports(&retry, &execution, artifacts_dir) {
            tracing::error!(target: "harness_integration::scenario", "serve report retry failed: {retry_error:#}");
        }
        retry.classification = classification;
    }

    let mut result = ServeResultV1 {
        schema_version: SchemaVersion1::V1,
        scenario_id: fixture.scenario.id.clone(),
        classification,
        invariants: runner.invariants.clone(),
        skipped_invariants: vec!["send.flags".to_string()],
        artifacts: runner
            .sink
            .as_ref()
            .map(|sink| sink.paths().to_vec())
            .unwrap_or_default(),
    };
    if let Err(error) = write_json(&run_root, &run_root.join("serve-result.json"), &result) {
        tracing::error!(target: "harness_integration::scenario", "serve result failed: {error:#}");
        classification = classification.combine(Classification::RunnerError);
        result.classification = classification;
        let _ = write_json(&run_root, &run_root.join("serve-result.json"), &result);
    }

    if classification == Classification::Pass {
        if let Some(sink) = &runner.sink {
            sink.trim_passing_run();
        }
    }

    ServeOutcome {
        result,
        run_id,
        run_root,
        duration_ms,
    }
}

async fn execute_serve(
    runner: &mut ScenarioRunner<'_>,
    artifacts_dir: &Path,
    ready_file: &Path,
) -> Classification {
    let paths = match RunPaths::allocate(artifacts_dir, &runner.run_id) {
        Ok(paths) => paths,
        Err(error) => {
            return RunError::with_source(
                RunPhase::Allocate,
                RunErrorKind::Runner,
                "allocate console run paths",
                error,
            )
            .classification();
        }
    };
    runner.sink = Some(ArtifactSink::new(paths.root.clone()));

    let ExpandedFixtureV1 {
        scenario,
        script,
        system_prompt: expected_prompt,
    } = match expand_compiled_fixture(
        &runner.fixture.compiled(),
        &runner.run_id,
        &runner.session_id,
    ) {
        Ok(expanded) => expanded,
        Err(error) => {
            let error = RunError::with_source(
                RunPhase::Allocate,
                RunErrorKind::Runner,
                "expand console fixture",
                error,
            );
            return runner.finish(Err(error), None);
        }
    };

    if scenario.fault.is_some() || scenario.release.is_some() {
        let error = RunError::new(
            RunPhase::Allocate,
            RunErrorKind::Setup,
            "serve scenarios cannot inject faults or hold responses",
        );
        return runner.finish(Err(error), None);
    }
    let teardown_budget = Duration::from_millis(scenario.deadlines.teardown_ms);
    let prepared = PreparedRun::new(scenario, expected_prompt);
    if let Err(error) = paths.scenario_dir(&prepared.scenario.id) {
        let error = RunError::with_source(
            RunPhase::Allocate,
            RunErrorKind::Runner,
            "allocate console scenario artifact directory",
            error,
        );
        return runner.finish(Err(error), None);
    }

    let mut stack = match Stack::boot(runner.bins, paths).await {
        Ok(stack) => stack,
        Err(failure) => {
            let error = RunError::with_source(
                RunPhase::Boot,
                RunErrorKind::Setup,
                "boot isolated console stack",
                failure.error,
            );
            let complete = failure.teardown.complete();
            let teardown = ServeTeardownReport {
                support_services: SupportServicesTeardown {
                    deadline_exceeded: false,
                    error: None,
                },
                processes: failure.teardown,
            };
            let artifact =
                runner.write_run_artifact("teardown.json", &teardown, RunPhase::Teardown);
            return combine_teardown(runner.finish(Err(error), None), complete, artifact);
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
                "start console support services",
                error,
            );
            let early_exit = stack.early_exit();
            let processes = stack.teardown().await;
            let complete = processes.complete();
            let teardown = ServeTeardownReport {
                support_services: SupportServicesTeardown {
                    deadline_exceeded: false,
                    error: None,
                },
                processes,
            };
            let artifact =
                runner.write_run_artifact("teardown.json", &teardown, RunPhase::Teardown);
            return combine_teardown(runner.finish(Err(error), early_exit), complete, artifact);
        }
    };

    let outcome = run_serve_phases(runner, &mut stack, &services, &prepared, ready_file).await;
    let mut early_exit = stack.early_exit();
    let teardown_deadline = Deadline::after(teardown_budget);
    let support_services = match teardown_deadline
        .timeout("console support service shutdown", services.shutdown())
        .await
    {
        Ok(()) => SupportServicesTeardown {
            deadline_exceeded: false,
            error: None,
        },
        Err(error) => SupportServicesTeardown {
            deadline_exceeded: true,
            error: Some(error.to_string()),
        },
    };
    if early_exit.is_none() {
        early_exit = stack.early_exit();
    }
    stack.set_teardown_budget(teardown_deadline.remaining());
    let processes = stack.teardown().await;
    let teardown = ServeTeardownReport {
        support_services,
        processes,
    };
    let complete = teardown.complete();
    let artifact = runner.write_run_artifact("teardown.json", &teardown, RunPhase::Teardown);
    let classification = runner.finish(outcome, early_exit);
    combine_teardown(classification, complete, artifact)
}

async fn run_serve_phases(
    runner: &mut ScenarioRunner<'_>,
    stack: &mut Stack,
    services: &RunServices,
    prepared: &PreparedRun,
    ready_file: &Path,
) -> Result<(), RunError> {
    runner.probe(stack, services, prepared).await?;
    runner.arm(stack, services, prepared).await?;

    let session_title = format!("Console E2E {} {}", prepared.scenario.id, runner.run_id);
    services
        .client()
        .call_with_deadline(
            "session::ensure",
            json!({
                "session_id": runner.session_id,
                "title": session_title,
                "metadata": {
                    "surface": "console",
                    "model": format!(
                        "{}::{}",
                        prepared.scenario.send.provider,
                        prepared.scenario.send.model
                    ),
                    "mode": "agent",
                    "title_manual": true,
                    "integration_run_id": runner.run_id
                }
            }),
            prepared.readiness_deadline,
            DEFAULT_CALL_TIMEOUT_MS,
        )
        .await
        .map_err(|error| {
            RunError::with_source(
                RunPhase::Arm,
                RunErrorKind::Setup,
                "ensure console test session",
                anyhow::anyhow!(error),
            )
        })?;

    let http_port = free_loopback_port().map_err(|error| {
        RunError::with_source(
            RunPhase::Arm,
            RunErrorKind::Setup,
            "allocate console HTTP port",
            error,
        )
    })?;
    stack
        .spawn_console(runner.bins, http_port)
        .map_err(|error| {
            RunError::with_source(
                RunPhase::Arm,
                RunErrorKind::Setup,
                "spawn production console",
                error,
            )
        })?;
    wait_for_console(
        services,
        &stack.ws_url,
        http_port,
        prepared.readiness_deadline,
    )
    .await?;

    let ready = build_ready_manifest(runner, prepared, stack, http_port, &session_title);
    runner.write_run_artifact("serve-ready.json", &ready, RunPhase::Report)?;
    write_atomic_json(ready_file, &ready).map_err(|error| {
        RunError::with_source(
            RunPhase::Report,
            RunErrorKind::Runner,
            "publish console ready manifest",
            error,
        )
    })?;

    let scenario_deadline = Deadline::after(Duration::from_millis(
        prepared.scenario.deadlines.scenario_ms,
    ));
    wait_for_shutdown(stack, scenario_deadline).await?;

    // Completion is event-driven: Arm bound harness::turn-completed to the
    // recorder. Once it arrives, make one status call as the durable-state
    // confirmation used by the existing grader.
    let lifecycle = services
        .recorder()
        .wait_for_lifecycle(scenario_deadline)
        .await
        .map_err(|error| {
            let kind = if scenario_deadline.is_expired() {
                RunErrorKind::Timeout
            } else {
                RunErrorKind::Runner
            };
            RunError::with_source(
                RunPhase::Await,
                kind,
                "wait for harness::turn-completed delivery",
                error,
            )
        })?;
    let turn_id = lifecycle
        .payload
        .get("turn_id")
        .and_then(Value::as_str)
        .map(String::from);
    let evidence_deadline = Deadline::after(FINAL_STATUS_TIMEOUT);
    let final_status = services
        .client()
        .call_with_deadline(
            "harness::status",
            json!({ "session_id": runner.session_id }),
            evidence_deadline,
            DEFAULT_CALL_TIMEOUT_MS,
        )
        .await
        .map_err(|error| {
            RunError::with_source(
                RunPhase::Collect,
                RunErrorKind::Runner,
                "confirm terminal harness status",
                anyhow::anyhow!(error),
            )
        })?;

    let mut active = ActiveTurn::new(evidence_deadline, turn_id, Value::Null);
    active.final_status = final_status;
    runner.collect(services, prepared, &mut active).await?;
    grade_without_direct_send(runner, services, prepared, &active)
}

fn grade_without_direct_send(
    runner: &mut ScenarioRunner<'_>,
    services: &RunServices,
    prepared: &PreparedRun,
    active: &ActiveTurn,
) -> Result<(), RunError> {
    let specs: Vec<_> = prepared
        .scenario
        .invariants
        .iter()
        .filter(|spec| spec.id != "send.flags")
        .cloned()
        .collect();
    let evidence = Evidence {
        run_id: runner.run_id.clone(),
        session_id: runner.session_id.clone(),
        turn_id: active.turn_id.clone(),
        send_response: None,
        status: active.final_status.clone(),
        transcript: active.transcript.clone(),
        generations_consumed: services.router().generations_consumed(),
        generations_total: services.router().total_generations(),
        recorder_events: active.recorder_events.clone(),
    };
    runner.invariants = grader::grade(&specs, &evidence);
    let invariants = runner.invariants.clone();
    runner.write_artifact(
        &prepared.scenario.id,
        "invariants.json",
        &invariants,
        RunPhase::Grade,
    )?;
    if runner.invariants.iter().any(|invariant| !invariant.passed)
        || services.router().contract_failed()
    {
        return Err(RunError::new(
            RunPhase::Grade,
            RunErrorKind::Contract,
            "one or more console scenario contracts failed",
        ));
    }
    Ok(())
}

fn build_ready_manifest(
    runner: &ScenarioRunner<'_>,
    prepared: &PreparedRun,
    stack: &Stack,
    http_port: u16,
    session_title: &str,
) -> ServeReadyV1 {
    let prefix = format!("{}::", runner.run_id);
    let functions = std::iter::once(&prepared.scenario.recorder.target)
        .chain(prepared.scenario.recorder.extra_functions.iter())
        .filter_map(|function| {
            function
                .function_id
                .strip_prefix(&prefix)
                .map(|alias| (alias.to_string(), function.function_id.clone()))
        })
        .collect();
    let run_root = stack.paths.root.clone();
    ServeReadyV1 {
        schema_version: SchemaVersion1::V1,
        run_id: runner.run_id.clone(),
        scenario_id: prepared.scenario.id.clone(),
        scenario_slug: runner.fixture.slug.clone(),
        driver: runner.fixture.driver,
        result_path: run_root.join("serve-result.json"),
        run_root,
        console_url: format!("http://127.0.0.1:{http_port}"),
        engine_url: stack.ws_url.clone(),
        session: ServeSessionV1 {
            id: runner.session_id.clone(),
            title: session_title.to_string(),
        },
        model: ServeModelV1 {
            id: prepared.scenario.send.model.clone(),
            provider: prepared.scenario.send.provider.clone(),
        },
        message: prepared.scenario.send.message.clone(),
        functions,
        send: (runner.fixture.driver == ScenarioDriver::Direct)
            .then(|| prepared.scenario.send.clone()),
    }
}

async fn wait_for_console(
    services: &RunServices,
    engine_url: &str,
    http_port: u16,
    deadline: Deadline,
) -> Result<(), RunError> {
    deadline
        .poll_until("console readiness", READY_POLL_INTERVAL, || async {
            let status = services
                .client()
                .call_with_deadline(
                    "console::status",
                    json!({}),
                    deadline,
                    DEFAULT_CALL_TIMEOUT_MS,
                )
                .await;
            let status_ready = status.as_ref().is_ok_and(|value| {
                value.get("http_port").and_then(Value::as_u64) == Some(u64::from(http_port))
                    && value.get("engine_url").and_then(Value::as_str) == Some(engine_url)
            });
            if !status_ready {
                return Ok(None);
            }
            Ok(http_root_available(http_port).await.then_some(()))
        })
        .await
        .map_err(|error| {
            RunError::with_source(
                RunPhase::Probe,
                RunErrorKind::Setup,
                "production console did not become ready",
                error,
            )
        })
}

async fn http_root_available(port: u16) -> bool {
    let Ok(mut stream) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await else {
        return false;
    };
    if stream
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .is_err()
    {
        return false;
    }
    let mut response = [0_u8; 64];
    let Ok(read) = stream.read(&mut response).await else {
        return false;
    };
    let status = String::from_utf8_lossy(&response[..read]);
    status.starts_with("HTTP/1.1 200") || status.starts_with("HTTP/1.0 200")
}

async fn wait_for_shutdown(stack: &mut Stack, deadline: Deadline) -> Result<(), RunError> {
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let mut health = tokio::time::interval(PROCESS_POLL_INTERVAL);
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            result = &mut shutdown => {
                result.map_err(|error| RunError::with_source(
                    RunPhase::Await,
                    RunErrorKind::Runner,
                    "wait for console test shutdown",
                    error,
                ))?;
                return Ok(());
            }
            _ = tokio::time::sleep_until(deadline.expires_at()) => {
                return Err(RunError::new(
                    RunPhase::Await,
                    RunErrorKind::Timeout,
                    "console test did not finish before the scenario deadline",
                ));
            }
            _ = health.tick() => {
                if let Some(exit) = stack.early_exit() {
                    return Err(RunError::new(
                        RunPhase::Await,
                        RunErrorKind::ProcessCrash,
                        format!("{} exited while the console test was running: {}", exit.name, exit.status),
                    ));
                }
            }
        }
    }
}

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

fn write_atomic_json(path: &Path, value: &impl Serialize) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("ready file needs a UTF-8 filename"))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let encoded = crate::canonical::canonical_json_pretty(&serde_json::to_value(value)?);
    std::fs::write(&temporary, encoded)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}
