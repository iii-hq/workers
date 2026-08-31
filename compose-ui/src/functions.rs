use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::logs::{read_log_tail, LogsInput};
use crate::project::{listening_ports, read_project, ProjectContainer, ProjectResult};
use crate::watch::{compose_status, locate, StateWatcher};

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ProjectInput {
    /// Compose file on the daemon host; defaults to the daemon project.
    pub file: Option<String>,
}

fn handler_error(prefix: &str, error: impl std::fmt::Display) -> Error {
    Error::Handler(format!("{prefix}: {error}"))
}

pub fn register(iii: &Arc<IIIClient>, watcher: StateWatcher) {
    {
        let iii = iii.clone();
        let watcher = watcher.clone();
        iii.clone().register_function(
            "compose-ui::logs",
            RegisterFunction::new_async(move |input: LogsInput| {
                let iii = iii.clone();
                let watcher = watcher.clone();
                async move {
                    let location = match input.file.as_deref() {
                        Some(file) => locate(&iii, Some(file)).await?,
                        None => watcher.ensure().await?,
                    }
                    .ok_or_else(|| {
                        Error::Handler(
                            "COMPOSE_UNAVAILABLE: compose::status answered without a state directory"
                                .to_string(),
                        )
                    })?;
                    read_log_tail(&location.state_dir, &input.container, input.lines)
                        .await
                        .map_err(|error| handler_error("LOG_TAIL_ERROR", error))
                }
            })
            .description(
                "Last lines of one compose container's log from the daemon state directory (default 200, at most 500; a missing log answers with missing: true).",
            ),
        );
    }

    {
        let iii = iii.clone();
        iii.clone().register_function(
            "compose-ui::project",
            RegisterFunction::new_async(move |input: ProjectInput| {
                let iii = iii.clone();
                async move { project(&iii, input).await }
            })
            .description(
                "The compose file as declared plus what each container listens on: namespace, engine endpoint, timeouts, worker source, dependencies, environment keys, run script, PID, and listening TCP ports.",
            ),
        );
    }
}

async fn project(iii: &IIIClient, input: ProjectInput) -> Result<ProjectResult, Error> {
    let status = compose_status(iii, input.file.as_deref()).await?;
    let file = status.file.ok_or_else(|| {
        Error::Handler(
            "COMPOSE_UNAVAILABLE: compose::status answered without a compose file".to_string(),
        )
    })?;
    let declaration = read_project(Path::new(&file))
        .await
        .map_err(|error| handler_error("PROJECT_READ_ERROR", error))?;

    let pids: HashMap<String, u32> = status
        .containers
        .into_iter()
        .filter_map(|container| {
            let running = matches!(container.state.as_deref(), Some("ready" | "starting"));
            running
                .then_some(container.pid)
                .flatten()
                .map(|pid| (container.container, pid))
        })
        .collect();
    let mut inspected_pids: Vec<u32> = pids.values().copied().collect();
    if let Some(daemon_pid) = status.daemon_pid {
        inspected_pids.push(daemon_pid);
    }
    let mut ports = listening_ports(&inspected_pids).await;
    let daemon_ports = status
        .daemon_pid
        .and_then(|pid| ports.remove(&pid))
        .unwrap_or_default();
    let containers = declaration
        .containers
        .into_iter()
        .map(|container| {
            let pid = pids.get(&container.name).copied();
            let listening = pid.and_then(|pid| ports.remove(&pid)).unwrap_or_default();
            ProjectContainer::from_declared(container, pid, listening)
        })
        .collect();

    Ok(ProjectResult {
        file: declaration.file,
        namespace: declaration.namespace,
        engine_url: declaration.engine_url,
        engine_host: declaration.engine_host,
        engine_port: declaration.engine_port,
        startup_timeout: declaration.startup_timeout,
        stop_timeout: declaration.stop_timeout,
        daemon_pid: status.daemon_pid,
        daemon_ports,
        containers,
    })
}
