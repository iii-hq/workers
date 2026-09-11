//! The whole conversation with the `iii compose` daemon.
//!
//! Lifecycle belongs to the daemon: this module calls `compose::*` and adds no
//! proxies for them. It owns exactly three things the daemon does not give a
//! standalone client — bringing a daemon into existence, the live progress
//! feed during the first `--up`, and the injectable-UI watchers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::{FunctionRef, IIIConnectionState, WorkerMetadata};
use iii_sdk::trigger::Trigger;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction, WorkerIdentityMode};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::process::{Child, Command};
use tokio::sync::OnceCell;
use tokio::time;

use crate::config::{
    Config, DAEMON_READY_TIMEOUT_MS, LIFECYCLE_TIMEOUT_MS, STATUS_TIMEOUT_MS, STOP_TIMEOUT_MS,
};

/// compose answers `function_not_found` for the first seconds of a cold engine
/// (the worker manager registers late) — the house idiom is a short backoff
/// rather than concluding "no daemon" and spawning a second one.
const COLD_ENGINE_ATTEMPTS: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Up,
    Down,
    Restart,
    Status,
    List,
    Stop,
    Cancel,
}

impl Operation {
    fn function(self) -> &'static str {
        match self {
            Self::Up => "compose::up",
            Self::Down => "compose::down",
            Self::Restart => "compose::restart",
            Self::Status => "compose::status",
            Self::List => "compose::list",
            Self::Stop => "compose::stop",
            Self::Cancel => "compose::cancel",
        }
    }

    fn timeout_ms(self) -> u64 {
        match self {
            Self::Status | Self::List | Self::Cancel => STATUS_TIMEOUT_MS,
            Self::Stop => STOP_TIMEOUT_MS,
            Self::Up | Self::Down | Self::Restart => LIFECYCLE_TIMEOUT_MS,
        }
    }

    /// compose registers the read-only controls before the startup `--up` and
    /// the mutations only after it returns, so these three are absent for the
    /// whole cold build.
    fn is_mutation(self) -> bool {
        matches!(self, Self::Up | Self::Down | Self::Restart)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContainerStatus {
    pub container: String,
    /// `ready` | `failed` | `stopped`. compose declares a `starting` variant
    /// but never writes one.
    pub state: String,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub last_error: Option<String>,
    /// Where compose retains this container's stdout and stderr. Given, not
    /// derived: each project has its own log directory.
    pub log_path: PathBuf,
}

/// One project this daemon has loaded, as `compose::list` reports it.
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectSummary {
    pub file: PathBuf,
    pub containers: Vec<ContainerStatus>,
}

#[derive(Debug, Clone, Deserialize)]
struct ListOutcome {
    #[serde(default)]
    daemon_pid: u32,
    #[serde(default)]
    projects: Vec<ProjectSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComposeStatus {
    pub namespace: String,
    pub file: PathBuf,
    #[serde(default)]
    pub daemon_pid: u32,
    pub containers: Vec<ContainerStatus>,
}

/// What a mutation answers. This response is the only copy of a startup
/// failure: a container that never starts writes no record, so `compose::status`
/// reports it as a plain `stopped` with no `last_error`.
#[derive(Debug, Clone, Deserialize)]
struct MutationOutcome {
    #[serde(default)]
    status: String,
    #[serde(default)]
    error: Option<MutationError>,
}

#[derive(Debug, Clone, Deserialize)]
struct MutationError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

/// One `compose-operation` event. Only the startup `--up` publishes these —
/// a trigger-initiated up passes an operation id compose never registers.
#[derive(Debug, Clone, Deserialize)]
struct ProgressEvent {
    operation_id: String,
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    terminal: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub operation_id: Option<String>,
    /// Per container, the last phase seen. The only progress compose reports
    /// that a counter can be built from: `plan` and `completed_one` emit
    /// nothing, and `total` on a per-container event is the dependency-tree
    /// depth, not a count.
    pub phases: HashMap<String, String>,
    pub finished: bool,
    /// We spawned a daemon with `--up` and its startup operation has not
    /// reported terminal yet. The first events of that operation fire in the
    /// few hundred ms before a subscription can exist, so without this the
    /// cold build — the longest window there is — would show nothing at all.
    pub starting_up: bool,
}

impl Progress {
    /// True while the project is being brought up — the window where
    /// `compose::status` answers from a snapshot written before it began.
    pub fn live(&self) -> bool {
        self.starting_up || (self.operation_id.is_some() && !self.finished)
    }
}

#[derive(Debug)]
pub struct DaemonInfo {
    pub pid: u32,
    /// We spawned it. Only then do the containers read our env file, and only
    /// then does a UI-watch toggle mean anything.
    pub ours: bool,
}

struct Daemon {
    pid: u32,
    ours: bool,
    child: Option<Child>,
}

pub struct Compose {
    pub config: Config,
    client: OnceCell<IIIClient>,
    daemon: tokio::sync::Mutex<Option<Daemon>>,
    progress: std::sync::Arc<Mutex<Progress>>,
    watch: Mutex<HashMap<String, bool>>,
    ui_children: tokio::sync::Mutex<HashMap<String, Child>>,
    handles: Mutex<Option<(FunctionRef, Trigger)>>,
}

impl Compose {
    pub fn new(config: Config) -> Result<Self> {
        let persisted = read_watch_flags(&config.ui_watch_env_path)?;
        let watch = config
            .workers
            .iter()
            .filter(|worker| worker.ui_dir.is_some())
            .map(|worker| worker.name.clone())
            .chain(
                config
                    .repo_workers
                    .iter()
                    .filter(|worker| worker.ui_dir.is_some())
                    .map(|worker| worker.name.clone()),
            )
            .map(|name| {
                let on = config.ui_watch
                    || persisted
                        .get(&ui_watch_env(&name))
                        .copied()
                        .unwrap_or(false);
                (name, on)
            })
            .collect();
        Ok(Self {
            config,
            client: OnceCell::new(),
            daemon: tokio::sync::Mutex::new(None),
            progress: std::sync::Arc::new(Mutex::new(Progress::default())),
            watch: Mutex::new(watch),
            ui_children: tokio::sync::Mutex::new(HashMap::new()),
            handles: Mutex::new(None),
        })
    }

    // ---------------------------------------------------------------- client

    async fn client(&self) -> &IIIClient {
        self.client
            .get_or_init(|| async {
                // In the project's namespace, not `default`: compose invokes a
                // bound trigger's function from its own client, so a progress
                // handler registered anywhere else is simply not found. The
                // cost is that a container which times out can be reported as
                // "registered as workers-dev-<pid>" instead of a plain
                // readiness timeout — a worse message on an already-failing
                // path, traded for the only live signal during a cold build.
                // The pid keeps two worktree instances from colliding.
                let client = register_worker(
                    &self.config.engine_url,
                    InitOptions {
                        metadata: Some(WorkerMetadata {
                            name: format!("workers-dev-{}", std::process::id()),
                            ..Default::default()
                        }),
                        namespace: Some(self.config.namespace.clone()),
                        identity: WorkerIdentityMode::Explicit,
                        ..Default::default()
                    },
                );
                let deadline = Instant::now() + Duration::from_millis(2000);
                while Instant::now() < deadline
                    && client.get_connection_state() != IIIConnectionState::Connected
                {
                    time::sleep(Duration::from_millis(50)).await;
                }
                client
            })
            .await
    }

    /// Subscribe to every operation this daemon runs. The startup `--up` is the
    /// one that matters: for its whole multi-minute duration `compose::status`
    /// reads a stale `state.json`, so these events are the only live signal.
    ///
    /// Bound only once a daemon has answered: the `compose-operation` trigger
    /// type is registered by the daemon, so binding before one exists — which
    /// is exactly when the first `compose::list` probe runs — would fail and
    /// leave the cold build with no feed at all.
    async fn bind_progress(&self) {
        if self.handles.lock().unwrap().is_some() {
            return;
        }
        let client = self.client().await;
        let function_id = format!("workers-dev::progress::{}", std::process::id());
        let progress = std::sync::Arc::clone(&self.progress);
        let function = client.register_function(
            &function_id,
            RegisterFunction::new_async(move |value: Value| {
                let progress = std::sync::Arc::clone(&progress);
                async move {
                    if let Ok(event) = serde_json::from_value::<ProgressEvent>(value) {
                        apply_progress(&progress, event);
                    }
                    Ok(json!({"ok": true}))
                }
            }),
        );
        let trigger = client.register_trigger(RegisterTriggerInput {
            trigger_type: "compose-operation".to_string(),
            function_id,
            config: json!({}),
            metadata: None,
            // Both default to this worker's namespace, which is the project's —
            // where compose registered the trigger type and where it looks for
            // the function it calls back.
            namespace: None,
            trigger_namespace: None,
        });
        // The feed is an extra, never a gate: without it the table still
        // reports what `compose::status` knows, just later. Say so in the
        // daemon log rather than silently, since a missing feed looks exactly
        // like a stack that is not moving.
        match trigger {
            Ok(trigger) => *self.handles.lock().unwrap() = Some((function, trigger)),
            Err(error) => self.note(&format!("progress feed unavailable: {error}")),
        }
    }

    /// One line into the daemon log, where the dashboard already shows it.
    fn note(&self, message: &str) {
        use std::io::Write;
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.daemon_log_path)
        {
            let _ = writeln!(log, "[workers-dev] {message}");
        }
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().unwrap().clone()
    }

    pub async fn daemon_info(&self) -> Option<DaemonInfo> {
        self.daemon.lock().await.as_ref().map(|daemon| DaemonInfo {
            pid: daemon.pid,
            ours: daemon.ours,
        })
    }

    // ------------------------------------------------------------ operations

    async fn call(&self, operation: Operation, target: Option<&str>) -> Result<Value> {
        self.call_on(&self.config.compose_path, operation, target, None)
            .await
    }

    /// The same call against a named project. The daemon serves both the
    /// tracked stack and the on-demand file, and `file` is what tells them
    /// apart — the namespace is only a guard.
    async fn call_on(
        &self,
        file: &Path,
        operation: Operation,
        target: Option<&str>,
        extra: Option<Value>,
    ) -> Result<Value> {
        let mut payload = compose_payload(operation, &self.config.namespace, file, target);
        if let Some(Value::Object(extra)) = extra {
            for (key, value) in extra {
                payload[key] = value;
            }
        }
        self.client()
            .await
            .trigger(
                TriggerRequest {
                    function_id: operation.function().to_string(),
                    payload,
                    action: None,
                    timeout_ms: Some(operation.timeout_ms()),
                }
                .namespace(self.config.namespace.clone()),
            )
            .await
            .map_err(|error| {
                if operation.is_mutation() && is_function_not_found(&error) {
                    anyhow!("compose is still bringing the project up — try again when it settles")
                } else {
                    anyhow::Error::new(error).context(format!("{} failed", operation.function()))
                }
            })
    }

    /// Declare a repo worker and start it, as a compose project of its own.
    ///
    /// One file per worker, deliberately. A container added to a project the
    /// daemon has already loaded is invisible until a whole-project restart —
    /// which stops everything else in it. A new file is a new project, so
    /// starting the fifth on-demand worker leaves the other four alone.
    ///
    /// (`compose::add` would splice into a shared file and reconcile just the
    /// new container, but this build rejects its container-object form, and the
    /// string form cannot carry the `cargo run --bin` that most of the repo's
    /// manifests do not declare.)
    pub async fn add_local(&self, name: &str, bin: &str) -> Result<()> {
        let file = self.write_local_file(name, bin)?;
        self.up(&file, None).await
    }

    fn write_local_file(&self, name: &str, bin: &str) -> Result<PathBuf> {
        let file = self.config.local_file(name);
        std::fs::create_dir_all(&self.config.local_dir)
            .with_context(|| format!("create {}", self.config.local_dir.display()))?;

        let document = serde_yaml::to_string(&json!({
            "containers": {
                name: {
                    // Relative to this file, which sits one level deeper than
                    // the stack's compose file.
                    "worker": format!("path://../../{name}"),
                    "scripts": { "run": format!("cargo run --bin {bin}") },
                    "environment": { "RUST_LOG": "info", "CI": "true" },
                    "env_file": [self.config.ui_watch_env_path.to_string_lossy()],
                }
            }
        }))
        .context("render the on-demand compose file")?;
        std::fs::write(
            &file,
            format!(
                "# Written by workers-dev: {name}, started on demand from the repo list.\n\
                 # Gitignored. Delete this file to forget it.\n{document}"
            ),
        )
        .with_context(|| format!("write {}", file.display()))?;
        Ok(file)
    }

    /// Carry a pre-existing shared on-demand file over to one project per
    /// worker. Anything the daemon is already running stays in the old project
    /// until it is stopped — `compose::list` still reports it, so the dashboard
    /// keeps addressing it correctly; the new files take over on the next start.
    fn migrate_legacy_local(&self) -> Result<()> {
        let legacy = self
            .config
            .repo_root
            .join("harness/worker-compose.local.yaml");
        let Ok(text) = std::fs::read_to_string(&legacy) else {
            return Ok(());
        };
        let containers = serde_yaml::from_str::<serde_yaml::Value>(&text)
            .ok()
            .and_then(|file| file.get("containers")?.as_mapping().cloned())
            .unwrap_or_default();

        for name in containers.keys().filter_map(|key| key.as_str()) {
            if self.config.local_file(name).is_file() {
                continue;
            }
            // Re-rendered rather than moved: the worker path is relative to the
            // file, and the new one sits a directory deeper.
            if let Some(bin) = self.repo_worker(name).and_then(|worker| worker.bin.clone()) {
                self.write_local_file(name, &bin)?;
            }
        }

        let _ = std::fs::rename(&legacy, legacy.with_extension("yaml.migrated"));
        self.note(&format!(
            "migrated {} to one project per worker under {}",
            legacy.display(),
            self.config.local_dir.display()
        ));
        Ok(())
    }

    /// Every project this daemon has loaded, the stack included. One call, so
    /// the poll cost does not grow with the number of on-demand workers.
    pub async fn projects(&self) -> Result<(u32, Vec<ProjectSummary>)> {
        let value = self.call(Operation::List, None).await?;
        let outcome: ListOutcome = serde_json::from_value(value).context("parse compose::list")?;
        Ok((outcome.daemon_pid, outcome.projects))
    }

    /// Which project declares this container today: the tracked stack, an
    /// on-demand file, or neither.
    pub async fn project_of(&self, container: &str) -> Option<PathBuf> {
        if self.config.worker(container).is_some() {
            return Some(self.config.compose_path.clone());
        }
        // Declared on demand: the file exists whether or not it is loaded.
        let file = self.config.local_file(container);
        file.is_file().then_some(file)
    }

    /// A repo worker this tool could start from source, if it is one.
    pub fn repo_worker(&self, name: &str) -> Option<&crate::config::RepoWorker> {
        self.config
            .repo_workers
            .iter()
            .find(|worker| worker.name == name)
    }

    pub async fn status(&self) -> Result<ComposeStatus> {
        let value = self.call(Operation::Status, None).await?;
        let status: ComposeStatus =
            serde_json::from_value(value).context("parse compose::status")?;
        // Belt and braces for the startup flag: the terminal event clears it,
        // but a settled project is proof enough on its own, and a stuck flag
        // would hide the real states for the rest of the session.
        if status
            .containers
            .iter()
            .all(|container| container.state == "ready" || container.state == "failed")
        {
            self.progress.lock().unwrap().starting_up = false;
        }
        Ok(status)
    }

    pub async fn up(&self, file: &Path, container: Option<&str>) -> Result<()> {
        ensure_ok(self.call_on(file, Operation::Up, container, None).await?)
    }

    pub async fn down(&self, file: &Path, container: &str) -> Result<()> {
        ensure_ok(
            self.call_on(file, Operation::Down, Some(container), None)
                .await?,
        )
    }

    pub async fn restart(&self, file: &Path, worker: &str) -> Result<()> {
        ensure_ok(
            self.call_on(file, Operation::Restart, Some(worker), None)
                .await?,
        )
    }

    /// Tears down every project, the daemon, and the engine when compose owns
    /// it. Not idempotent — only ever from an explicit request.
    pub async fn stop(&self) -> Result<()> {
        self.stop_ui_children().await;
        self.call(Operation::Stop, None).await?;
        *self.daemon.lock().await = None;
        Ok(())
    }

    /// Cancel the in-flight operation. `compose::cancel` is a control, not a
    /// mutation, so it is the one lifecycle action available during a cold build.
    pub async fn cancel(&self) -> Result<()> {
        let Some(id) = self.progress.lock().unwrap().operation_id.clone() else {
            bail!("no compose operation in flight");
        };
        self.call(Operation::Cancel, Some(&id)).await?;
        Ok(())
    }

    // ---------------------------------------------------------------- daemon

    /// Make sure some compose daemon is serving our namespace, spawning one if
    /// not. Returns as soon as the daemon answers — not when the containers are
    /// ready, which on a cold checkout is many minutes away.
    pub async fn ensure_daemon(&self) -> Result<()> {
        if self.daemon.lock().await.is_some() {
            return Ok(());
        }

        // The on-demand projects point their `env_file` here, and a missing one
        // fails a whole project load — so it exists before any call can read it.
        self.flush_watch_flags()?;
        self.migrate_legacy_local()?;
        let engine_up = self.engine_reachable().await;
        if engine_up {
            // `compose::list` rather than `compose::status`: list answers from
            // the daemon itself, while status loads and binds the project.
            match self.wait_for_daemon(COLD_ENGINE_ATTEMPTS).await {
                Ok(pid) => {
                    *self.daemon.lock().await = Some(Daemon {
                        pid,
                        ours: false,
                        child: None,
                    });
                    self.bind_progress().await;
                    return Ok(());
                }
                Err(error) if !is_missing_daemon(&error) => return Err(error),
                Err(_) => {}
            }
        }

        let child = self.spawn_daemon(engine_up)?;
        let pid = child.id().unwrap_or_default();
        *self.daemon.lock().await = Some(Daemon {
            pid,
            ours: true,
            child: Some(child),
        });
        self.await_daemon().await?;
        self.progress.lock().unwrap().starting_up = true;
        self.bind_progress().await;
        Ok(())
    }

    fn spawn_daemon(&self, external_engine: bool) -> Result<Child> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.daemon_log_path)
            .with_context(|| format!("open {}", self.config.daemon_log_path.display()))?;
        let stderr = log.try_clone().context("clone daemon log handle")?;

        let mut command = Command::new("iii");
        // No `--config`: `iii --config X compose` is refused outright. Managed
        // engine settings live under `engine:` in the compose file.
        command.arg("compose");
        if external_engine {
            // An engine is already up; attach to it so stopping compose leaves
            // it running. Also the only form that survives a previous unclean
            // death, where the orphan engine still holds the port. The flag is
            // compose's own, so it goes after the subcommand — the root `iii`
            // CLI rejects it and the child would exit before serving anything.
            command.arg("--engine").arg(&self.config.engine_url);
        }
        command
            .arg("--namespace")
            .arg(&self.config.namespace)
            .arg("--up")
            .arg("--file")
            .arg(&self.config.compose_path)
            .current_dir(&self.config.repo_root)
            .env("WORKERS_DEV_ENV_FILE", &self.config.ui_watch_env_path)
            .stdin(Stdio::null())
            // A file, never a pipe: compose prints with bare `println!` and
            // Rust ignores SIGPIPE, so a closed read end would panic the
            // daemon the moment this process exits. The redirect also turns
            // off compose's spinner, leaving plain lines the pane can read.
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr));
        #[cfg(unix)]
        command.process_group(0);
        // Deliberately no kill_on_drop: quitting the dashboard must leave the
        // stack running. The child reparents to init.
        command.spawn().context("spawn iii compose")
    }

    /// Wait for the daemon we just spawned, failing early (with the tail of its
    /// log) if it exits instead.
    async fn await_daemon(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(DAEMON_READY_TIMEOUT_MS);
        loop {
            if let Some(daemon) = self.daemon.lock().await.as_mut() {
                if let Some(child) = daemon.child.as_mut() {
                    if let Ok(Some(exit)) = child.try_wait() {
                        let tail = crate::logs::tail_file(&self.config.daemon_log_path, 4096);
                        let reason = tail.iter().rev().take(8).rev().cloned().collect::<Vec<_>>();
                        bail!(
                            "iii compose exited ({exit}) before serving {}:\n{}",
                            self.config.namespace,
                            reason.join("\n")
                        );
                    }
                }
            }
            if self.wait_for_daemon(1).await.is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "iii compose did not answer in {}s — see {}",
                    DAEMON_READY_TIMEOUT_MS / 1000,
                    self.config.daemon_log_path.display()
                );
            }
            time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// `compose::list` with the cold-engine backoff. Returns the daemon pid.
    async fn wait_for_daemon(&self, attempts: u32) -> Result<u32> {
        let mut last = None;
        for attempt in 1..=attempts {
            match self.call(Operation::List, None).await {
                Ok(value) => {
                    return Ok(value
                        .get("daemon_pid")
                        .and_then(Value::as_u64)
                        .unwrap_or_default() as u32)
                }
                Err(error) => {
                    let missing = is_missing_daemon(&error);
                    last = Some(error);
                    if !missing || attempt == attempts {
                        break;
                    }
                    time::sleep(Duration::from_secs(u64::from(attempt) * 2)).await;
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("compose::list failed")))
    }

    /// Cheap TCP probe — cheaper than a trigger round-trip, and it is the
    /// question that decides managed vs attached.
    pub async fn engine_reachable(&self) -> bool {
        matches!(
            time::timeout(
                Duration::from_millis(500),
                tokio::net::TcpStream::connect((
                    self.config.engine_host.as_str(),
                    self.config.engine_port,
                )),
            )
            .await,
            Ok(Ok(_))
        )
    }

    // -------------------------------------------------------------- ui watch

    pub fn ui_watch(&self, worker: &str) -> Option<bool> {
        self.watch.lock().unwrap().get(worker).copied()
    }

    /// Flip the watcher for one worker: rewrite the env file, start or stop the
    /// `pnpm watch` sidecar, and bounce that one container. `env_file` contents
    /// are the one thing compose re-reads at every spawn, which is what makes a
    /// single-container restart enough.
    pub async fn toggle_ui_watch(&self, file: &Path, worker: &str) -> Result<()> {
        let Some(ui_dir) = self.ui_dir(worker) else {
            bail!("{worker} ships no watchable ui/");
        };
        if let Some(daemon) = self.daemon.lock().await.as_ref() {
            if !daemon.ours {
                bail!(
                    "this compose daemon was not started by workers-dev — it resolved its env \
                     file at its own launch, so UI watch cannot be toggled from here"
                );
            }
        }

        let on = {
            let mut watch = self.watch.lock().unwrap();
            let entry = watch.entry(worker.to_string()).or_insert(false);
            *entry = !*entry;
            *entry
        };
        self.flush_watch_flags()?;

        if on {
            self.start_ui_child(worker, &ui_dir).await?;
        } else {
            self.stop_ui_child(worker).await;
        }
        self.restart(file, worker).await
    }

    /// The watchable `ui/` of a container, whether the stack declares it or the
    /// repo list offered it.
    pub fn ui_dir(&self, worker: &str) -> Option<PathBuf> {
        self.config
            .worker(worker)
            .and_then(|spec| spec.ui_dir.clone())
            .or_else(|| {
                self.config
                    .repo_workers
                    .iter()
                    .find(|repo| repo.name == worker)
                    .and_then(|repo| repo.ui_dir.clone())
            })
    }

    /// Start every enabled watcher. Called once when the dashboard opens.
    pub async fn start_enabled_ui_children(&self) {
        let enabled: Vec<(String, PathBuf)> = self
            .config
            .workers
            .iter()
            .filter_map(|worker| {
                let dir = worker.ui_dir.clone()?;
                self.ui_watch(&worker.name)
                    .unwrap_or(false)
                    .then(|| (worker.name.clone(), dir))
            })
            .collect();
        for (worker, dir) in enabled {
            let _ = self.start_ui_child(&worker, &dir).await;
        }
    }

    async fn start_ui_child(&self, worker: &str, ui_dir: &Path) -> Result<()> {
        let mut children = self.ui_children.lock().await;
        if children.contains_key(worker) {
            return Ok(());
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.daemon_log_path)?;
        let stderr = log.try_clone()?;
        let mut command = Command::new("pnpm");
        command
            .arg("run")
            .arg("watch")
            .current_dir(ui_dir)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr))
            // Ours to clean up, unlike the compose daemon.
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let child = command
            .spawn()
            .with_context(|| format!("spawn pnpm run watch in {}", ui_dir.display()))?;
        children.insert(worker.to_string(), child);
        Ok(())
    }

    async fn stop_ui_child(&self, worker: &str) {
        if let Some(mut child) = self.ui_children.lock().await.remove(worker) {
            terminate_group(&mut child).await;
        }
    }

    pub async fn stop_ui_children(&self) {
        let mut children = self.ui_children.lock().await;
        for (_, mut child) in children.drain() {
            terminate_group(&mut child).await;
        }
    }

    fn flush_watch_flags(&self) -> Result<()> {
        let mut entries = forwarded_api_keys();
        entries.extend(
            self.watch
                .lock()
                .unwrap()
                .iter()
                .map(|(name, on)| (ui_watch_env(name), u8::from(*on).to_string())),
        );
        write_env_file(&self.config.ui_watch_env_path, &entries)
    }

    pub async fn shutdown(&self) {
        self.stop_ui_children().await;
        if let Some(client) = self.client.get() {
            client.shutdown_async().await;
        }
    }
}

fn apply_progress(progress: &Mutex<Progress>, event: ProgressEvent) {
    let mut progress = progress.lock().unwrap();
    if progress.operation_id.as_deref() != Some(event.operation_id.as_str()) {
        *progress = Progress {
            operation_id: Some(event.operation_id.clone()),
            starting_up: progress.starting_up,
            ..Progress::default()
        };
    }
    if let Some(container) = event.container {
        progress.phases.insert(container, event.phase);
    } else if event.terminal {
        progress.finished = true;
        progress.starting_up = false;
    }
}

/// `{namespace, file}` always: the namespace is a guard (the route is the
/// trigger's own namespace) and the file is what names the project. `up` and
/// `down` target a `container`; `restart` targets a `worker`.
fn compose_payload(
    operation: Operation,
    namespace: &str,
    file: &Path,
    target: Option<&str>,
) -> Value {
    let mut payload = json!({ "namespace": namespace });
    if !matches!(operation, Operation::Stop | Operation::Cancel) {
        payload["file"] = json!(file.to_string_lossy());
    }
    if let Some(target) = target {
        let field = match operation {
            Operation::Restart => "worker",
            Operation::Cancel => "operation_id",
            _ => "container",
        };
        payload[field] = json!(target);
    }
    payload
}

fn ensure_ok(value: Value) -> Result<()> {
    let outcome: MutationOutcome = serde_json::from_value(value.clone())
        .with_context(|| format!("unexpected compose response: {value}"))?;
    match outcome.error {
        Some(error) => bail!("{} — {}", error.code, error.message),
        None if outcome.status == "failed" => bail!("compose reported a failed operation"),
        None => Ok(()),
    }
}

fn remote_code(error: &anyhow::Error) -> Option<&str> {
    error
        .downcast_ref::<iii_sdk::Error>()
        .and_then(|error| match error {
            iii_sdk::Error::Remote { code, .. } => Some(code.as_str()),
            _ => None,
        })
}

fn is_function_not_found(error: &iii_sdk::Error) -> bool {
    matches!(error, iii_sdk::Error::Remote { code, .. } if code == "function_not_found")
}

/// The single predicate that decides between attaching and spawning. A
/// `WRONG_DAEMON` means a daemon is there and answering — never spawn a second.
fn is_missing_daemon(error: &anyhow::Error) -> bool {
    remote_code(error) == Some("function_not_found")
}

/// The env stem is the name the worker registers its UI under, which is not
/// always the container key: `ade` registers as `console` (ade/src/ui.rs) and
/// `ide` as `shell` (ide/src/ui.rs). Deriving from the key fails silently for
/// exactly those two — the watcher runs and nothing hot-reloads.
pub fn ui_watch_env(container: &str) -> String {
    let stem = match container {
        "ade" => "console",
        "ide" => "shell",
        other => other,
    };
    let upper: String = stem
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("III_{upper}_UI_WATCH")
}

/// The API keys in this shell, on their way to the workers.
///
/// compose builds a child's environment from a fixed baseline and drops
/// everything else, so a key you exported reaches nothing unless the compose
/// file names it — and it only names two. Rather than edit a tracked file per
/// provider, forward them through the env file every container already reads.
///
/// Sorted, so the file does not churn between launches.
fn forwarded_api_keys() -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = std::env::vars()
        .filter(|(name, _)| name.ends_with("_API_KEY") && !name.starts_with("III_"))
        // A newline would end the record and smuggle the rest in as its own
        // entry; an empty key is worse than an absent one, since `environment`
        // is the only thing that could still supply it.
        .filter(|(_, value)| !value.is_empty() && !value.contains(['\n', '\r']))
        .collect();
    keys.sort();
    keys
}

fn read_watch_flags(path: &Path) -> Result<HashMap<String, bool>> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(HashMap::new());
    };
    Ok(text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_string(), value.trim() == "1"))
        .collect())
}

/// Written whole, atomically: compose reads this file at every container spawn,
/// so a half-written one is a half-configured worker. Owner-only, because the
/// forwarded API keys land here.
fn write_env_file(path: &Path, entries: &[(String, String)]) -> Result<()> {
    let mut body = String::from(
        "# Written by workers-dev: API keys forwarded from the launching shell,\n\
         # and the injectable-UI watch flags the `w` key toggles.\n",
    );
    for (key, value) in entries {
        body.push_str(&format!("{key}={value}\n"));
    }

    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temp, body).with_context(|| format!("write {}", temp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("restrict {}", temp.display()))?;
    }
    std::fs::rename(&temp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

/// `pnpm run watch` forks esbuild; killing only pnpm orphans the rebuild loop.
async fn terminate_group(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
        for _ in 0..20 {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => time::sleep(Duration::from_millis(50)).await,
                Err(_) => break,
            }
        }
    }
    let _ = child.start_kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_payload_routes_by_namespace_and_absolute_file() {
        let file = Path::new("/repo/harness/worker-compose.yaml");
        assert_eq!(
            compose_payload(Operation::Up, "ns", file, Some("state")),
            json!({"namespace": "ns", "file": "/repo/harness/worker-compose.yaml", "container": "state"})
        );
        assert_eq!(
            compose_payload(Operation::Restart, "ns", file, Some("state")),
            json!({"namespace": "ns", "file": "/repo/harness/worker-compose.yaml", "worker": "state"})
        );
        // stop addresses the daemon, not a project
        assert_eq!(
            compose_payload(Operation::Stop, "ns", file, None),
            json!({"namespace": "ns"})
        );
        assert_eq!(
            compose_payload(Operation::Cancel, "ns", file, Some("op-1")),
            json!({"namespace": "ns", "operation_id": "op-1"})
        );
    }

    #[test]
    fn failed_mutation_outcome_surfaces_code_and_message() {
        let error = ensure_ok(json!({
            "status": "failed",
            "worker": "harness",
            "error": {"code": "STARTUP_TIMEOUT", "message": "harness did not register"}
        }))
        .unwrap_err()
        .to_string();
        assert!(error.contains("STARTUP_TIMEOUT"), "{error}");
        assert!(error.contains("did not register"), "{error}");
        assert!(ensure_ok(json!({"status": "ok", "changed": true})).is_ok());
    }

    #[test]
    fn ui_watch_env_uses_the_registered_name_not_the_container_key() {
        assert_eq!(ui_watch_env("ade"), "III_CONSOLE_UI_WATCH");
        assert_eq!(ui_watch_env("ide"), "III_SHELL_UI_WATCH");
        assert_eq!(ui_watch_env("harness"), "III_HARNESS_UI_WATCH");
        assert_eq!(ui_watch_env("llm-router"), "III_LLM_ROUTER_UI_WATCH");
    }

    #[test]
    fn watch_flags_round_trip_through_the_atomic_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env.workers-dev");
        write_env_file(
            &path,
            &[
                ("DEEPSEEK_API_KEY".to_string(), "sk-secret".to_string()),
                ("III_CONSOLE_UI_WATCH".to_string(), "1".to_string()),
                ("III_HARNESS_UI_WATCH".to_string(), "0".to_string()),
            ],
        )
        .unwrap();
        let flags = read_watch_flags(&path).unwrap();
        assert_eq!(flags.get("III_CONSOLE_UI_WATCH"), Some(&true));
        assert_eq!(flags.get("III_HARNESS_UI_WATCH"), Some(&false));
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("DEEPSEEK_API_KEY=sk-secret"));
        // no temp file left where `git status` would show it
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        // the file carries credentials
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{mode:o}");
        }
    }

    /// A key that cannot survive the file format must not reach it: compose
    /// reads one `KEY=VALUE` per line with no escaping.
    #[test]
    fn only_well_formed_api_keys_are_forwarded() {
        std::env::set_var("WORKERS_DEV_TEST_API_KEY", "sk-good");
        std::env::set_var("WORKERS_DEV_EMPTY_API_KEY", "");
        std::env::set_var("WORKERS_DEV_MULTILINE_API_KEY", "sk-bad\nSHELL=/x");
        std::env::set_var("WORKERS_DEV_TEST_TOKEN", "not-a-key");
        let keys = forwarded_api_keys();
        assert!(keys.contains(&(
            "WORKERS_DEV_TEST_API_KEY".to_string(),
            "sk-good".to_string()
        )));
        let names: Vec<&str> = keys.iter().map(|(name, _)| name.as_str()).collect();
        assert!(!names.contains(&"WORKERS_DEV_EMPTY_API_KEY"));
        assert!(!names.contains(&"WORKERS_DEV_MULTILINE_API_KEY"));
        assert!(!names.contains(&"WORKERS_DEV_TEST_TOKEN"));
        assert!(names.windows(2).all(|pair| pair[0] <= pair[1]), "sorted");
    }

    #[test]
    fn function_not_found_identifies_a_missing_compose_daemon() {
        let missing = anyhow::Error::new(iii_sdk::Error::Remote {
            code: "function_not_found".into(),
            message: "Function compose::list not found".into(),
            stacktrace: None,
        });
        let wrong = anyhow::Error::new(iii_sdk::Error::Remote {
            code: "WRONG_DAEMON".into(),
            message: "this daemon serves namespace 'a'".into(),
            stacktrace: None,
        });
        assert!(is_missing_daemon(&missing));
        assert!(!is_missing_daemon(&wrong));
    }

    #[test]
    fn progress_tracks_the_current_operation_only() {
        let progress = Mutex::new(Progress::default());
        apply_progress(
            &progress,
            serde_json::from_value(json!({
                "sequence": 1, "operation_id": "op-1", "container": "state",
                "phase": "starting", "detail": "starting worker", "total": 13,
                "current": 0, "elapsed_ms": 0, "terminal": false
            }))
            .unwrap(),
        );
        assert!(progress.lock().unwrap().live());
        assert_eq!(progress.lock().unwrap().phases["state"], "starting");
        // a second operation replaces the first rather than merging into it
        apply_progress(
            &progress,
            serde_json::from_value(json!({
                "sequence": 1, "operation_id": "op-2", "container": "ade",
                "phase": "ready", "detail": "registered with engine",
                "elapsed_ms": 0, "terminal": false
            }))
            .unwrap(),
        );
        let snapshot = progress.lock().unwrap().clone();
        assert!(!snapshot.phases.contains_key("state"));
        assert_eq!(snapshot.phases["ade"], "ready");

        // A terminal event ends the startup window; a per-container one does not.
        let mut seeded = Progress {
            starting_up: true,
            ..Progress::default()
        };
        assert!(seeded.live());
        seeded.operation_id = Some("op-3".into());
        seeded.finished = true;
        assert!(
            seeded.live(),
            "a finished op must not end the startup window"
        );
        seeded.starting_up = false;
        assert!(!seeded.live());
    }
}
