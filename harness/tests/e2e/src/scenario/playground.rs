//! Playground driver: the integration owns the stack and production Console;
//! a person or Playwright owns the turn stimulus.

use std::collections::BTreeMap;
use std::io::Write;
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
use super::state::{ActiveTurn, PreparedRun};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(100);
const SHUTDOWN_COMPLETION_GRACE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaygroundReadyV1 {
    pub schema_version: SchemaVersion1,
    pub run_id: String,
    pub scenario_id: String,
    pub scenario_slug: String,
    pub driver: ScenarioDriver,
    pub run_root: PathBuf,
    pub result_path: PathBuf,
    pub engine_url: String,
    pub console_url: String,
    pub session: PlaygroundSessionV1,
    pub model: PlaygroundModelV1,
    pub message: String,
    pub functions: BTreeMap<String, String>,
    pub send: CompiledSendV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaygroundSessionV1 {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaygroundModelV1 {
    pub id: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaygroundResultV1 {
    pub schema_version: SchemaVersion1,
    pub scenario_id: String,
    pub classification: Classification,
    pub failure: Option<String>,
    pub evidence: serde_json::Value,
    pub artifacts: Vec<String>,
}

pub struct PlaygroundOutcome {
    pub result: PlaygroundResultV1,
    pub run_id: String,
    pub run_root: PathBuf,
    pub duration_ms: u64,
}

pub async fn playground_scenario(
    bins: &StackBins,
    console_bin: &Path,
    fixture: &ScenarioFixture,
    artifacts_dir: &Path,
    ready_file: Option<&Path>,
) -> PlaygroundOutcome {
    let started = std::time::Instant::now();
    let run_id = format!("ip{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let run_root = artifacts_dir.join(&run_id);
    let session_id = format!("s_{}", uuid::Uuid::new_v4().simple());
    let mut runner = ScenarioRunner::new(bins, fixture, run_id.clone(), session_id);

    let mut classification =
        execute_playground(&mut runner, console_bin, artifacts_dir, ready_file).await;
    let duration_ms = started.elapsed().as_millis() as u64;
    let mut result = PlaygroundResultV1 {
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
    let result_path = run_root.join("playground-result.json");
    if let Err(error) = write_json(&run_root, &result_path, &result) {
        tracing::error!(target: "harness_integration::scenario", "playground result failed: {error:#}");
        classification = classification.combine(Classification::RunnerError);
        result.classification = classification;
        let _ = write_json(&run_root, &result_path, &result);
    }

    if classification == Classification::Pass {
        if let Some(sink) = &runner.sink {
            sink.trim_passing_run();
        }
    }

    PlaygroundOutcome {
        result,
        run_id,
        run_root,
        duration_ms,
    }
}

async fn execute_playground(
    runner: &mut ScenarioRunner<'_>,
    console_bin: &Path,
    artifacts_dir: &Path,
    ready_file: Option<&Path>,
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
        run_playground_phases(runner, &mut booted, console_bin, ready_file).await
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

async fn run_playground_phases(
    runner: &mut ScenarioRunner<'_>,
    booted: &mut BootedRun,
    console_bin: &Path,
    ready_file: Option<&Path>,
) -> Result<(), RunError> {
    let stack = &mut booted.stack;
    let services = &booted.services;
    let prepared = &booted.prepared;
    let session_title = format!("Integration {} {}", prepared.scenario.id, runner.run_id);

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
                "ensure playground session",
                anyhow::anyhow!(error),
            )
        })?;

    let console_url = stack
        .spawn_console(console_bin)
        .map_err(|error| RunError::setup(RunPhase::Arm, "spawn production Console", error))?;
    wait_for_console(stack, &console_url, prepared.setup_deadline).await?;

    let ready = build_ready_manifest(runner, prepared, stack, &session_title, &console_url);
    runner.write_run_artifact("playground-ready.json", &ready, RunPhase::Report)?;
    if let Some(path) = ready_file {
        write_atomic_json(path, &ready).map_err(|error| {
            RunError::runner(RunPhase::Report, "publish playground ready manifest", error)
        })?;
    }
    println!("Console: {console_url}");
    std::io::stdout()
        .flush()
        .map_err(|error| RunError::runner(RunPhase::Report, "flush Console URL", error))?;

    let deadline = Deadline::after(Duration::from_millis(
        prepared.scenario.deadlines.scenario_ms,
    ));
    let mut active = ActiveTurn::external(deadline);
    let shutdown_consumed = wait_for_external_turn(runner, stack, services, &mut active).await?;
    if !shutdown_consumed {
        wait_for_shutdown(stack, deadline).await?;
    }

    runner.collect(services, prepared, &mut active).await?;
    let evidence = runner.build_evidence(services, &active, None);
    runner.evidence = serde_json::to_value(&evidence).map_err(|error| {
        RunError::runner(RunPhase::Grade, "serialize playground run evidence", error)
    })?;
    runner.verify_evidence(services, &evidence, active.timed_out)
}

async fn wait_for_external_turn(
    runner: &mut ScenarioRunner<'_>,
    stack: &mut Stack,
    services: &crate::services::RunServices,
    active: &mut ActiveTurn,
) -> Result<bool, RunError> {
    let completion = runner.r#await(services, active);
    let shutdown = shutdown_signal();
    tokio::pin!(completion, shutdown);
    let mut health = tokio::time::interval(PROCESS_POLL_INTERVAL);
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            result = &mut completion => return result.map(|()| false),
            result = &mut shutdown => {
                result.map_err(|error| RunError::runner(
                    RunPhase::Await,
                    "wait for playground shutdown",
                    error,
                ))?;
                return match tokio::time::timeout(
                    SHUTDOWN_COMPLETION_GRACE,
                    &mut completion,
                )
                .await
                {
                    Ok(result) => result.map(|()| true),
                    Err(_) => Err(RunError::new(
                        RunPhase::Await,
                        RunErrorKind::Contract,
                        "playground stopped before a turn completed",
                    )),
                };
            }
            _ = health.tick() => {
                if let Some(exit) = stack.early_exit() {
                    return Err(process_exit_error(exit, "before a turn completed"));
                }
            }
        }
    }
}

async fn wait_for_console(
    stack: &mut Stack,
    console_url: &str,
    deadline: Deadline,
) -> Result<(), RunError> {
    let port = console_url
        .rsplit(':')
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| {
            RunError::new(
                RunPhase::Arm,
                RunErrorKind::Runner,
                format!("invalid Console URL {console_url:?}"),
            )
        })?;
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Ok(());
        }
        if let Some(exit) = stack.early_exit() {
            return Err(process_exit_error(
                exit,
                "before its HTTP port became ready",
            ));
        }
        if deadline.is_expired() {
            return Err(RunError::new(
                RunPhase::Arm,
                RunErrorKind::Setup,
                "Console HTTP port did not become ready",
            ));
        }
        tokio::time::sleep(PROCESS_POLL_INTERVAL).await;
    }
}

fn build_ready_manifest(
    runner: &ScenarioRunner<'_>,
    prepared: &PreparedRun,
    stack: &Stack,
    session_title: &str,
    console_url: &str,
) -> PlaygroundReadyV1 {
    let prefix = format!("{}::", runner.run_id);
    let functions = prepared
        .scenario
        .recorder
        .target
        .function_id
        .strip_prefix(&prefix)
        .filter(|alias| *alias != "unused")
        .map(|alias| {
            BTreeMap::from([(
                alias.to_string(),
                prepared.scenario.recorder.target.function_id.clone(),
            )])
        })
        .unwrap_or_default();
    let run_root = stack.paths.root.clone();
    PlaygroundReadyV1 {
        schema_version: SchemaVersion1::V1,
        run_id: runner.run_id.clone(),
        scenario_id: prepared.scenario.id.clone(),
        scenario_slug: runner.fixture.slug.clone(),
        driver: runner.fixture.driver,
        result_path: run_root.join("playground-result.json"),
        run_root,
        engine_url: stack.ws_url.clone(),
        console_url: console_url.to_string(),
        session: PlaygroundSessionV1 {
            id: runner.session_id.clone(),
            title: session_title.to_string(),
        },
        model: PlaygroundModelV1 {
            id: prepared.scenario.send.model.clone(),
            provider: prepared.scenario.send.provider.clone(),
        },
        message: prepared.scenario.send.message.clone(),
        functions,
        send: prepared.scenario.send.clone(),
    }
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
                    RunError::runner(RunPhase::Await, "wait for playground shutdown", error)
                })?;
                return Ok(());
            }
            _ = tokio::time::sleep_until(deadline.expires_at()) => {
                return Err(RunError::new(
                    RunPhase::Await,
                    RunErrorKind::Timeout,
                    "playground was not stopped before the scenario deadline",
                ));
            }
            _ = health.tick() => {
                if let Some(exit) = stack.early_exit() {
                    return Err(process_exit_error(exit, "while the playground was running"));
                }
            }
        }
    }
}

fn process_exit_error(exit: crate::stack::EarlyExit, context: &str) -> RunError {
    RunError::new(
        RunPhase::Await,
        RunErrorKind::ProcessCrash,
        format!("{} exited {context}: {}", exit.name, exit.status),
    )
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
