use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time;

use crate::config::Config;
use crate::discover::SpawnKind;
use crate::graph::WorkerGraph;
use crate::logs;
use crate::runtime::{ProcState, SharedRuntimes, WorkerRuntime};
use crate::status::{self, EngineWorker, WorkerView};

pub struct Orchestrator {
    pub config: Config,
    pub graph: WorkerGraph,
    pub runtimes: SharedRuntimes,
    /// When true, CLI start/restart paths stream per-worker progress to stderr.
    /// Off for the TUI, which owns the screen and shows status in its own panes.
    pub progress: bool,
    /// One persistent engine connection, lazily opened on first status query
    /// and reused for every poll. Replaces the per-poll `iii trigger`
    /// subprocess that made the engine log a register/unregister pair every
    /// tick.
    engine_client: tokio::sync::OnceCell<IIIClient>,
}

impl Orchestrator {
    pub fn new(config: Config, progress: bool) -> Result<Self> {
        let graph = WorkerGraph::load(&config.repo_root, &config.workers)?;
        let runtimes = crate::runtime::new_runtimes(&config.workers);
        Ok(Self {
            config,
            graph,
            runtimes,
            progress,
            engine_client: tokio::sync::OnceCell::new(),
        })
    }

    /// Lazily open (and cache) the shared engine connection. Connects only
    /// when status is actually queried, and waits briefly for the background
    /// handshake so the first query doesn't race it.
    async fn engine_client(&self) -> &IIIClient {
        self.engine_client
            .get_or_init(|| async {
                // Label the connection so it shows as `workers-dev` in engine
                // logs / `engine::workers::list` instead of a bare hostname.
                let client = register_worker(
                    &self.config.engine_url,
                    InitOptions {
                        metadata: Some(iii_sdk::runtime::WorkerMetadata {
                            name: "workers-dev".to_string(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                );
                let deadline = Instant::now() + Duration::from_millis(2000);
                while Instant::now() < deadline
                    && client.get_connection_state()
                        != iii_sdk::runtime::IIIConnectionState::Connected
                {
                    time::sleep(Duration::from_millis(50)).await;
                }
                client
            })
            .await
    }

    /// Fetch connected workers from the engine over the shared connection.
    pub async fn engine_workers(&self) -> Result<Vec<EngineWorker>> {
        let client = self.engine_client().await;
        status::fetch_engine_workers(client, crate::config::ENGINE_QUERY_TIMEOUT_MS).await
    }

    /// Cleanly close the engine connection so the engine logs a prompt
    /// unregister instead of waiting for the socket to time out.
    pub async fn shutdown(&self) {
        if let Some(client) = self.engine_client.get() {
            client.shutdown_async().await;
        }
    }

    /// Fast TCP probe of the engine's WebSocket port. Cheaper than a full
    /// trigger round-trip, so the engine-boot wait loop can poll quickly.
    async fn engine_reachable(&self) -> bool {
        let Ok((host, port)) = crate::config::parse_engine_url(&self.config.engine_url, None)
        else {
            return false;
        };
        matches!(
            tokio::time::timeout(
                Duration::from_millis(500),
                tokio::net::TcpStream::connect((host.as_str(), port)),
            )
            .await,
            Ok(Ok(_))
        )
    }

    /// Ensure the engine is running, starting `iii -c harness/engine.config.yaml`
    /// if it isn't. The engine is detached (own process group, no kill-on-drop)
    /// so it outlives the dashboard, like the workers it hosts; then we wait for
    /// it to accept connections.
    pub async fn ensure_engine(&self) -> Result<()> {
        if self.engine_reachable().await {
            return Ok(());
        }

        let config_rel = "harness/engine.config.yaml";
        let config_path = self.config.repo_root.join(config_rel);
        if !config_path.is_file() {
            bail!(
                "engine not reachable at {} and {} not found — start it manually: iii -c {config_rel}",
                self.config.engine_url,
                config_path.display()
            );
        }

        eprintln!(
            "engine not reachable at {} — starting `iii -c {config_rel}`…",
            self.config.engine_url
        );

        let mut cmd = Command::new("iii");
        cmd.arg("-c").arg(config_rel);
        cmd.current_dir(&self.config.repo_root);
        // Detach: the engine should outlive the dashboard. Suppress its output
        // so it can't corrupt the TUI's alternate screen; run it manually if
        // you need to watch engine logs.
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        #[cfg(unix)]
        cmd.process_group(0);
        cmd.spawn().context("spawn iii engine")?;

        let deadline = Instant::now() + Duration::from_millis(self.config.connect_timeout_ms);
        while Instant::now() < deadline {
            time::sleep(Duration::from_millis(200)).await;
            if self.engine_reachable().await {
                eprintln!("engine is up at {}", self.config.engine_url);
                return Ok(());
            }
        }
        bail!(
            "started the engine but it did not become reachable within {}ms",
            self.config.connect_timeout_ms
        );
    }

    pub async fn engine_preflight(&self) -> Result<()> {
        self.engine_workers().await.with_context(|| {
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
        // Run the worker in its own process group so stopping it can signal the
        // whole group. `cargo run` forks the worker binary as a child; killing
        // only cargo orphans the worker (it keeps running, still connected to
        // the engine). See `terminate_process_group`.
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn cargo run for {name}"))?;

        if self.progress {
            eprintln!("▶ {name}: starting (cargo run)…");
        }

        let stdout = child.stdout.take().context("stdout pipe")?;
        let stderr = child.stderr.take().context("stderr pipe")?;

        let expected_pid = child.id();

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
            wait_for_exit(&name_wait, expected_pid, runtimes_wait).await;
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
            terminate_process_group(&mut child).await;
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
        let deadline = Instant::now() + Duration::from_millis(self.config.connect_timeout_ms);
        let mut last_engine_err: Option<anyhow::Error> = None;
        loop {
            if Instant::now() >= deadline {
                match last_engine_err {
                    Some(err) => bail!(
                        "timed out waiting for {name} to connect to engine \
                         (engine query failing: {err:#})"
                    ),
                    None => bail!("timed out waiting for {name} to connect to engine"),
                }
            }
            let engine = match self.engine_workers().await {
                Ok(engine) => {
                    last_engine_err = None;
                    engine
                }
                Err(err) => {
                    last_engine_err = Some(err);
                    Vec::new()
                }
            };
            if engine
                .iter()
                .any(|w| w.name.as_deref() == Some(name) && w.status == "connected")
            {
                if self.progress {
                    let elapsed = {
                        let runtimes = self.runtimes.read().await;
                        runtimes
                            .get(name)
                            .and_then(|rt| rt.started_at)
                            .map(|t| t.elapsed())
                    };
                    match elapsed {
                        Some(d) => eprintln!("✓ {name} connected ({:.1}s)", d.as_secs_f64()),
                        None => eprintln!("✓ {name} connected"),
                    }
                }
                return Ok(());
            }
            let proc = self.proc_state(name).await;
            if proc == ProcState::Crashed || proc == ProcState::Stopped {
                let exit_code = {
                    let runtimes = self.runtimes.read().await;
                    runtimes.get(name).and_then(|rt| rt.exit_code)
                };
                match exit_code {
                    Some(code) => bail!(
                        "worker {name} exited (status {code}) before connecting to engine — \
                         check logs: workers-dev logs {name}"
                    ),
                    None => bail!(
                        "worker {name} exited before connecting to engine — \
                         check logs: workers-dev logs {name}"
                    ),
                }
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
        Ok(self.dashboard_snapshot().await.0)
    }

    /// Like `worker_views`, but also reports an engine-query error (if any) so
    /// the dashboard can show "engine unreachable" instead of silently blanking
    /// every engine status. Never fails: on engine error the views still render
    /// (all disconnected) alongside the error string.
    pub async fn dashboard_snapshot(&self) -> (Vec<WorkerView>, Option<String>) {
        let (engine, engine_error) = match self.engine_workers().await {
            Ok(list) => (list, None),
            Err(err) => (Vec::new(), Some(format!("{err:#}"))),
        };
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
        (views, engine_error)
    }

    pub async fn logs_tail(&self, name: &str, n: usize) -> Result<Vec<String>> {
        let runtimes = self.runtimes.read().await;
        let rt = runtimes.get(name).context("unknown worker")?;
        Ok(rt.logs.tail(n))
    }

    pub async fn subscribe_logs(
        &self,
        name: &str,
    ) -> Result<tokio::sync::broadcast::Receiver<String>> {
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
        (w.status.clone(), format_uptime(w.connected_at_ms))
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
        uptime,
        exit_code: rt.exit_code,
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

async fn wait_for_exit(worker: &str, expected_pid: Option<u32>, runtimes: SharedRuntimes) {
    loop {
        time::sleep(Duration::from_millis(250)).await;

        let status = {
            let mut guard = runtimes.write().await;
            let Some(rt) = guard.get_mut(worker) else {
                return;
            };
            if rt.pid() != expected_pid {
                return;
            }
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
                let code = status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into());
                rt.logs.push(format!("process exited with status {code}"));
                ProcState::Crashed
            };
        }
        return;
    }
}

/// Stop a worker and the process group it leads. `cargo run` forks the worker
/// binary as a child; SIGKILLing only cargo would orphan the worker (it keeps
/// running and connected to the engine). Workers are spawned in their own
/// process group (see `start_one`), so signal the whole group: SIGTERM for a
/// graceful stop, then SIGKILL if it hasn't exited within ~1s.
#[cfg(unix)]
async fn terminate_process_group(child: &mut tokio::process::Child) {
    let Some(pid) = child.id() else {
        let _ = child.start_kill();
        return;
    };
    // The child is its own group leader, so its pid is the pgid; the negative
    // pid targets the whole group.
    let pgid = pid as i32;
    unsafe {
        libc::kill(-pgid, libc::SIGTERM);
    }
    for _ in 0..20 {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => time::sleep(Duration::from_millis(50)).await,
            Err(_) => break,
        }
    }
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
async fn terminate_process_group(child: &mut tokio::process::Child) {
    let _ = child.start_kill();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Stdio;

    /// A worker started via `cargo run` forks the real worker binary as a
    /// child. Group-killing must take BOTH down — killing only the supervisor
    /// orphans the worker. Simulate with a parent shell that backgrounds a long
    /// sleep (the "worker") in its own process group, then assert the
    /// backgrounded pid is dead after `terminate_process_group`.
    #[tokio::test]
    async fn terminate_process_group_kills_forked_child() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("sleep 300 & echo $! ; wait");
        cmd.stdout(Stdio::piped());
        cmd.process_group(0);
        let mut child = cmd.spawn().expect("spawn sh");

        let stdout = child.stdout.take().expect("stdout");
        let mut reader = BufReader::new(stdout).lines();
        let line = reader
            .next_line()
            .await
            .expect("read pid line")
            .expect("pid line present");
        let worker_pid: i32 = line.trim().parse().expect("parse worker pid");

        // The backgrounded "worker" is alive before we stop the supervisor.
        assert_eq!(
            unsafe { libc::kill(worker_pid, 0) },
            0,
            "worker should be running"
        );

        terminate_process_group(&mut child).await;
        let _ = child.wait().await;

        let mut dead = false;
        for _ in 0..40 {
            if unsafe { libc::kill(worker_pid, 0) } != 0 {
                dead = true;
                break;
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        assert!(dead, "forked worker {worker_pid} was orphaned, not killed");
    }
}
