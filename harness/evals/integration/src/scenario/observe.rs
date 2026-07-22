//! Observe driver: armed stack for Playwright UI tests.
//!
//! The integration owns stimulus (`harness::send` after a start signal).
//! Playwright owns the Console process and DOM assertions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::artifacts::write_json;
use crate::client::DEFAULT_CALL_TIMEOUT_MS;
use crate::deadline::Deadline;
use crate::fixtures::ScenarioFixture;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::scenarios::ScenarioDriver;
use crate::stack::{Stack, StackBins};
use crate::types::scenario::{Classification, CompiledSendV1};
use crate::types::script::SchemaVersion1;

use super::runner::{BootedRun, ExpandedRun, ScenarioRunner};
use super::state::PreparedRun;

const START_POLL_INTERVAL: Duration = Duration::from_millis(100);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveReadyV1 {
    pub schema_version: SchemaVersion1,
    pub run_id: String,
    pub scenario_id: String,
    pub scenario_slug: String,
    pub driver: ScenarioDriver,
    pub run_root: PathBuf,
    pub result_path: PathBuf,
    pub engine_url: String,
    pub session: ObserveSessionV1,
    pub model: ObserveModelV1,
    pub message: String,
    pub functions: BTreeMap<String, String>,
    pub send: CompiledSendV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveSessionV1 {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveModelV1 {
    pub id: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveStartV1 {
    pub schema_version: SchemaVersion1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveResultV1 {
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

pub struct ObserveOutcome {
    pub result: ObserveResultV1,
    pub run_id: String,
    pub run_root: PathBuf,
    pub duration_ms: u64,
}

pub async fn observe_scenario(
    bins: &StackBins,
    fixture: &ScenarioFixture,
    artifacts_dir: &Path,
    ready_file: &Path,
) -> ObserveOutcome {
    let started = std::time::Instant::now();
    let run_id = format!("iu{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let run_root = artifacts_dir.join(&run_id);
    let session_id = format!("s_{}", uuid::Uuid::new_v4().simple());

    let mut runner = ScenarioRunner::new(bins, fixture, run_id.clone(), session_id);

    // The observe consumer contract is observe-ready.json / ready-file ->
    // observe-result.json; the direct-run result.json/execution.json pair is
    // not written here.
    let mut classification = execute_observe(&mut runner, artifacts_dir, ready_file).await;
    let duration_ms = started.elapsed().as_millis() as u64;

    let mut result = ObserveResultV1 {
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
    if let Err(error) = write_json(&run_root, &run_root.join("observe-result.json"), &result) {
        tracing::error!(target: "harness_integration::scenario", "observe result failed: {error:#}");
        classification = classification.combine(Classification::RunnerError);
        result.classification = classification;
        let _ = write_json(&run_root, &run_root.join("observe-result.json"), &result);
    }

    if classification == Classification::Pass {
        if let Some(sink) = &runner.sink {
            sink.trim_passing_run();
        }
    }

    ObserveOutcome {
        result,
        run_id,
        run_root,
        duration_ms,
    }
}

async fn execute_observe(
    runner: &mut ScenarioRunner<'_>,
    artifacts_dir: &Path,
    ready_file: &Path,
) -> Classification {
    let ExpandedRun { paths, expanded } = match runner.expand_for_run(artifacts_dir) {
        Ok(expanded) => expanded,
        Err(classification) => return classification,
    };

    let mut booted = match runner.boot_prepared(paths, expanded).await {
        Ok(booted) => booted,
        Err(classification) => return classification,
    };

    let outcome = async {
        runner.arm_booted(&mut booted).await?;
        run_observe_phases(runner, &mut booted, ready_file).await
    }
    .await;
    runner
        .finalize(
            booted.stack,
            booted.services,
            booted.teardown_budget,
            outcome,
        )
        .await
}

async fn run_observe_phases(
    runner: &mut ScenarioRunner<'_>,
    booted: &mut BootedRun,
    ready_file: &Path,
) -> Result<(), RunError> {
    let stack = &mut booted.stack;
    let services = &booted.services;
    let prepared = &booted.prepared;

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
            prepared.setup_deadline,
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

    let ready = build_ready_manifest(runner, prepared, stack, &session_title);
    runner.write_run_artifact("observe-ready.json", &ready, RunPhase::Report)?;
    write_atomic_json(ready_file, &ready).map_err(|error| {
        RunError::runner(RunPhase::Report, "publish observe ready manifest", error)
    })?;

    let start_file = start_file_path(ready_file)?;
    wait_for_start(&start_file, prepared.setup_deadline).await?;

    let mut active = runner.send(services, prepared).await?;
    runner.fault(stack, services, prepared, &active).await?;
    runner.release(services, prepared, &active).await?;
    runner.r#await(services, &mut active).await?;

    let scenario_deadline = active.deadline;
    wait_for_shutdown(stack, scenario_deadline).await?;

    runner.collect(services, prepared, &mut active).await?;
    let evidence = runner.build_evidence(services, &active, Some(active.send_response.clone()));
    runner.evidence = serde_json::to_value(&evidence).map_err(|error| {
        RunError::runner(RunPhase::Grade, "serialize observe run evidence", error)
    })?;
    runner.verify_evidence(services, &evidence, active.timed_out)
}

fn build_ready_manifest(
    runner: &ScenarioRunner<'_>,
    prepared: &PreparedRun,
    stack: &Stack,
    session_title: &str,
) -> ObserveReadyV1 {
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
    ObserveReadyV1 {
        schema_version: SchemaVersion1::V1,
        run_id: runner.run_id.clone(),
        scenario_id: prepared.scenario.id.clone(),
        scenario_slug: runner.fixture.slug.clone(),
        driver: runner.fixture.driver,
        result_path: run_root.join("observe-result.json"),
        run_root,
        engine_url: stack.ws_url.clone(),
        session: ObserveSessionV1 {
            id: runner.session_id.clone(),
            title: session_title.to_string(),
        },
        model: ObserveModelV1 {
            id: prepared.scenario.send.model.clone(),
            provider: prepared.scenario.send.provider.clone(),
        },
        message: prepared.scenario.send.message.clone(),
        functions,
        send: prepared.scenario.send.clone(),
    }
}

fn start_file_path(ready_file: &Path) -> Result<PathBuf, RunError> {
    let parent = ready_file.parent().ok_or_else(|| {
        RunError::new(
            RunPhase::Probe,
            RunErrorKind::Runner,
            "ready file path has no parent directory for start.json",
        )
    })?;
    Ok(parent.join("start.json"))
}

async fn wait_for_start(start_file: &Path, deadline: Deadline) -> Result<(), RunError> {
    deadline
        .poll_until("observer start signal", START_POLL_INTERVAL, || {
            let start_file = start_file.to_path_buf();
            async move {
                match std::fs::read_to_string(&start_file) {
                    Ok(contents) => {
                        let parsed: ObserveStartV1 = serde_json::from_str(&contents)
                            .map_err(|error| anyhow::anyhow!("invalid start.json: {error}"))?;
                        let _ = parsed;
                        Ok(Some(()))
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(anyhow::anyhow!(
                        "read start signal {}: {error}",
                        start_file.display()
                    )),
                }
            }
        })
        .await
        .map_err(|error| {
            let kind = if deadline.is_expired() {
                RunErrorKind::Timeout
            } else {
                RunErrorKind::Setup
            };
            RunError::with_source(
                RunPhase::Probe,
                kind,
                "observer did not publish start.json",
                error,
            )
        })
}

async fn wait_for_shutdown(stack: &mut Stack, deadline: Deadline) -> Result<(), RunError> {
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let mut health = tokio::time::interval(PROCESS_POLL_INTERVAL);
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            result = &mut shutdown => {
                result.map_err(|error| {
                    RunError::runner(RunPhase::Await, "wait for observe test shutdown", error)
                })?;
                return Ok(());
            }
            _ = tokio::time::sleep_until(deadline.expires_at()) => {
                return Err(RunError::new(
                    RunPhase::Await,
                    RunErrorKind::Timeout,
                    "observe test did not finish before the scenario deadline",
                ));
            }
            _ = health.tick() => {
                if let Some(exit) = stack.early_exit() {
                    return Err(RunError::new(
                        RunPhase::Await,
                        RunErrorKind::ProcessCrash,
                        format!(
                            "{} exited while the observe test was running: {}",
                            exit.name, exit.status
                        ),
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
