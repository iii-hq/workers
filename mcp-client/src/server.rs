//! One task per configured server: connect, list, register, call; and the
//! wire types of `servers::list`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http::{HeaderName, HeaderValue};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
};
use rmcp::service::{ClientCacheConfig, ClientInitializeError, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, InsufficientScopeError, StreamableHttpClientTransportConfig,
    StreamableHttpError,
};
use rmcp::{Peer, RoleClient, ServiceError, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::config::{
    Auth, Checked, Entry, HttpEntry, RawConfig, Transport, check_entry, check_url_text,
};
use crate::host::{EventKind, Handler, Host, ServerEvent};
use crate::reconcile::{
    Registry, Skipped, Target, ToolInfo, Wanted, WantedSet, function_id, tool_wanted,
};
use crate::refs::{Text, resolve_all};
use crate::translate::{BlockCtx, CallFailure, failure_text, strip_args, tool_result};

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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Replaces each secret value in `text` with `[redacted]`, longest first so a
/// value that contains another goes whole.
fn scrub(text: &str, secrets: &[String]) -> String {
    let mut secrets: Vec<&String> = secrets.iter().filter(|s| !s.is_empty()).collect();
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    secrets
        .iter()
        .fold(text.to_string(), |t, s| t.replace(s.as_str(), "[redacted]"))
}

fn client_config() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("iii-mcp-client", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE)
}

type Session = RunningService<RoleClient, ClientConfig>;

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

struct Server {
    name: String,
    raw: Value,
    checked: Checked,
    host: Arc<dyn Host>,
    timings: Timings,
    status: watch::Sender<Status>,
    registry: Registry,
    info: Mutex<Info>,
    /// The resolved reference values, replaced in every text this server shows.
    secrets: Mutex<Vec<String>>,
    /// A no-`auth` entry the server answered with a sign-in challenge.
    challenged: AtomicBool,
    peer: RwLock<Option<Peer<RoleClient>>>,
    task: Mutex<Option<(oneshot::Sender<()>, JoinHandle<()>)>>,
}

/// The state an entry starts in, from what its config alone says.
fn first_status(name: &str, checked: &Checked) -> (State, Option<Stage>, Option<ServerError>) {
    let fail = |state, stage, code, message: String| {
        (state, Some(stage), Some(ServerError::new(code, message)))
    };
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
        Transport::Stdio(_) => fail(
            State::ConfigError,
            Stage::Connect,
            ErrorCode::StdioDisabled,
            "stdio servers are not supported by this version of mcp-client".into(),
        ),
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
    fn start(host: &Arc<dyn Host>, opts: &ManagerOptions, name: &str, raw: &Value) -> Arc<Server> {
        let checked = check_entry(name, raw, opts.stdio_enabled);
        let (state, stage, error) = first_status(name, &checked);
        let status = Status {
            state,
            stage,
            error,
            since: now_ms(),
        };
        let server = Arc::new(Server {
            name: name.into(),
            raw: raw.clone(),
            checked,
            host: host.clone(),
            timings: opts.timings,
            status: watch::channel(status).0,
            registry: Registry::new(),
            info: Mutex::default(),
            secrets: Mutex::default(),
            challenged: AtomicBool::default(),
            peer: RwLock::default(),
            task: Mutex::default(),
        });
        server.emit(EventKind::State);
        if state == State::Connecting {
            let (stop, stopped) = oneshot::channel();
            let handle = tokio::spawn(server.clone().run(stopped));
            *server.task.lock().unwrap() = Some((stop, handle));
        }
        server
    }

    fn emit(&self, kind: EventKind) {
        self.host.emit(ServerEvent {
            server: self.name.clone(),
            kind,
        });
    }

    fn set(&self, state: State, stage: Option<Stage>, error: Option<ServerError>) {
        self.status.send_modify(|s| {
            if s.state != state {
                s.since = now_ms();
            }
            s.state = state;
            s.stage = stage;
            s.error = error;
        });
        self.emit(EventKind::State);
    }

    fn fail(&self, state: State, stage: Stage, code: ErrorCode, message: String) {
        self.set(state, Some(stage), Some(ServerError::new(code, message)));
    }

    async fn run(self: Arc<Self>, mut stop: oneshot::Receiver<()>) {
        let session = tokio::select! {
            session = self.connect() => session,
            _ = &mut stop => return,
        };
        if let Some(session) = session {
            let _ = stop.await;
            self.close(session).await;
        }
    }

    async fn close(&self, mut session: Session) {
        self.peer.write().unwrap().take();
        let _ = timeout(self.timings.stop_grace, session.close()).await;
    }

    /// Stops the task, closes its session and unregisters every function.
    async fn stop(&self) {
        let task = self.task.lock().unwrap().take();
        if let Some((stop, handle)) = task {
            let _ = stop.send(());
            // The task bounds its own close by `stop_grace`.
            let _ = handle.await;
        }
        self.peer.write().unwrap().take();
        self.registry.clear().await;
    }

    async fn connect(self: &Arc<Self>) -> Option<Session> {
        let entry = self.checked.entry.as_ref()?;
        let Transport::Http(http) = &entry.transport else {
            return None;
        };
        let resolved = match resolve_all(&*self.host, &entry.refs()).await {
            Ok(resolved) => resolved,
            Err(e) => {
                let state = match e.code {
                    ErrorCode::ConnectFailed => State::Unavailable,
                    _ => State::ConfigError,
                };
                self.set(state, Some(Stage::Secret), Some(e));
                return None;
            }
        };
        let secrets = resolved.secrets();
        *self.secrets.lock().unwrap() = secrets.clone();
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
            return None;
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
                return None;
            };
            value.set_sensitive(matches!(text, Text::Ref(_)));
            headers.insert(header, value);
        }
        let mut config = StreamableHttpClientTransportConfig::with_uri(url).custom_headers(headers);
        if let Auth::Bearer { token } = &http.auth {
            config = config.auth_header(resolved.text(&Text::Ref(token.clone())));
        }
        let transport = StreamableHttpClientTransport::from_config(config);

        self.set(State::Connecting, Some(Stage::Connect), None);
        let limit = self.timings.http_handshake;
        let session = match timeout(limit, client_config().serve(transport)).await {
            Ok(Ok(session)) => session,
            Ok(Err(e)) => {
                let (state, stage, code, message) =
                    self.handshake_failure(&e, entry, http, &secrets);
                self.fail(state, stage, code, message);
                return None;
            }
            Err(_) => {
                self.fail(
                    State::Unavailable,
                    Stage::Connect,
                    ErrorCode::HandshakeFailed,
                    self.no_answer(limit),
                );
                return None;
            }
        };
        let peer = session.peer().clone();
        peer.set_response_cache_config(ClientCacheConfig::disabled())
            .await;
        let handshake = peer
            .peer_info()
            .and_then(|i| serde_json::to_value(&*i).ok())
            .unwrap_or_default();

        self.set(State::Connecting, Some(Stage::Tools), None);
        let limit = self.timings.list;
        let listed = match timeout(limit, peer.list_all_tools()).await {
            Ok(Ok(tools)) => serde_json::to_value(tools)
                .and_then(serde_json::from_value::<Vec<ToolInfo>>)
                .map_err(|e| e.to_string()),
            Ok(Err(e)) => Err(scrub(&e.to_string(), &secrets)),
            Err(_) => Err(self.no_answer(limit)),
        };
        let tools = match listed {
            Ok(tools) => tools,
            Err(message) => {
                self.fail(
                    State::Unavailable,
                    Stage::Tools,
                    ErrorCode::ListFailed,
                    message,
                );
                self.close(session).await;
                return None;
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
                tools: wanted
                    .values()
                    .filter_map(|w| match &w.target {
                        Target::Tool { name, .. } => Some((
                            name.clone(),
                            w.def.metadata.pointer("/mcp/annotations").cloned(),
                        )),
                        _ => None,
                    })
                    .collect(),
                skipped,
            };
        }
        *self.peer.write().unwrap() = Some(peer);
        self.reconcile(wanted).await;
        self.set(State::Ready, None, None);
        Some(session)
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
        secrets: &[String],
    ) -> (State, Stage, ErrorCode, String) {
        type HttpError = StreamableHttpError<reqwest::Error>;
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
        if (challenged || bare_401) && matches!(auth, "bearer" | "header") {
            let reference = match &http.auth {
                Auth::Bearer { token } => token.to_string(),
                _ => http
                    .headers
                    .values()
                    .find_map(|v| match v {
                        Text::Ref(r) => Some(r.to_string()),
                        Text::Literal(_) => None,
                    })
                    .unwrap_or_default(),
            };
            return rejected(CallFailure::CredentialRejected { reference });
        }
        if bare_401 {
            let message = format!("{} answered 401 and offers no sign-in", self.name);
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
                let message = format!("could not reach {target}: {}", scrub(&cause, secrets));
                (ErrorCode::ConnectFailed, message)
            }
            _ => {
                let text = match e {
                    ClientInitializeError::JsonRpcError(d) => {
                        format!("MCP error {}: {}", d.code.0, d.message)
                    }
                    _ => e.to_string(),
                };
                (ErrorCode::HandshakeFailed, scrub(&text, secrets))
            }
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
        self.emit(EventKind::Tools);
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
                out.map_err(|e| scrub(&e, &server.secrets.lock().unwrap()))
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
            Ok(Err(ServiceError::McpError(e))) => CallFailure::JsonRpc {
                code: e.code.0,
                message: e.message.into(),
            },
            Ok(Err(e)) => CallFailure::Transport(e.to_string()),
            Err(_) => CallFailure::Timeout {
                secs: limit.as_secs(),
            },
        };
        Err(failure_text(&self.name, &failure))
    }

    fn view(&self, registered: &HashSet<String>) -> ServerView {
        let status = self.status.borrow().clone();
        let info = self.info.lock().unwrap();
        let (transport, target, auth) = match &self.checked.entry {
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
                let command = self.raw.get("command");
                let transport = if command.is_some() { "stdio" } else { "http" };
                let target = self.raw.get("url").or(command).and_then(Value::as_str);
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
            approved: true,
            fingerprint: self.checked.fingerprint.clone(),
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
            process: None,
        }
    }
}

#[derive(Default)]
struct Servers {
    applied: Option<RawConfig>,
    by_name: BTreeMap<String, Arc<Server>>,
}

pub struct Manager {
    host: Arc<dyn Host>,
    opts: ManagerOptions,
    servers: tokio::sync::Mutex<Servers>,
}

impl Manager {
    pub fn new(host: Arc<dyn Host>, opts: ManagerOptions) -> Arc<Manager> {
        Arc::new(Manager {
            host,
            opts,
            servers: tokio::sync::Mutex::default(),
        })
    }

    pub fn stdio_enabled(&self) -> bool {
        self.opts.stdio_enabled
    }

    pub async fn apply_config(self: &Arc<Self>, raw: RawConfig) {
        let mut servers = self.servers.lock().await;
        if servers.applied.as_ref() == Some(&raw) {
            return;
        }
        stop_all(&mut servers).await;
        for (name, value) in &raw.0 {
            let server = Server::start(&self.host, &self.opts, name, value);
            servers.by_name.insert(name.clone(), server);
        }
        servers.applied = Some(raw);
    }

    pub async fn list(&self) -> ServersList {
        let registered = self.host.registered().await;
        let port = self.host.http_port().await.unwrap_or(3111);
        let servers = self.servers.lock().await;
        ServersList {
            stdio_enabled: self.opts.stdio_enabled,
            callback_url: format!("http://127.0.0.1:{port}/mcp-client/oauth/callback"),
            servers: servers
                .by_name
                .values()
                .map(|s| s.view(&registered))
                .collect(),
        }
    }

    pub async fn wait_for(&self, server: &str, state: State, within: Duration) -> bool {
        let status = self
            .servers
            .lock()
            .await
            .by_name
            .get(server)
            .map(|s| s.status.subscribe());
        let Some(mut status) = status else {
            return false;
        };
        let reached = timeout(within, status.wait_for(|s| s.state == state)).await;
        matches!(reached, Ok(Ok(_)))
    }

    pub async fn shutdown(&self) {
        stop_all(&mut *self.servers.lock().await).await;
    }
}

async fn stop_all(servers: &mut Servers) {
    let stopping = std::mem::take(&mut servers.by_name);
    futures::future::join_all(stopping.values().map(|s| s.stop())).await;
    servers.applied = None;
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
