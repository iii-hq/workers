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
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::scenarios::ScenarioDriver;
use crate::services::RunServices;
use crate::stack::{free_loopback_port, RunPaths, Stack, StackBins};
use crate::types::scenario::{Classification, CompiledSendV1};
use crate::types::script::SchemaVersion1;

use super::runner::ScenarioRunner;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServeResultV1 {
    pub schema_version: SchemaVersion1,
    pub scenario_id: String,
    pub classification: Classification,
    /// First floor or verify failure, scrubbed of run-scoped ids.
    pub failure: Option<String>,
    /// Raw serialized [`crate::evidence_data::RunEvidence`] (JSON null when
    /// the run failed before collection). Ids are real, so Playwright can
    /// check evidence against the ready manifest.
    pub evidence: serde_json::Value,
    pub artifacts: Vec<String>,
}

pub struct ServeOutcome {
    pub result: ServeResultV1,
    pub run_id: String,
    pub run_root: PathBuf,
    pub duration_ms: u64,
}

pub async fn serve_scenario(
    bins: &StackBins,
    fixture: &ScenarioFixture,
    artifacts_dir: &Path,
    ready_file: &Path,
) -> ServeOutcome {
    let started = std::time::Instant::now();
    let run_id = format!("iu{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let run_root = artifacts_dir.join(&run_id);
    let session_id = format!("s_{}", uuid::Uuid::new_v4().simple());

    let mut runner = ScenarioRunner {
        bins,
        fixture,
        run_id: run_id.clone(),
        session_id,
        sink: None,
        failure: None,
        evidence: Value::Null,
    };

    // The serve consumer contract is serve-ready.json -> serve-result.json;
    // the direct-run result.json/execution.json pair is not written here.
    let mut classification = execute_serve(&mut runner, artifacts_dir, ready_file).await;
    let duration_ms = started.elapsed().as_millis() as u64;

    let mut result = ServeResultV1 {
        schema_version: SchemaVersion1::V1,
        scenario_id: fixture.scenario.id.clone(),
        classification,
        failure: runner.failure.clone(),
        evidence: runner.evidence.clone(),
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
            return RunError::runner(RunPhase::Allocate, "allocate console run paths", error)
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
            let error = RunError::runner(RunPhase::Allocate, "expand console fixture", error);
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
        let error = RunError::runner(
            RunPhase::Allocate,
            "allocate console scenario artifact directory",
            error,
        );
        return runner.finish(Err(error), None);
    }

    let mut stack = match Stack::boot(runner.bins, paths).await {
        Ok(stack) => stack,
        Err(failure) => {
            let error =
                RunError::setup(RunPhase::Boot, "boot isolated console stack", failure.error);
            return runner.fail_before_services(error, failure.teardown, None);
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
            let error = RunError::setup(RunPhase::Boot, "start console support services", error);
            let early_exit = stack.early_exit();
            let processes = stack.teardown().await;
            return runner.fail_before_services(error, processes, early_exit);
        }
    };

    let outcome = run_serve_phases(runner, &mut stack, &services, &prepared, ready_file).await;
    runner
        .finalize(stack, services, teardown_budget, outcome)
        .await
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
            RunError::setup(
                RunPhase::Arm,
                "ensure console test session",
                anyhow::anyhow!(error),
            )
        })?;

    let http_port = free_loopback_port()
        .map_err(|error| RunError::setup(RunPhase::Arm, "allocate console HTTP port", error))?;
    stack
        .spawn_console(runner.bins, http_port)
        .map_err(|error| RunError::setup(RunPhase::Arm, "spawn production console", error))?;
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
        RunError::runner(RunPhase::Report, "publish console ready manifest", error)
    })?;

    let scenario_deadline = Deadline::after(Duration::from_millis(
        prepared.scenario.deadlines.scenario_ms,
    ));
    wait_for_shutdown(stack, scenario_deadline).await?;

    // Completion is event-driven: Arm bound harness::turn-completed to the
    // recorder. Once it arrives, make one status call as the durable-state
    // confirmation checked by the floor.
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
            RunError::runner(
                RunPhase::Collect,
                "confirm terminal harness status",
                anyhow::anyhow!(error),
            )
        })?;

    let mut active = ActiveTurn::new(evidence_deadline, turn_id, Value::Null);
    active.final_status = final_status;
    runner.collect(services, prepared, &mut active).await?;

    // Shared floor + verify path with Rpc mode. The Console (not the runner)
    // submitted the send, so there is no send response and the send-flags
    // floor check is skipped.
    let evidence = runner.build_evidence(services, &active, None);
    runner.evidence = serde_json::to_value(&evidence).map_err(|error| {
        RunError::runner(RunPhase::Grade, "serialize console run evidence", error)
    })?;
    runner.verify_evidence(services, &evidence, false)
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
            RunError::setup(
                RunPhase::Probe,
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
                result.map_err(|error| RunError::runner(RunPhase::Await, "wait for console test shutdown", error))?;
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
