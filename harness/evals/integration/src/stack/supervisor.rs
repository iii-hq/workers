use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::process::{ProcessSpec, ProcessSupervisor, DEFAULT_TEARDOWN_BUDGET};

use super::config::{write_engine_config, write_seed, ENV_ALLOWLIST, WORKER_START_ORDER};
use super::manifest::write_stack_manifest;
use super::{EarlyExit, RunLayout, StackBins};

pub struct Stack {
    pub ws_url: String,
    pub paths: RunLayout,
    processes: ProcessSupervisor,
    /// Engine spawn recipe, kept for fault-injection respawns:
    /// (binary, args, cwd). `None` only in test stacks.
    engine_recipe: Option<(PathBuf, Vec<String>, PathBuf)>,
    engine_restarts: u32,
}

impl Stack {
    /// Spawn the engine and every pre-harness worker in declared order.
    pub async fn boot(bins: &StackBins, paths: RunLayout) -> anyhow::Result<Stack> {
        match Self::boot_once(bins, &paths).await {
            Ok(stack) => Ok(stack),
            Err(BootError::BindRace) => {
                tracing::warn!(
                    target: "harness_integration::stack",
                    "engine bind race; retrying with a fresh port set"
                );
                Self::boot_once(bins, &paths)
                    .await
                    .map_err(BootError::into_anyhow)
            }
            Err(error) => Err(error.into_anyhow()),
        }
    }

    async fn boot_once(bins: &StackBins, paths: &RunLayout) -> Result<Stack, BootError> {
        let missing = bins.missing_workers();
        if !missing.is_empty() {
            return Err(BootError::Other(anyhow::anyhow!(
                "missing --worker-bin for: {}",
                missing.join(", ")
            )));
        }

        let port = free_loopback_port().map_err(BootError::Other)?;
        let ws_url = format!("ws://127.0.0.1:{port}");
        let engine_yaml_path = write_engine_config(paths, port).map_err(BootError::Other)?;
        write_stack_manifest(bins, paths, port).map_err(BootError::Other)?;

        let engine_args = vec![
            "--config".to_string(),
            engine_yaml_path.to_string_lossy().into_owned(),
            "--no-update-check".to_string(),
        ];
        let mut stack = Stack {
            ws_url,
            paths: paths.clone(),
            processes: ProcessSupervisor::default(),
            engine_recipe: Some((
                bins.engine.clone(),
                engine_args.clone(),
                paths.engine_dir.clone(),
            )),
            engine_restarts: 0,
        };

        stack
            .spawn_child("engine", &bins.engine, &engine_args, &paths.engine_dir)
            .map_err(BootError::Other)?;

        // Classify an immediate engine death as the one retryable bind race.
        tokio::time::sleep(Duration::from_millis(600)).await;
        if let Some(exit) = stack.early_exit() {
            stack.teardown().await;
            if exit.name == "engine" {
                return Err(BootError::BindRace);
            }
            return Err(BootError::Other(anyhow::anyhow!(
                "{} exited during boot: {}",
                exit.name,
                exit.status
            )));
        }

        for worker in WORKER_START_ORDER {
            let bin = bins
                .resolve(worker)
                .expect("presence checked above")
                .to_path_buf();
            stack.spawn_worker(worker, &bin).map_err(BootError::Other)?;
        }

        Ok(stack)
    }

    /// Spawn the harness after Arm so its first registry snapshot includes
    /// the run-scoped target.
    pub fn spawn_harness(&mut self, bins: &StackBins) -> anyhow::Result<()> {
        self.spawn_worker("harness", &bins.harness)
    }

    pub async fn kill_engine(&mut self) -> anyhow::Result<()> {
        let mut engine = self
            .processes
            .remove("engine")
            .ok_or_else(|| anyhow::anyhow!("no live engine child to kill"))?;
        engine.kill_now().await?;
        tracing::info!(
            target: "harness_integration::stack",
            "engine SIGKILLed (fault injection)"
        );
        Ok(())
    }

    pub fn respawn_engine(&mut self) -> anyhow::Result<()> {
        let (bin, args, cwd) = self
            .engine_recipe
            .clone()
            .ok_or_else(|| anyhow::anyhow!("stack has no engine recipe (test stack?)"))?;
        self.engine_restarts += 1;
        let log_name = format!("engine.restart{}", self.engine_restarts);
        self.spawn_child_logged("engine", &log_name, &bin, &args, &cwd)
    }

    fn spawn_worker(&mut self, worker: &str, bin: &Path) -> anyhow::Result<()> {
        let paths = self.paths.clone();
        let mut args = vec!["--url".to_string(), self.ws_url.clone()];
        if let Some(seed_path) = write_seed(worker, &paths)? {
            args.push("--config".to_string());
            args.push(seed_path.to_string_lossy().into_owned());
        }
        self.spawn_child(worker, bin, &args, &paths.root)
    }

    #[doc(hidden)]
    pub fn for_tests(paths: RunLayout) -> Stack {
        Stack {
            ws_url: "ws://127.0.0.1:0".to_string(),
            paths,
            processes: ProcessSupervisor::new(DEFAULT_TEARDOWN_BUDGET),
            engine_recipe: None,
            engine_restarts: 0,
        }
    }

    #[doc(hidden)]
    pub fn for_tests_with_teardown_budget(paths: RunLayout, teardown_budget: Duration) -> Stack {
        let mut stack = Self::for_tests(paths);
        stack.set_teardown_budget(teardown_budget);
        stack
    }

    pub fn set_teardown_budget(&mut self, teardown_budget: Duration) {
        self.processes.set_teardown_budget(teardown_budget);
    }

    pub fn teardown_budget(&self) -> Duration {
        self.processes.teardown_budget()
    }

    #[doc(hidden)]
    pub fn spawn_child(
        &mut self,
        name: &str,
        bin: &Path,
        args: &[String],
        cwd: &Path,
    ) -> anyhow::Result<()> {
        self.spawn_child_logged(name, name, bin, args, cwd)
    }

    fn spawn_child_logged(
        &mut self,
        name: &str,
        log_name: &str,
        bin: &Path,
        args: &[String],
        cwd: &Path,
    ) -> anyhow::Result<()> {
        let stdout_log = self.paths.log_path(log_name, "out")?;
        let stderr_log = self.paths.log_path(log_name, "err")?;
        let mut spec =
            ProcessSpec::new(name, bin, cwd, stdout_log, stderr_log).args(args.iter().cloned());
        for key in ENV_ALLOWLIST {
            if let Ok(value) = std::env::var(key) {
                spec = spec.env(key, value);
            }
        }
        self.processes.spawn(spec).map(|_| ())
    }

    pub fn early_exit(&mut self) -> Option<EarlyExit> {
        self.processes.early_exit()
    }

    pub async fn teardown(&mut self) {
        self.processes.teardown().await;
    }
}

/// A free loopback port, found by bind-and-release. The boot path retries
/// once if another process takes it before the engine binds.
pub fn free_loopback_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

enum BootError {
    BindRace,
    Other(anyhow::Error),
}

impl BootError {
    fn into_anyhow(self) -> anyhow::Error {
        match self {
            BootError::BindRace => anyhow::anyhow!("engine failed to bind twice in a row"),
            BootError::Other(error) => error,
        }
    }
}
