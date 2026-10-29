//! End-to-end: spawn the `iii` engine and the `rbac-proxy` worker, register an
//! auth function + target functions on the engine, then drive a downstream
//! worker **through the proxy port** and assert the RBAC contract.
//!
//! The engine is the `iii` on `PATH`, or `III_BIN`. It runs from its own
//! temporary directory on its own port ([`ENGINE_PORT`], not the default
//! 49134), so the test never reaches an engine a developer has running.
//!
//! Self-skips (printing why) when:
//!   - there is no `iii` binary, or
//!   - the engine or proxy port is already taken, or
//!   - the proxy does not come up within the timeout (e.g. the `configuration`
//!     worker — a required boot dependency — is not deployed in this engine).
//!
//! Covers spec *Testing* (a)–(e): an exposed call succeeds, a forbidden call
//! rejects with `FORBIDDEN`, a trigger bound to a forbidden function is
//! rejected at registration, a channel round-trips through `/ws/channels/{id}`,
//! and `engine::functions::list` returns only exposed ids. Also: an exposed id
//! called in another namespace is `FORBIDDEN`, and `observe` is refused.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;

const ENGINE_PORT: u16 = 49381;
const ENGINE_WS: &str = "ws://127.0.0.1:49381";
const PROXY_PORT: u16 = 49382;

struct Harness {
    proxy: Child,
    iii: Child,
    _support: IIIClient,
    dir: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.proxy.kill();
        let _ = self.proxy.wait();
        kill_tree(&mut self.iii);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Stop the engine and anything it spawned.
fn kill_tree(child: &mut Child) {
    let _ = Command::new("pkill")
        .args(["-P", &child.id().to_string()])
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

fn iii_bin() -> Option<PathBuf> {
    match std::env::var_os("III_BIN").filter(|path| !path.is_empty()) {
        Some(path) => Some(PathBuf::from(path)),
        None => which::which("iii").ok(),
    }
}

async fn wait_for_listen(port: u16, max: Duration) -> bool {
    let deadline = std::time::Instant::now() + max;
    while std::time::Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Register the auth function and target functions on the engine's internal
/// listener (the proxy fronts these from its own port).
fn register_support() -> IIIClient {
    let support = register_worker(
        ENGINE_WS,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                name: "rbac-proxy-itest-support".to_string(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );

    // Auth: accept every upgrade with the default (empty) AuthResult — the
    // proxy's `expose_functions` does the gating. A context is attached so
    // middleware/hook plumbing has something to carry.
    support.register_function(
        "test::auth",
        RegisterFunction::new_async(|_input: Value| async move {
            Ok::<Value, Error>(json!({ "context": { "tenant": "itest" } }))
        }),
    );
    support.register_function(
        "api::echo",
        RegisterFunction::new_async(|input: Value| async move { Ok::<Value, Error>(input) }),
    );
    support.register_function(
        "secret::echo",
        RegisterFunction::new_async(|input: Value| async move { Ok::<Value, Error>(input) }),
    );
    support
}

async fn boot(seed_path: &str) -> Option<Harness> {
    let Some(iii_bin) = iii_bin() else {
        eprintln!("skipping: no `iii` binary (set III_BIN or put iii on PATH)");
        return None;
    };

    for port in [ENGINE_PORT, PROXY_PORT] {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            eprintln!("skipping: port {port} already in use on 127.0.0.1");
            return None;
        }
    }

    // The engine keeps its state (configuration entries) under its working
    // directory: a fresh one per run.
    let dir = std::env::temp_dir().join(format!("rbac-proxy-itest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(
        dir.join("config.yaml"),
        format!(
            "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {ENGINE_PORT}\n"
        ),
    )
    .ok()?;

    let spawned = Command::new(&iii_bin)
        .args(["-c", "config.yaml"])
        .current_dir(&dir)
        .env_remove("III_URL")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut iii = match spawned {
        Ok(child) => child,
        Err(e) => {
            eprintln!("skipping: could not start {}: {e}", iii_bin.display());
            let _ = std::fs::remove_dir_all(&dir);
            return None;
        }
    };

    if !wait_for_listen(ENGINE_PORT, Duration::from_secs(15)).await {
        eprintln!("skipping: engine did not come up on :{ENGINE_PORT}");
        kill_tree(&mut iii);
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }

    // Register auth + targets, give the engine a moment to index them.
    let support = register_support();
    sleep(Duration::from_millis(600)).await;

    // Seed config: RBAC on, exposing only api::* and the functions-discovery
    // surface.
    std::fs::write(
        seed_path,
        format!(
            "host: 127.0.0.1\nport: {PROXY_PORT}\nengine_url: {ENGINE_WS}\n\
             rbac:\n  auth_function_id: test::auth\n  expose_functions:\n    - match(\"api::*\")\n    - match(\"engine::functions::*\")\n"
        ),
    )
    .ok()?;

    let proxy_bin = env!("CARGO_BIN_EXE_rbac-proxy");
    let proxy = Command::new(proxy_bin)
        .args(["--url", ENGINE_WS, "--config", seed_path])
        .env_remove("III_URL")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    if !wait_for_listen(PROXY_PORT, Duration::from_secs(10)).await {
        eprintln!(
            "skipping: rbac-proxy did not come up on :{PROXY_PORT} \
             (is the `configuration` worker deployed in this engine?)"
        );
        let mut h = Harness {
            proxy,
            iii,
            _support: support,
            dir,
        };
        // Ensure the proxy child is reaped by the Drop; signal skip via None.
        let _ = h.proxy.kill();
        drop(h);
        return None;
    }

    Some(Harness {
        proxy,
        iii,
        _support: support,
        dir,
    })
}

fn downstream() -> IIIClient {
    let mut headers = HashMap::new();
    headers.insert("authorization".to_string(), "Bearer test-token".to_string());
    register_worker(
        &format!("ws://127.0.0.1:{PROXY_PORT}"),
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                name: "rbac-proxy-itest-downstream".to_string(),
                ..WorkerMetadata::default()
            }),
            headers: Some(headers),
            ..InitOptions::default()
        },
    )
}

#[tokio::test]
async fn end_to_end_rbac_through_proxy() {
    let seed_path = format!(
        "{}/rbac-proxy-itest-seed.yaml",
        std::env::temp_dir().display()
    );
    let Some(_h) = boot(&seed_path).await else {
        eprintln!("skipping: prerequisites not met (see logs above)");
        return;
    };

    let client = downstream();
    // Give the downstream connection time to upgrade + authenticate.
    sleep(Duration::from_millis(800)).await;

    // (a) Exposed call succeeds and echoes.
    let echoed = timeout(
        Duration::from_secs(8),
        client.trigger(TriggerRequest {
            function_id: "api::echo".to_string(),
            payload: json!({ "hi": 1 }),
            action: None,
            timeout_ms: Some(5_000),
        }),
    )
    .await
    .expect("api::echo did not return in time")
    .expect("api::echo should be allowed");
    // The engine injects `_caller_worker_id` into dispatched payloads, so
    // assert the echoed field is present rather than full equality.
    assert_eq!(
        echoed["hi"],
        json!(1),
        "exposed call should echo the payload"
    );

    // (b) Forbidden call rejects with FORBIDDEN.
    let denied = client
        .trigger(TriggerRequest {
            function_id: "secret::echo".to_string(),
            payload: json!({ "x": 1 }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    match denied {
        Err(Error::Remote { code, .. }) => assert_eq!(code, "FORBIDDEN"),
        other => panic!("expected FORBIDDEN remote error, got {other:?}"),
    }

    // (c) Trigger bound to a forbidden function is rejected at registration.
    //     The Rust SDK's `register_trigger` is fire-and-forget (it never awaits
    //     the result frame), so we observe the proxy's wire reply directly with
    //     a raw client: send a RegisterTrigger for a forbidden target and read
    //     until the REGISTRATION_DENIED frame.
    {
        let url = format!("ws://127.0.0.1:{PROXY_PORT}/");
        let (mut ws, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("raw connect to proxy");
        let frame = json!({
            "type": "registertrigger",
            "id": "itest-trig-1",
            "trigger_type": "http",
            "function_id": "secret::echo",
            "config": {}
        });
        ws.send(Message::Text(frame.to_string()))
            .await
            .expect("send registertrigger");
        let denied = timeout(Duration::from_secs(5), async {
            while let Some(Ok(msg)) = ws.next().await {
                if let Message::Text(t) = msg {
                    if let Ok(v) = serde_json::from_str::<Value>(t.as_str()) {
                        if v["type"] == "triggerregistrationresult"
                            && v["error"]["code"] == "REGISTRATION_DENIED"
                        {
                            return true;
                        }
                    }
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        assert!(
            denied,
            "proxy must reply REGISTRATION_DENIED for a trigger bound to a forbidden function"
        );
        let _ = ws.close(None).await;
    }

    // (d) Channel round-trips through the proxy's /ws/channels bridge.
    let ch = client
        .trigger(TriggerRequest {
            function_id: "engine::channels::create".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("engine::channels::create is in the carve-out and must be allowed");
    let cid = ch["writer"]["channel_id"].as_str().expect("channel_id");
    let wkey = ch["writer"]["access_key"].as_str().expect("writer key");
    let rkey = ch["reader"]["access_key"].as_str().expect("reader key");

    let read_url = format!("ws://127.0.0.1:{PROXY_PORT}/ws/channels/{cid}?key={rkey}&dir=read");
    let write_url = format!("ws://127.0.0.1:{PROXY_PORT}/ws/channels/{cid}?key={wkey}&dir=write");
    let (mut reader, _) = tokio_tungstenite::connect_async(&read_url)
        .await
        .expect("dial reader channel via proxy");
    let (mut writer, _) = tokio_tungstenite::connect_async(&write_url)
        .await
        .expect("dial writer channel via proxy");
    writer
        .send(Message::Text("ping".into()))
        .await
        .expect("send on channel");
    let frame = timeout(Duration::from_secs(5), reader.next())
        .await
        .expect("channel read timed out")
        .expect("channel stream ended")
        .expect("channel read error");
    assert!(
        matches!(&frame, Message::Text(t) if t.as_str() == "ping"),
        "channel should relay the frame 1:1, got {frame:?}"
    );
    let _ = writer.close(None).await;
    let _ = reader.close(None).await;

    // (e) engine::functions::list is filtered to the exposed surface.
    let list = client
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("engine::functions::list is exposed");
    let ids: Vec<String> = list["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|f| f["function_id"].as_str().map(str::to_string))
        .collect();
    assert!(
        ids.iter().any(|id| id == "api::echo"),
        "exposed api::echo should be visible; got {ids:?}"
    );
    assert!(
        !ids.iter().any(|id| id == "secret::echo"),
        "forbidden secret::echo must be filtered out of discovery; got {ids:?}"
    );

    // (f) The exposed id, called in another namespace, is not exposed there:
    //     a rule without a namespace answers in `default` only.
    {
        let mut ws = raw_client().await;
        let reply = request(
            &mut ws,
            json!({
                "type": "invokefunction",
                "invocation_id": "11111111-1111-4111-8111-111111111111",
                "function_id": "api::echo",
                "data": {},
                "namespace": "orders"
            }),
            |v| v["type"] == "invocationresult",
        )
        .await;
        assert_eq!(
            reply["error"]["code"], "FORBIDDEN",
            "api::echo in `orders` must be refused; got {reply}"
        );
        let _ = ws.close(None).await;
    }

    // (g) `observe` is refused by the proxy (whatever the engine supports).
    {
        let mut ws = raw_client().await;
        let reply = request(
            &mut ws,
            json!({
                "type": "observe",
                "id": "5b0c3e1a-8f2d-4c6b-9a7e-1d2f3a4b5c6d",
                "source": { "function_id": "api::echo" },
                "target": { "function_id": "api::echo2" }
            }),
            |v| v["type"] == "observeresult",
        )
        .await;
        assert_eq!(reply["state"], "error");
        assert_eq!(reply["error"]["code"], "UNSUPPORTED");
        let _ = ws.close(None).await;
    }

    let _ = std::fs::remove_file(&seed_path);
}

type RawClient =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn raw_client() -> RawClient {
    let url = format!("ws://127.0.0.1:{PROXY_PORT}/");
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
            url.as_str(),
        )
        .expect("request");
    request.headers_mut().insert(
        "authorization",
        "Bearer test-token".parse().expect("header"),
    );
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("raw connect to proxy");
    ws
}

/// Send `frame` and return the first text frame `pick` accepts.
async fn request(ws: &mut RawClient, frame: Value, pick: impl Fn(&Value) -> bool) -> Value {
    ws.send(Message::Text(frame.to_string()))
        .await
        .expect("send frame");
    timeout(Duration::from_secs(5), async {
        while let Some(Ok(msg)) = ws.next().await {
            if let Message::Text(t) = msg {
                if let Ok(v) = serde_json::from_str::<Value>(t.as_str()) {
                    if pick(&v) {
                        return v;
                    }
                }
            }
        }
        panic!("connection closed before the reply");
    })
    .await
    .expect("no reply in time")
}
