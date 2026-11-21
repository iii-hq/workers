//! One task per configured server: connect, list, register, call; and the
//! wire types of `servers::list`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::path::PathBuf;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::future::join_all;
use http::{HeaderName, HeaderValue};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::service::{
    ClientCacheConfig, ClientInitializeError, NotificationContext, RunningService,
};
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, InsufficientScopeError, StreamableHttpClientTransportConfig,
    StreamableHttpError,
};
use rmcp::transport::{DynamicTransportError, StreamableHttpClientTransport};
use rmcp::{ClientHandler, Peer, RoleClient, ServiceError, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};

use crate::approvals::{APPROVALS_REF, APPROVALS_SECRET, Approvals, is_fingerprint};
use crate::config::{
    Auth, Checked, Entry, HttpEntry, RawConfig, STDIO_OFF, StdioEntry, Transport, check_entry,
    check_url_text, fingerprint,
};
use crate::functions::SecretChange;
use crate::host::{EventKind, Handler, Host, ServerEvent};
use crate::reconcile::{
    Registry, Skipped, Target, ToolInfo, Wanted, WantedSet, function_id, tool_wanted,
};
use crate::refs::{Ref, Resolved, Store, Text, event_matches, ref_error, resolve_all};
use crate::stdio::{self, ChildGuard, LogLine, RestartPolicy, StderrRing};
use crate::translate::{
    BlockCtx, CallFailure, ServerSecrets, failure_text, scrub, strip_args, tool_result,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Disabled,
    ConfigError,
    Unapproved,
    Connecting,
    Ready,
    Unavailable,
    AuthFailed,
    NeedsLogin,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Disabled => "disabled",
            State::ConfigError => "config_error",
            State::Unapproved => "unapproved",
            State::Connecting => "connecting",
            State::Ready => "ready",
            State::Unavailable => "unavailable",
            State::AuthFailed => "auth_failed",
            State::NeedsLogin => "needs_login",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Config,
    Approval,
    Secret,
    Connect,
    Authorize,
    Tools,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidConfig,
    StdioDisabled,
    Unapproved,
    SecretMissing,
    SecretNotShared,
    SpawnFailed,
    Exited,
    ConnectFailed,
    HandshakeFailed,
    CredentialRejected,
    LoginRequired,
    RateLimited,
    ListFailed,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ServerError {
    pub code: ErrorCode,
    pub message: String,
}

impl ServerError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        ServerError {
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Timings {
    pub http_handshake: Duration,
    pub stdio_start: Duration,
    pub list: Duration,
    pub call: Duration,
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    pub budget_starts: u32,
    pub budget_window: Duration,
    pub cooldown: Duration,
    pub stable_after: Duration,
    pub stop_grace: Duration,
    pub relist_min_interval: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        let s = Duration::from_secs;
        Timings {
            http_handshake: s(30),
            stdio_start: s(60),
            list: s(30),
            call: s(120),
            backoff_min: s(1),
            backoff_max: s(60),
            budget_starts: 5,
            budget_window: s(600),
            cooldown: s(300),
            stable_after: s(60),
            stop_grace: s(5),
            relist_min_interval: s(60),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ManagerOptions {
    pub stdio_enabled: bool,
    pub timings: Timings,
    pub worker_cwd: PathBuf,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ServersList {
    pub stdio_enabled: bool,
    pub callback_url: String,
    pub servers: Vec<ServerView>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ServerView {
    pub name: String,
    /// `http` or `stdio`.
    pub transport: String,
    pub target: String,
    pub auth: String,
    pub state: State,
    pub stage: Option<Stage>,
    /// Unix milliseconds of the last state change.
    pub since: u64,
    pub error: Option<ServerError>,
    pub approved: bool,
    pub fingerprint: String,
    pub protocol_version: Option<String>,
    pub server_info: Option<ServerInfoView>,
    pub tools_listed: bool,
    pub tools: Vec<ToolView>,
    pub skipped: Vec<Skipped>,
    pub has_instructions: bool,
    pub capabilities: CapabilitiesView,
    pub resource_functions: Vec<ResourceFnView>,
    pub prompts: Vec<PromptView>,
    pub list_errors: ListErrors,
    pub login: Option<LoginView>,
    pub process: Option<ProcessView>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ServerInfoView {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ToolView {
    pub name: String,
    pub function_id: String,
    pub registered: bool,
    pub annotations: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct CapabilitiesView {
    pub resources: bool,
    pub prompts: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ResourceFnView {
    /// `list`, `read` or `templates`.
    pub name: String,
    pub function_id: String,
    pub registered: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PromptView {
    pub name: String,
    pub function_id: String,
    pub registered: bool,
    pub title: Option<String>,
    pub arguments: u32,
    pub required: u32,
}

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct ListErrors {
    pub prompts: Option<String>,
    pub templates: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LoginView {
    pub pending: bool,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProcessView {
    pub pid: Option<u32>,
    pub started_at: Option<u64>,
    pub restarts: u32,
    pub restart_budget: u32,
    pub next_restart_at: Option<u64>,
    pub last_exit: Option<ExitInfo>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// A stdio server's stderr tail, redacted when it was read.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Logs {
    pub lines: Vec<LogLine>,
    /// Lines pushed out of the ring.
    pub dropped: u64,
}

/// What `ProcessView` shows, and the policy that spaces the starts.
struct ProcessFacts {
    policy: RestartPolicy,
    pid: Option<u32>,
    started_at: Option<u64>,
    next_restart_at: Option<u64>,
    last_exit: Option<ExitInfo>,
}

/// The wait between two failed attempts: `min`, doubling, capped at `max`.
pub struct Backoff {
    min: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    pub fn new(min: Duration, max: Duration) -> Self {
        Backoff {
            min,
            max,
            next: min,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Duration {
        let wait = self.next.min(self.max);
        self.next = wait.saturating_mul(2);
        wait
    }

    pub fn reset(&mut self) {
        self.next = self.min;
    }
}

/// A `Retry-After` value given as a number of seconds; an HTTP date is `None`.
pub fn retry_after_secs(value: Option<&str>) -> Option<Duration> {
    let value = value?.trim();
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().map(Duration::from_secs)
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn client_config() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("iii-mcp-client", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
}

type Session = RunningService<RoleClient, Notifier>;

type HttpError = StreamableHttpError<reqwest::Error>;

/// The list a server said changed.
enum ListChanged {
    Tools,
    Prompts,
    Resources,
}

/// The rmcp handler of a session: hands list-changed notifications to the
/// server task.
struct Notifier {
    info: ClientConfig,
    tx: mpsc::UnboundedSender<ListChanged>,
    /// rmcp's session task holds this until its last log line (the session
    /// DELETE), which may come after the server itself is gone.
    _secrets: Arc<ServerSecrets>,
}

impl ClientHandler for Notifier {
    fn get_info(&self) -> ClientConfig {
        self.info.clone()
    }

    async fn on_tool_list_changed(&self, _: NotificationContext<RoleClient>) {
        let _ = self.tx.send(ListChanged::Tools);
    }

    async fn on_prompt_list_changed(&self, _: NotificationContext<RoleClient>) {
        let _ = self.tx.send(ListChanged::Prompts);
    }

    async fn on_resource_list_changed(&self, _: NotificationContext<RoleClient>) {
        let _ = self.tx.send(ListChanged::Resources);
    }
}

/// A stdio server's child and the task reading its stderr.
struct Process {
    child: ChildGuard,
    reader: Reader,
}

/// The stderr reader of one child; aborted when dropped.
struct Reader {
    task: JoinHandle<()>,
    /// The ring's push count when the child started: older lines are not its.
    from: u64,
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A ready session, and for stdio the process behind it.
struct Live {
    session: Session,
    process: Option<Process>,
}

/// The exit of the session's process; never, for http.
async fn exited(process: &mut Option<Process>) -> ExitInfo {
    match process {
        Some(p) => p.child.wait().await,
        None => std::future::pending().await,
    }
}

fn exit_text(exit: &ExitInfo) -> String {
    match (exit.code, exit.signal) {
        (Some(code), _) => format!("exited with code {code}"),
        (None, Some(signal)) => format!("killed by signal {signal}"),
        (None, None) => "exited".into(),
    }
}

/// A failed handshake's text, with nothing a response body could put in it.
fn init_text(e: &ClientInitializeError) -> String {
    match e {
        ClientInitializeError::JsonRpcError(d) => {
            format!("MCP error {}: {}", d.code.0, d.message)
        }
        ClientInitializeError::TransportError { error, .. } => transport_text(error),
        // Their text prints what the server sent.
        ClientInitializeError::ExpectedInitResponse(_)
        | ClientInitializeError::ExpectedInitResult(_) => "unexpected response".into(),
        _ => e.to_string(),
    }
}

/// The reference a 401 blames: the bearer token, else the first secret header.
fn credential_ref(http: &HttpEntry) -> Option<String> {
    if let Auth::Bearer { token } = &http.auth {
        return Some(token.to_string());
    }
    http.headers.values().find_map(|v| match v {
        Text::Ref(r) => Some(r.to_string()),
        Text::Literal(_) => None,
    })
}

/// The wait a 429 at connect asks for.
fn retry_wait(e: &ClientInitializeError) -> Option<Duration> {
    let ClientInitializeError::TransportError { error, .. } = e else {
        return None;
    };
    match error.error.downcast_ref::<HttpError>() {
        Some(HttpError::HttpStatus(s)) if s.status == 429 => {
            retry_after_secs(s.retry_after.as_deref())
        }
        _ => None,
    }
}

/// A transport error's text with nothing a response body could have put in
/// it: an unexpected response keeps only its `HTTP NNN Reason` prefix.
fn transport_text(e: &DynamicTransportError) -> String {
    match e.error.downcast_ref::<HttpError>() {
        Some(HttpError::UnexpectedServerResponse(t)) if t.starts_with("HTTP ") => {
            t.split(": ").next().unwrap_or_default().to_string()
        }
        Some(HttpError::UnexpectedServerResponse(_) | HttpError::Deserialize(_)) => {
            "unexpected response".into()
        }
        Some(other) => other.to_string(),
        None => e.error.to_string(),
    }
}

/// A failed request's text, as body-free as `transport_text`.
fn service_text(error: &ServiceError) -> String {
    match error {
        ServiceError::TransportSend(e) => transport_text(e),
        other => other.to_string(),
    }
}

fn classify_call_error(error: &ServiceError, entry: &Entry) -> CallFailure {
    let e = match error {
        ServiceError::McpError(e) => {
            return CallFailure::JsonRpc {
                code: e.code.0,
                message: e.message.to_string(),
            };
        }
        ServiceError::TransportSend(e) => e,
        _ => return CallFailure::Transport(service_text(error)),
    };
    // `HttpStatus` is `#[error(transparent)]`: only this downcast finds a 429.
    let status: Option<u16> = match e.error.downcast_ref::<HttpError>() {
        Some(HttpError::HttpStatus(s)) if s.status == 429 => {
            let retry_after = s.retry_after.clone();
            return CallFailure::RateLimited { retry_after };
        }
        Some(HttpError::HttpStatus(s)) => Some(s.status),
        Some(HttpError::UnexpectedServerResponse(t)) => t
            .strip_prefix("HTTP ")
            .and_then(|t| t.get(..3))
            .and_then(|code| code.parse().ok()),
        _ => None,
    };
    let top: &(dyn Error + 'static) = e.error.as_ref();
    let mut challenged = false;
    for x in std::iter::successors(Some(top), |&x| x.source()) {
        if let Some(s) = x.downcast_ref::<InsufficientScopeError>() {
            let scope = s.required_scope.clone();
            return CallFailure::InsufficientScope { scope };
        }
        challenged |= x.is::<AuthRequiredError>();
    }
    if challenged || status == Some(401) {
        let reference = match &entry.transport {
            Transport::Http(http) => credential_ref(http),
            Transport::Stdio(_) => None,
        };
        return match (reference, challenged) {
            (Some(reference), _) => CallFailure::CredentialRejected { reference },
            (None, true) => CallFailure::LoginRequired,
            (None, false) => CallFailure::NoSignIn,
        };
    }
    match status {
        Some(403) => CallFailure::InsufficientScope { scope: None },
        // A 404 is a lost session: an outage like a 5xx.
        Some(s) if (400..500).contains(&s) && s != 404 => CallFailure::Rejected(transport_text(e)),
        _ => CallFailure::Transport(transport_text(e)),
    }
}

/// Tool name to its annotations, for the tools a listing wants.
fn tool_views(wanted: &WantedSet) -> BTreeMap<String, Option<Value>> {
    wanted
        .values()
        .filter_map(|w| match &w.target {
            Target::Tool { name, .. } => Some((
                name.clone(),
                w.def.metadata.pointer("/mcp/annotations").cloned(),
            )),
            _ => None,
        })
        .collect()
}

#[derive(Debug, Clone)]
struct Status {
    state: State,
    stage: Option<Stage>,
    error: Option<ServerError>,
    since: u64,
}

/// What a handshake and a listing learned.
#[derive(Default)]
struct Info {
    protocol_version: Option<String>,
    server_info: Option<ServerInfoView>,
    instructions: Option<String>,
    capabilities: CapabilitiesView,
    tools_listed: bool,
    /// Tool name to its annotations, for the tools that were not skipped.
    tools: BTreeMap<String, Option<Value>>,
    skipped: Vec<Skipped>,
}

const UNAPPROVED: &str = "this entry is not the one an operator approved: review it on the MCP servers page, or call mcp-client::servers::approve";

/// An entry as configured and as checked.
struct Spec {
    raw: Value,
    checked: Checked,
}

struct Server {
    name: String,
    /// Replaced when the entry changes; the server and its registry stay.
    spec: RwLock<Arc<Spec>>,
    host: Arc<dyn Host>,
    approvals: Arc<Approvals>,
    timings: Timings,
    worker_cwd: PathBuf,
    status: watch::Sender<Status>,
    registry: Registry,
    info: Mutex<Info>,
    /// Replaced in every error this server stores or a handler returns, and
    /// in every log line of the process.
    secrets: Arc<ServerSecrets>,
    /// A no-`auth` entry the server answered with a sign-in challenge.
    challenged: AtomicBool,
    peer: RwLock<Option<Peer<RoleClient>>>,
    /// Each session's `Notifier` holds a clone; the server task reads them.
    notes: mpsc::UnboundedSender<ListChanged>,
    /// Held by the running task, so a restarted task reads the same channel.
    notes_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<ListChanged>>,
    /// A call answered -32602 asks the server task to list the tools again.
    relist: Notify,
    task: Mutex<Option<(oneshot::Sender<()>, JoinHandle<()>)>>,
    /// The stderr of every process this server started, across restarts.
    ring: Arc<Mutex<StderrRing>>,
    facts: Mutex<ProcessFacts>,
}

/// The state an entry starts in, from what its config alone says.
fn first_status(name: &str, checked: &Checked) -> (State, Option<Stage>, Option<ServerError>) {
    let fail = |state, stage, code, message: String| {
        (state, Some(stage), Some(ServerError::new(code, message)))
    };
    // Rule 9 fires only with stdio off; nothing is resolved or spawned.
    if checked.errors.iter().any(|i| i.message == STDIO_OFF) {
        let code = ErrorCode::StdioDisabled;
        return fail(State::ConfigError, Stage::Config, code, STDIO_OFF.into());
    }
    let Some(entry) = &checked.entry else {
        let message = checked
            .errors
            .iter()
            .map(|i| format!("{}: {}", i.path, i.message))
            .collect::<Vec<_>>()
            .join("; ");
        return fail(
            State::ConfigError,
            Stage::Config,
            ErrorCode::InvalidConfig,
            message,
        );
    };
    if entry.disabled {
        return (State::Disabled, None, None);
    }
    match &entry.transport {
        Transport::Stdio(_) => (State::Connecting, Some(Stage::Secret), None),
        Transport::Http(http) => match http.auth {
            Auth::ClientCredentials { .. } => fail(
                State::ConfigError,
                Stage::Authorize,
                ErrorCode::InvalidConfig,
                "`client_credentials` is not supported by this version of mcp-client".into(),
            ),
            Auth::OAuth { .. } => fail(
                State::NeedsLogin,
                Stage::Authorize,
                ErrorCode::LoginRequired,
                failure_text(name, &CallFailure::LoginRequired),
            ),
            _ => (State::Connecting, Some(Stage::Secret), None),
        },
    }
}

impl Server {
    fn start(
        host: &Arc<dyn Host>,
        approvals: &Arc<Approvals>,
        opts: &ManagerOptions,
        name: &str,
        raw: &Value,
    ) -> Arc<Server> {
        let checked = check_entry(name, raw, opts.stdio_enabled);
        let (state, stage, error) = first_status(name, &checked);
        let (notes, notes_rx) = mpsc::unbounded_channel();
        let status = Status {
            state,
            stage,
            error,
            since: now_ms(),
        };
        let server = Arc::new(Server {
            name: name.into(),
            spec: RwLock::new(Arc::new(Spec {
                raw: raw.clone(),
                checked,
            })),
            host: host.clone(),
            approvals: approvals.clone(),
            timings: opts.timings,
            worker_cwd: opts.worker_cwd.clone(),
            status: watch::channel(status).0,
            registry: Registry::new(),
            info: Mutex::default(),
            secrets: ServerSecrets::new(),
            challenged: AtomicBool::default(),
            peer: RwLock::default(),
            notes,
            notes_rx: tokio::sync::Mutex::new(notes_rx),
            relist: Notify::new(),
            task: Mutex::default(),
            ring: Arc::default(),
            facts: Mutex::new(ProcessFacts {
                policy: RestartPolicy::new(&opts.timings),
                pid: None,
                started_at: None,
                next_restart_at: None,
                last_exit: None,
            }),
        });
        server.emit(EventKind::State);
        if state == State::Connecting {
            server.spawn();
        }
        server
    }

    fn spec(&self) -> Arc<Spec> {
        self.spec.read().unwrap().clone()
    }

    fn is_stdio(&self) -> bool {
        let spec = self.spec();
        let entry = spec.checked.entry.as_ref();
        entry.is_some_and(|e| matches!(e.transport, Transport::Stdio(_)))
    }

    fn spawned(&self, pid: Option<u32>) {
        {
            let mut f = self.facts.lock().unwrap();
            (f.pid, f.started_at) = (pid, Some(now_ms()));
        }
        self.emit(EventKind::Process);
    }

    fn ended(&self, exit: &ExitInfo) {
        {
            let mut f = self.facts.lock().unwrap();
            (f.pid, f.last_exit) = (None, Some(exit.clone()));
        }
        self.emit(EventKind::Process);
    }

    /// The wait before the next start, kept as `next_restart_at`. An exit
    /// after `stable_after` of `ready` is not a failed start.
    fn schedule(&self, stable: bool) -> Duration {
        let wait = {
            let mut f = self.facts.lock().unwrap();
            let wait = match stable {
                true => self.timings.backoff_min,
                false => f.policy.on_failed_start(Instant::now()),
            };
            f.next_restart_at = Some(now_ms() + wait.as_millis() as u64);
            wait
        };
        self.emit(EventKind::Process);
        wait
    }

    /// Starts the task, with a fresh backoff.
    fn spawn(self: &Arc<Self>) {
        let (stop, stopped) = oneshot::channel();
        let handle = tokio::spawn(self.clone().run(stopped));
        *self.task.lock().unwrap() = Some((stop, handle));
    }

    /// Stops the task and starts it again on `spec`, or on the entry it has,
    /// with a fresh restart budget. The registry stays and the next listing
    /// reconciles it; only a disabled entry loses its functions.
    async fn restart(self: &Arc<Self>, spec: Option<Spec>) {
        self.halt().await;
        self.facts.lock().unwrap().policy.reset();
        if let Some(spec) = spec {
            self.challenged.store(false, Ordering::Relaxed);
            *self.spec.write().unwrap() = Arc::new(spec);
        }
        let (state, stage, error) = first_status(&self.name, &self.spec().checked);
        if state == State::Disabled {
            self.registry.clear().await;
            *self.info.lock().unwrap() = Info::default();
        }
        self.set(state, stage, error);
        if state == State::Connecting {
            self.spawn();
        }
    }

    fn emit(&self, kind: EventKind) {
        self.host.emit(ServerEvent {
            server: self.name.clone(),
            kind,
        });
    }

    fn set(&self, state: State, stage: Option<Stage>, error: Option<ServerError>) {
        self.set_if(|_| true, state, stage, error);
    }

    /// Sets the status when `from` holds for the current state. A `state`
    /// event goes out only when the state, the stage or the error code changed.
    fn set_if(
        &self,
        from: impl FnOnce(State) -> bool,
        state: State,
        stage: Option<Stage>,
        error: Option<ServerError>,
    ) {
        let error = error.map(|mut e| {
            e.message = scrub(&e.message, &self.secrets.values());
            e
        });
        let logged = error.as_ref().map(|e| e.message.clone());
        let code = |e: &Option<ServerError>| e.as_ref().map(|e| e.code);
        let changed = self.status.send_if_modified(|s| {
            if !from(s.state) {
                return false;
            }
            let changed = s.state != state || s.stage != stage || code(&s.error) != code(&error);
            if s.state != state {
                s.since = now_ms();
            }
            (s.state, s.stage, s.error) = (state, stage, error);
            changed
        });
        if changed {
            if let Some(message) = logged {
                tracing::warn!(
                    "`{}` MCP server is {}: {message}",
                    self.name,
                    state.as_str()
                );
            }
            self.emit(EventKind::State);
        }
    }

    fn fail(&self, state: State, stage: Stage, code: ErrorCode, message: String) {
        self.set(state, Some(stage), Some(ServerError::new(code, message)));
    }

    /// A connect's progress; a retry stays `unavailable` until it is `ready`.
    fn progress(&self, stage: Stage) {
        let state = State::Connecting;
        self.set_if(|s| s == state, state, Some(stage), None);
    }

    /// Leaves `ready`; a server that already left it keeps its status.
    fn leave_ready(&self, state: State, stage: Stage, code: ErrorCode, message: String) {
        let error = Some(ServerError::new(code, message));
        self.set_if(|s| s == State::Ready, state, Some(stage), error);
    }

    /// Connects, serves and reconnects until `stop`. Only `unavailable` is
    /// tried again; every other failure waits for a change from outside.
    async fn run(self: Arc<Self>, mut stop: oneshot::Receiver<()>) {
        let mut notes = self.notes_rx.lock().await;
        let mut backoff = Backoff::new(self.timings.backoff_min, self.timings.backoff_max);
        let stdio = self.is_stdio();
        loop {
            // The connect lists the tools: what came before it is stale.
            while notes.try_recv().is_ok() {}
            let attempt = tokio::select! {
                attempt = self.connect() => attempt,
                _ = &mut stop => return,
            };
            let (wait, stable) = match attempt {
                Ok(live) => match self.serve(live, &mut stop, &mut notes, &mut backoff).await {
                    Some(stable) => (None, Some(stable)),
                    None => return,
                },
                Err(wait) => (wait, None),
            };
            // `connecting` after a failed attempt: the approvals could not be read.
            let (state, stage) = {
                let s = self.status.borrow();
                (s.state, s.stage)
            };
            if !matches!(state, State::Unavailable | State::Connecting) {
                let _ = (&mut stop).await;
                return;
            }
            // A spawn, an exit, and a handshake or first listing that failed
            // are failed starts; a failed approval or secret step is not.
            let start = match stable {
                Some(stable) => Some(stable),
                None => matches!(stage, Some(Stage::Connect | Stage::Tools)).then_some(false),
            };
            // A 429's `Retry-After` as sent: a reconnect or a change cuts it short.
            let wait = match (wait, start) {
                (Some(wait), _) => wait,
                (None, Some(stable)) if stdio => self.schedule(stable),
                _ => backoff.next(),
            };
            tokio::select! {
                _ = sleep(wait) => {}
                _ = &mut stop => return,
            }
            self.facts.lock().unwrap().next_restart_at = None;
        }
    }

    /// Serves a ready session until `stop` (`None`) or until the server
    /// leaves `ready` (whether it was `ready` for `stable_after`), then
    /// closes the session and stops its process.
    async fn serve(
        self: &Arc<Self>,
        live: Live,
        stop: &mut oneshot::Receiver<()>,
        notes: &mut mpsc::UnboundedReceiver<ListChanged>,
        backoff: &mut Backoff,
    ) -> Option<bool> {
        let Live {
            session,
            mut process,
        } = live;
        let cancel = session.cancellation_token();
        let mut ended = pin!(session.waiting());
        let mut status = self.status.subscribe();
        let mut stable = pin!(sleep(self.timings.stable_after));
        let mut stable_seen = false;
        let mut last_relist: Option<Instant> = None;
        let (stopped, open, exit) = loop {
            let relist = tokio::select! {
                _ = &mut *stop => break (true, true, None),
                _ = status.wait_for(|s| s.state != State::Ready) => break (false, true, None),
                _ = &mut ended => {
                    if process.is_none() {
                        let message = format!("the MCP session with {} ended", self.name);
                        let (state, code) = (State::Unavailable, ErrorCode::ConnectFailed);
                        self.leave_ready(state, Stage::Connect, code, message);
                    }
                    break (false, false, None);
                }
                exit = exited(&mut process) => break (false, true, Some(exit)),
                _ = &mut stable, if !stable_seen => {
                    stable_seen = true;
                    backoff.reset();
                    if process.is_some() {
                        self.facts.lock().unwrap().policy.on_stable();
                        self.emit(EventKind::Process);
                    }
                    false
                }
                // Prompts and resources are not served by this version.
                Some(change) = notes.recv() => matches!(change, ListChanged::Tools),
                _ = self.relist.notified() => {
                    let interval = self.timings.relist_min_interval;
                    last_relist.is_none_or(|t| t.elapsed() >= interval)
                }
            };
            if relist {
                // This re-list covers every change queued so far, so a burst
                // costs one; a change that arrives during it gets its own.
                while notes.try_recv().is_ok() {}
                last_relist = Some(Instant::now());
                tokio::select! {
                    _ = self.relist_tools() => {}
                    _ = &mut *stop => break (true, true, None),
                }
            }
        };
        self.peer.write().unwrap().take();
        if open {
            // Closes the child's stdin too. A stdio session ends with its
            // process: waiting for it here would add to the stop grace.
            cancel.cancel();
            if process.is_none() {
                let _ = timeout(self.timings.stop_grace, ended).await;
            }
        }
        if let Some(Process { child, mut reader }) = process {
            let exit = match exit {
                Some(exit) => exit,
                None => child.stop(self.timings.stop_grace).await,
            };
            self.ended(&exit);
            // Any end but a stop is the process ending, closed stdout included.
            if !stopped {
                let message = self.with_stderr(exit_text(&exit), &mut reader).await;
                let (state, code) = (State::Unavailable, ErrorCode::Exited);
                self.leave_ready(state, Stage::Connect, code, message);
            }
        }
        (!stopped).then_some(stable_seen)
    }

    /// `head`, then the child's last stderr line once the reader reached
    /// EOF, or after 500 ms: a grandchild may hold the pipe open.
    async fn with_stderr(&self, head: String, reader: &mut Reader) -> String {
        let _ = timeout(Duration::from_millis(500), &mut reader.task).await;
        let ring = self.ring.lock().unwrap();
        match ring.last().filter(|_| ring.pushed() > reader.from) {
            Some(line) => format!("{head}: {line}"),
            None => head,
        }
    }

    /// `tools/list` again on the live session, then the reconcile. A failure
    /// leaves the registry as it was.
    async fn relist_tools(self: &Arc<Self>) {
        let Some(peer) = self.peer.read().unwrap().clone() else {
            return;
        };
        let limit = self.timings.list;
        let listed = match timeout(limit, peer.list_all_tools()).await {
            Ok(Ok(tools)) => serde_json::to_value(tools)
                .and_then(serde_json::from_value::<Vec<ToolInfo>>)
                .map_err(|e| e.to_string()),
            Ok(Err(e)) => Err(failure_text(&self.name, &self.request_failed(&e))),
            Err(_) => Err(self.no_answer(limit)),
        };
        let tools = match listed {
            Ok(tools) => tools,
            Err(message) => {
                let message = scrub(&message, &self.secrets.values());
                tracing::warn!(
                    "listing the tools of `{}` again failed, its functions stay as they were: {message}",
                    self.name
                );
                return;
            }
        };
        let has_instructions = self.info.lock().unwrap().instructions.is_some();
        let (wanted, skipped) = tool_wanted(&self.name, &tools, has_instructions);
        {
            let mut info = self.info.lock().unwrap();
            info.tools = tool_views(&wanted);
            info.skipped = skipped;
        }
        self.reconcile(wanted).await;
    }

    /// Classifies a failed request and applies its state rule: a rejected
    /// credential or a sign-in challenge stops the server, an unclassified
    /// transport failure is an outage, and anything else fails this request
    /// only. The registry is never touched.
    fn request_failed(&self, error: &ServiceError) -> CallFailure {
        let spec = self.spec();
        let failure = match &spec.checked.entry {
            Some(entry) => classify_call_error(error, entry),
            None => CallFailure::Transport(service_text(error)),
        };
        let stdio = self.is_stdio();
        let (state, stage, code) = match &failure {
            CallFailure::CredentialRejected { .. } | CallFailure::NoSignIn => (
                State::AuthFailed,
                Stage::Authorize,
                ErrorCode::CredentialRejected,
            ),
            CallFailure::LoginRequired => {
                self.challenged.store(true, Ordering::Relaxed);
                (
                    State::NeedsLogin,
                    Stage::Authorize,
                    ErrorCode::LoginRequired,
                )
            }
            // A stdio pipe fails when its process ends, which `serve` reports
            // as `exited` with the code.
            CallFailure::Transport(_) if !stdio => {
                (State::Unavailable, Stage::Connect, ErrorCode::ConnectFailed)
            }
            _ => return failure,
        };
        self.leave_ready(state, stage, code, failure_text(&self.name, &failure));
        failure
    }

    async fn close(&self, mut session: Session) {
        self.peer.write().unwrap().take();
        let _ = timeout(self.timings.stop_grace, session.close()).await;
    }

    /// Stops the task and closes its session; the registry stays.
    async fn halt(&self) {
        let task = self.task.lock().unwrap().take();
        if let Some((stop, handle)) = task {
            let _ = stop.send(());
            // The task bounds its own close by `stop_grace`.
            let _ = handle.await;
        }
        self.peer.write().unwrap().take();
        // A child the stop caught mid-connect went with its guard.
        let mut f = self.facts.lock().unwrap();
        (f.pid, f.next_restart_at) = (None, None);
    }

    /// Closes the session and moves to `unapproved`; the registry stays.
    async fn unapprove(&self) {
        let state = self.status.borrow().state;
        if matches!(state, State::Disabled | State::ConfigError) {
            return;
        }
        self.halt().await;
        let code = ErrorCode::Unapproved;
        self.fail(State::Unapproved, Stage::Approval, code, UNAPPROVED.into());
    }

    /// Stops the task, closes its session and unregisters every function.
    async fn stop(&self) {
        self.halt().await;
        self.registry.clear().await;
    }

    /// One attempt, from the secrets to `ready`. `Err` carries the wait a
    /// 429 asked for.
    async fn connect(self: &Arc<Self>) -> Result<Live, Option<Duration>> {
        let spec = self.spec();
        let Some(entry) = spec.checked.entry.as_ref() else {
            return Err(None);
        };
        // The gate: nothing is resolved, sent or spawned for an entry no operator approved.
        if let Err(e) = self.approvals.load(&*self.host).await {
            let error = ref_error(APPROVALS_REF, e);
            self.set(State::Connecting, Some(Stage::Approval), Some(error));
            return Err(None);
        }
        let approved = || {
            self.approvals
                .is_approved(&self.name, &spec.checked.fingerprint)
        };
        if !approved() {
            let code = ErrorCode::Unapproved;
            self.fail(State::Unapproved, Stage::Approval, code, UNAPPROVED.into());
            // An approve between the check and `fail` saw no `unapproved` to restart.
            if !approved() {
                return Err(None);
            }
            self.set(State::Connecting, Some(Stage::Secret), None);
        }
        // Resolved again on every attempt.
        self.progress(Stage::Secret);
        let resolved = match resolve_all(&*self.host, &entry.refs()).await {
            Ok(resolved) => resolved,
            Err(e) => {
                let state = match e.code {
                    ErrorCode::ConnectFailed => State::Unavailable,
                    _ => State::ConfigError,
                };
                self.set(state, Some(Stage::Secret), Some(e));
                return Err(None);
            }
        };
        self.secrets.set_resolved(resolved.secrets());
        let (session, mut process) = match &entry.transport {
            Transport::Http(http) => (self.open_http(entry, http, &resolved).await?, None),
            Transport::Stdio(stdio) => {
                let (session, process) = self.open_stdio(stdio, &resolved).await?;
                (session, Some(process))
            }
        };
        let peer = session.peer().clone();
        peer.set_response_cache_config(ClientCacheConfig::disabled())
            .await;
        let handshake = peer
            .peer_info()
            .and_then(|i| serde_json::to_value(&*i).ok())
            .unwrap_or_default();

        self.progress(Stage::Tools);
        let limit = self.timings.list;
        let listed = match timeout(limit, peer.list_all_tools()).await {
            Ok(Ok(tools)) => serde_json::to_value(tools)
                .and_then(serde_json::from_value::<Vec<ToolInfo>>)
                .map_err(|e| e.to_string()),
            Ok(Err(e)) => Err(service_text(&e)),
            Err(_) => Err(self.no_answer(limit)),
        };
        let tools = match listed {
            Ok(tools) => tools,
            Err(message) => {
                let mut error = (Stage::Tools, ErrorCode::ListFailed, message);
                // A child that died during the listing is `exited` with its code;
                // asked before the session closes stdin, which would end it.
                if let Some(p) = &mut process
                    && let Ok(exit) = timeout(Duration::from_millis(500), p.child.wait()).await
                {
                    let message = self.with_stderr(exit_text(&exit), &mut p.reader).await;
                    error = (Stage::Connect, ErrorCode::Exited, message);
                }
                let (stage, code, message) = error;
                self.fail(State::Unavailable, stage, code, message);
                self.close(session).await;
                if let Some(process) = process {
                    let exit = process.child.stop(self.timings.stop_grace).await;
                    self.ended(&exit);
                }
                return Err(None);
            }
        };
        let instructions = handshake["instructions"]
            .as_str()
            .filter(|t| !t.trim().is_empty())
            .map(String::from);
        let (wanted, skipped) = tool_wanted(&self.name, &tools, instructions.is_some());
        {
            let mut info = self.info.lock().unwrap();
            let caps = &handshake["capabilities"];
            let server_info = &handshake["serverInfo"];
            *info = Info {
                protocol_version: handshake["protocolVersion"].as_str().map(String::from),
                server_info: server_info.is_object().then(|| ServerInfoView {
                    name: server_info["name"].as_str().unwrap_or("").into(),
                    title: server_info["title"].as_str().map(String::from),
                    version: server_info["version"].as_str().unwrap_or("").into(),
                }),
                instructions,
                capabilities: CapabilitiesView {
                    resources: !caps["resources"].is_null(),
                    prompts: !caps["prompts"].is_null(),
                },
                tools_listed: true,
                tools: tool_views(&wanted),
                skipped,
            };
        }
        *self.peer.write().unwrap() = Some(peer);
        self.reconcile(wanted).await;
        self.set(State::Ready, None, None);
        Ok(Live { session, process })
    }

    fn notifier(&self) -> Notifier {
        Notifier {
            info: client_config(),
            tx: self.notes.clone(),
            _secrets: self.secrets.clone(),
        }
    }

    /// The http transport from the resolved entry, then the handshake.
    async fn open_http(
        &self,
        entry: &Entry,
        http: &HttpEntry,
        resolved: &Resolved,
    ) -> Result<Session, Option<Duration>> {
        let url = resolved.text(&http.url);
        if let Text::Ref(r) = &http.url
            && let Err(m) = check_url_text(&url)
        {
            let message = format!("the address stored in `{r}` {m}");
            self.fail(
                State::ConfigError,
                Stage::Secret,
                ErrorCode::InvalidConfig,
                message,
            );
            return Err(None);
        }
        let mut headers = HashMap::new();
        for (name, text) in &http.headers {
            let parsed = HeaderName::from_bytes(name.as_bytes())
                .ok()
                .zip(HeaderValue::from_str(&resolved.text(text)).ok());
            let Some((header, mut value)) = parsed else {
                let message = format!("header `{name}` is not a valid HTTP header value");
                self.fail(
                    State::ConfigError,
                    Stage::Secret,
                    ErrorCode::InvalidConfig,
                    message,
                );
                return Err(None);
            };
            value.set_sensitive(matches!(text, Text::Ref(_)));
            headers.insert(header, value);
        }
        let mut config = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(headers);
        if let Auth::Bearer { token } = &http.auth {
            config = config.auth_header(resolved.text(&Text::Ref(token.clone())));
        }
        let transport = StreamableHttpClientTransport::from_config(config);

        self.progress(Stage::Connect);
        let limit = self.timings.http_handshake;
        match timeout(limit, self.notifier().serve(transport)).await {
            Ok(Ok(session)) => Ok(session),
            Ok(Err(e)) => {
                let (state, stage, code, message) = self.handshake_failure(&e, entry, http);
                self.fail(state, stage, code, message);
                Err(retry_wait(&e))
            }
            Err(_) => {
                self.fail(
                    State::Unavailable,
                    Stage::Connect,
                    ErrorCode::HandshakeFailed,
                    self.no_answer(limit),
                );
                Err(None)
            }
        }
    }

    /// Spawns the child, starts its stderr reader, then the handshake over
    /// its stdout and stdin. A child that exits first is `exited`.
    async fn open_stdio(
        &self,
        stdio: &StdioEntry,
        resolved: &Resolved,
    ) -> Result<(Session, Process), Option<Duration>> {
        self.progress(Stage::Connect);
        let spawned = match stdio::spawn(stdio, resolved, &self.worker_cwd) {
            Ok(spawned) => spawned,
            Err(e) => {
                self.set(State::Unavailable, Some(Stage::Connect), Some(e));
                return Err(None);
            }
        };
        let mut child = spawned.child;
        self.spawned(spawned.pid);
        let from = self.ring.lock().unwrap().pushed();
        let ring = self.ring.clone();
        let (host, server) = (self.host.clone(), self.name.clone());
        let on_line = move || {
            let (server, kind) = (server.clone(), EventKind::Stderr);
            host.emit(ServerEvent { server, kind });
        };
        let read = stdio::read_stderr(spawned.stderr, ring, resolved.secrets(), on_line);
        let mut reader = Reader {
            task: tokio::spawn(read),
            from,
        };
        let limit = self.timings.stdio_start;
        let handshake = self.notifier().serve((spawned.stdout, spawned.stdin));
        let (exit, failure) = tokio::select! {
            served = timeout(limit, handshake) => match served {
                Ok(Ok(session)) => return Ok((session, Process { child, reader })),
                // stdout EOF usually fails `serve` before the exit is reaped.
                Ok(Err(e)) => {
                    let exit = timeout(Duration::from_millis(500), child.wait()).await;
                    (exit.ok(), format!("MCP handshake failed: {}", init_text(&e)))
                }
                Err(_) => (None, format!("no MCP handshake after {} s", limit.as_secs())),
            },
            exit = child.wait() => (Some(exit), String::new()),
        };
        let (code, head, exit) = match exit {
            Some(exit) => (ErrorCode::Exited, exit_text(&exit), exit),
            None => {
                let exit = child.stop(self.timings.stop_grace).await;
                (ErrorCode::HandshakeFailed, failure, exit)
            }
        };
        self.ended(&exit);
        let message = self.with_stderr(head, &mut reader).await;
        self.fail(State::Unavailable, Stage::Connect, code, message);
        Err(None)
    }

    fn no_answer(&self, limit: Duration) -> String {
        format!("no answer from {} within {} s", self.name, limit.as_secs())
    }

    /// The row of the connect sequence a failed handshake stops at.
    fn handshake_failure(
        &self,
        e: &ClientInitializeError,
        entry: &Entry,
        http: &HttpEntry,
    ) -> (State, Stage, ErrorCode, String) {
        let auth = entry.auth_kind();
        let chain: Vec<&(dyn Error + 'static)> = match e {
            ClientInitializeError::TransportError { error, .. } => {
                let top: &(dyn Error + 'static) = error.error.as_ref();
                std::iter::successors(Some(top), |&x| x.source()).collect()
            }
            _ => vec![],
        };
        let challenged = chain.iter().any(|x| x.is::<AuthRequiredError>());
        let scope = chain
            .iter()
            .find_map(|x| x.downcast_ref::<InsufficientScopeError>());
        let http_error = chain.first().and_then(|x| x.downcast_ref::<HttpError>());
        // Its text carries the response body: matched, never passed on.
        let bare_401 = matches!(http_error,
            Some(HttpError::UnexpectedServerResponse(t)) if t.starts_with("HTTP 401"));
        let rejected = |failure: CallFailure| {
            let message = failure_text(&self.name, &failure);
            (
                State::AuthFailed,
                Stage::Authorize,
                ErrorCode::CredentialRejected,
                message,
            )
        };
        if challenged && auth == "none" {
            self.challenged.store(true, Ordering::Relaxed);
            let message = failure_text(&self.name, &CallFailure::LoginRequired);
            return (
                State::NeedsLogin,
                Stage::Authorize,
                ErrorCode::LoginRequired,
                message,
            );
        }
        if (challenged || bare_401)
            && let Some(reference) = credential_ref(http)
        {
            return rejected(CallFailure::CredentialRejected { reference });
        }
        if bare_401 {
            let message = failure_text(&self.name, &CallFailure::NoSignIn);
            return (
                State::AuthFailed,
                Stage::Authorize,
                ErrorCode::CredentialRejected,
                message,
            );
        }
        if let Some(s) = scope {
            let scope = s.required_scope.clone();
            return rejected(CallFailure::InsufficientScope { scope });
        }
        let (code, message) = match http_error {
            Some(HttpError::HttpStatus(s)) if s.status == 429 => {
                let retry_after = s.retry_after.clone();
                let failure = CallFailure::RateLimited { retry_after };
                (ErrorCode::RateLimited, failure_text(&self.name, &failure))
            }
            Some(HttpError::Client(e)) => {
                // reqwest's own text holds the resolved URL; its causes do not.
                let cause = std::iter::successors(e.source(), |&x| x.source())
                    .last()
                    .map_or_else(|| "no HTTP answer".into(), |c| c.to_string());
                let target = entry.target();
                let message = format!("could not reach {target}: {cause}");
                (ErrorCode::ConnectFailed, message)
            }
            _ => (ErrorCode::HandshakeFailed, init_text(e)),
        };
        (State::Unavailable, Stage::Connect, code, message)
    }

    /// The one caller of `Registry::apply`.
    async fn reconcile(self: &Arc<Self>, wanted: WantedSet) {
        let plan = self
            .registry
            .apply(&*self.host, wanted, &|_| false, &|w| self.handler_for(w))
            .await;
        for id in &plan.lost_read_only {
            tracing::warn!(
                "`{id}` is no longer marked read-only by its server; an approval-gate allow rule for it stays in force until someone updates the rules"
            );
        }
        if !(plan.remove.is_empty() && plan.change.is_empty() && plan.add.is_empty()) {
            self.emit(EventKind::Tools);
        }
    }

    fn handler_for(self: &Arc<Self>, wanted: &Wanted) -> Handler {
        // Weak: the host holds the handler, and the server holds the host.
        let server = Arc::downgrade(self);
        let name = self.name.clone();
        let target = wanted.target.clone();
        Arc::new(move |payload| {
            let (server, name, target) = (server.upgrade(), name.clone(), target.clone());
            Box::pin(async move {
                let Some(server) = server else {
                    return Err(format!("`{name}` MCP server was removed"));
                };
                let out = server.call(&target, payload).await;
                out.map_err(|e| scrub(&e, &server.secrets.values()))
            })
        })
    }

    async fn call(&self, target: &Target, payload: Value) -> Result<Value, String> {
        let status = self.status.borrow().clone();
        let peer = self.peer.read().unwrap().clone();
        let (State::Ready, Some(peer)) = (status.state, peer) else {
            let for_secs = now_ms().saturating_sub(status.since) / 1000;
            let state = status.state.as_str();
            let failure = CallFailure::Unavailable { state, for_secs };
            return Err(failure_text(&self.name, &failure));
        };
        let (name, declared) = match target {
            Target::Tool { name, declared } => (name, declared),
            Target::Instructions => {
                let text = self.info.lock().unwrap().instructions.clone();
                return Ok(json!({"content": [{"type": "text", "text": text}]}));
            }
            _ => {
                return Err(format!(
                    "{target:?} is not supported by this version of mcp-client"
                ));
            }
        };
        let params =
            CallToolRequestParams::new(name.clone()).with_arguments(strip_args(payload, declared));
        let limit = self.timings.call;
        let failure = match timeout(limit, peer.call_tool(params)).await {
            Ok(Ok(result)) => {
                let ctx = BlockCtx {
                    server: &self.name,
                    read_registered: false,
                    asked_uri: None,
                    many: false,
                };
                let result = serde_json::to_value(result).unwrap_or_default();
                return tool_result(&result, &ctx);
            }
            Ok(Err(e)) => {
                let failure = self.request_failed(&e);
                if matches!(failure, CallFailure::JsonRpc { code: -32602, .. }) {
                    self.relist.notify_one();
                }
                failure
            }
            Err(_) => CallFailure::Timeout {
                secs: limit.as_secs(),
            },
        };
        Err(failure_text(&self.name, &failure))
    }

    fn view(&self, registered: &HashSet<String>) -> ServerView {
        let status = self.status.borrow().clone();
        let info = self.info.lock().unwrap();
        let spec = self.spec();
        let (transport, target, auth) = match &spec.checked.entry {
            Some(entry) => {
                let transport = match entry.transport {
                    Transport::Http(_) => "http",
                    Transport::Stdio(_) => "stdio",
                };
                let auth = match self.challenged.load(Ordering::Relaxed) {
                    true => "oauth",
                    false => entry.auth_kind(),
                };
                (transport, entry.target(), auth)
            }
            None => {
                let command = spec.raw.get("command");
                let transport = if command.is_some() { "stdio" } else { "http" };
                let target = spec.raw.get("url").or(command).and_then(Value::as_str);
                (transport, target.unwrap_or("").to_string(), "none")
            }
        };
        let tools = info
            .tools
            .iter()
            .map(|(name, annotations)| {
                let function_id = function_id(&self.name, name);
                ToolView {
                    name: name.clone(),
                    registered: registered.contains(&function_id),
                    function_id,
                    annotations: annotations.clone(),
                }
            })
            .collect();
        ServerView {
            name: self.name.clone(),
            transport: transport.into(),
            target,
            auth: auth.into(),
            state: status.state,
            stage: status.stage,
            since: status.since,
            error: status.error,
            approved: self
                .approvals
                .is_approved(&self.name, &spec.checked.fingerprint),
            fingerprint: spec.checked.fingerprint.clone(),
            protocol_version: info.protocol_version.clone(),
            server_info: info.server_info.clone(),
            tools_listed: info.tools_listed,
            tools,
            skipped: info.skipped.clone(),
            has_instructions: info.instructions.is_some(),
            capabilities: info.capabilities.clone(),
            resource_functions: vec![],
            prompts: vec![],
            list_errors: ListErrors::default(),
            login: None,
            process: self.is_stdio().then(|| {
                let f = self.facts.lock().unwrap();
                ProcessView {
                    pid: f.pid,
                    started_at: f.started_at,
                    restarts: f.policy.restarts(),
                    restart_budget: f.policy.budget(),
                    next_restart_at: f.next_restart_at,
                    last_exit: f.last_exit.clone(),
                }
            }),
        }
    }
}

pub struct Manager {
    host: Arc<dyn Host>,
    approvals: Arc<Approvals>,
    opts: ManagerOptions,
    /// Serialises config applies and secret changes.
    servers: tokio::sync::Mutex<BTreeMap<String, Arc<Server>>>,
    /// The start-up load of the approvals, retried until it succeeds.
    load: JoinHandle<()>,
    /// Set first by `shutdown`: a config applied during or after it starts nothing.
    closed: AtomicBool,
}

impl Manager {
    /// Starts reading the approvals at once, whatever the config holds, so a
    /// write by anyone else after the start is seen as one.
    pub fn new(host: Arc<dyn Host>, opts: ManagerOptions) -> Arc<Manager> {
        let approvals = Arc::new(Approvals::new());
        let (h, a, t) = (host.clone(), approvals.clone(), opts.timings);
        let load = tokio::spawn(async move {
            let mut backoff = Backoff::new(t.backoff_min, t.backoff_max);
            while a.load(&*h).await.is_err() {
                sleep(backoff.next()).await;
            }
        });
        Arc::new(Manager {
            host,
            approvals,
            opts,
            servers: tokio::sync::Mutex::default(),
            load,
            closed: AtomicBool::default(),
        })
    }

    pub fn stdio_enabled(&self) -> bool {
        self.opts.stdio_enabled
    }

    /// Restarts only the entries whose raw value changed: an equal entry
    /// sees no reconnect, no registry operation and no event.
    pub async fn apply_config(self: &Arc<Self>, raw: RawConfig) {
        let mut servers = self.servers.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        let (kept, removed): (BTreeMap<_, _>, BTreeMap<_, _>) = std::mem::take(&mut *servers)
            .into_iter()
            .partition(|(name, _)| raw.0.contains_key(name));
        *servers = kept;
        join_all(removed.iter().map(|(name, s)| async move {
            s.stop().await;
            s.emit(EventKind::State);
            self.approvals.clear_pending(name);
            if let Err(e) = self.approvals.forget(&*self.host, name).await {
                tracing::warn!("forgetting the approval of removed `{name}` failed: {e}");
            }
        }))
        .await;
        let mut restarts = Vec::new();
        for (name, value) in raw.0 {
            if servers.get(&name).is_some_and(|s| s.spec().raw == value) {
                continue;
            }
            self.promote(&name, &fingerprint(&value)).await;
            match servers.get(&name) {
                Some(s) => {
                    let checked = check_entry(&name, &value, self.opts.stdio_enabled);
                    let spec = Spec {
                        raw: value,
                        checked,
                    };
                    let s = s.clone();
                    restarts.push(async move { s.restart(Some(spec)).await });
                }
                None => {
                    let server =
                        Server::start(&self.host, &self.approvals, &self.opts, &name, &value);
                    servers.insert(name, server);
                }
            }
        }
        join_all(restarts).await;
    }

    /// Writes the pending approval an arriving entry matches, before its
    /// task starts.
    async fn promote(&self, name: &str, fingerprint: &str) {
        if self.approvals.pending(name).as_deref() != Some(fingerprint) {
            return;
        }
        self.approvals.clear_pending(name);
        if let Err(e) = self.approvals.approve(&*self.host, name, fingerprint).await {
            tracing::warn!("saving the approval of `{name}` failed, it stays unapproved: {e}");
        }
    }

    /// Approves `fingerprint` for `name`, or the entry as loaded. A new
    /// fingerprint for a running, approved entry (an edit approved before
    /// its write) waits in memory for that write and revokes nothing.
    pub async fn approve(&self, name: &str, fingerprint: Option<&str>) -> Result<(), String> {
        let servers = self.servers.lock().await;
        let loaded = servers.get(name);
        let current = loaded.map(|s| s.spec().checked.fingerprint.clone());
        let fingerprint = match (fingerprint, &current) {
            (Some(fp), _) if !is_fingerprint(fp) => {
                return Err("`fingerprint` must be 64 lower-case hex characters".into());
            }
            (Some(fp), _) => fp.to_string(),
            (None, Some(current)) => current.clone(),
            (None, None) => return Err(format!("no MCP server named `{name}` is configured")),
        };
        if let Some(current) = &current
            && *current != fingerprint
            && self.approvals.is_approved(name, current)
        {
            self.approvals.set_pending(name, &fingerprint);
            return Ok(());
        }
        self.approvals
            .approve(&*self.host, name, &fingerprint)
            .await?;
        if let Some(s) = loaded
            && current == Some(fingerprint)
        {
            let state = s.status.borrow().state;
            if state == State::Unapproved {
                s.restart(None).await;
            }
        }
        Ok(())
    }

    /// Starts the server again with a fresh backoff and budget. A disabled,
    /// unapproved or invalid entry is left as it is.
    pub async fn reconnect(&self, name: &str) -> Result<State, String> {
        let servers = self.servers.lock().await;
        let s = servers
            .get(name)
            .ok_or_else(|| format!("no MCP server named `{name}` is configured"))?;
        let state = s.status.borrow().state;
        if matches!(state, State::Disabled | State::Unapproved) || s.spec().checked.entry.is_none()
        {
            return Ok(state);
        }
        s.restart(None).await;
        let state = s.status.borrow().state;
        Ok(state)
    }

    /// The stderr tail of a stdio server; an http server has none.
    pub async fn logs(&self, server: &str) -> Result<Logs, String> {
        // Not held while reading: an apply holds it across a stop.
        let s = self.servers.lock().await.get(server).cloned();
        let s = s.ok_or_else(|| format!("no server named `{server}`"))?;
        let (lines, dropped) = match s.is_stdio() {
            true => s.ring.lock().unwrap().snapshot(),
            false => (vec![], 0),
        };
        Ok(Logs { lines, dropped })
    }

    /// Resolves again the references of each server whose entry names the
    /// changed one, and restarts it when a value changed or it is not
    /// `ready`: the way out of `auth_failed` and of a secret `config_error`.
    /// A change of the approvals goes to the tamper check only.
    pub async fn on_secret_change(&self, change: &SecretChange) {
        let servers = self.servers.lock().await;
        let approvals = Ref {
            store: Store::Secret,
            name: APPROVALS_SECRET.into(),
        };
        if event_matches(change, &[approvals]) {
            if self.approvals.on_change(&*self.host).await {
                join_all(servers.values().map(|s| s.unapprove())).await;
            }
            return;
        }
        let mut restarts = Vec::new();
        for server in servers.values() {
            let spec = server.spec();
            let Some(entry) = &spec.checked.entry else {
                continue;
            };
            let state = server.status.borrow().state;
            let refs = entry.refs();
            if matches!(state, State::Disabled | State::Unapproved) || !event_matches(change, &refs)
            {
                continue;
            }
            let unchanged = state == State::Ready
                && match resolve_all(&*self.host, &refs).await {
                    // shortcut: compares the whole set, exact while nothing calls `add`;
                    // compare only the resolved part once a minted token is added.
                    Ok(resolved) => resolved.secrets() == server.secrets.values(),
                    Err(_) => false,
                };
            if !unchanged {
                restarts.push(server.restart(None));
            }
        }
        join_all(restarts).await;
    }

    pub async fn list(&self) -> ServersList {
        let registered = self.host.registered().await;
        let port = self.host.http_port().await.unwrap_or(3111);
        let servers = self.servers.lock().await;
        ServersList {
            stdio_enabled: self.opts.stdio_enabled,
            callback_url: format!("http://127.0.0.1:{port}/mcp-client/oauth/callback"),
            servers: servers.values().map(|s| s.view(&registered)).collect(),
        }
    }

    pub async fn wait_for(&self, server: &str, state: State, within: Duration) -> bool {
        let status = self
            .servers
            .lock()
            .await
            .get(server)
            .map(|s| s.status.subscribe());
        let Some(mut status) = status else {
            return false;
        };
        let reached = timeout(within, status.wait_for(|s| s.state == state)).await;
        matches!(reached, Ok(Ok(_)))
    }

    pub async fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.load.abort();
        let mut servers = self.servers.lock().await;
        join_all(std::mem::take(&mut *servers).values().map(|s| s.stop())).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_are_snake_case() {
        assert_eq!(json!(State::ConfigError), json!("config_error"));
        assert_eq!(json!(State::NeedsLogin), json!("needs_login"));
        assert_eq!(json!(Stage::Authorize), json!("authorize"));
        assert_eq!(
            json!(ErrorCode::CredentialRejected),
            json!("credential_rejected")
        );
        for state in [
            State::Disabled,
            State::ConfigError,
            State::Unapproved,
            State::Connecting,
            State::Ready,
            State::Unavailable,
            State::AuthFailed,
            State::NeedsLogin,
        ] {
            assert_eq!(json!(state), json!(state.as_str()));
        }
    }
}
