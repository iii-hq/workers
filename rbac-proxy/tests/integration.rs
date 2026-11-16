//! End-to-end: spawn the `iii` engine and the `rbac-proxy` worker, register an
//! auth function + target functions on the engine, then drive a downstream
//! worker **through the proxy port** and assert the RBAC contract.
//!
//! The engine is the `iii` on `PATH`, or `III_BIN`. It runs from its own
//! temporary directory on a free port picked at boot (never the default
//! 49134), so the test never reaches an engine a developer has running.
//!
//! Self-skips (printing why) when:
//!   - there is no `iii` binary, or
//!   - the proxy does not come up within the timeout (e.g. the `configuration`
//!     worker — a required boot dependency — is not deployed in this engine).
//!
//! Covers spec *Testing* (a)–(e): an exposed call succeeds, a forbidden call
//! rejects with `FORBIDDEN`, an observer whose target is a forbidden function
//! is refused with `FORBIDDEN`, a channel round-trips through
//! `/ws/channels/{id}`, and `engine::functions::list` returns only exposed
//! ids. Also: an exposed id called in another namespace is `FORBIDDEN`, an
//! observer of an exposed function with an exposed target is accepted, and an
//! engine event that lists functions cannot be observed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{CallRequest, IIIClient, InitOptions, RegisterFunction, register_worker};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;

/// A call of `function_id` with a 5 s timeout.
fn call(function_id: &str, payload: Value) -> CallRequest {
    CallRequest {
        timeout_ms: Some(5_000),
        ..CallRequest::new(function_id, payload)
    }
}

struct Harness {
    proxy: Child,
    iii: Child,
    _support: IIIClient,
    dir: PathBuf,
    proxy_port: u16,
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

/// Distinct free ports: every listener stays bound until all are picked.
fn free_ports<const N: usize>() -> [u16; N] {
    [(); N]
        .map(|_| std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port"))
        .map(|listener| listener.local_addr().expect("local addr").port())
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
fn register_support(engine_ws: &str) -> IIIClient {
    let support = register_worker(
        engine_ws,
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
    // middleware/hook plumbing has something to carry. The `narrow` token's
    // session may not reach `api::echo`. It receives bearer tokens, so nobody
    // may observe it.
    support.register_function(
        "test::auth",
        RegisterFunction::new_async(|input: Value| async move {
            let mut session = json!({ "context": { "tenant": "itest" } });
            if input["headers"]["authorization"] == "Bearer narrow" {
                session["forbidden_functions"] = json!(["api::echo"]);
            }
            Ok::<Value, Error>(session)
        })
        .observable(false),
    );
    support.register_function(
        "api::echo",
        RegisterFunction::new_async(|input: Value| async move { Ok::<Value, Error>(input) }),
    );
    support.register_function(
        "secret::echo",
        RegisterFunction::new_async(|input: Value| async move { Ok::<Value, Error>(input) }),
    );
    // An exposed observer target.
    support.register_function(
        "api::sink",
        RegisterFunction::new_async(|_input: Value| async move { Ok::<Value, Error>(Value::Null) }),
    );
    support
}

async fn boot() -> Option<Harness> {
    let Some(iii_bin) = iii_bin() else {
        eprintln!("skipping: no `iii` binary (set III_BIN or put iii on PATH)");
        return None;
    };

    let [engine_port, proxy_port] = free_ports();
    let engine_ws = format!("ws://127.0.0.1:{engine_port}");

    // The engine keeps its state (configuration entries) under its working
    // directory: a fresh one per run.
    let dir = std::env::temp_dir().join(format!("rbac-proxy-itest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(
        dir.join("config.yaml"),
        format!(
            "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {engine_port}\n"
        ),
    )
    .ok()?;

    // Seed config: RBAC on, exposing only api::* and the functions-discovery
    // surface. Written before any child starts, so failing here leaks none.
    // It lives in the run's directory: concurrent runs no longer share a path.
    let seed_path = dir.join("seed.yaml");
    std::fs::write(
        &seed_path,
        format!(
            "host: 127.0.0.1\nport: {proxy_port}\nengine_url: {engine_ws}\n\
             rbac:\n  auth_function_id: test::auth\n  expose_functions:\n    - match(\"api::*\")\n    - match(\"engine::functions::*\")\n"
        ),
    )
    .ok()?;

    let spawned = Command::new(&iii_bin)
        .args(["-c", "config.yaml"])
        .current_dir(&dir)
        .env_remove("III_URL")
        // Engine and proxy write `iii ps` markers under $HOME/.iii/run.
        .env("HOME", &dir)
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

    if !wait_for_listen(engine_port, Duration::from_secs(15)).await {
        eprintln!("skipping: engine did not come up on :{engine_port}");
        kill_tree(&mut iii);
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    }

    // Register auth + targets, give the engine a moment to index them.
    let support = register_support(&engine_ws);
    sleep(Duration::from_millis(600)).await;

    let proxy_bin = env!("CARGO_BIN_EXE_rbac-proxy");
    let spawned = Command::new(proxy_bin)
        .arg("--url")
        .arg(&engine_ws)
        .arg("--config")
        .arg(&seed_path)
        .env_remove("III_URL")
        .env("HOME", &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(proxy) = spawned else {
        kill_tree(&mut iii);
        let _ = std::fs::remove_dir_all(&dir);
        return None;
    };

    if !wait_for_listen(proxy_port, Duration::from_secs(10)).await {
        eprintln!(
            "skipping: rbac-proxy did not come up on :{proxy_port} \
             (is the `configuration` worker deployed in this engine?)"
        );
        let mut h = Harness {
            proxy,
            iii,
            _support: support,
            dir,
            proxy_port,
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
        proxy_port,
    })
}

fn downstream(proxy_port: u16) -> IIIClient {
    let mut headers = HashMap::new();
    headers.insert("authorization".to_string(), "Bearer test-token".to_string());
    register_worker(
        &format!("ws://127.0.0.1:{proxy_port}"),
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

/// Runs the end-to-end body in a child with a private `HOME`: its SDK clients
/// would otherwise write `iii ps` markers under the real `~/.iii/run`.
#[test]
fn end_to_end_rbac_through_proxy() {
    let home = std::env::temp_dir().join(format!("rbac-proxy-itest-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("create the private HOME");
    let output = Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "end_to_end_child", "--ignored", "--nocapture"])
        .env("HOME", &home)
        .env("RBAC_PROXY_E2E_CHILD", "1")
        .output();
    let _ = std::fs::remove_dir_all(&home);
    let output = output.expect("run the end-to-end child");
    let (stdout, stderr) = (
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    // A filter that matches nothing also exits 0.
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}\n{stderr}"
    );
    eprint!("{stderr}");
}

#[tokio::test]
#[ignore = "isolated subprocess invoked by end_to_end_rbac_through_proxy"]
async fn end_to_end_child() {
    // `--ignored` runs would otherwise use the real HOME; only the parent sets this.
    if std::env::var_os("RBAC_PROXY_E2E_CHILD").is_none() {
        eprintln!("skipping: runs only as the isolated child of end_to_end_rbac_through_proxy");
        return;
    }
    let Some(h) = boot().await else {
        eprintln!("skipping: prerequisites not met (see logs above)");
        return;
    };
    let proxy_port = h.proxy_port;

    let client = downstream(proxy_port);
    // Give the downstream connection time to upgrade + authenticate.
    sleep(Duration::from_millis(800)).await;

    // (a) Exposed call succeeds and echoes.
    let echoed = timeout(
        Duration::from_secs(8),
        client.call(call("api::echo", json!({ "hi": 1 }))),
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
    let denied = client.call(call("secret::echo", json!({ "x": 1 }))).await;
    match denied {
        Err(Error::Remote { code, .. }) => assert_eq!(code, "FORBIDDEN"),
        other => panic!("expected FORBIDDEN remote error, got {other:?}"),
    }

    // (c) An observer whose target is a forbidden function is refused with
    //     the engine's answer, `observeresult { state: error }`, and never
    //     reaches the engine.
    {
        let mut ws = raw_client(proxy_port).await;
        let reply = request(
            &mut ws,
            json!({
                "type": "observe",
                "id": "3a5c7e9b-1d3f-4a5c-8e9b-1d3f5a7c9e1b",
                "source": { "function_id": "api::echo" },
                "target": { "function_id": "secret::echo" }
            }),
            |v| v["type"] == "observeresult",
        )
        .await;
        assert_eq!(reply["state"], "error", "got {reply}");
        assert_eq!(
            reply["error"]["code"], "FORBIDDEN",
            "an observer targeting secret::echo must be refused; got {reply}"
        );
        let _ = ws.close(None).await;
    }

    // (d) Channel round-trips through the proxy's /ws/channels bridge.
    let ch = client
        .call(call("engine::channels::create", json!({})))
        .await
        .expect("engine::channels::create is in the carve-out and must be allowed");
    let cid = ch["writer"]["channel_id"].as_str().expect("channel_id");
    let wkey = ch["writer"]["access_key"].as_str().expect("writer key");
    let rkey = ch["reader"]["access_key"].as_str().expect("reader key");

    let read_url = format!("ws://127.0.0.1:{proxy_port}/ws/channels/{cid}?key={rkey}&dir=read");
    let write_url = format!("ws://127.0.0.1:{proxy_port}/ws/channels/{cid}?key={wkey}&dir=write");
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
        .call(call("engine::functions::list", json!({})))
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
        let mut ws = raw_client(proxy_port).await;
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

    // (g) An observer of an exposed function with an exposed target reaches
    //     the engine, which accepts it. Declaring the worker first settles
    //     the connection's namespace, so the engine answers at once.
    {
        let mut ws = raw_client(proxy_port).await;
        ws.send(Message::text(
            json!({
                "type": "invokefunction",
                "function_id": "engine::workers::register",
                "data": {
                    "runtime": "rust",
                    "name": "rbac-proxy-itest-raw",
                    "features": ["observe.v1", "observe.v2"]
                },
                "action": { "type": "void" }
            })
            .to_string(),
        ))
        .await
        .expect("send engine::workers::register");
        let reply = request(
            &mut ws,
            json!({
                "type": "observe",
                "id": "5b0c3e1a-8f2d-4c6b-9a7e-1d2f3a4b5c6d",
                "source": { "function_id": "api::echo" },
                "target": { "function_id": "api::sink" }
            }),
            |v| v["type"] == "observeresult",
        )
        .await;
        assert_ne!(
            reply["state"], "error",
            "an observer of api::echo targeting api::sink must be accepted; got {reply}"
        );
        let _ = ws.close(None).await;
    }

    // (h) An engine event that lists functions cannot be observed: its calls
    //     carry the unfiltered function list.
    {
        let mut ws = raw_client(proxy_port).await;
        let reply = request(
            &mut ws,
            json!({
                "type": "observe",
                "id": "7d9f1b3c-5e7a-4c9e-8b1d-3f5a7c9e1b3d",
                "source": { "function_id": "engine::on-functions-changed" },
                "target": { "function_id": "api::echo" }
            }),
            |v| v["type"] == "observeresult",
        )
        .await;
        assert_eq!(reply["state"], "error", "got {reply}");
        assert_eq!(reply["error"]["code"], "FORBIDDEN", "got {reply}");
        let _ = ws.close(None).await;
    }

    // (i) A `Reattach` gives a connection its previous connection's
    //     observers, and the proxy's connections carry no session the engine
    //     could check them against. A narrower session's refusal of the
    //     replayed observe disposes the observer: no activation reaches the
    //     new connection.
    {
        const OBSERVER: &str = "9c1e5a7b-3d5f-4b7d-9e1f-5a7c9e1b3d5f";
        let register_worker = json!({
            "type": "invokefunction",
            "function_id": "engine::workers::register",
            "data": {
                "runtime": "rust",
                "name": "rbac-proxy-itest-reattach",
                "features": ["observe.v1", "observe.v2"]
            },
            "action": { "type": "void" }
        });
        let register_target = json!({ "type": "registerfunction", "id": "itest::observed-echo" });
        let observe = json!({
            "type": "observe",
            "id": OBSERVER,
            "source": { "function_id": "api::echo" },
            "target": { "function_id": "itest::observed-echo" }
        });

        let mut first = raw_client(proxy_port).await;
        let registered = request(&mut first, register_worker.clone(), |v| {
            v["type"] == "workerregistered"
        })
        .await;
        request(&mut first, register_target.clone(), |v| {
            v["type"] == "functionregistered"
        })
        .await;
        let answer = request(&mut first, observe.clone(), |v| {
            v["type"] == "observeresult"
        })
        .await;
        assert_ne!(answer["state"], "error", "got {answer}");
        let _ = first.close(None).await;
        sleep(Duration::from_millis(300)).await;

        let mut narrow = raw_client_with(proxy_port, "Bearer narrow").await;
        narrow
            .send(Message::text(
                json!({
                    "type": "reattach",
                    "previous_worker_id": registered["worker_id"],
                    "reattach_token": registered["reattach_token"]
                })
                .to_string(),
            ))
            .await
            .expect("send reattach");
        request(&mut narrow, register_worker, |v| {
            v["type"] == "workerregistered"
        })
        .await;
        request(&mut narrow, register_target, |v| {
            v["type"] == "functionregistered"
        })
        .await;
        let replay = request(&mut narrow, observe, |v| v["type"] == "observeresult").await;
        assert_eq!(replay["error"]["code"], "FORBIDDEN", "got {replay}");

        let _ = client
            .call(call("api::echo", json!({ "after": "reattach" })))
            .await;
        let leaked = timeout(Duration::from_secs(2), async {
            while let Some(Ok(msg)) = narrow.next().await {
                if let Message::Text(t) = msg {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap_or_default();
                    if v["type"] == "invokefunction" {
                        return v;
                    }
                }
            }
            Value::Null
        })
        .await;
        assert!(
            !matches!(&leaked, Ok(v) if !v.is_null()),
            "the reattached observer reached the narrower session: {leaked:?}"
        );
        let _ = narrow.close(None).await;
    }

    // (j) Two sessions without a prefix declaring one name are two workers:
    //     the second is not refused as the name's second live worker.
    {
        let register = json!({
            "type": "invokefunction",
            "invocation_id": "22222222-2222-4222-8222-222222222222",
            "function_id": "engine::workers::register",
            "data": { "runtime": "rust", "name": "rbac-proxy-itest-same-name" }
        });
        let answered = |v: &Value| v["type"] == "invocationresult";
        let mut first = raw_client(proxy_port).await;
        let mut second = raw_client(proxy_port).await;
        let a = request(&mut first, register.clone(), answered).await;
        let b = request(&mut second, register, answered).await;
        assert_eq!(a["result"]["success"], true, "got {a}");
        assert_eq!(b["result"]["success"], true, "got {b}");
        let _ = first.close(None).await;
        let _ = second.close(None).await;
    }
}

type RawClient =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn raw_client(proxy_port: u16) -> RawClient {
    raw_client_with(proxy_port, "Bearer test-token").await
}

async fn raw_client_with(proxy_port: u16, authorization: &str) -> RawClient {
    let url = format!("ws://127.0.0.1:{proxy_port}/");
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
            url.as_str(),
        )
        .expect("request");
    request
        .headers_mut()
        .insert("authorization", authorization.parse().expect("header"));
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("raw connect to proxy");
    ws
}

/// Send `frame` and return the first text frame `pick` accepts.
async fn request(ws: &mut RawClient, frame: Value, pick: impl Fn(&Value) -> bool) -> Value {
    ws.send(Message::text(frame.to_string()))
        .await
        .expect("send frame");
    timeout(Duration::from_secs(5), async {
        while let Some(Ok(msg)) = ws.next().await {
            if let Message::Text(t) = msg
                && let Ok(v) = serde_json::from_str::<Value>(t.as_str())
                && pick(&v)
            {
                return v;
            }
        }
        panic!("connection closed before the reply");
    })
    .await
    .expect("no reply in time")
}
