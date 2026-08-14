//! Hermetic provider contract suite.
//!
//! The production engine, router, and provider code run unchanged. Only the
//! vendor HTTP boundary is replaced with a loopback server. Secrets in this
//! crate are fixed dummy values and captured requests are redacted before
//! rendering diagnostics.

#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use std::collections::VecDeque;
use std::convert::Infallible;
use std::fs::File;
use std::future::Future;
use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Router;
use iii_sdk::errors::Error as IiiError;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use llm_router::register::register_router;
use serde::Serialize;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub const ANTHROPIC_MESSAGES: &str = "anthropic-messages";
pub const OPENAI_CHAT_COMPLETIONS: &str = "openai-chat-completions";
pub const OPENAI_RESPONSES: &str = "openai-responses";

const API_KEY: &str = "provider-contract-api-key";
const OAUTH_TOKEN: &str = "provider-contract-oauth-token";
const ACCOUNT_ID: &str = "provider-contract-account";

type RegisterFuture = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;
type RegisterProvider = fn(IIIClient) -> RegisterFuture;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolFamily {
    AnthropicMessages,
    OpenAiChatCompletions,
    OpenAiResponses,
}

impl ProtocolFamily {
    pub fn id(self) -> &'static str {
        match self {
            Self::AnthropicMessages => ANTHROPIC_MESSAGES,
            Self::OpenAiChatCompletions => OPENAI_CHAT_COMPLETIONS,
            Self::OpenAiResponses => OPENAI_RESPONSES,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
enum CredentialMode {
    ApiKey,
    ClaudeOauth,
    CodexOauth,
}

#[derive(Clone, Copy)]
struct ProviderCase {
    id: &'static str,
    family: ProtocolFamily,
    model: &'static str,
    alternate_model: &'static str,
    upstream_model: &'static str,
    alternate_upstream_model: &'static str,
    generation_path: &'static str,
    credential: CredentialMode,
    register: RegisterProvider,
}

impl std::fmt::Debug for ProviderCase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderCase")
            .field("id", &self.id)
            .field("family", &self.family)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl CapturedRequest {
    fn redacted(&self) -> Self {
        let headers = self
            .headers
            .iter()
            .map(|(name, value)| {
                let value = if matches!(
                    name.as_str(),
                    "authorization" | "x-api-key" | "chatgpt-account-id"
                ) {
                    "<redacted>".to_string()
                } else {
                    value.clone()
                };
                (name.clone(), value)
            })
            .collect();
        Self {
            method: self.method.clone(),
            path: self.path.clone(),
            headers,
            body: self.body.clone(),
        }
    }

    fn header(&self, wanted: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone)]
enum StubBody {
    Complete(String),
    Hanging(Arc<AtomicBool>),
}

#[derive(Clone)]
struct StubResponse {
    status: StatusCode,
    content_type: &'static str,
    body: StubBody,
}

impl StubResponse {
    fn sse(body: impl Into<String>) -> Self {
        Self {
            status: StatusCode::OK,
            content_type: "text/event-stream",
            body: StubBody::Complete(body.into()),
        }
    }

    fn json(status: StatusCode, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: StubBody::Complete(body.into()),
        }
    }

    fn hanging(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            status: StatusCode::OK,
            content_type: "text/event-stream",
            body: StubBody::Hanging(cancelled),
        }
    }
}

#[derive(Default)]
struct StubState {
    post_responses: Mutex<VecDeque<StubResponse>>,
    requests: Mutex<Vec<CapturedRequest>>,
    models_body: Mutex<String>,
}

struct StubUpstream {
    address: String,
    state: Arc<StubState>,
    task: tokio::task::JoinHandle<()>,
}

impl StubUpstream {
    async fn start(family: ProtocolFamily) -> anyhow::Result<Self> {
        let state = Arc::new(StubState {
            models_body: Mutex::new(models_body(family).to_string()),
            ..StubState::default()
        });
        let app = Router::new()
            .fallback(stub_handler)
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            address,
            state,
            task,
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.address, path)
    }

    fn respond(&self, responses: impl IntoIterator<Item = StubResponse>) {
        let mut plan = self.state.post_responses.lock().expect("stub plan lock");
        *plan = responses.into_iter().collect();
    }

    fn clear_requests(&self) {
        self.state.requests.lock().expect("requests lock").clear();
    }

    fn post_requests(&self) -> Vec<CapturedRequest> {
        self.state
            .requests
            .lock()
            .expect("requests lock")
            .iter()
            .filter(|request| request.method == "POST")
            .cloned()
            .collect()
    }

    async fn wait_for_post_count(&self, count: usize) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self.post_requests().len() >= count {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!(
                    "stub observed {} POST requests, expected {count}",
                    self.post_requests().len()
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

impl Drop for StubUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn stub_handler(State(state): State<Arc<StubState>>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let headers = request.headers().clone();
    let body = to_bytes(request.into_body(), 1024 * 1024)
        .await
        .unwrap_or_default();
    state
        .requests
        .lock()
        .expect("requests lock")
        .push(CapturedRequest {
            method: method.to_string(),
            path,
            headers: capture_headers(&headers),
            body: String::from_utf8_lossy(&body).into_owned(),
        });

    let response = if method == axum::http::Method::GET {
        StubResponse::json(
            StatusCode::OK,
            state.models_body.lock().expect("models body lock").clone(),
        )
    } else {
        let mut responses = state.post_responses.lock().expect("stub plan lock");
        match responses.len() {
            0 => StubResponse::json(
                StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"error":{"message":"stub response plan exhausted"}}"#,
            ),
            1 => responses.front().expect("one response").clone(),
            _ => responses.pop_front().expect("queued response"),
        }
    };
    build_stub_response(response)
}

fn build_stub_response(response: StubResponse) -> Response {
    let builder = Response::builder()
        .status(response.status)
        .header("content-type", response.content_type);
    match response.body {
        StubBody::Complete(body) => builder.body(Body::from(body)).expect("stub response"),
        StubBody::Hanging(cancelled) => {
            let (sender, receiver) = mpsc::channel::<Result<axum::body::Bytes, Infallible>>(1);
            tokio::spawn(async move {
                sender.closed().await;
                cancelled.store(true, Ordering::SeqCst);
            });
            builder
                .body(Body::from_stream(ReceiverStream::new(receiver)))
                .expect("hanging response")
        }
    }
}

fn capture_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or("<non-utf8>").to_string(),
            )
        })
        .collect()
}

struct Engine {
    url: String,
    child: Child,
    _directory: TempDir,
}

impl Engine {
    async fn start() -> anyhow::Result<Self> {
        let binary = std::env::var_os("III_ENGINE_BIN")
            .map(PathBuf::from)
            .context("III_ENGINE_BIN is required for provider contracts")?;
        if !binary.is_file() {
            bail!(
                "III_ENGINE_BIN does not point to a file: {}",
                binary.display()
            );
        }
        let port = StdTcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let directory = tempfile::tempdir()?;
        let config_path = directory.path().join("config.yaml");
        let config = format!(
            r#"workers:
  - name: iii-worker-manager
    config:
      port: {port}
  - name: iii-pubsub
    config:
      adapter:
        name: local
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: {directory}/configuration
      ttl_seconds: 0
  - name: iii-state
    config:
      adapter:
        name: kv
        config:
          file_path: {directory}/state.db
          store_method: file_based
"#,
            directory = directory.path().display()
        );
        std::fs::write(&config_path, config)?;
        let stdout = File::create(directory.path().join("engine.stdout.log"))?;
        let stderr = File::create(directory.path().join("engine.stderr.log"))?;
        let child = Command::new(&binary)
            .arg("--no-update-check")
            .arg("--config")
            .arg(&config_path)
            .current_dir(directory.path())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .with_context(|| format!("spawn iii engine {}", binary.display()))?;
        let url = format!("ws://127.0.0.1:{port}");
        wait_for_engine(&url).await?;
        Ok(Self {
            url,
            child,
            _directory: directory,
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn wait_for_engine(url: &str) -> anyhow::Result<()> {
    let probe = register_worker(url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if call(&probe, "engine::workers::list", json!({}))
            .await
            .is_ok()
        {
            probe.shutdown();
            return Ok(());
        }
        if Instant::now() >= deadline {
            probe.shutdown();
            bail!("iii engine did not become ready in 20 seconds");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, IiiError> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(30_000),
    })
    .await
}

async fn register_fake_vault(engine_url: &str, mode: CredentialMode) -> Option<IIIClient> {
    let credential = match mode {
        CredentialMode::ApiKey => return None,
        CredentialMode::ClaudeOauth => json!({
            "type": "oauth",
            "provider": "claude-code",
            "access_token": OAUTH_TOKEN,
            "expires_at": 4_102_444_800i64
        }),
        CredentialMode::CodexOauth => json!({
            "type": "oauth",
            "provider": "openai-codex",
            "access_token": OAUTH_TOKEN,
            "expires_at": 4_102_444_800i64,
            "provider_extra": { "account_id": ACCOUNT_ID }
        }),
    };
    let vault = register_worker(engine_url, InitOptions::default());
    vault.register_function(
        "auth::get_token",
        RegisterFunction::new_async(move |_input: Value| {
            let credential = credential.clone();
            async move { Ok::<Value, IiiError>(credential) }
        }),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let listed = call(
            &vault,
            "engine::functions::list",
            json!({ "include_internal": true }),
        )
        .await
        .ok()
        .and_then(|value| value.get("functions").cloned())
        .and_then(|value| value.as_array().cloned())
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("function_id").and_then(Value::as_str) == Some("auth::get_token")
            })
        });
        if listed || Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Some(vault)
}

async fn configure(router: &IIIClient, case: ProviderCase, endpoint: &str) -> anyhow::Result<()> {
    let slice = match case.credential {
        CredentialMode::ApiKey => json!({ "api_key": API_KEY, "api_url": endpoint }),
        CredentialMode::ClaudeOauth | CredentialMode::CodexOauth => {
            json!({ "api_url": endpoint })
        }
    };
    call(
        router,
        "configuration::set",
        json!({
            "id": "llm-router",
            "value": {
                "settings": {
                    "retry_max": 1,
                    "stream_timeout_ms": 15_000,
                    "idle_timeout_ms": 5_000
                },
                "providers": { case.id: slice }
            }
        }),
    )
    .await
    .context("configure llm-router")?;
    Ok(())
}

async fn wait_for_provider(router: &IIIClient, provider: &str) -> anyhow::Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let list = call(router, "router::provider::list", json!({})).await?;
        if let Some(found) = list["providers"]
            .as_array()
            .and_then(|providers| providers.iter().find(|item| item["id"] == provider))
        {
            return Ok(found.clone());
        }
        if Instant::now() >= deadline {
            bail!("provider {provider} did not register: {list}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

struct ChatResult {
    response: Value,
    frames: Vec<Value>,
}

async fn chat(
    engine_url: &str,
    case: ProviderCase,
    model: &str,
    request_id: &str,
) -> anyhow::Result<ChatResult> {
    let consumer = register_worker(engine_url, InitOptions::default());
    let channel = iii_sdk::helpers::create_channel(&consumer, None).await?;
    let frames = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = frames.clone();
    channel
        .reader
        .on_message(move |message| {
            if let Ok(frame) = serde_json::from_str(&message) {
                captured.lock().expect("frames lock").push(frame);
            }
        })
        .await;
    let writer_ref = channel.writer_ref.clone();
    let pump = tokio::spawn(async move {
        let _ = channel.reader.read_all().await;
    });
    let response = call(
        &consumer,
        "router::chat",
        json!({
            "writer_ref": writer_ref,
            "request_id": request_id,
            "session_id": "provider-contract-session",
            "model": model,
            "provider": case.id,
            "system_prompt": "Be concise.",
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": "contract probe" }],
                "timestamp": 1
            }],
            "tools": [{
                "name": "contract::probe",
                "description": "Return deterministic contract data.",
                "parameters": {
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"]
                }
            }]
        }),
    )
    .await?;
    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;
    consumer.shutdown();
    let collected = frames.lock().expect("frames lock").clone();
    Ok(ChatResult {
        response,
        frames: collected,
    })
}

async fn run_contract(case: ProviderCase) -> anyhow::Result<()> {
    eprintln!("provider contract: {} ({})", case.id, case.family.id());
    let engine = Engine::start().await?;
    let isolated_home = tempfile::tempdir()?;
    std::env::set_var("HOME", isolated_home.path());
    std::env::set_var("CODEX_HOME", isolated_home.path().join("codex"));
    std::env::set_var("CLAUDE_CONFIG_DIR", isolated_home.path().join("claude"));
    std::env::set_var("PROVIDER_READ_TIMEOUT_SECS", "5");

    let stub = StubUpstream::start(case.family).await?;
    stub.respond([StubResponse::sse(happy_sse(case.family))]);
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone())
        .await
        .context("register router")?;
    configure(&router, case, &stub.endpoint(case.generation_path)).await?;
    let vault = register_fake_vault(&engine.url, case.credential).await;
    let provider = register_worker(&engine.url, InitOptions::default());
    (case.register)(provider.clone()).await?;

    let listed = wait_for_provider(&router, case.id).await?;
    match case.credential {
        CredentialMode::ApiKey => anyhow::ensure!(
            listed["configured"] == true,
            "API-key provider not configured: {listed}"
        ),
        CredentialMode::ClaudeOauth | CredentialMode::CodexOauth => anyhow::ensure!(
            listed["available"] == true,
            "OAuth provider not available: {listed}"
        ),
    }
    let token = call(
        &provider,
        "state::get",
        json!({
            "scope": format!("provider-{}", case.id),
            "key": "registration_token"
        }),
    )
    .await?;
    anyhow::ensure!(
        token.as_str().is_some_and(|value| !value.is_empty()),
        "registration token was not persisted: {token}"
    );

    // Successful streaming plus exact request/auth/tool serialization.
    stub.clear_requests();
    stub.respond([StubResponse::sse(happy_sse(case.family))]);
    let first = chat(&engine.url, case, case.model, "contract-happy-1").await?;
    anyhow::ensure!(
        first.response["ok"] == true,
        "happy response: {}",
        first.response
    );
    assert_terminal(&first.frames, "done")?;
    let requests = stub.post_requests();
    anyhow::ensure!(
        requests.len() == 1,
        "happy request count: {}",
        requests.len()
    );
    assert_request(case, &requests[0], case.upstream_model)?;

    // A second model is honored without restarting the provider.
    stub.clear_requests();
    stub.respond([StubResponse::sse(happy_sse(case.family))]);
    let second = chat(&engine.url, case, case.alternate_model, "contract-happy-2").await?;
    anyhow::ensure!(
        second.response["ok"] == true,
        "second model: {}",
        second.response
    );
    let requests = stub.post_requests();
    anyhow::ensure!(
        requests.len() == 1,
        "second request count: {}",
        requests.len()
    );
    assert_request(case, &requests[0], case.alternate_upstream_model)?;

    // Billing/quota failures are terminal and are never retried.
    stub.clear_requests();
    stub.respond([quota_response(case)]);
    let quota = chat(&engine.url, case, case.model, "contract-quota").await?;
    anyhow::ensure!(
        quota.response["ok"] == false,
        "quota response: {}",
        quota.response
    );
    assert_error_kind(&quota.frames, "permanent")?;
    anyhow::ensure!(stub.post_requests().len() == 1, "quota request was retried");

    // A pre-content transient failure is retried once by the router.
    stub.clear_requests();
    stub.respond([
        StubResponse::json(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":{"message":"temporary upstream failure","type":"server_error"}}"#,
        ),
        StubResponse::sse(happy_sse(case.family)),
    ]);
    let transient = chat(&engine.url, case, case.model, "contract-transient").await?;
    anyhow::ensure!(
        transient.response["ok"] == true,
        "transient response: {}",
        transient.response
    );
    anyhow::ensure!(
        stub.post_requests().len() == 2,
        "transient retry count mismatch"
    );

    // Router abort reaches the provider and cancels the in-flight HTTP body.
    stub.clear_requests();
    let cancelled = Arc::new(AtomicBool::new(false));
    stub.respond([StubResponse::hanging(cancelled.clone())]);
    let engine_url = engine.url.clone();
    let abort_case = case;
    let pending = tokio::spawn(async move {
        chat(&engine_url, abort_case, abort_case.model, "contract-abort").await
    });
    stub.wait_for_post_count(1).await?;
    let aborted = call(
        &router,
        "router::abort",
        json!({ "request_id": "contract-abort" }),
    )
    .await?;
    anyhow::ensure!(aborted["aborted"] == true, "abort response: {aborted}");
    let aborted_chat = tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .context("aborted chat timed out")???;
    anyhow::ensure!(
        aborted_chat.response["stop_reason"] == "aborted",
        "aborted chat response: {}",
        aborted_chat.response
    );
    let cancel_deadline = Instant::now() + Duration::from_secs(5);
    while !cancelled.load(Ordering::SeqCst) && Instant::now() < cancel_deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    anyhow::ensure!(
        cancelled.load(Ordering::SeqCst),
        "upstream body was not cancelled"
    );

    // Authentication errors are terminal and clear the provider cache.
    stub.clear_requests();
    stub.respond([auth_response(case.family)]);
    let auth = chat(&engine.url, case, case.model, "contract-auth").await?;
    anyhow::ensure!(
        auth.response["ok"] == false,
        "auth response: {}",
        auth.response
    );
    assert_error_kind(&auth.frames, "auth_expired")?;
    anyhow::ensure!(stub.post_requests().len() == 1, "auth request was retried");

    write_contract_result(case)?;

    if let Some(vault) = vault {
        vault.shutdown();
    }
    provider.shutdown();
    router.shutdown();
    Ok(())
}

fn write_contract_result(case: ProviderCase) -> anyhow::Result<()> {
    let Some(directory) = std::env::var_os("PROVIDER_CONTRACT_ARTIFACTS_DIR") else {
        return Ok(());
    };
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    let result = json!({
        "schema_version": 1,
        "provider": case.id,
        "protocol_family": case.family.id(),
        "status": "passed",
        "network": "loopback-only",
        "credentials": "synthetic",
        "scenarios": [
            "registration-and-configuration",
            "request-and-stream",
            "model-change",
            "quota-no-retry",
            "transient-retry",
            "abort",
            "auth-no-retry"
        ]
    });
    let filename = format!("{}-{}.json", case.id, case.family.id());
    std::fs::write(
        directory.join(filename),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(())
}

fn assert_terminal(frames: &[Value], expected: &str) -> anyhow::Result<()> {
    let last = frames.last().context("stream emitted no frames")?;
    anyhow::ensure!(last["type"] == expected, "terminal frame: {last}");
    Ok(())
}

fn assert_error_kind(frames: &[Value], expected: &str) -> anyhow::Result<()> {
    let last = frames.last().context("error stream emitted no frames")?;
    anyhow::ensure!(last["type"] == "error", "terminal frame: {last}");
    anyhow::ensure!(
        last["error"]["error_kind"] == expected,
        "error kind mismatch: {last}"
    );
    Ok(())
}

fn assert_request(
    case: ProviderCase,
    request: &CapturedRequest,
    expected_model: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        request.path == case.generation_path,
        "unexpected endpoint; request={}",
        serde_json::to_string(&request.redacted())?
    );
    match case.credential {
        CredentialMode::ApiKey if case.family == ProtocolFamily::AnthropicMessages => {
            anyhow::ensure!(
                request.header("x-api-key") == Some(API_KEY),
                "x-api-key missing"
            );
        }
        CredentialMode::ApiKey => {
            anyhow::ensure!(
                request.header("authorization") == Some(&format!("Bearer {API_KEY}")),
                "bearer key missing"
            );
        }
        CredentialMode::ClaudeOauth => {
            anyhow::ensure!(
                request.header("authorization") == Some(&format!("Bearer {OAUTH_TOKEN}")),
                "Claude OAuth bearer missing"
            );
        }
        CredentialMode::CodexOauth => {
            anyhow::ensure!(
                request.header("authorization") == Some(&format!("Bearer {OAUTH_TOKEN}")),
                "Codex OAuth bearer missing"
            );
            anyhow::ensure!(
                request.header("chatgpt-account-id") == Some(ACCOUNT_ID),
                "Codex account header missing"
            );
        }
    }
    let body: Value = serde_json::from_str(&request.body).context("request body JSON")?;
    anyhow::ensure!(body["model"] == expected_model, "request model: {body}");
    anyhow::ensure!(body["stream"] == true, "stream flag missing: {body}");
    anyhow::ensure!(
        body.get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty()),
        "tool schema missing: {body}"
    );
    Ok(())
}

fn happy_sse(family: ProtocolFamily) -> &'static str {
    match family {
        ProtocolFamily::AnthropicMessages => concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"provider contract ok\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        ),
        ProtocolFamily::OpenAiChatCompletions => concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"provider contract ok\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\n",
            "data: [DONE]\n\n"
        ),
        ProtocolFamily::OpenAiResponses => concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"provider contract ok\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":3}}}\n\n"
        ),
    }
}

fn models_body(family: ProtocolFamily) -> &'static str {
    match family {
        ProtocolFamily::AnthropicMessages => {
            r#"{"data":[{"id":"claude-sonnet-4-6","display_name":"Claude Sonnet 4.6","max_input_tokens":200000,"max_tokens":8192}]}"#
        }
        ProtocolFamily::OpenAiChatCompletions => {
            r#"{"data":[{"id":"provider-contract-model","object":"model","owned_by":"provider-contract"}]}"#
        }
        ProtocolFamily::OpenAiResponses => {
            r#"{"data":[{"id":"gpt-5.2","object":"model"}],"models":[{"slug":"gpt-5.2","display_name":"GPT 5.2","visibility":"list","priority":1,"context_window":128000,"supported_reasoning_levels":[],"input_modalities":["text"]}]}"#
        }
    }
}

fn auth_response(family: ProtocolFamily) -> StubResponse {
    match family {
        ProtocolFamily::AnthropicMessages => StubResponse::json(
            StatusCode::UNAUTHORIZED,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid test credential"}}"#,
        ),
        ProtocolFamily::OpenAiChatCompletions | ProtocolFamily::OpenAiResponses => {
            StubResponse::json(
                StatusCode::UNAUTHORIZED,
                r#"{"error":{"message":"invalid test credential","type":"authentication_error","code":"invalid_api_key"}}"#,
            )
        }
    }
}

fn quota_response(case: ProviderCase) -> StubResponse {
    match case.id {
        "anthropic" | "claude-code" => StubResponse::json(
            StatusCode::BAD_REQUEST,
            r#"{"type":"error","error":{"type":"billing_error","message":"credit balance is too low"}}"#,
        ),
        "deepseek" => StubResponse::json(
            StatusCode::PAYMENT_REQUIRED,
            r#"{"error":{"message":"insufficient balance","type":"invalid_request_error","code":"insufficient_balance"}}"#,
        ),
        "openrouter" => StubResponse::json(
            StatusCode::PAYMENT_REQUIRED,
            r#"{"error":{"message":"insufficient credits","code":"insufficient_credits"}}"#,
        ),
        "kimi" => StubResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"quota exhausted","type":"exceeded_current_quota_error","code":"insufficient_quota"}}"#,
        ),
        "xai" => StubResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"quota exhausted","type":"invalid_request_error","code":"insufficient_quota"}}"#,
        ),
        "zai" => StubResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"insufficient balance","code":"1113"}}"#,
        ),
        _ => StubResponse::json(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"message":"You have no credits remaining.","type":"insufficient_quota","code":"credit_balance_exhausted"}}"#,
        ),
    }
}

fn enabled_cases() -> Vec<ProviderCase> {
    let mut cases = Vec::new();
    #[cfg(feature = "provider-anthropic")]
    cases.push(ProviderCase {
        id: "anthropic",
        family: ProtocolFamily::AnthropicMessages,
        model: "claude-sonnet-4-6",
        alternate_model: "claude-opus-4-8",
        upstream_model: "claude-sonnet-4-6",
        alternate_upstream_model: "claude-opus-4-8",
        generation_path: "/v1/messages",
        credential: CredentialMode::ApiKey,
        register: |iii| {
            Box::pin(async move {
                provider_anthropic::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    });
    #[cfg(feature = "provider-claude-code")]
    cases.push(ProviderCase {
        id: "claude-code",
        family: ProtocolFamily::AnthropicMessages,
        model: "claude-code/claude-sonnet-4-6",
        alternate_model: "claude-code/claude-opus-4-8",
        upstream_model: "claude-sonnet-4-6",
        alternate_upstream_model: "claude-opus-4-8",
        generation_path: "/v1/messages",
        credential: CredentialMode::ClaudeOauth,
        register: |iii| {
            Box::pin(async move {
                provider_claude_code::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    });
    #[cfg(feature = "provider-deepseek")]
    cases.push(openai_chat_case(
        "deepseek",
        "deepseek-v4-pro",
        "deepseek-v4-flash",
        "/chat/completions",
        |iii| {
            Box::pin(async move {
                provider_deepseek::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    ));
    #[cfg(feature = "provider-kimi")]
    cases.push(openai_chat_case(
        "kimi",
        "kimi-k2-0905-preview",
        "kimi-k2-thinking",
        "/v1/chat/completions",
        |iii| {
            Box::pin(async move {
                provider_kimi::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    ));
    #[cfg(feature = "provider-openai")]
    {
        cases.push(ProviderCase {
            id: "openai",
            family: ProtocolFamily::OpenAiResponses,
            model: "gpt-5.2",
            alternate_model: "gpt-5.6-luna",
            upstream_model: "gpt-5.2",
            alternate_upstream_model: "gpt-5.6-luna",
            generation_path: "/v1/responses",
            credential: CredentialMode::ApiKey,
            register: |iii| {
                Box::pin(async move {
                    provider_openai::register::register_provider(iii)
                        .await
                        .map_err(anyhow::Error::from)
                })
            },
        });
        cases.push(openai_chat_case(
            "openai",
            "gpt-5.2",
            "gpt-5.6-luna",
            "/v1/chat/completions",
            |iii| {
                Box::pin(async move {
                    provider_openai::register::register_provider(iii)
                        .await
                        .map_err(anyhow::Error::from)
                })
            },
        ));
    }
    #[cfg(feature = "provider-openai-codex")]
    cases.push(ProviderCase {
        id: "openai-codex",
        family: ProtocolFamily::OpenAiResponses,
        model: "codex/gpt-5.2",
        alternate_model: "codex/gpt-5.6-luna",
        upstream_model: "gpt-5.2",
        alternate_upstream_model: "gpt-5.6-luna",
        generation_path: "/backend-api/codex/responses",
        credential: CredentialMode::CodexOauth,
        register: |iii| {
            Box::pin(async move {
                provider_openai_codex::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    });
    #[cfg(feature = "provider-openrouter")]
    cases.push(openai_chat_case(
        "openrouter",
        "openrouter/vendor-a/agentic",
        "openrouter/vendor-b/reasoning",
        "/api/v1/chat/completions",
        |iii| {
            Box::pin(async move {
                provider_openrouter::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    ));
    #[cfg(feature = "provider-xai")]
    cases.push(openai_chat_case(
        "xai",
        "grok-4",
        "grok-4-fast",
        "/v1/chat/completions",
        |iii| {
            Box::pin(async move {
                provider_xai::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    ));
    #[cfg(feature = "provider-zai")]
    cases.push(openai_chat_case(
        "zai",
        "glm-4.7",
        "glm-5",
        "/api/coding/paas/v4/chat/completions",
        |iii| {
            Box::pin(async move {
                provider_zai::register::register_provider(iii)
                    .await
                    .map_err(anyhow::Error::from)
            })
        },
    ));
    cases
}

fn openai_chat_case(
    id: &'static str,
    model: &'static str,
    alternate_model: &'static str,
    generation_path: &'static str,
    register: RegisterProvider,
) -> ProviderCase {
    let upstream_model = model.strip_prefix("openrouter/").unwrap_or(model);
    let alternate_upstream_model = alternate_model
        .strip_prefix("openrouter/")
        .unwrap_or(alternate_model);
    ProviderCase {
        id,
        family: ProtocolFamily::OpenAiChatCompletions,
        model,
        alternate_model,
        upstream_model,
        alternate_upstream_model,
        generation_path,
        credential: CredentialMode::ApiKey,
        register,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires III_ENGINE_BIN; executed by the provider-contract CI job"]
    async fn provider_contract() {
        let cases = enabled_cases();
        assert!(
            !cases.is_empty(),
            "enable exactly one provider-* feature when running this contract"
        );
        for case in cases {
            if let Err(error) = run_contract(case).await {
                panic!("{} contract failed: {error:#}", case.id);
            }
        }
    }

    #[test]
    fn rendered_requests_are_redacted() {
        let request = CapturedRequest {
            method: "POST".into(),
            path: "/v1/responses".into(),
            headers: vec![
                ("authorization".into(), "Bearer secret".into()),
                ("x-api-key".into(), "secret".into()),
                ("content-type".into(), "application/json".into()),
            ],
            body: "{}".into(),
        };
        let redacted = serde_json::to_string(&request.redacted()).unwrap();
        assert!(!redacted.contains("secret"));
        assert!(redacted.contains("<redacted>"));
    }

    #[test]
    fn every_enabled_provider_has_one_or_more_cases() {
        #[allow(unused_mut)]
        let mut expected = 0;
        #[cfg(feature = "provider-anthropic")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-claude-code")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-deepseek")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-kimi")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-openai")]
        {
            expected += 2;
        }
        #[cfg(feature = "provider-openai-codex")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-openrouter")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-xai")]
        {
            expected += 1;
        }
        #[cfg(feature = "provider-zai")]
        {
            expected += 1;
        }
        assert_eq!(enabled_cases().len(), expected);
    }
}
