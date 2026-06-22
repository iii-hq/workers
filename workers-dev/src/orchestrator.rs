use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time;

use crate::config::{Config, DASHBOARD_LOG_LINES};
use crate::discover::SpawnKind;
use crate::graph::WorkerGraph;
use crate::logs;
use crate::runtime::{ProcState, SharedRuntimes, WorkerRuntime};
use crate::status::{self, EngineWorker, WorkerView};

pub struct Orchestrator {
    pub config: Config,
    pub graph: WorkerGraph,
    pub runtimes: SharedRuntimes,
}

impl Orchestrator {
    pub fn new(config: Config) -> Result<Self> {
        let graph = WorkerGraph::load(&config.repo_root, &config.workers)?;
        let runtimes = crate::runtime::new_runtimes(&config.workers);
        Ok(Self {
            config,
            graph,
            runtimes,
        })
    }

    pub async fn engine_preflight(&self) -> Result<()> {
        status::fetch_engine_workers(&self.config)
            .await
            .with_context(|| {
                format!(
                    "engine not reachable at {} (start with: iii -c harness/engine.config.yaml)",
                    self.config.engine_url
                )
            })?;
        Ok(())
    }

    pub async fn start_harness_stack(&self, wait_connected: bool) -> Result<()> {
        self.start_workers(&self.config.harness_stack, wait_connected)
            .await
    }

    pub async fn start_all_managed(&self, wait_connected: bool) -> Result<()> {
        self.start_workers(&self.config.workers, wait_connected)
            .await
    }

    pub async fn start_workers(&self, names: &[String], wait_connected: bool) -> Result<()> {
        self.engine_preflight().await?;
        let order = if names.is_empty() {
            self.graph.topo_start_order(self.graph.workers())?
        } else {
            self.graph.closure_with_deps(names)?
        };

        for worker in order {
            self.start_one(&worker, wait_connected).await?;
        }
        Ok(())
    }

    pub async fn stop_workers(&self, names: &[String]) -> Result<()> {
        let order = if names.is_empty() {
            self.graph.topo_stop_order(self.graph.workers())?
        } else {
            self.graph.topo_stop_order(names)?
        };

        for worker in order {
            self.stop_one(&worker).await;
        }
        Ok(())
    }

    pub async fn restart_worker(&self, name: &str) -> Result<()> {
        let closure = self.graph.restart_closure(name)?;
        let stop_order = self.graph.topo_stop_order(&closure)?;
        for worker in &stop_order {
            self.stop_one(worker).await;
        }
        for worker in &closure {
            self.start_one(worker, true).await?;
        }
        Ok(())
    }

    async fn start_one(&self, name: &str, wait_connected: bool) -> Result<()> {
        self.stop_one(name).await;

        let spec = self
            .config
            .worker_spec(name)
            .with_context(|| format!("unknown worker {name}"))?;
        match &spec.spawn {
            SpawnKind::CargoRun => {}
            SpawnKind::Unsupported { reason } => {
                bail!("worker {name} cannot be started from workers-dev: {reason}");
            }
        }

        let worker_dir = &spec.dir;
        if !worker_dir.is_dir() {
            bail!("worker directory not found: {}", worker_dir.display());
        }

        let mut cmd = Command::new("cargo");
        cmd.arg("run");
        if self.config.release {
            cmd.arg("--release");
        }
        cmd.args(["--", "--url", &self.config.engine_url]);
        cmd.current_dir(worker_dir);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);
        cmd.env("CARGO_TERM_COLOR", "never");
        cmd.env("CARGO_TERM_PROGRESS", "never");
        cmd.env("CLICOLOR_FORCE", "0");

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn cargo run for {name}"))?;

        let stdout = child.stdout.take().context("stdout pipe")?;
        let stderr = child.stderr.take().context("stderr pipe")?;

        {
            let mut runtimes = self.runtimes.write().await;
            let rt = runtimes.get_mut(name).context("unknown worker runtime")?;
            rt.proc_state = ProcState::Compiling;
            rt.exit_code = None;
            rt.started_at = Some(Instant::now());
            rt.set_child(child);
        }

        let name_stdout = name.to_string();
        let name_stderr = name.to_string();
        let runtimes_stdout = self.runtimes.clone();
        let runtimes_stderr = self.runtimes.clone();
        tokio::spawn(async move {
            read_stream(name_stdout, stdout, runtimes_stdout).await;
        });
        tokio::spawn(async move {
            read_stream(name_stderr, stderr, runtimes_stderr).await;
        });

        let name_wait = name.to_string();
        let runtimes_wait = self.runtimes.clone();
        tokio::spawn(async move {
            wait_for_exit(&name_wait, runtimes_wait).await;
        });

        if wait_connected {
            self.wait_connected(name).await?;
        }

        Ok(())
    }

    async fn stop_one(&self, name: &str) {
        let child = {
            let mut runtimes = self.runtimes.write().await;
            let Some(rt) = runtimes.get_mut(name) else {
                return;
            };
            rt.take_child()
        };

        if let Some(mut child) = child {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }

        let mut runtimes = self.runtimes.write().await;
        if let Some(rt) = runtimes.get_mut(name) {
            if rt.proc_state != ProcState::Crashed {
                rt.proc_state = ProcState::Stopped;
            }
            rt.clear_child();
        }
    }

    pub async fn wait_connected(&self, name: &str) -> Result<()> {
        let deadline =
            Instant::now() + Duration::from_millis(self.config.connect_timeout_ms);
        loop {
            if Instant::now() >= deadline {
                bail!("timed out waiting for {name} to connect to engine");
            }
            let engine = status::fetch_engine_workers(&self.config).await.unwrap_or_default();
            if engine.iter().any(|w| w.name.as_deref() == Some(name) && w.status == "connected") {
                return Ok(());
            }
            let proc = self.proc_state(name).await;
            if proc == ProcState::Crashed || proc == ProcState::Stopped {
                bail!("worker {name} exited before connecting to engine");
            }
            time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn proc_state(&self, name: &str) -> ProcState {
        let runtimes = self.runtimes.read().await;
        runtimes
            .get(name)
            .map(|rt| rt.proc_state)
            .unwrap_or(ProcState::Stopped)
    }

    pub async fn worker_views(&self) -> Result<Vec<WorkerView>> {
        let engine = status::fetch_engine_workers(&self.config)
            .await
            .unwrap_or_default();
        let engine_by_name: HashMap<String, EngineWorker> = engine
            .into_iter()
            .filter_map(|w| w.name.clone().map(|name| (name, w)))
            .collect();

        let runtimes = self.runtimes.read().await;
        let mut views = Vec::new();
        for worker in self.graph.workers() {
            let rt = runtimes.get(worker).expect("runtime");
            let spec = self.config.worker_spec(worker).expect("spec");
            views.push(build_view(spec, rt, engine_by_name.get(worker)));
        }
        Ok(views)
    }

    pub async fn logs_tail(&self, name: &str, n: usize) -> Result<Vec<String>> {
        let runtimes = self.runtimes.read().await;
        let rt = runtimes.get(name).context("unknown worker")?;
        Ok(rt.logs.tail(n))
    }

    pub async fn subscribe_logs(&self, name: &str) -> Result<tokio::sync::broadcast::Receiver<String>> {
        let runtimes = self.runtimes.read().await;
        let rt = runtimes.get(name).context("unknown worker")?;
        Ok(rt.subscribe_logs())
    }
}

fn build_view(
    spec: &crate::discover::WorkerSpec,
    rt: &WorkerRuntime,
    engine: Option<&EngineWorker>,
) -> WorkerView {
    let process = rt.proc_state.label().to_string();
    let (engine_status, uptime) = if let Some(w) = engine {
        (
            w.status.clone(),
            format_uptime(w.connected_at_ms),
        )
    } else {
        ("—".to_string(), "—".to_string())
    };

    let display = if engine.map(|w| w.status.as_str()) == Some("connected") {
        "connected"
    } else if rt.proc_state == ProcState::Crashed {
        "crashed"
    } else if rt.proc_state == ProcState::Compiling {
        "compiling"
    } else if rt.proc_state == ProcState::Running {
        "disconnected"
    } else {
        "stopped"
    };

    WorkerView {
        name: spec.name.clone(),
        group: spec.group,
        spawnable: matches!(spec.spawn, SpawnKind::CargoRun),
        display_status: display.to_string(),
        process_status: process,
        engine_status,
        local_pid: rt.pid(),
        engine_pid: None,
        uptime,
        last_logs: rt.logs.tail(DASHBOARD_LOG_LINES),
    }
}

fn format_uptime(connected_at_ms: u64) -> String {
    if connected_at_ms == 0 {
        return "—".to_string();
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let secs = now_ms.saturating_sub(connected_at_ms) / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

async fn read_stream(
    worker: String,
    stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    runtimes: SharedRuntimes,
) {
    let mut lines = BufReader::new(stream).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let normalized = logs::normalize_log_line(&line);
                if normalized.is_empty() {
                    continue;
                }
                let mut runtimes = runtimes.write().await;
                if let Some(rt) = runtimes.get_mut(&worker) {
                    rt.logs.push(normalized.clone());
                    update_proc_state_from_line(rt, &normalized);
                    let _ = rt.log_tx.send(normalized);
                }
            }
            Ok(None) => break,
            Err(err) => {
                let mut runtimes = runtimes.write().await;
                if let Some(rt) = runtimes.get_mut(&worker) {
                    let msg = format!("log read error: {err}");
                    rt.logs.push(msg.clone());
                    let _ = rt.log_tx.send(msg);
                }
                break;
            }
        }
    }
}

fn update_proc_state_from_line(rt: &mut WorkerRuntime, line: &str) {
    if line.contains("Compiling")
        || line.contains("Building")
        || line.contains("Downloading")
        || line.contains("Updating")
    {
        rt.proc_state = ProcState::Compiling;
    } else if line.contains("Finished")
        || line.contains("Running")
        || line.contains("INFO")
        || line.contains("registered")
    {
        if rt.proc_state != ProcState::Crashed {
            rt.proc_state = ProcState::Running;
        }
    } else if line.contains("error:")
        || line.contains("error[E")
        || line.contains("could not compile")
    {
        rt.proc_state = ProcState::Compiling;
    }
}

async fn wait_for_exit(worker: &str, runtimes: SharedRuntimes) {
    loop {
        time::sleep(Duration::from_millis(250)).await;

        let status = {
            let mut guard = runtimes.write().await;
            let Some(rt) = guard.get_mut(worker) else {
                return;
            };
            let Some(child) = rt.child_mut() else {
                return;
            };
            match child.try_wait() {
                Ok(Some(status)) => Some(status),
                Ok(None) => None,
                Err(err) => {
                    rt.logs.push(format!("process wait error: {err}"));
                    rt.proc_state = ProcState::Crashed;
                    rt.clear_child();
                    return;
                }
            }
        };

        let Some(status) = status else {
            continue;
        };

        let mut guard = runtimes.write().await;
        if let Some(rt) = guard.get_mut(worker) {
            rt.clear_child();
            rt.exit_code = status.code();
            rt.proc_state = if status.success() {
                ProcState::Stopped
            } else {
                let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".into());
                rt.logs
                    .push(format!("process exited with status {code}"));
                ProcState::Crashed
            };
        }
        return;
    }
}
