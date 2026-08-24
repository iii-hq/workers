use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::bail;
use axum::body::{to_bytes, Body};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Router;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::case::ProviderCase;
use crate::protocol::models_body;

#[derive(Clone, Debug, Serialize)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl CapturedRequest {
    pub(crate) fn redacted(&self) -> Self {
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

    pub(crate) fn header(&self, wanted: &str) -> Option<&str> {
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
pub(crate) struct StubResponse {
    status: StatusCode,
    content_type: &'static str,
    body: StubBody,
}

impl StubResponse {
    pub(crate) fn sse(body: impl Into<String>) -> Self {
        Self {
            status: StatusCode::OK,
            content_type: "text/event-stream",
            body: StubBody::Complete(body.into()),
        }
    }

    pub(crate) fn json(status: StatusCode, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: StubBody::Complete(body.into()),
        }
    }

    pub(crate) fn hanging(cancelled: Arc<AtomicBool>) -> Self {
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

pub(crate) struct StubUpstream {
    address: String,
    state: Arc<StubState>,
    task: tokio::task::JoinHandle<()>,
}

impl StubUpstream {
    pub(crate) async fn start(case: ProviderCase) -> anyhow::Result<Self> {
        let state = Arc::new(StubState {
            models_body: Mutex::new(models_body(case).to_string()),
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

    pub(crate) fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.address, path)
    }

    pub(crate) fn respond(&self, responses: impl IntoIterator<Item = StubResponse>) {
        let mut plan = self.state.post_responses.lock().expect("stub plan lock");
        *plan = responses.into_iter().collect();
    }

    pub(crate) fn clear_requests(&self) {
        self.state.requests.lock().expect("requests lock").clear();
    }

    pub(crate) fn post_requests(&self) -> Vec<CapturedRequest> {
        self.state
            .requests
            .lock()
            .expect("requests lock")
            .iter()
            .filter(|request| request.method == "POST")
            .cloned()
            .collect()
    }

    pub(crate) async fn wait_for_post_count(&self, count: usize) -> anyhow::Result<()> {
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
