//! Builds [`Overview`]: one tmux listing, the process table and listening
//! sockets (re-scanned on their own cadence, or at once when a pane's
//! foreground process changes), and per agent pane a `capture-pane` plus
//! what the agent records about itself on disk.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::accounts::panes::{pane_claude, ClaudeProc, Marks, PaneState};
use crate::accounts::Accounts;
use crate::activity::{ActivityReader, ClaudeRecord, Recorder};
use crate::agents::{host_names, session_title, AgentSet, AgentTracker};
use crate::binary::{BinaryCache, Resolved};
use crate::compose;
use crate::config::{SharedConfig, WorkerConfig};
use crate::install::installable;
use crate::model::{
    now_ms, AgentState, AgentStatus, ComposeRef, ComposeStack, Listener, Overview, Pane,
    PaneClaude, PaneRef, PortsSnapshot, Session, SessionsSnapshot, TmuxInfo, Window,
};
use crate::procs::{self, IiiEnv, ProcTable, Scanner, SocketListener};
use crate::tmux_cli::{list_all_args, parse_list_all, RawSnapshot, Socket, Tmux, TmuxError};

/// How old the agents of the last collection may be when a function asks
/// for one pane's.
const SEEN_FRESH: Duration = Duration::from_secs(3);

/// ttys of the tmux clients this worker runs for console terminals.
#[derive(Clone, Default)]
pub struct ConsoleClients(Arc<RwLock<HashSet<String>>>);

impl ConsoleClients {
    pub fn insert(&self, tty: &str) {
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(tty.to_string());
    }

    pub fn remove(&self, tty: &str) {
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(tty);
    }

    pub fn contains(&self, tty: &str) -> bool {
        self.0
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .contains(tty)
    }
}

#[derive(Default)]
struct ProcCache {
    table: Arc<ProcTable>,
    listeners: Arc<Vec<SocketListener>>,
    env: Arc<HashMap<u32, IiiEnv>>,
    error: Option<String>,
    scanned_at: Option<Instant>,
    pane_signature: u64,
}

struct CompiledAgents {
    config: Arc<WorkerConfig>,
    set: Arc<AgentSet>,
}

pub struct Collector {
    config: SharedConfig,
    binary: BinaryCache,
    scanner: Arc<StdMutex<Scanner>>,
    activity: Arc<StdMutex<ActivityReader>>,
    procs: Mutex<ProcCache>,
    agents: Mutex<AgentTracker>,
    compiled: StdMutex<Option<CompiledAgents>>,
    /// The agents the last collection found, by pane id, and when.
    seen: StdMutex<Option<(Instant, HashMap<String, AgentStatus>)>>,
    /// The Claude Code panes the last collection found, by pane id, when,
    /// and whether tmux listed its panes then (an empty map from a listing
    /// that failed is not "no Claude Code").
    claude_seen: StdMutex<Option<ClaudeSeen>>,
    /// The running tmux server's version, as the last collection read it:
    /// what it accepts, which an upgraded client's `-V` doesn't say.
    server_version: StdMutex<Option<String>>,
    accounts: Arc<Accounts>,
    console: ConsoleClients,
    /// One collection at a time: concurrent callers would only race each
    /// other's scans.
    gate: Mutex<()>,
}

impl Collector {
    pub fn new(config: SharedConfig, console: ConsoleClients) -> Self {
        Self::with_accounts(config, console, Accounts::from_env())
    }

    /// For tests: accounts in a temporary folder of their own (see
    /// [`Accounts::sandbox`]), so no test reads this computer's Claude Code
    /// sign-in or asks the usage endpoint.
    pub fn sandboxed(config: SharedConfig, console: ConsoleClients) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "tmux-collector-sandbox-{}",
            uuid::Uuid::new_v4().simple()
        ));
        Self::with_accounts(config, console, Accounts::sandbox(&dir))
    }

    pub fn with_accounts(
        config: SharedConfig,
        console: ConsoleClients,
        accounts: Arc<Accounts>,
    ) -> Self {
        Self {
            config,
            binary: BinaryCache::default(),
            scanner: Arc::new(StdMutex::new(Scanner::new())),
            activity: Arc::new(StdMutex::new(ActivityReader::default())),
            procs: Mutex::new(ProcCache::default()),
            agents: Mutex::new(AgentTracker::default()),
            compiled: StdMutex::new(None),
            seen: StdMutex::new(None),
            claude_seen: StdMutex::new(None),
            server_version: StdMutex::new(None),
            accounts,
            console,
            gate: Mutex::new(()),
        }
    }

    /// Claude Code accounts and usage.
    pub fn accounts(&self) -> &Arc<Accounts> {
        &self.accounts
    }

    /// The last collection's Claude Code panes, when it is at most
    /// `max_age` old (any age with `None`).
    fn claude_seen(&self, max_age: Option<Duration>) -> Option<ClaudeSeen> {
        let seen = self.claude_seen.lock().unwrap_or_else(|p| p.into_inner());
        seen.as_ref()
            .filter(|seen| max_age.is_none_or(|max| seen.at.elapsed() < max))
            .cloned()
    }

    /// The panes running Claude Code or carrying an account mark, from a
    /// collection at most [`SEEN_FRESH`] old (`fresh`: made now). Empty when
    /// tmux could not list them; see [`Collector::claude_panes_checked`].
    pub async fn claude_panes(&self, fresh: bool) -> HashMap<String, PaneState> {
        if !fresh {
            if let Some(seen) = self.claude_seen(Some(SEEN_FRESH)) {
                return seen.panes;
            }
        }
        self.overview().await;
        // The collection just made (or a newer one), however long it took.
        self.claude_seen(None)
            .map(|seen| seen.panes)
            .unwrap_or_default()
    }

    /// [`Collector::claude_panes`] from a collection made now, or why tmux
    /// could not list its panes: for what must not take a failed look for
    /// "no Claude Code" (deleting an account's folder, dropping waiting
    /// moves).
    pub async fn claude_panes_checked(&self) -> Result<HashMap<String, PaneState>, String> {
        let overview = self.overview().await;
        match self.claude_seen(None) {
            Some(seen) if seen.listed => Ok(seen.panes),
            _ => Err(overview
                .sessions
                .tmux
                .error
                .unwrap_or_else(|| "tmux did not list its panes".to_string())),
        }
    }

    pub fn config(&self) -> Arc<WorkerConfig> {
        self.config.load_full()
    }

    /// The agent running in `pane`, from a collection at most
    /// [`SEEN_FRESH`] old.
    pub async fn pane_agent(&self, pane: &str) -> Option<AgentStatus> {
        let fresh = |collector: &Self| {
            let seen = collector.seen.lock().unwrap_or_else(|p| p.into_inner());
            match seen.as_ref() {
                Some((at, agents)) if at.elapsed() < SEEN_FRESH => Some(agents.get(pane).cloned()),
                _ => None,
            }
        };
        if let Some(agent) = fresh(self) {
            return agent;
        }
        self.overview().await;
        fresh(self).flatten()
    }

    /// The process table of the last scan.
    pub async fn proc_table(&self) -> Arc<ProcTable> {
        self.procs.lock().await.table.clone()
    }

    /// The configured tmux, or why there is none. Its version is the
    /// running server's when a collection read it (what the server accepts
    /// decides, e.g. pane options from 3.0), else the client's.
    pub async fn tmux(&self) -> Result<Tmux, String> {
        let config = self.config();
        let resolved = self.binary.get(&config.tmux_binary).await?;
        let server = self
            .server_version
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        Ok(Tmux {
            path: resolved.path,
            socket: Socket::from_config(&config),
            timeout: Duration::from_millis(config.command_timeout_ms),
            version: server.unwrap_or(resolved.version),
        })
    }

    /// The resolved tmux's version as `-V` printed it (`3.6`, `next-3.7`).
    pub async fn tmux_version(&self) -> Option<String> {
        let config = self.config();
        self.binary
            .get(&config.tmux_binary)
            .await
            .ok()
            .map(|resolved| resolved.version)
    }

    /// Forget what is known and look for tmux again: an install just changed
    /// what's there. Only `-V`, which talks to no server.
    pub async fn resolve_tmux_again(&self) -> Result<Resolved, String> {
        self.binary.forget();
        self.binary.get(&self.config().tmux_binary).await
    }

    pub async fn info(&self) -> TmuxInfo {
        let config = self.config();
        let resolved = match self.binary.get(&config.tmux_binary).await {
            Ok(resolved) => resolved,
            Err(error) => {
                return TmuxInfo {
                    error: Some(error),
                    installable: installable(&config.tmux_binary),
                    ..TmuxInfo::default()
                }
            }
        };
        let tmux = Tmux {
            path: resolved.path.clone(),
            socket: Socket::from_config(&config),
            timeout: Duration::from_millis(config.command_timeout_ms),
            version: resolved.version.clone(),
        };
        let mut info = TmuxInfo {
            available: true,
            path: Some(resolved.path.to_string_lossy().into_owned()),
            version: Some(resolved.version),
            ..TmuxInfo::default()
        };
        match tmux.run(["display-message", "-p", "#{socket_path}"]).await {
            Ok(output) => {
                info.server_running = true;
                info.socket_path = Some(output.trim().to_string()).filter(|path| !path.is_empty());
            }
            Err(TmuxError::NoServer) => {}
            Err(error) => info.error = Some(error.to_string()),
        }
        info
    }

    fn agent_set(&self, config: &Arc<WorkerConfig>) -> Arc<AgentSet> {
        let mut compiled = self.compiled.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(current) = compiled.as_ref().filter(|c| Arc::ptr_eq(&c.config, config)) {
            return current.set.clone();
        }
        // The configuration was validated before it was stored, so this
        // only falls back on a value that bypassed validation.
        let set = Arc::new(AgentSet::compile(&config.agents).unwrap_or_else(|error| {
            tracing::error!(%error, "agent matchers invalid; agent detection disabled");
            AgentSet::default()
        }));
        *compiled = Some(CompiledAgents {
            config: config.clone(),
            set: set.clone(),
        });
        set
    }

    /// Re-scan processes and sockets when stale or when the panes changed.
    async fn refresh_procs(&self, config: &WorkerConfig, pane_signature: u64) -> ProcSnapshot {
        let mut cache = self.procs.lock().await;
        let stale = cache
            .scanned_at
            .is_none_or(|at| at.elapsed() >= Duration::from_millis(config.ports_interval_ms));
        if stale || cache.pane_signature != pane_signature {
            let scanner = self.scanner.clone();
            let scanned = tokio::task::spawn_blocking(move || {
                let mut scanner = scanner.lock().unwrap_or_else(|p| p.into_inner());
                let table = scanner.processes();
                let listeners = scanner.listeners(&table);
                let candidates =
                    compose::env_candidates(&table, listeners.as_deref().unwrap_or_default());
                let env = scanner.iii_env(&candidates);
                (table, listeners, env)
            })
            .await;
            match scanned {
                Ok((table, listeners, env)) => {
                    cache.table = Arc::new(table);
                    match listeners {
                        Ok(listeners) => {
                            cache.listeners = Arc::new(listeners);
                            cache.error = None;
                        }
                        Err(error) => {
                            cache.listeners = Arc::new(Vec::new());
                            cache.error = Some(error);
                        }
                    }
                    cache.env = Arc::new(env);
                }
                Err(error) => cache.error = Some(format!("process scan failed: {error}")),
            }
            cache.scanned_at = Some(Instant::now());
            cache.pane_signature = pane_signature;
        }
        ProcSnapshot {
            table: cache.table.clone(),
            listeners: cache.listeners.clone(),
            env: cache.env.clone(),
            error: cache.error.clone(),
        }
    }

    pub async fn overview(&self) -> Overview {
        let _gate = self.gate.lock().await;
        let config = self.config();
        let generated_at_ms = now_ms();

        let mut info = TmuxInfo::default();
        let mut raw = RawSnapshot::default();
        let mut listed = true;
        let tmux = match self.tmux().await {
            Ok(mut tmux) => {
                info.available = true;
                info.path = Some(tmux.path.to_string_lossy().into_owned());
                info.version = self
                    .binary
                    .get(&config.tmux_binary)
                    .await
                    .ok()
                    .map(|r| r.version);
                match tmux.run(list_all_args()).await {
                    Ok(output) => {
                        raw = parse_list_all(&output);
                        info.server_running = true;
                        info.socket_path =
                            raw.server.as_ref().map(|server| server.socket_path.clone());
                        let server_version = raw
                            .server
                            .as_ref()
                            .map(|server| server.version.clone())
                            .filter(|version| !version.is_empty());
                        if let Some(version) = &server_version {
                            tmux.version = version.clone();
                        }
                        *self
                            .server_version
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = server_version;
                    }
                    Err(TmuxError::NoServer) => {
                        *self
                            .server_version
                            .lock()
                            .unwrap_or_else(|p| p.into_inner()) = None;
                    }
                    Err(error) => {
                        listed = false;
                        info.error = Some(error.to_string());
                    }
                }
                Some(tmux)
            }
            Err(error) => {
                info.error = Some(error);
                info.installable = installable(&config.tmux_binary);
                None
            }
        };

        let procs = self.refresh_procs(&config, pane_signature(&raw)).await;
        let panes = pane_index(&raw);
        let locate = |pid: u32| -> Option<PaneRef> {
            procs
                .table
                .lineage(pid)
                .into_iter()
                .find_map(|ancestor| panes.get(&ancestor).cloned())
        };

        let listeners: Vec<Listener> = procs
            .listeners
            .iter()
            .map(|socket| {
                let (role, worker, compose) =
                    compose::classify_owner(&procs.table, socket.pid, &procs.env);
                Listener {
                    port: socket.port,
                    address: socket.address.clone(),
                    family: socket.family,
                    pid: socket.pid,
                    process: procs
                        .table
                        .get(socket.pid)
                        .map(|p| p.name.clone())
                        .unwrap_or_default(),
                    worker,
                    compose,
                    role,
                    location: locate(socket.pid),
                }
            })
            .collect();
        let mut composes: Vec<ComposeStack> = compose::stacks(
            &procs.table,
            &compose::ports_by_pid(&procs.listeners),
            &procs.env,
        );
        for stack in &mut composes {
            stack.location = locate(stack.pid);
        }

        let agents = self.agent_set(&config);
        let (statuses, records) = match &tmux {
            Some(tmux) => {
                self.agent_statuses(tmux, &raw, procs.table.clone(), &agents, &config)
                    .await
            }
            None => (HashMap::new(), HashMap::new()),
        };
        // tmux before 3.0 keeps the account marks per window (D4).
        let window_marks = tmux.as_ref().is_some_and(|tmux| tmux.pane_scope() == "-w");
        let claude = self.claude_states(&raw, &statuses, &records, &procs.table, window_marks);
        let counts = crate::accounts::panes::counts(&claude);
        let infos: HashMap<String, PaneClaude> = claude
            .iter()
            .map(|(id, state)| (id.clone(), state.info.clone()))
            .collect();
        *self.claude_seen.lock().unwrap_or_else(|p| p.into_inner()) = Some(ClaudeSeen {
            at: Instant::now(),
            listed,
            panes: claude,
        });
        // Sign-ins and profiles are read in the background (M2).
        self.accounts.load_soon().await;
        let usage = self.accounts.snapshot(&counts);
        let sessions = build_sessions(
            &raw,
            &statuses,
            &infos,
            &composes,
            &listeners,
            &self.console,
        );

        Overview {
            sessions: SessionsSnapshot {
                tmux: info,
                sessions,
                generated_at_ms,
            },
            ports: PortsSnapshot {
                supported: procs::PORTS_SUPPORTED,
                error: procs.error,
                composes,
                listeners,
                generated_at_ms,
            },
            usage: Some(usage),
        }
    }

    /// Each pane that runs Claude Code or carries an account mark. A pane
    /// running another agent (Codex) gets no account (BadgeStyles).
    /// `window_marks`: tmux keeps the marks per window (before 3.0), so
    /// two Claude Code panes in one window can't have marks of their own.
    fn claude_states(
        &self,
        raw: &RawSnapshot,
        statuses: &HashMap<String, AgentStatus>,
        records: &HashMap<String, ClaudeRecord>,
        table: &ProcTable,
        window_marks: bool,
    ) -> HashMap<String, PaneState> {
        let registry = self.accounts.registry();
        // Distinct panes: a window linked into two sessions lists its
        // panes once per session.
        let mut claude_per_window: HashMap<&str, HashSet<&str>> = HashMap::new();
        for pane in &raw.panes {
            if statuses
                .get(&pane.id)
                .is_some_and(|agent| agent.id == "claude")
            {
                claude_per_window
                    .entry(pane.window_id.as_str())
                    .or_default()
                    .insert(pane.id.as_str());
            }
        }
        let alive: std::collections::HashSet<&str> =
            raw.panes.iter().map(|pane| pane.id.as_str()).collect();
        let move_errors = self.accounts.move_errors(&alive);
        let mut out = HashMap::new();
        for pane in &raw.panes {
            let marks = Marks::from_raw(&pane.claude_mark, pane.skip_mark, &pane.pending_mark);
            let agent = statuses.get(&pane.id);
            let claude = match agent {
                Some(agent) if agent.id == "claude" => Some(ClaudeProc {
                    pid: agent.pid,
                    record: records.get(&pane.id),
                }),
                Some(_) => continue,
                None => None,
            };
            // Before tmux 3.0 a pane's marks are its window's: another pane
            // in a window that runs Claude Code has that one's marks, not
            // marks of its own, and is left out (a move asked on it, or
            // its look at a pending move, would act on the other pane).
            if window_marks
                && claude.is_none()
                && claude_per_window.contains_key(pane.window_id.as_str())
            {
                continue;
            }
            let claude_pid = claude.as_ref().map(|c| c.pid);
            if marks.pending.is_some() {
                self.accounts.note_waiting(&pane.id, claude_pid);
            }
            let marks_shared = window_marks
                && claude_per_window
                    .get(pane.window_id.as_str())
                    .is_some_and(|panes| panes.len() > 1);
            let Some((mut info, place)) =
                pane_claude(&self.accounts, &registry, table, pane.pid, &marks, claude)
            else {
                continue;
            };
            if marks_shared {
                info.movable = false;
                info.not_movable_reason = Some(crate::accounts::panes::SHARED_MARKS.to_string());
            }
            info.move_error = move_errors.get(&pane.id).cloned();
            out.insert(
                pane.id.clone(),
                PaneState {
                    pane_id: pane.id.clone(),
                    pane_pid: pane.pid,
                    claude_pid,
                    record: records.get(&pane.id).cloned(),
                    marks,
                    info,
                    agent_state: agent.map(|a| a.state),
                    cwd: pane.cwd.clone(),
                    place,
                    kind: claude_pid.and_then(|pid| {
                        crate::accounts::panes::move_kind(table, pane.pid, pid).ok()
                    }),
                    marks_shared,
                },
            );
        }
        out
    }

    async fn agent_statuses(
        &self,
        tmux: &Tmux,
        raw: &RawSnapshot,
        table: Arc<ProcTable>,
        agents: &AgentSet,
        config: &WorkerConfig,
    ) -> (HashMap<String, AgentStatus>, HashMap<String, ClaudeRecord>) {
        let found: Vec<(String, usize, u32, String)> = raw
            .panes
            .iter()
            .filter(|pane| !pane.dead)
            .filter_map(|pane| {
                let (index, pid) = agents.find(&table, pane.pid)?;
                Some((pane.id.clone(), index, pid, pane.title.clone()))
            })
            .collect();

        // The on-disk records are read while the captures run.
        let recorded: Vec<(String, Recorder, u32)> = found
            .iter()
            .filter_map(|(pane_id, index, pid, _)| {
                let recorder = Recorder::of(&agents.get(*index)?.id)?;
                Some((pane_id.clone(), recorder, *pid))
            })
            .collect();
        let reader = self.activity.clone();
        let activity = tokio::task::spawn_blocking(move || {
            reader
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .read(&table, &recorded)
        });

        let mut captures = tokio::task::JoinSet::new();
        for (pane_id, index, pid, title) in found {
            let tmux = tmux.clone();
            captures.spawn(async move {
                let text = tmux
                    .run(["capture-pane", "-p", "-t", pane_id.as_str()])
                    .await;
                (pane_id, index, pid, title, text)
            });
        }
        let mut activity = activity.await.unwrap_or_else(|error| {
            tracing::warn!(%error, "reading agent activity failed");
            HashMap::new()
        });
        let now = now_ms();
        let mut tracker = self.agents.lock().await;
        let mut out = HashMap::new();
        let mut records = HashMap::new();
        while let Some(joined) = captures.join_next().await {
            let Ok((pane_id, index, pid, pane_title, text)) = joined else {
                continue;
            };
            let Some(agent) = agents.get(index) else {
                continue;
            };
            let mut recorded = activity.remove(&pane_id).unwrap_or_default();
            if let Some(record) = recorded.claude.take() {
                records.insert(pane_id.clone(), record);
            }
            let (state, since_ms) = match &text {
                Ok(text) => tracker.observe(
                    &pane_id,
                    agent,
                    pid,
                    text,
                    recorded.state,
                    now,
                    config.busy_window_ms,
                ),
                // The screen could not be read this time, but the process
                // is there (a waiting move must not take it for gone): the
                // state stays what it was, else what the agent records,
                // else working (never idle without evidence).
                Err(_) => tracker
                    .last(&pane_id, &agent.id, pid)
                    .unwrap_or((recorded.state.unwrap_or(AgentState::Working), now)),
            };
            out.insert(
                pane_id,
                AgentStatus {
                    id: agent.id.clone(),
                    label: agent.label.clone(),
                    pid,
                    state,
                    since_ms,
                    title: session_title(agent, &pane_title, host_names()),
                    subagents: recorded.subagents,
                },
            );
        }
        tracker.retain(&out.keys().cloned().collect());
        *self.seen.lock().unwrap_or_else(|p| p.into_inner()) = Some((Instant::now(), out.clone()));
        (out, records)
    }
}

/// The Claude Code panes of one collection.
#[derive(Clone)]
struct ClaudeSeen {
    at: Instant,
    /// tmux listed its panes (or no server runs).
    listed: bool,
    panes: HashMap<String, PaneState>,
}

struct ProcSnapshot {
    table: Arc<ProcTable>,
    listeners: Arc<Vec<SocketListener>>,
    env: Arc<HashMap<u32, IiiEnv>>,
    error: Option<String>,
}

/// Changes when a pane appears, goes, or runs a different foreground process.
fn pane_signature(raw: &RawSnapshot) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for pane in &raw.panes {
        (&pane.id, pane.pid, &pane.command).hash(&mut hasher);
    }
    hasher.finish()
}

/// `pane_pid` → where that pane is.
fn pane_index(raw: &RawSnapshot) -> HashMap<u32, PaneRef> {
    let sessions: HashMap<&str, &str> = raw
        .sessions
        .iter()
        .map(|session| (session.id.as_str(), session.name.as_str()))
        .collect();
    let windows: HashMap<(&str, &str), u32> = raw
        .windows
        .iter()
        .map(|window| {
            (
                (window.session_id.as_str(), window.id.as_str()),
                window.index,
            )
        })
        .collect();
    raw.panes
        .iter()
        .filter_map(|pane| {
            Some((
                pane.pid,
                PaneRef {
                    session: sessions.get(pane.session_id.as_str())?.to_string(),
                    session_id: pane.session_id.clone(),
                    window_index: *windows
                        .get(&(pane.session_id.as_str(), pane.window_id.as_str()))?,
                    window_id: pane.window_id.clone(),
                    pane_id: pane.id.clone(),
                },
            ))
        })
        .collect()
}

fn build_sessions(
    raw: &RawSnapshot,
    agents: &HashMap<String, AgentStatus>,
    claude: &HashMap<String, PaneClaude>,
    composes: &[ComposeStack],
    listeners: &[Listener],
    console: &ConsoleClients,
) -> Vec<Session> {
    let pane_of = |location: &Option<PaneRef>| location.as_ref().map(|l| l.pane_id.clone());
    let mut sessions: Vec<Session> = raw
        .sessions
        .iter()
        .map(|session| {
            let mut windows: Vec<Window> = raw
                .windows
                .iter()
                .filter(|window| window.session_id == session.id)
                .map(|window| {
                    let mut panes: Vec<Pane> = raw
                        .panes
                        .iter()
                        .filter(|pane| pane.session_id == session.id && pane.window_id == window.id)
                        .map(|pane| Pane {
                            id: pane.id.clone(),
                            index: pane.index,
                            active: pane.active,
                            pid: pane.pid,
                            command: pane.command.clone(),
                            cwd: pane.cwd.clone(),
                            title: pane.title.clone(),
                            width: pane.width,
                            height: pane.height,
                            dead: pane.dead,
                            agent: agents.get(&pane.id).cloned(),
                            compose: composes
                                .iter()
                                .find(|stack| {
                                    pane_of(&stack.location).as_deref() == Some(pane.id.as_str())
                                })
                                .map(|stack| ComposeRef {
                                    namespace: stack.namespace.clone(),
                                    pid: stack.pid,
                                }),
                            ports: listeners
                                .iter()
                                .filter(|listener| {
                                    pane_of(&listener.location).as_deref() == Some(pane.id.as_str())
                                })
                                .cloned()
                                .collect(),
                            claude: claude.get(&pane.id).cloned(),
                        })
                        .collect();
                    panes.sort_by_key(|pane| pane.index);
                    Window {
                        id: window.id.clone(),
                        index: window.index,
                        name: window.name.clone(),
                        active: window.active,
                        activity_at_ms: window.activity * 1000,
                        bell: window.bell,
                        zoomed: window.zoomed,
                        panes,
                    }
                })
                .collect();
            windows.sort_by_key(|window| window.index);
            let console_attached = raw
                .clients
                .iter()
                .filter(|client| client.session_id == session.id && console.contains(&client.tty))
                .count() as u32;
            Session {
                id: session.id.clone(),
                name: session.name.clone(),
                created_at_ms: session.created * 1000,
                activity_at_ms: session.activity * 1000,
                attached: session.attached,
                console_attached,
                windows,
            }
        })
        .collect();
    sessions.sort_by(|a, b| a.name.cmp(&b.name));
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AddressFamily, ListenerRole};
    use crate::tmux_cli::{RawClient, RawPane, RawSession, RawWindow};

    fn collector_for(binary: &str, found: Result<Resolved, String>) -> Collector {
        let config = WorkerConfig {
            tmux_binary: binary.to_string(),
            ..WorkerConfig::default()
        };
        let collector = Collector::new(config.into_shared(), ConsoleClients::default());
        collector.binary.seed(binary, found);
        collector
    }

    // Only where an official build exists is the answer known without looking
    // for a package manager (simple-installs D52).
    #[cfg(all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    #[tokio::test]
    async fn a_missing_plain_tmux_is_installable_everywhere_it_is_reported() {
        let collector = collector_for("tmux", Err("not installed".to_string()));
        let info = collector.info().await;
        assert!(!info.available);
        assert!(info.installable);
        let overview = collector.overview().await;
        assert!(!overview.sessions.tmux.available);
        assert!(overview.sessions.tmux.installable);
    }

    #[tokio::test]
    async fn a_configured_path_that_is_missing_is_not_installable() {
        let collector = collector_for("/opt/tmux/bin/tmux", Err("not installed".to_string()));
        assert!(!collector.info().await.installable);
        assert!(!collector.overview().await.sessions.tmux.installable);
    }

    #[tokio::test]
    async fn a_tmux_that_is_there_is_not_installable() {
        // A script, never a real tmux: this reads the default socket.
        let script = std::env::temp_dir().join(format!(
            "iii-tmux-collector-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let collector = collector_for(
            "tmux",
            Ok(Resolved {
                path: script.clone(),
                version: "3.7c".to_string(),
            }),
        );
        let info = collector.info().await;
        assert!(info.available);
        assert!(!info.installable);
        let _ = std::fs::remove_file(script);
    }

    fn raw() -> RawSnapshot {
        RawSnapshot {
            sessions: vec![
                RawSession {
                    id: "$2".into(),
                    name: "work".into(),
                    created: 10,
                    activity: 20,
                    attached: 2,
                },
                RawSession {
                    id: "$1".into(),
                    name: "api".into(),
                    created: 5,
                    activity: 6,
                    attached: 0,
                },
            ],
            windows: vec![
                RawWindow {
                    session_id: "$2".into(),
                    id: "@5".into(),
                    index: 1,
                    name: "b".into(),
                    active: false,
                    activity: 30,
                    bell: false,
                    zoomed: false,
                },
                RawWindow {
                    session_id: "$2".into(),
                    id: "@4".into(),
                    index: 0,
                    name: "a".into(),
                    active: true,
                    activity: 40,
                    bell: true,
                    zoomed: false,
                },
                RawWindow {
                    session_id: "$1".into(),
                    id: "@1".into(),
                    index: 0,
                    name: "srv".into(),
                    active: true,
                    activity: 7,
                    bell: false,
                    zoomed: false,
                },
            ],
            panes: vec![
                RawPane {
                    session_id: "$2".into(),
                    window_id: "@4".into(),
                    id: "%9".into(),
                    index: 0,
                    active: true,
                    pid: 900,
                    command: "claude".into(),
                    cwd: "/w".into(),
                    width: 80,
                    height: 24,
                    dead: false,
                    claude_mark: String::new(),
                    skip_mark: false,
                    pending_mark: String::new(),
                    title: "✳ x".into(),
                },
                RawPane {
                    session_id: "$1".into(),
                    window_id: "@1".into(),
                    id: "%1".into(),
                    index: 0,
                    active: true,
                    pid: 100,
                    command: "iii:c:default".into(),
                    cwd: "/p".into(),
                    width: 80,
                    height: 24,
                    dead: false,
                    claude_mark: String::new(),
                    skip_mark: false,
                    pending_mark: String::new(),
                    title: "".into(),
                },
            ],
            clients: vec![
                RawClient {
                    tty: "/dev/pts/7".into(),
                    session_id: "$2".into(),
                    pid: 1,
                },
                RawClient {
                    tty: "/dev/pts/3".into(),
                    session_id: "$2".into(),
                    pid: 2,
                },
            ],
            server: None,
        }
    }

    #[test]
    fn sessions_are_assembled_sorted_with_statuses() {
        let raw = raw();
        let console = ConsoleClients::default();
        console.insert("/dev/pts/7");
        let location = pane_index(&raw).get(&100).cloned();
        let composes = vec![ComposeStack {
            namespace: "default".into(),
            pid: 100,
            project_dir: None,
            compose_file: None,
            engine: None,
            workers: vec![],
            location: location.clone(),
        }];
        let listeners = vec![Listener {
            port: 3111,
            address: "0.0.0.0".into(),
            family: AddressFamily::Ipv4,
            pid: 102,
            process: "http".into(),
            worker: Some("http".into()),
            compose: Some("default".into()),
            role: ListenerRole::Worker,
            location,
        }];
        let agents = HashMap::from([(
            "%9".to_string(),
            AgentStatus {
                id: "claude".into(),
                label: "Claude Code".into(),
                pid: 901,
                state: AgentState::Working,
                since_ms: 1,
                title: Some("Fix login".into()),
                subagents: vec![],
            },
        )]);
        let sessions = build_sessions(
            &raw,
            &agents,
            &HashMap::new(),
            &composes,
            &listeners,
            &console,
        );
        assert_eq!(
            sessions.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["api", "work"]
        );
        let api = &sessions[0];
        assert_eq!(
            api.windows[0].panes[0].compose.as_ref().unwrap().namespace,
            "default"
        );
        assert_eq!(api.windows[0].panes[0].ports[0].port, 3111);
        let work = &sessions[1];
        assert_eq!((work.attached, work.console_attached), (2, 1));
        assert_eq!(
            work.windows.iter().map(|w| w.index).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(work.windows[0].activity_at_ms, 40_000);
        assert_eq!(
            work.windows[0].panes[0].agent.as_ref().unwrap().state,
            AgentState::Working
        );
    }

    #[test]
    fn pane_signature_follows_foreground_commands() {
        let mut raw = raw();
        let before = pane_signature(&raw);
        raw.panes[0].command = "zsh".into();
        assert_ne!(before, pane_signature(&raw));
    }
}
