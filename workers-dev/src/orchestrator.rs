use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use iii_sdk::{register_worker, IIIClient, InitOptions};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time;

use crate::config::Config;
use crate::discover::{SpawnKind, WorkerGroup};
use crate::graph::WorkerGraph;
use crate::logs;
use crate::runtime::{ProcState, SharedRuntimes, WorkerRuntime};
use crate::status::{self, EngineWorker, WorkerView};

/// Engine config path, relative to the repo root. `ensure_engine` launches
/// `iii -c <this>`; the TUI's engine-down banner names it so the remedy shown
/// matches what `e` actually runs.
pub const ENGINE_CONFIG_REL: &str = "harness/engine.config.yaml";

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
        let runtimes = crate::runtime::new_runtimes(&config.workers, |name| {
            config.ui_watch && config.worker_spec(name).is_some_and(|s| s.ui_dir.is_some())
        });
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
    /// it to accept connections. Streams progress to stderr — call only before
    /// the TUI takes the screen.
    pub async fn ensure_engine(&self) -> Result<()> {
        self.ensure_engine_inner(true, self.config.connect_timeout_ms)
            .await
    }

    /// `ensure_engine` for the TUI's `e` key: silent (stderr would corrupt the
    /// alt-screen) and with a short reachability wait — once the spawn worked,
    /// the dashboard poller shows the engine coming up on its own.
    pub async fn start_engine_quiet(&self, wait_ms: u64) -> Result<()> {
        self.ensure_engine_inner(false, wait_ms).await
    }

    async fn ensure_engine_inner(&self, verbose: bool, wait_ms: u64) -> Result<()> {
        if self.engine_reachable().await {
            return Ok(());
        }

        let config_rel = ENGINE_CONFIG_REL;
        let config_path = self.config.repo_root.join(config_rel);
        if !config_path.is_file() {
            bail!(
                "engine not reachable at {} and {} not found — start it manually: iii -c {config_rel}",
                self.config.engine_url,
                config_path.display()
            );
        }

        if verbose {
            eprintln!(
                "engine not reachable at {} — starting `iii -c {config_rel}`…",
                self.config.engine_url
            );
        }

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
        let mut child = cmd.spawn().context("spawn iii engine")?;

        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        while Instant::now() < deadline {
            time::sleep(Duration::from_millis(200)).await;
            // `iii` dying (e.g. a config error) with its stderr nulled would
            // otherwise mean spinning out the whole wait to report a vague
            // timeout — surface the exit immediately instead.
            if let Ok(Some(status)) = child.try_wait() {
                bail!(
                    "iii exited immediately ({status}) — run `iii -c {config_rel}` manually to see its output"
                );
            }
            if self.engine_reachable().await {
                if verbose {
                    eprintln!("engine is up at {}", self.config.engine_url);
                }
                return Ok(());
            }
        }
        // Dropping `child` leaves the engine running (no kill-on-drop): it may
        // still come up late; under the TUI the poller will show it if so.
        let hint = if verbose {
            ""
        } else {
            " (it keeps trying in the background — watch the header)"
        };
        bail!("started the engine but it was not reachable within {wait_ms}ms{hint}");
    }

    /// Engine preflight that also returns which workers the engine currently
    /// reports as connected, so start paths can leave healthy dependencies
    /// alone. Errors carry the start-the-engine remedy.
    async fn connected_worker_names(&self) -> Result<HashSet<String>> {
        let workers = self.engine_workers().await.with_context(|| {
            format!(
                "engine not reachable at {} (start with: iii -c {ENGINE_CONFIG_REL})",
                self.config.engine_url
            )
        })?;
        Ok(workers
            .into_iter()
            .filter(|w| w.status == "connected")
            .filter_map(|w| w.name)
            .collect())
    }

    /// True when `worker` is only in the start set as a pulled-in dependency
    /// (not asked for by name) and the engine already has it connected.
    /// Recompiling and restarting such a worker would only disrupt a healthy
    /// process — or spawn a duplicate of one started outside workers-dev.
    fn skip_connected_dep(names: &[String], worker: &str, connected: &HashSet<String>) -> bool {
        !names.iter().any(|n| n == worker) && connected.contains(worker)
    }

    /// Start a configured stack by name.
    pub async fn start_stack(&self, name: &str, wait_connected: bool) -> Result<()> {
        let roots = self
            .config
            .stacks
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, roots)| roots.clone())
            .with_context(|| format!("unknown stack {name}"))?;
        self.start_roots(name, &roots, wait_connected).await
    }

    /// Start a stack given its roots directly — the path the TUI uses, since
    /// stacks created mid-session are not in `config.stacks`. Roots are the
    /// requested set (always (re)started); missing deps are pulled in, and
    /// deps already connected to the engine are left alone.
    pub async fn start_roots(
        &self,
        stack: &str,
        roots: &[String],
        wait_connected: bool,
    ) -> Result<()> {
        // Empty roots mean every name the stack listed was filtered out of
        // the managed `workers:` set (config.rs warns and drops). Refuse
        // instead of falling through to `start_workers`' "no names" meaning
        // — "start every managed worker" — which an empty stack must never
        // trigger.
        if roots.is_empty() {
            bail!("stack {stack} has no startable workers");
        }
        self.start_workers(roots, wait_connected).await
    }

    /// Member set (roots + transitive deps) of a configured stack.
    pub fn stack_members(&self, name: &str) -> Result<HashSet<String>> {
        let roots = self
            .config
            .stacks
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, roots)| roots.clone())
            .with_context(|| format!("unknown stack {name}"))?;
        Ok(crate::discover::stack_members(
            &self.config.worker_specs,
            &roots,
        ))
    }

    pub async fn start_all_managed(&self, wait_connected: bool) -> Result<()> {
        // "All" means all spawnable workers; external ones (non-Rust, managed
        // via `iii worker add`) are not an error here.
        let spawnable: Vec<String> = self
            .config
            .workers
            .iter()
            .filter(|w| self.unsupported_reason(w).is_none())
            .cloned()
            .collect();
        self.start_workers(&spawnable, wait_connected).await
    }

    /// `Some(reason)` when workers-dev cannot spawn this worker locally.
    fn unsupported_reason(&self, name: &str) -> Option<&str> {
        match &self.config.worker_spec(name)?.spawn {
            SpawnKind::Unsupported { reason } => Some(reason),
            SpawnKind::CargoRun => None,
        }
    }

    pub async fn start_workers(&self, names: &[String], wait_connected: bool) -> Result<()> {
        // Doubles as the engine preflight; bails with the remedy when down.
        let connected = self.connected_worker_names().await?;
        let order = if names.is_empty() {
            self.graph.topo_start_order(self.graph.workers())?
        } else {
            self.graph.closure_with_deps(names)?
        };

        for worker in order {
            // A dependency that's already connected to the engine is doing its
            // job — don't recompile/restart it. Only workers asked for by name
            // always (re)start.
            if Self::skip_connected_dep(names, &worker, &connected) {
                if self.progress {
                    eprintln!("↷ {worker}: already connected to the engine, left running");
                }
                continue;
            }
            // Non-spawnable workers pulled in as dependencies run externally
            // (`iii worker add`); skip them. Bail only when the user asked for
            // that worker by name.
            if let Some(reason) = self.unsupported_reason(&worker) {
                if names.contains(&worker) {
                    bail!("worker {worker} cannot be started from workers-dev: {reason}");
                }
                if self.progress {
                    eprintln!("↷ {worker}: external, not started ({reason})");
                }
                continue;
            }
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
        if let Some(reason) = self.unsupported_reason(name) {
            bail!("worker {name} cannot be started from workers-dev: {reason}");
        }
        let closure = self.graph.restart_closure(name)?;
        let stop_order = self.graph.topo_stop_order(&closure)?;
        for worker in &stop_order {
            self.stop_one(worker).await;
        }
        for worker in &closure {
            if self.unsupported_reason(worker).is_some() {
                continue; // external dependent; not ours to restart
            }
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

        // Injectable-UI dev loop (docs/sops/injectable-console-ui.md): arm the
        // worker's iii-console-ui watcher via its env var and run the esbuild
        // rebuild (`pnpm watch`) alongside. Effective only when the worker
        // ships a ui/ project and its runtime flag is on.
        let ui_dir = spec.ui_dir.clone();
        let ui_watch = ui_dir.is_some() && {
            let runtimes = self.runtimes.read().await;
            runtimes.get(name).is_some_and(|rt| rt.ui_watch)
        };

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
        if ui_watch {
            // `1` = the crate's default ui/dist, relative to the worker cwd.
            cmd.env(ui_watch_env(name), "1");
        }
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

        if ui_watch {
            if let Some(ui_dir) = &ui_dir {
                self.spawn_ui_watcher(name, ui_dir).await;
            }
        }

        if wait_connected {
            self.wait_connected(name).await?;
        }

        Ok(())
    }

    /// Spawn the `pnpm watch` companion for a worker's ui/ project (the SOP
    /// dev loop's build half; the worker's own `III_<WORKER>_UI_WATCH` poller
    /// is the re-register half). Output streams into the worker's log ring
    /// tagged `[ui]`. Spawn failure is a warn line, not a start failure —
    /// the worker itself is unaffected.
    async fn spawn_ui_watcher(&self, name: &str, ui_dir: &std::path::Path) {
        let mut cmd = Command::new("pnpm");
        cmd.args(["run", "watch"]);
        cmd.current_dir(ui_dir);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);
        cmd.env("NO_COLOR", "1");
        // Own process group, same as the worker: `pnpm run` forks node, and
        // stopping must take the whole tree down (see terminate_process_group).
        #[cfg(unix)]
        cmd.process_group(0);

        match cmd.spawn() {
            Ok(mut child) => {
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();
                {
                    let mut runtimes = self.runtimes.write().await;
                    if let Some(rt) = runtimes.get_mut(name) {
                        push_log(
                            rt,
                            format!(
                                "[ui] watch: pnpm watch in {} · {}=1 (console tabs hot-reload on rebuild)",
                                ui_dir.display(),
                                ui_watch_env(name)
                            ),
                        );
                        rt.set_ui_child(child);
                    }
                }
                if let Some(stdout) = stdout {
                    let worker = name.to_string();
                    let runtimes = self.runtimes.clone();
                    tokio::spawn(async move {
                        read_stream_tagged(worker, stdout, runtimes, UI_LOG_TAG, false).await;
                    });
                }
                if let Some(stderr) = stderr {
                    let worker = name.to_string();
                    let runtimes = self.runtimes.clone();
                    tokio::spawn(async move {
                        read_stream_tagged(worker, stderr, runtimes, UI_LOG_TAG, false).await;
                    });
                }
            }
            Err(err) => {
                let mut runtimes = self.runtimes.write().await;
                if let Some(rt) = runtimes.get_mut(name) {
                    push_log(
                        rt,
                        format!(
                            "[ui] failed to spawn `pnpm watch` in {}: {err} — UI hot reload \
                             inactive (is pnpm on PATH?)",
                            ui_dir.display()
                        ),
                    );
                }
            }
        }
    }

    /// Flip the injectable-UI watcher flag for one worker (TUI `w` key).
    /// Errors when the worker ships no ui/ project. A running worker is
    /// restarted so the `III_<WORKER>_UI_WATCH` env var takes effect; a
    /// stopped one just starts with the new flag next time. Returns the new
    /// state.
    pub async fn toggle_ui_watch(&self, name: &str) -> Result<bool> {
        let spec = self
            .config
            .worker_spec(name)
            .with_context(|| format!("unknown worker {name}"))?;
        if spec.ui_dir.is_none() {
            bail!("worker {name} ships no ui/ project — nothing to watch");
        }
        let (next, running) = {
            let mut runtimes = self.runtimes.write().await;
            let rt = runtimes.get_mut(name).context("unknown worker runtime")?;
            rt.ui_watch = !rt.ui_watch;
            (rt.ui_watch, rt.pid().is_some())
        };
        if running {
            self.start_one(name, false).await?;
        }
        Ok(next)
    }

    async fn stop_one(&self, name: &str) {
        let (child, ui_child) = {
            let mut runtimes = self.runtimes.write().await;
            let Some(rt) = runtimes.get_mut(name) else {
                return;
            };
            (rt.take_child(), rt.take_ui_child())
        };

        if let Some(mut child) = ui_child {
            terminate_process_group(&mut child).await;
            let _ = child.wait().await;
        }

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
        let members = self.stack_members(&self.config.default_stack)?;
        Ok(self.dashboard_snapshot(&members).await.0)
    }

    /// Like `worker_views`, but also reports an engine-query error (if any) so
    /// the dashboard can show "engine unreachable" instead of silently blanking
    /// every engine status. Never fails: on engine error the views still render
    /// (all disconnected) alongside the error string. Views come back grouped
    /// and ordered for `members` (the current stack's member set).
    pub async fn dashboard_snapshot(
        &self,
        members: &HashSet<String>,
    ) -> (Vec<WorkerView>, Option<String>) {
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
        status::assign_view_groups(&mut views, members);
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

/// Log-line prefix for the `pnpm watch` companion's output, so esbuild
/// chatter is tellable apart from the worker's own logs in the shared ring.
const UI_LOG_TAG: &str = "[ui] ";

/// The worker's hot-reload env var, derived exactly like the
/// `iii-console-ui` crate does in `ConsoleUi::new` (non-alphanumerics → `_`,
/// uppercased): `III_<WORKER>_UI_WATCH`. Workers that override
/// `.watch_env(…)` in their builder won't match — none in this repo do.
fn ui_watch_env(worker: &str) -> String {
    let stem: String = worker
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("III_{stem}_UI_WATCH")
}

/// Push a line into a worker's ring buffer AND its live-follow channel —
/// the same pair every streamed log line goes through.
fn push_log(rt: &mut WorkerRuntime, line: String) {
    rt.logs.push(line.clone());
    let _ = rt.log_tx.send(line);
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
        group: WorkerGroup::Other, // reassigned by assign_view_groups
        spawnable: matches!(spec.spawn, SpawnKind::CargoRun),
        display_status: display.to_string(),
        process_status: process,
        engine_status,
        local_pid: rt.pid(),
        uptime,
        exit_code: rt.exit_code,
        ui_watch: spec.ui_dir.is_some().then_some(rt.ui_watch),
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
    read_stream_tagged(worker, stream, runtimes, "", true).await;
}

/// Stream lines into a worker's log ring. `tag` prefixes every line (the ui
/// watcher's `[ui] `); `track_state` gates the cargo-output → ProcState
/// heuristics, which must never run on watcher output (an esbuild "error"
/// line is not the WORKER compiling or crashing).
async fn read_stream_tagged(
    worker: String,
    stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    runtimes: SharedRuntimes,
    tag: &'static str,
    track_state: bool,
) {
    let mut lines = BufReader::new(stream).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let normalized = logs::normalize_log_line(&line);
                if normalized.is_empty() {
                    continue;
                }
                let tagged = format!("{tag}{normalized}");
                let mut runtimes = runtimes.write().await;
                if let Some(rt) = runtimes.get_mut(&worker) {
                    if track_state {
                        update_proc_state_from_line(rt, &normalized);
                    }
                    push_log(rt, tagged);
                }
            }
            Ok(None) => break,
            Err(err) => {
                let mut runtimes = runtimes.write().await;
                if let Some(rt) = runtimes.get_mut(&worker) {
                    push_log(rt, format!("{tag}log read error: {err}"));
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

    /// A non-Rust dependency (e.g. harness → scrapling) must classify as
    /// unsupported so start paths skip it instead of aborting the stack start.
    #[test]
    fn unsupported_reason_flags_non_rust_dep() {
        let tmp = tempfile::TempDir::new().unwrap();
        let write = |name: &str, body: &str| {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("iii.worker.yaml"), body).unwrap();
            dir
        };
        let harness_dir = write(
            "harness",
            "iii: v1\nname: harness\nlanguage: rust\ndeploy: binary\ndependencies:\n  scrapling: \"^0.2.4\"\n",
        );
        std::fs::write(harness_dir.join("Cargo.toml"), "[workspace]\n").unwrap();
        write(
            "scrapling",
            "iii: v1\nname: scrapling\nlanguage: python\ndeploy: bundle\n",
        );

        let worker_specs = crate::discover::discover_repo_workers(tmp.path()).unwrap();
        let workers = crate::discover::order_worker_names(&worker_specs);
        let config = Config {
            repo_root: tmp.path().to_path_buf(),
            config_path: tmp.path().join("workers-dev.yaml"),
            engine_url: crate::config::DEFAULT_ENGINE_URL.to_string(),
            release: false,
            poll_interval_ms: crate::config::DEFAULT_POLL_INTERVAL_MS,
            connect_timeout_ms: crate::config::DEFAULT_CONNECT_TIMEOUT_MS,
            workers,
            stacks: vec![("harness".to_string(), vec!["harness".to_string()])],
            default_stack: "harness".to_string(),
            worker_specs,
            stop_on_exit: false,
            color_mode: Default::default(),
            ui_watch: false,
        };
        let orch = Orchestrator::new(config, false).unwrap();

        assert!(orch.unsupported_reason("harness").is_none());
        assert!(orch.unsupported_reason("scrapling").is_some());
        // The harness closure pulls scrapling in — the start loop must see it
        // as skippable, not bail.
        let order = orch
            .graph
            .closure_with_deps(&["harness".to_string()])
            .unwrap();
        assert!(order.contains(&"scrapling".to_string()));
    }

    /// Shared fixture for the empty-roots tests below: a repo with a single
    /// `harness` worker, plus a `ghost` stack with no roots — what config.rs
    /// produces once every named root has been filtered out of `workers:`.
    /// Returns the `TempDir` too: it backs `repo_root`/`config_path` and must
    /// outlive the `Orchestrator`.
    fn test_orchestrator() -> (tempfile::TempDir, Orchestrator) {
        let tmp = tempfile::TempDir::new().unwrap();
        let write = |name: &str, body: &str| {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("iii.worker.yaml"), body).unwrap();
            dir
        };
        let harness_dir = write(
            "harness",
            "iii: v1\nname: harness\nlanguage: rust\ndeploy: binary\n",
        );
        std::fs::write(harness_dir.join("Cargo.toml"), "[workspace]\n").unwrap();

        let worker_specs = crate::discover::discover_repo_workers(tmp.path()).unwrap();
        let workers = crate::discover::order_worker_names(&worker_specs);
        let config = Config {
            repo_root: tmp.path().to_path_buf(),
            config_path: tmp.path().join("workers-dev.yaml"),
            engine_url: crate::config::DEFAULT_ENGINE_URL.to_string(),
            release: false,
            poll_interval_ms: crate::config::DEFAULT_POLL_INTERVAL_MS,
            connect_timeout_ms: crate::config::DEFAULT_CONNECT_TIMEOUT_MS,
            workers,
            // "ghost" mirrors what config.rs produces once every root of a
            // configured stack has been dropped: the stack survives with an
            // empty roots list rather than being removed outright.
            stacks: vec![
                ("harness".to_string(), vec!["harness".to_string()]),
                ("ghost".to_string(), Vec::new()),
            ],
            default_stack: "harness".to_string(),
            worker_specs,
            stop_on_exit: false,
            color_mode: Default::default(),
            ui_watch: false,
        };
        let orch = Orchestrator::new(config, false).unwrap();
        (tmp, orch)
    }

    /// A stack can legitimately end up with empty roots when every root it
    /// names is filtered out of the managed `workers:` set (config.rs warns
    /// and drops). `start_stack` must refuse rather than fall through to
    /// `start_workers`' "no names" meaning ("start every managed worker").
    /// No engine is reachable in this test, and `start_workers`' first move
    /// is an engine round-trip (`connected_worker_names`) — so the "stack
    /// ghost has no startable workers" assertion below only passes if the
    /// guard in `start_stack` returns first; were it missing or placed
    /// after that call, this would fail on a connection error instead.
    #[tokio::test]
    async fn start_stack_with_empty_roots_refuses_to_start_everything() {
        let (_tmp, orch) = test_orchestrator();

        let err = orch
            .start_stack("ghost", false)
            .await
            .expect_err("a stack with no startable workers must refuse to start");
        assert!(
            err.to_string().contains("ghost"),
            "error should name the stack, got: {err}"
        );
    }

    /// The empty-roots guard lives in `start_roots`, so it protects the
    /// TUI's roots-based path too — and fires before any engine contact.
    #[tokio::test]
    async fn start_roots_refuses_an_empty_root_list() {
        let (_tmp, orch) = test_orchestrator();
        let err = orch.start_roots("ghost", &[], false).await.unwrap_err();
        assert!(err.to_string().contains("ghost"), "{err:#}");
    }

    /// The env var must match what `iii-console-ui`'s `ConsoleUi::new`
    /// derives, or the worker's poller never arms. Locks the two known
    /// shapes: plain names and hyphenated ones.
    #[test]
    fn ui_watch_env_matches_console_ui_crate_derivation() {
        assert_eq!(ui_watch_env("state"), "III_STATE_UI_WATCH");
        assert_eq!(ui_watch_env("iii-directory"), "III_III_DIRECTORY_UI_WATCH");
        assert_eq!(ui_watch_env("console"), "III_CONSOLE_UI_WATCH");
    }

    /// Dependencies already connected to the engine are left alone; only
    /// workers asked for by name always (re)start.
    #[test]
    fn skip_connected_dep_spares_healthy_deps_only() {
        let names = vec!["harness".to_string()];
        let connected: HashSet<String> = ["session-manager", "harness"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Pulled-in dep, already connected → skipped.
        assert!(Orchestrator::skip_connected_dep(
            &names,
            "session-manager",
            &connected
        ));
        // Asked for by name → restarted even though connected.
        assert!(!Orchestrator::skip_connected_dep(
            &names, "harness", &connected
        ));
        // Dep not connected → started.
        assert!(!Orchestrator::skip_connected_dep(
            &names,
            "approval-gate",
            &connected
        ));
    }

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
