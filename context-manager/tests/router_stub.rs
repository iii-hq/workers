//! The production `RouterSummarizer` against a stub `router::chat` on a
//! private engine (D161, D162): the stub records each request, so the
//! caller's `provider_options` (the conversation's account) are checked on
//! the wire `context::compact` and an inline `context::assemble` compaction
//! send; and a usage-limit reply comes back as `failure.kind: "usage_limit"`
//! with the router's `error_details`. Before the stub registers, the engine's
//! own answer for a missing `router::chat` reads as `unavailable`.
//!
//! Needs `iii` on PATH; without it the test prints `[skip]` and passes. The
//! engine runs on a free loopback port with a throwaway `HOME`, so no
//! developer engine or `~/.iii` is touched.

#[path = "common/fakes.rs"]
#[allow(dead_code)]
mod fakes;

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use context_manager::adapters::router::RouterSummarizer;
use context_manager::config::WorkerConfig;
use context_manager::functions::{assemble, compact};
use context_manager::ports::{Deps, lease_cell};
use fakes::{FakeClock, FakeModelResolver, InMemoryLeaseStore};
use iii_sdk::errors::Error;
use iii_sdk::protocol::CallRequest;
use iii_sdk::{IIIClient, InitOptions, OutputStream, RegisterFunction, register_worker};
use serde_json::{Value, json};
use tokio::sync::RwLock;

const SUMMARY: &str = "## Goal\n- stub summary";

/// Kills and reaps the engine on drop, so a panic cannot orphan it.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `iii` on a free loopback port with `dir` as its `HOME`; `None` when it is
/// not on PATH or never listens.
fn spawn_engine(dir: &Path) -> Option<(Reaped, String)> {
    let iii_bin = which::which("iii").ok()?;
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("free port")
        .port();
    let config = dir.join("engine.yaml");
    std::fs::write(
        &config,
        format!(
            "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n"
        ),
    )
    .expect("write engine config");
    let engine = Command::new(iii_bin)
        .args(["--no-update-check", "--config"])
        .arg(&config)
        .current_dir(dir)
        .env("HOME", dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(Reaped)
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Some((engine, format!("ws://127.0.0.1:{port}")))
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Summary,
    UsageLimit,
}

#[derive(Clone)]
struct Stub {
    mode: Arc<Mutex<Mode>>,
    /// Every request, without its `writer_ref`.
    requests: Arc<Mutex<Vec<Value>>>,
}

impl Stub {
    fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }

    fn set(&self, mode: Mode) {
        *self.mode.lock().unwrap() = mode;
        self.requests.lock().unwrap().clear();
    }
}

async fn chat(stub: Stub, input: Value) -> Result<Value, Error> {
    if input.get("probe").is_some() {
        return Ok(json!({ "ok": true }));
    }
    let mut recorded = input.clone();
    if let Some(object) = recorded.as_object_mut() {
        object.remove("writer_ref");
    }
    stub.requests.lock().unwrap().push(recorded);
    let mode = *stub.mode.lock().unwrap();
    if mode == Mode::UsageLimit {
        return Ok(json!({
            "ok": false, "provider": "claude-code", "model": "claude-x",
            "stop_reason": "error",
            "error": {
                "code": "router/provider_rate_limited",
                "kind": "rate_limited",
                "message": "Provider \"claude-code\" reached the usage limit of its account. Try again after the limit resets.",
                "retryable": false,
                "error_details": { "usage_limit": true, "window": "five_hour",
                                   "resets_at_ms": 1_700_000_000_000u64 }
            }
        }));
    }
    let writer: OutputStream = serde_json::from_value(input["writer_ref"].clone())
        .map_err(|error| Error::Serde(error.to_string()))?;
    let done = json!({ "type": "done", "message": {
        "role": "assistant", "content": [{ "type": "text", "text": SUMMARY }],
        "stop_reason": "end", "model": "claude-x", "provider": "claude-code", "timestamp": 1
    }});
    writer.send_text(&done.to_string()).await?;
    writer.close().await?;
    Ok(json!({ "ok": true, "provider": "claude-code", "model": "claude-x", "stop_reason": "end" }))
}

/// A connection with its own worker name: two connections from one process
/// under the auto-detected name conflict.
fn connect(url: &str, name: &str) -> IIIClient {
    let metadata = iii_sdk::iii::WorkerMetadata {
        name: name.to_string(),
        pid: Some(std::process::id()),
        ..Default::default()
    };
    register_worker(
        url,
        InitOptions {
            metadata: Some(metadata),
            ..Default::default()
        },
    )
}

/// Register the stub on its own connection, as llm-router would be, and
/// wait until the engine routes to it.
async fn start_stub(url: &str) -> (IIIClient, Stub) {
    let iii = connect(url, "stub-llm-router");
    let stub = Stub {
        mode: Arc::new(Mutex::new(Mode::Summary)),
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let handler = stub.clone();
    iii.register_function(
        "router::chat",
        RegisterFunction::new_async(move |input: Value| {
            let stub = handler.clone();
            async move { chat(stub, input).await }
        }),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let probe = iii
            .call(CallRequest {
                function_id: "router::chat".into(),
                payload: json!({ "probe": true }),
                action: None,
                timeout_ms: Some(1_000),
            })
            .await;
        if probe.is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the stub router::chat never became routable: {probe:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    (iii, stub)
}

fn prose(pairs: usize) -> Vec<Value> {
    let mut messages = Vec::new();
    for i in 0..pairs {
        let text = |role: &str| format!("{role} {i}: {}", "lorem ".repeat(1_666));
        messages.push(json!({
            "role": "user", "content": [{ "type": "text", "text": text("user") }], "timestamp": 1
        }));
        messages.push(json!({
            "role": "assistant", "content": [{ "type": "text", "text": text("assistant") }],
            "stop_reason": "end", "model": "claude-x", "provider": "claude-code", "timestamp": 2
        }));
    }
    messages
}

fn account() -> Value {
    json!({ "claude-code": { "account": "acct-work" } })
}

fn model() -> Value {
    json!({ "id": "claude-x", "provider": "claude-code",
            "limits": { "context_window": 100_000, "max_output_tokens": 10_000 } })
}

async fn compact(deps: &Deps, options: Value) -> Value {
    let request = serde_json::from_value(json!({
        "messages": prose(4), "model": model(), "options": options,
    }))
    .unwrap();
    serde_json::to_value(compact::handle(deps, request).await.unwrap()).unwrap()
}

async fn assemble(deps: &Deps, options: Value) -> Value {
    let request = serde_json::from_value(json!({
        "messages": prose(8), "model": model(), "system_prompt": "You are an iii agent.",
        "options": options,
    }))
    .unwrap();
    serde_json::to_value(assemble::handle(deps, request).await.unwrap()).unwrap()
}

async fn run(url: String) {
    let iii = Arc::new(connect(&url, "context-manager-under-test"));
    let config = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
    let deps = Deps {
        summarizer: Arc::new(RouterSummarizer::new(iii.clone(), config.clone())),
        resolver: Arc::new(FakeModelResolver::new()),
        leases: lease_cell(Arc::new(InMemoryLeaseStore::new())),
        clock: Arc::new(FakeClock::new()),
        config,
    };

    // No router::chat yet: the engine's own `function_not_found` reads as
    // "no summariser".
    let out = compact(&deps, json!({})).await;
    assert_eq!(out["status"], "overflow", "{out}");
    assert_eq!(out["failure"]["kind"], "unavailable", "{out}");

    let (stub_client, stub) = start_stub(&url).await;

    // context::compact: the account rides the router::chat request.
    let out = compact(&deps, json!({ "provider_options": account() })).await;
    assert_eq!(out["status"], "ok", "{out}");
    assert_eq!(out["summary"], SUMMARY);
    let requests = stub.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["provider_options"], account());
    assert_eq!(requests[0]["provider"], "claude-code");

    // Without provider_options the request carries none.
    stub.set(Mode::Summary);
    compact(&deps, json!({})).await;
    assert!(stub.requests()[0].get("provider_options").is_none());

    // An inline compaction at the budget carries it too.
    stub.set(Mode::Summary);
    let out = assemble(
        &deps,
        json!({ "reserved_tokens": 0, "compaction_budget": 30_000,
                "provider_options": account() }),
    )
    .await;
    assert_eq!(
        out["compaction"],
        json!({ "trigger": "budget", "status": "compacted" })
    );
    assert_eq!(out["applied"]["summary"], SUMMARY);
    let requests = stub.requests();
    assert!(!requests.is_empty());
    for request in &requests {
        assert_eq!(request["provider_options"], account());
    }

    // A usage limit comes back as its cause, with the router's details.
    stub.set(Mode::UsageLimit);
    let out = compact(&deps, json!({ "provider_options": account() })).await;
    assert_eq!(out["status"], "overflow", "{out}");
    assert_eq!(out["failure"]["kind"], "usage_limit", "{out}");
    assert_eq!(
        out["failure"]["error_details"],
        json!({ "usage_limit": true, "window": "five_hour", "resets_at_ms": 1_700_000_000_000u64 })
    );
    assert!(
        out["failure"]["message"]
            .as_str()
            .unwrap()
            .contains("usage limit")
    );
    // Never retried by the summariser: one call.
    assert_eq!(stub.requests().len(), 1);

    stub.set(Mode::UsageLimit);
    let out = assemble(
        &deps,
        json!({ "reserved_tokens": 0, "compaction_budget": 30_000,
                "provider_options": account() }),
    )
    .await;
    assert_eq!(out["compaction"]["status"], "failed", "{out}");
    assert_eq!(out["compaction"]["failure"]["kind"], "usage_limit");
    assert_eq!(out["applied"]["compacted"], false);

    iii.shutdown_async().await;
    stub_client.shutdown_async().await;
}

/// One test in this binary: it sets `HOME` before any runtime thread starts.
#[test]
fn the_router_adapter_forwards_the_account_and_reports_a_usage_limit() {
    let scratch = tempfile::tempdir().expect("tempdir");
    // SAFETY: the only test in this binary, before its runtime starts; the
    // private HOME keeps the SDK clients' `iii ps` markers out of ~/.iii.
    unsafe { std::env::set_var("HOME", scratch.path()) };
    let Some((_engine, url)) = spawn_engine(scratch.path()) else {
        eprintln!("[skip] no `iii` on PATH, or it never listened");
        return;
    };
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run(url));
}
