//! End-to-end: spawn the `iii` engine and the `rbac-proxy` worker, register an
//! auth function + target functions on the engine, then drive a downstream
//! worker **through the proxy port** and assert the RBAC contract.
//!
//! Self-skips (printing why) when:
//!   - the `iii` binary is not on `PATH`, or
//!   - the proxy port is already taken, or
//!   - the proxy does not come up within the timeout (e.g. the `configuration`
//!     worker — a required boot dependency — is not deployed in this engine).
//!
//! Covers spec *Testing* (a)–(e): an exposed call succeeds, a forbidden call
//! rejects with `FORBIDDEN`, a trigger bound to a forbidden function is
//! rejected at registration, a channel round-trips through `/ws/channels/{id}`,
//! and `engine::functions::list` returns only exposed ids.

use std::collections::HashMap;
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

const ENGINE_WS: &str = "ws://127.0.0.1:49134";
const PROXY_PORT: u16 = 49271;

struct Harness {
    proxy: Child,
    iii: Child,
    _support: IIIClient,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.proxy.kill();
        let _ = self.proxy.wait();
        let _ = self.iii.kill();
        let _ = self.iii.wait();
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
    let iii_bin = which::which("iii").ok()?;

    if TcpStream::connect(("127.0.0.1", PROXY_PORT)).await.is_ok() {
        eprintln!("skipping: port {PROXY_PORT} already in use on 127.0.0.1");
        return None;
    }

    let iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    if !wait_for_listen(49134, Duration::from_secs(8)).await {
        eprintln!("skipping: engine did not come up on :49134");
        return Some(Harness {
            proxy: Command::new("true").spawn().ok()?,
            iii,
            _support: register_support(),
        });
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
    assert_eq!(echoed["hi"], json!(1), "exposed call should echo the payload");

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

    let _ = std::fs::remove_file(&seed_path);
}
