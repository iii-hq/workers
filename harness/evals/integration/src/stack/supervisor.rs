use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::process::{ProcessSpec, ProcessSupervisor, TeardownReport, DEFAULT_TEARDOWN_BUDGET};

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

#[derive(Debug)]
pub struct StackBootFailure {
    pub error: anyhow::Error,
    pub teardown: TeardownReport,
}

impl Stack {
    /// Spawn the engine and every pre-harness worker in declared order.
    pub async fn boot(bins: &StackBins, paths: RunLayout) -> Result<Stack, StackBootFailure> {
        match Self::boot_once(bins, &paths).await {
            Ok(stack) => Ok(stack),
            Err(BootError::BindRace(teardown)) if teardown.complete() => {
                tracing::warn!(
                    target: "harness_integration::stack",
                    "engine bind race; retrying with a fresh port set"
                );
                Self::boot_once(bins, &paths)
                    .await
                    .map_err(BootError::into_failure)
            }
            Err(BootError::BindRace(teardown)) => Err(StackBootFailure {
                error: anyhow::anyhow!("engine bind race cleanup was incomplete"),
                teardown,
            }),
            Err(error) => Err(error.into_failure()),
        }
    }

    async fn boot_once(bins: &StackBins, paths: &RunLayout) -> Result<Stack, BootError> {
        let missing = bins.missing_workers();
        if !missing.is_empty() {
            return Err(BootError::without_processes(anyhow::anyhow!(
                "missing --worker-bin for: {}",
                missing.join(", ")
            )));
        }

        let port = free_loopback_port().map_err(BootError::without_processes)?;
        let ws_url = format!("ws://127.0.0.1:{port}");
        let engine_yaml_path =
            write_engine_config(paths, port).map_err(BootError::without_processes)?;
        write_stack_manifest(bins, paths, port).map_err(BootError::without_processes)?;

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

        if let Err(error) =
            stack.spawn_child("engine", &bins.engine, &engine_args, &paths.engine_dir)
        {
            let teardown = stack.teardown().await;
            return Err(BootError::Other { error, teardown });
        }

        // Retry only an explicit address-in-use failure. Other immediate
        // exits are configuration/binary failures, not port collisions.
        tokio::time::sleep(Duration::from_millis(600)).await;
        if let Some(exit) = stack.early_exit() {
            let retryable_bind_race =
                exit.name == "engine" && stderr_indicates_bind_race(&exit.stderr_log);
            let teardown = stack.teardown().await;
            if retryable_bind_race {
                return Err(BootError::BindRace(teardown));
            }
            return Err(BootError::Other {
                error: anyhow::anyhow!("{} exited during boot: {}", exit.name, exit.status),
                teardown,
            });
        }

        for worker in WORKER_START_ORDER {
            let bin = bins
                .resolve(worker)
                .expect("presence checked above")
                .to_path_buf();
            if let Err(error) = stack.spawn_worker(worker, &bin) {
                let teardown = stack.teardown().await;
                return Err(BootError::Other { error, teardown });
            }
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

    pub async fn teardown(&mut self) -> TeardownReport {
        self.processes.teardown().await
    }
}

/// A free loopback port, found by bind-and-release. The boot path retries
/// once if another process takes it before the engine binds.
pub fn free_loopback_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

enum BootError {
    BindRace(TeardownReport),
    Other {
        error: anyhow::Error,
        teardown: TeardownReport,
    },
}

impl BootError {
    fn without_processes(error: anyhow::Error) -> Self {
        Self::Other {
            error,
            teardown: TeardownReport::default(),
        }
    }

    fn into_failure(self) -> StackBootFailure {
        match self {
            BootError::BindRace(teardown) => StackBootFailure {
                error: anyhow::anyhow!("engine failed to bind twice in a row"),
                teardown,
            },
            BootError::Other { error, teardown } => StackBootFailure { error, teardown },
        }
    }
}

fn stderr_indicates_bind_race(path: &Path) -> bool {
    let Ok(stderr) = std::fs::read_to_string(path) else {
        return false;
    };
    let stderr = stderr.to_ascii_lowercase();
    [
        "address already in use",
        "addrinuse",
        "os error 48",
        "os error 98",
        "failed to bind",
    ]
    .iter()
    .any(|needle| stderr.contains(needle))
}
