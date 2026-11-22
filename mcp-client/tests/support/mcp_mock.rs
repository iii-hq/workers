//! wiremock as an MCP server over streamable HTTP.
//!
//! What rmcp 3.11.0's reqwest client does on the wire, read in its source and
//! checked by tests/http_server.rs:
//! - every POST carries `Accept: text/event-stream, application/json`, and a
//!   200 with `Content-Type: application/json` is a complete answer, so this
//!   mock answers JSON, and SSE only for `on_call_notifying`;
//! - a notification POST must come back 202, so `notifications/*` get 202 with
//!   an empty body;
//! - the GET stream is opened only when the `initialize` answer carried
//!   `Mcp-Session-Id`, a 405 to it is skipped, and the same session gets a
//!   DELETE on close where a 405 is accepted: this mock sends
//!   `Mcp-Session-Id: mock-session` and answers GET and DELETE with 405, the
//!   session path a real server takes;
//! - a 401 is `AuthRequired` only with a `WWW-Authenticate` header; without one
//!   it is `UnexpectedServerResponse("HTTP 401 Unauthorized: <body>")`;
//! - `Peer::list_tools` serves a cached answer when the server sent `ttlMs`, so
//!   this mock never sends it.
//!
//! After an `initialize` answer with `protocolVersion: "2025-11-25"`, later
//! requests carry `MCP-Protocol-Version: 2025-11-25`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

#[derive(Default)]
struct Settings {
    tools: Option<Value>,
    instructions: Option<String>,
    /// Per tool name: the `result` or `error` member of the JSON-RPC answer.
    calls: HashMap<String, Value>,
    /// Per tool name: the notification sent before the answer, on an SSE stream.
    notify: HashMap<String, String>,
    statuses: HashMap<String, (u16, Vec<(String, String)>)>,
    delays: HashMap<String, Duration>,
    /// What `POST /oauth/token` issues; set once the endpoint is mounted.
    token: Option<String>,
    /// The only `Authorization: Bearer` value the server takes.
    bearer: Option<String>,
}

struct Responder(Arc<Mutex<Settings>>);

fn rpc_error(code: i32, message: &str) -> Value {
    json!({"error": {"code": code, "message": message}})
}

impl Respond for Responder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let method = body["method"].as_str().unwrap_or("");
        let s = self.0.lock().unwrap();
        let authorization = request.headers.get("authorization");
        let refused = s.bearer.as_ref().is_some_and(|token| {
            authorization.and_then(|v| v.to_str().ok()) != Some(format!("Bearer {token}").as_str())
        });
        let answer = if refused {
            ResponseTemplate::new(401)
        } else if let Some((status, headers)) = s.statuses.get(method) {
            headers
                .iter()
                .fold(ResponseTemplate::new(*status), |t, (k, v)| {
                    t.insert_header(k.as_str(), v.as_str())
                })
        } else if let Some(id) = body.get("id") {
            let tool = body["params"]["name"].as_str().unwrap_or("");
            let mut reply = match method {
                "initialize" => {
                    let mut result = json!({
                        "protocolVersion": "2025-11-25",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "mock", "version": "1.0.0"}
                    });
                    if let Some(text) = &s.instructions {
                        result["instructions"] = json!(text);
                    }
                    json!({ "result": result })
                }
                "tools/list" => json!({"result": {"tools": s.tools.clone().unwrap_or(json!([]))}}),
                "tools/call" => s
                    .calls
                    .get(tool)
                    .cloned()
                    .unwrap_or_else(|| rpc_error(-32602, "unknown tool")),
                _ => rpc_error(-32601, "method not found"),
            };
            reply["jsonrpc"] = json!("2.0");
            reply["id"] = id.clone();
            let notification = s.notify.get(tool).filter(|_| method == "tools/call");
            let answer = match notification {
                Some(n) => {
                    let first = json!({"jsonrpc": "2.0", "method": n});
                    let events = format!("data: {first}\n\ndata: {reply}\n\n");
                    ResponseTemplate::new(200).set_body_raw(events, "text/event-stream")
                }
                None => ResponseTemplate::new(200).set_body_json(reply),
            };
            if method == "initialize" {
                answer.insert_header("Mcp-Session-Id", "mock-session")
            } else {
                answer
            }
        } else {
            ResponseTemplate::new(202)
        };
        match s.delays.get(method) {
            Some(by) => answer.set_delay(*by),
            None => answer,
        }
    }
}

pub struct McpMock {
    server: MockServer,
    settings: Arc<Mutex<Settings>>,
}

impl McpMock {
    /// Its own server, not one from wiremock's pool: dropping the mock closes
    /// the port. Every path answers, so a URL without `/mcp` works too.
    pub async fn start() -> McpMock {
        let server = MockServer::builder().start().await;
        let settings = Arc::new(Mutex::new(Settings::default()));
        Mock::given(method("POST"))
            .respond_with(Responder(settings.clone()))
            .mount(&server)
            .await;
        for verb in ["GET", "DELETE"] {
            Mock::given(method(verb))
                .respond_with(ResponseTemplate::new(405))
                .mount(&server)
                .await;
        }
        McpMock { server, settings }
    }

    /// For a test that mounts its own `Mock` in front of this one.
    pub fn server(&self) -> &MockServer {
        &self.server
    }

    pub fn url(&self) -> String {
        format!("{}/mcp", self.server.uri())
    }

    fn settings(&self) -> std::sync::MutexGuard<'_, Settings> {
        self.settings.lock().unwrap()
    }

    pub async fn tools(&self, tools: Value) {
        self.settings().tools = Some(tools);
    }

    pub async fn instructions(&self, text: &str) {
        self.settings().instructions = Some(text.into());
    }

    pub async fn on_call(&self, tool: &str, result: Value) {
        self.settings()
            .calls
            .insert(tool.into(), json!({ "result": result }));
    }

    pub async fn on_call_error(&self, tool: &str, code: i32, message: &str) {
        self.settings()
            .calls
            .insert(tool.into(), rpc_error(code, message));
    }

    /// Answers a call of `tool` on an SSE stream: first the `notification`,
    /// then `result`.
    pub async fn on_call_notifying(&self, tool: &str, result: Value, notification: &str) {
        self.on_call(tool, result).await;
        self.settings()
            .notify
            .insert(tool.into(), notification.into());
    }

    /// Answers `method` with `status`, `headers` and an empty body until
    /// `clear_http_status`.
    pub async fn http_status(&self, method: &str, status: u16, headers: &[(&str, &str)]) {
        let headers = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.settings()
            .statuses
            .insert(method.into(), (status, headers));
    }

    pub async fn clear_http_status(&self) {
        self.settings().statuses.clear();
    }

    /// Holds back the answer to `method` by `by`.
    pub async fn delay(&self, method: &str, by: Duration) {
        self.settings().delays.insert(method.into(), by);
    }

    async fn posts(&self, method: &str) -> Vec<(Value, HashMap<String, String>)> {
        let received = self.server.received_requests().await.unwrap_or_default();
        received
            .into_iter()
            .filter(|r| r.method.as_str() == "POST")
            .filter_map(|r| {
                let body: Value = serde_json::from_slice(&r.body).ok()?;
                let headers = r
                    .headers
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.as_str().to_ascii_lowercase(),
                            v.to_str().unwrap_or("").into(),
                        )
                    })
                    .collect();
                (body["method"] == method).then_some((body, headers))
            })
            .collect()
    }

    /// The JSON-RPC bodies of the POSTs for `method`, in arrival order.
    pub async fn requests(&self, method: &str) -> Vec<Value> {
        self.posts(method)
            .await
            .into_iter()
            .map(|(b, _)| b)
            .collect()
    }

    /// The headers of the POSTs for `method`, names lower-cased.
    pub async fn headers_of(&self, method: &str) -> Vec<HashMap<String, String>> {
        self.posts(method)
            .await
            .into_iter()
            .map(|(_, h)| h)
            .collect()
    }

    /// Waits until the session's GET stream was asked for (and refused with
    /// 405), so a later change to what the server takes cannot catch it.
    pub async fn wait_for_get_stream(&self) {
        let deadline = tokio::time::Instant::now() + super::WAIT;
        loop {
            let requests = self.server.received_requests().await.unwrap_or_default();
            if requests
                .iter()
                .any(|r| r.method.as_str() == "GET" && r.url.path() == "/mcp")
            {
                return;
            }
            assert!(tokio::time::Instant::now() < deadline, "no GET stream");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Mounts `POST /oauth/token`, answering a client-credentials token
    /// request with `token`; its URL. A second call replaces the token.
    pub async fn token_endpoint(&self, token: &str) -> String {
        let mounted = self.settings().token.replace(token.into()).is_some();
        if !mounted {
            let settings = self.settings.clone();
            Mock::given(method("POST"))
                .and(path("/oauth/token"))
                .respond_with(move |_: &Request| {
                    let token = settings.lock().unwrap().token.clone();
                    ResponseTemplate::new(200).set_body_json(
                        json!({"access_token": token, "token_type": "Bearer", "expires_in": 3600}),
                    )
                })
                .with_priority(1)
                .mount(&self.server)
                .await;
        }
        format!("{}/oauth/token", self.server.uri())
    }

    /// The form bodies of the token requests, in arrival order.
    pub async fn token_requests(&self) -> Vec<String> {
        let received = self.server.received_requests().await.unwrap_or_default();
        received
            .into_iter()
            .filter(|r| r.url.path() == "/oauth/token")
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    /// Answers 401 without `WWW-Authenticate` to every MCP POST whose
    /// `Authorization` is not `Bearer <token>`; a second call replaces the
    /// token. A refused request still shows in `requests`.
    pub async fn require_bearer(&self, token: &str) {
        self.settings().bearer = Some(token.into());
    }

    /// Every request of any method and path.
    pub async fn request_count(&self) -> usize {
        self.server
            .received_requests()
            .await
            .unwrap_or_default()
            .len()
    }
}

/// A mock with one tool `echo` (a string property `text`) answering `hi`.
pub async fn echo_mock() -> McpMock {
    let mock = McpMock::start().await;
    let echo = json!({"name": "echo", "description": "echo", "inputSchema": {
        "type": "object", "properties": {"text": {"type": "string"}}}});
    mock.tools(json!([echo])).await;
    mock.on_call("echo", json!({"content": [{"type": "text", "text": "hi"}]}))
        .await;
    mock
}
