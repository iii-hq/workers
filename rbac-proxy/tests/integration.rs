//! End-to-end: spawn the `iii` engine and the `rbac-proxy` worker, register an
//! auth function + target functions on the engine, then drive a downstream
//! worker **through the proxy port** and assert the RBAC contract.
//!
//! The engine is the `iii` on `PATH`, or `III_BIN`. It runs from its own
//! temporary directory on a free port picked at boot (never the default
//! 49134), so the test never reaches an engine a developer has running.
//!
//! Skips only when the optional engine is absent. A supplied engine and
//! proxy must start successfully; startup failures include their logs.
//!
//! Covers spec *Testing* (a)–(e): an exposed call succeeds, a forbidden call
//! rejects with `FORBIDDEN`, an observer whose target is a forbidden function
//! is refused with `FORBIDDEN`, a channel round-trips through
//! `/ws/channels/{id}`, and `engine::functions::list` returns only exposed
//! ids. Also: an exposed id called in another namespace is `FORBIDDEN`, an
//! observer of an exposed function with an exposed target is accepted, and an
//! engine event that lists functions cannot be observed.
//!
//! A session pinned to a namespace (`Bearer pinned`) is probed apart: it
//! registers in its pin when silent past the grace window, is closed when it
//! declares another namespace, cannot host the middleware (and is closed by a
//! reload that renames the middleware to an id it holds), keeps the metadata
//! its worker declares, and serves an SDK worker's functions and observers
//! there.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{CallRequest, IIIClient, InitOptions, RegisterFunction, Source, register_worker};
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
    support: IIIClient,
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

/// Every `endpoint` the auth function was asked about.
static AUTH_ENDPOINTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Whether an auth input said the proxy pins (`registration_namespace: true`).
static AUTH_SAID_PIN: AtomicBool = AtomicBool::new(false);

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
    // may observe it. The `pinned` token's session registers in `laptop` and
    // holds a grant in `orders`, a namespace it must still not declare.
    support.register_function(
        "test::auth",
        RegisterFunction::new_async(|input: Value| async move {
            if let Some(endpoint) = input["endpoint"].as_str() {
                AUTH_ENDPOINTS.lock().unwrap().push(endpoint.to_string());
            }
            if input["registration_namespace"] == true {
                AUTH_SAID_PIN.store(true, Ordering::Relaxed);
            }
            let mut session = json!({ "context": { "tenant": "itest" } });
            if input["headers"]["authorization"] == "Bearer reject" {
                return Err(Error::Remote {
                    code: "UNAUTHORIZED".to_string(),
                    message: "rejected by test::auth".to_string(),
                    stacktrace: None,
                });
            }
            if input["headers"]["authorization"] == "Bearer narrow" {
                session["forbidden_functions"] = json!(["api::echo"]);
            }
            if input["headers"]["authorization"] == "Bearer pinned" {
                session["registration_namespace"] = json!("laptop");
                session["namespaces"] = json!({ "orders": ["api::echo"] });
            }
            // A pin the proxy cannot isolate: its session must be refused.
            if input["headers"]["authorization"] == "Bearer pinned-default" {
                session["registration_namespace"] = json!("default");
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
    // The middleware of the pin probes: a pinned session must not host its id.
    support.register_function(
        "test::middleware",
        RegisterFunction::new_async(|_input: Value| async move { Ok::<Value, Error>(Value::Null) }),
    );
    // The registration hook of the middleware probe: it maps `probe::evil`
    // onto the middleware id and keeps every other registration as it is.
    support.register_function(
        "test::on-register",
        RegisterFunction::new_async(|input: Value| async move {
            Ok::<Value, Error>(if input["function_id"] == "probe::evil" {
                json!({ "function_id": "test::middleware" })
            } else {
                json!({})
            })
        }),
    );
    support
}

/// The `rbac:` block of the main end-to-end run: an auth function.
const AUTH_FUNCTION_RBAC: &str = "rbac:\n  auth_function_id: test::auth\n  \
     expose_endpoints:\n    - match(\"/\")\n    - match(\"/ws/channels/*\")\n    - match(\"/otel\")\n  \
     expose_functions:\n    - match(\"api::*\")\n    - match(\"engine::functions::*\")\n";

/// The `rbac:` block of the loopback run: no auth function, the built-in
/// loopback grant only. `secret::*` is exposed, so only the grant's
/// `forbidden_functions` keeps `secret::echo` out.
const LOOPBACK_RBAC: &str = "rbac:\n  loopback_auth:\n    \
     worker_names: [rbac-proxy-itest-loopback]\n    \
     forbidden_functions: [secret::echo]\n  \
     expose_endpoints:\n    - match(\"/\")\n    - match(\"/otel\")\n  \
     expose_functions:\n    - match(\"api::*\")\n    - match(\"secret::*\")\n";

async fn boot() -> Option<Harness> {
    boot_env(AUTH_FUNCTION_RBAC, "", &[]).await
}

/// Boot the engine with `engine_env` and the proxy, the proxy seeded with the
/// `rbac:` block `rbac` followed by `seed`.
async fn boot_env(rbac: &str, seed: &str, engine_env: &[(&str, &str)]) -> Option<Harness> {
    let Some(iii_bin) = iii_bin() else {
        eprintln!("skipping: no `iii` binary (set III_BIN or put iii on PATH)");
        return None;
    };

    let [engine_port, proxy_port] = free_ports();
    let engine_ws = format!("ws://127.0.0.1:{engine_port}");

    // The engine keeps its state (configuration entries) under its working
    // directory: a fresh one per run.
    let dir = std::env::temp_dir().join(format!("rbac-proxy-itest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create integration directory");
    std::fs::write(
        dir.join("config.yaml"),
        format!(
            "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {engine_port}\n"
        ),
    )
    .expect("write integration configuration");

    // Seed config: the run's `rbac:` block. Written before any child starts, so failing here leaks none.
    // It lives in the run's directory: concurrent runs no longer share a path.
    let seed_path = dir.join("seed.yaml");
    std::fs::write(
        &seed_path,
        format!("host: 127.0.0.1\nport: {proxy_port}\nengine_url: {engine_ws}\n{rbac}{seed}"),
    )
    .expect("write integration configuration");

    let engine_log = std::fs::File::create(dir.join("engine.log")).expect("create engine log");
    let spawned = Command::new(&iii_bin)
        .args(["--no-update-check", "-c", "config.yaml"])
        .current_dir(&dir)
        .env_remove("III_URL")
        // Engine and proxy write `iii ps` markers under $HOME/.iii/run.
        .env("HOME", &dir)
        .envs(engine_env.iter().copied())
        .stdout(engine_log.try_clone().expect("clone engine log"))
        .stderr(engine_log)
        .spawn();
    let mut iii = match spawned {
        Ok(child) => child,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            panic!("could not start supplied engine {}: {e}", iii_bin.display());
        }
    };

    if !wait_for_listen(engine_port, Duration::from_secs(15)).await {
        let log = std::fs::read_to_string(dir.join("engine.log")).unwrap_or_default();
        kill_tree(&mut iii);
        let _ = std::fs::remove_dir_all(&dir);
        panic!("supplied engine did not come up on :{engine_port}: {log}");
    }

    // Register auth + targets, give the engine a moment to index them.
    let support = register_support(&engine_ws);
    sleep(Duration::from_millis(600)).await;

    let proxy_bin = env!("CARGO_BIN_EXE_rbac-proxy");
    let proxy_log = std::fs::File::create(dir.join("proxy.log")).expect("create proxy log");
    let spawned = Command::new(proxy_bin)
        .arg("--url")
        .arg(&engine_ws)
        .arg("--config")
        .arg(&seed_path)
        .env_remove("III_URL")
        // The reload probe sets the entry under the built-in id.
        .env_remove("III_CONFIG_NAME")
        .env("HOME", &dir)
        .stdout(proxy_log.try_clone().expect("clone proxy log"))
        .stderr(proxy_log)
        .spawn();
    let proxy = match spawned {
        Ok(child) => child,
        Err(error) => {
            kill_tree(&mut iii);
            let _ = std::fs::remove_dir_all(&dir);
            panic!("start proxy: {error}");
        }
    };

    if !wait_for_listen(proxy_port, Duration::from_secs(10)).await {
        let log = std::fs::read_to_string(dir.join("proxy.log")).unwrap_or_default();
        let mut h = Harness {
            proxy,
            iii,
            support,
            dir,
            proxy_port,
        };
        // Reap both processes before reporting the failed prerequisite.
        let _ = h.proxy.kill();
        drop(h);
        panic!("rbac-proxy did not come up on :{proxy_port}: {log}");
    }

    Some(Harness {
        proxy,
        iii,
        support,
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
    run_isolated_child("end_to_end_child");
}

/// Run the ignored test `child` in a child process with a private `HOME`.
fn run_isolated_child(child: &str) {
    let home = std::env::temp_dir().join(format!(
        "rbac-proxy-itest-home-{child}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&home).expect("create the private HOME");
    let output = Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", child, "--ignored", "--nocapture"])
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

    // (k) /otel is authenticated before the upgrade by the same auth function
    // as `/`, which is told the endpoint, then relayed to the engine.
    {
        let url = format!("ws://127.0.0.1:{proxy_port}/otel");
        let mut request =
            tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                url.as_str(),
            )
            .expect("request");
        request
            .headers_mut()
            .insert("authorization", "Bearer reject".parse().unwrap());
        match tokio_tungstenite::connect_async(request).await {
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
                assert_eq!(resp.status(), 401, "a refused /otel is an HTTP status");
            }
            other => panic!("expected a 401 before the upgrade, got {other:?}"),
        }

        let mut otel = raw_client_at(proxy_port, "/otel", "Bearer test-token").await;
        assert!(
            AUTH_ENDPOINTS.lock().unwrap().iter().any(|e| e == "/otel"),
            "the auth function was told the endpoint"
        );
        // A log frame as the SDKs send it: `LOGS` + OTLP JSON.
        let mut frame = b"LOGS".to_vec();
        frame.extend_from_slice(
            json!({ "resourceLogs": [{
                "resource": { "attributes": [{ "key": "service.name",
                    "value": { "stringValue": "rbac-proxy-itest" } }] },
                "scopeLogs": [{ "logRecords": [{ "severityText": "INFO",
                    "body": { "stringValue": "relayed through rbac-proxy" } }] }]
            }]})
            .to_string()
            .as_bytes(),
        );
        otel.send(Message::Binary(frame.into()))
            .await
            .expect("an accepted /otel connection takes binary frames");
        // The relay stays open: anything but silence (a close, EOF, an error,
        // a frame) means it did not relay.
        let quiet = timeout(Duration::from_millis(500), otel.next()).await;
        assert!(
            quiet.is_err(),
            "the /otel relay did not stay open: {quiet:?}"
        );
        let _ = otel.close(None).await;
    }

    // (l) A path that is not an engine endpoint is a bare 404 before the
    // upgrade, and so is one no rule exposes.
    for path in ["/nope", "/ws/channels/a/b"] {
        let url = format!("ws://127.0.0.1:{proxy_port}{path}");
        match tokio_tungstenite::connect_async(url.as_str()).await {
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
                assert_eq!(resp.status(), 404, "{path}");
            }
            other => panic!("expected a 404 for {path}, got {other:?}"),
        }
    }

    // (m) An auth result pinned to `default` (where the auth function and the
    // hooks resolve) is refused at auth, before any engine connection.
    {
        let mut ws = raw_client_with(proxy_port, "Bearer pinned-default").await;
        let first = timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("no refusal in time");
        let Some(Ok(Message::Text(t))) = first else {
            panic!("expected an error frame, got {first:?}");
        };
        let frame: Value = serde_json::from_str(t.as_str()).expect("json frame");
        assert_eq!(frame["type"], "error", "{frame}");
        assert_eq!(frame["error"]["code"], "AUTH_ERROR", "{frame}");
        assert!(
            frame["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("registration_namespace 'default'")),
            "{frame}"
        );
    }

    assert!(
        AUTH_ENDPOINTS.lock().unwrap().iter().any(|e| e == "/"),
        "the auth function was told the endpoint for `/` too"
    );
}

/// A worker on 127.0.0.1 with no credentials, through a proxy with no auth
/// function and `rbac.loopback_auth` set, is admitted with the configured
/// grants.
#[test]
fn loopback_auth_through_proxy() {
    run_isolated_child("loopback_auth_child");
}

#[tokio::test]
#[ignore = "isolated subprocess invoked by loopback_auth_through_proxy"]
async fn loopback_auth_child() {
    if std::env::var_os("RBAC_PROXY_E2E_CHILD").is_none() {
        eprintln!("skipping: runs only as the isolated child of loopback_auth_through_proxy");
        return;
    }
    let Some(h) = boot_env(LOOPBACK_RBAC, "", &[]).await else {
        eprintln!("skipping: prerequisites not met (see logs above)");
        return;
    };
    let proxy_port = h.proxy_port;

    // No authorization header at all: the peer address admits it.
    let client = register_worker(
        &format!("ws://127.0.0.1:{proxy_port}"),
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                name: "rbac-proxy-itest-loopback".to_string(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    sleep(Duration::from_millis(800)).await;

    // An exposed function answers.
    let echoed = timeout(
        Duration::from_secs(8),
        client.call(call("api::echo", json!({ "via": "loopback" }))),
    )
    .await
    .expect("api::echo did not return in time")
    .expect("a loopback worker may call an exposed function");
    assert_eq!(echoed["via"], "loopback");

    // The grant's forbidden_functions applies, over expose_functions.
    match client.call(call("secret::echo", json!({}))).await {
        Err(Error::Remote { code, .. }) => assert_eq!(code, "FORBIDDEN"),
        other => panic!("expected FORBIDDEN remote error, got {other:?}"),
    }

    // The granted worker name reaches the engine as declared.
    let workers = h
        .support
        .call(call("engine::workers::list", json!({})))
        .await
        .expect("engine::workers::list on the engine");
    let names: Vec<&str> = workers["workers"]
        .as_array()
        .expect("workers")
        .iter()
        .filter_map(|w| w["name"].as_str())
        .collect();
    assert!(
        names.contains(&"rbac-proxy-itest-loopback"),
        "granted name not kept: {names:?}"
    );

    // The session probe says a loopback grant is in force.
    {
        let mut ws = raw_client_at(proxy_port, "/", "Bearer ignored").await;
        let probe = request(
            &mut ws,
            json!({
                "type": "invokefunction",
                "invocation_id": "44444444-4444-4444-8444-444444444444",
                "function_id": "rbac-proxy::session",
                "data": {}
            }),
            |v| v["type"] == "invocationresult",
        )
        .await;
        let policy = &probe["result"]["policy"];
        assert_eq!(policy["auth"], "loopback", "got {probe}");
        assert_eq!(policy["auth_function"], false, "got {probe}");
        assert_eq!(
            probe["result"]["worker_names"],
            json!(["rbac-proxy-itest-loopback"])
        );
        let _ = ws.close(None).await;
    }

    // rbac-proxy::status names the auth in force.
    let status = h
        .support
        .call(call("rbac-proxy::status", json!({})))
        .await
        .expect("rbac-proxy::status");
    assert_eq!(status["auth"], "loopback", "got {status}");
    assert_eq!(status["rbac_enabled"], false, "got {status}");

    // Signs of a non-local client: a browser's Origin, a proxy's forwarding
    // header. Refused as a non-loopback peer; the same upgrade without them
    // is admitted.
    for header in [
        ("origin", "http://evil.example"),
        ("x-forwarded-for", "1.2.3.4"),
    ] {
        let mut ws = raw_upgrade(proxy_port, "/", &[header])
            .await
            .expect("/ upgrades before authenticating");
        let first = timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("no refusal in time");
        let Some(Ok(Message::Text(t))) = first else {
            panic!("{header:?}: expected an error frame, got {first:?}");
        };
        let frame: Value = serde_json::from_str(t.as_str()).expect("json frame");
        assert_eq!(frame["type"], "error", "{header:?}: {frame}");
        assert_eq!(frame["error"]["code"], "AUTH_ERROR", "{header:?}: {frame}");
        assert_eq!(
            frame["error"]["message"], "rbac.auth_function_id is not set",
            "{header:?}: {frame}"
        );
        assert!(
            raw_upgrade(proxy_port, "/otel", &[header]).await.is_err(),
            "{header:?}: /otel admitted"
        );
    }
    let mut ws = raw_upgrade(proxy_port, "/", &[("user-agent", "iii-sdk")])
        .await
        .expect("plain upgrade");
    let probe = request(
        &mut ws,
        json!({
            "type": "invokefunction",
            "invocation_id": "55555555-5555-4555-8555-555555555555",
            "function_id": "rbac-proxy::session",
            "data": {}
        }),
        |v| v["type"] == "invocationresult",
    )
    .await;
    assert_eq!(probe["result"]["policy"]["auth"], "loopback", "got {probe}");
    let _ = ws.close(None).await;
    let otel = raw_upgrade(proxy_port, "/otel", &[])
        .await
        .expect("a plain /otel upgrade is admitted");
    drop(otel);
}

/// A WebSocket upgrade of `path` carrying exactly `headers`.
async fn raw_upgrade(
    proxy_port: u16,
    path: &str,
    headers: &[(&'static str, &str)],
) -> Result<RawClient, tokio_tungstenite::tungstenite::Error> {
    let url = format!("ws://127.0.0.1:{proxy_port}{path}");
    let mut request =
        tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
            url.as_str(),
        )
        .expect("request");
    for (name, value) in headers {
        request
            .headers_mut()
            .insert(*name, value.parse().expect("header"));
    }
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(ws, _)| ws)
}

type RawClient =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn raw_client(proxy_port: u16) -> RawClient {
    raw_client_with(proxy_port, "Bearer test-token").await
}

async fn raw_client_with(proxy_port: u16, authorization: &str) -> RawClient {
    raw_client_at(proxy_port, "/", authorization).await
}

async fn raw_client_at(proxy_port: u16, path: &str, authorization: &str) -> RawClient {
    let url = format!("ws://127.0.0.1:{proxy_port}{path}");
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

#[cfg(unix)]
#[test]
fn supplied_failed_engine_cannot_skip() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "end_to_end_rbac_through_proxy", "--nocapture"])
        .env("III_BIN", "/bin/false")
        .output()
        .expect("run supplied-engine failure in isolated process");
    assert!(!output.status.success(), "failed engine passed integration");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("supplied engine did not come up"),
        "{stderr}"
    );
    assert!(!stderr.contains("skipping:"), "{stderr}");
}

/// The `engine::functions::list` rows of `id`, engine internals included,
/// read on the engine itself.
async fn function_rows(support: &IIIClient, id: &str) -> Vec<Value> {
    let list = support
        .call(call(
            "engine::functions::list",
            json!({ "include_internal": true }),
        ))
        .await
        .expect("engine::functions::list on the engine");
    list["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .filter(|f| f["function_id"] == id)
        .cloned()
        .collect()
}

/// The namespace of every row of `id`.
async fn rows(support: &IIIClient, id: &str) -> Vec<String> {
    function_rows(support, id)
        .await
        .iter()
        .map(|f| f["namespace"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// Boot a pin probe: `test::middleware` as the middleware and a one-second
/// grace window. One second, not less: the proxy's declaration must reach the
/// engine inside it, and a slow CI machine can take a few hundred
/// milliseconds from the accept to the first frame. `rbac_extra` holds more
/// keys of the `rbac:` block, each line indented by two spaces.
async fn boot_pinned(rbac_extra: &str) -> Option<Harness> {
    if std::env::var_os("RBAC_PROXY_E2E_CHILD").is_none() {
        eprintln!("skipping: runs only as an isolated child of a pin probe");
        return None;
    }
    let h = boot_env(
        AUTH_FUNCTION_RBAC,
        &format!("{rbac_extra}middleware_function_id: test::middleware\n"),
        &[("III_NAMESPACE_GRACE_MS", "1000")],
    )
    .await;
    if h.is_none() {
        eprintln!("skipping: prerequisites not met (see logs above)");
    }
    h
}

#[test]
fn a_silent_pinned_session_registers_in_its_pin_after_the_grace_window() {
    run_isolated_child("silent_pin_child");
}

#[tokio::test]
#[ignore = "isolated subprocess invoked by a_silent_pinned_session_registers_in_its_pin_after_the_grace_window"]
async fn silent_pin_child() {
    let Some(h) = boot_pinned("").await else {
        return;
    };
    let mut ws = raw_client_with(h.proxy_port, "Bearer pinned").await;
    ws.send(Message::text(
        json!({ "type": "registerfunction", "id": "probe::silent" }).to_string(),
    ))
    .await
    .unwrap();
    sleep(Duration::from_millis(2500)).await; // well past the grace window
    assert_eq!(rows(&h.support, "probe::silent").await, ["laptop"]);
    assert!(
        AUTH_SAID_PIN.load(Ordering::Relaxed),
        "the auth input did not say the proxy pins"
    );
}

#[test]
fn a_pinned_session_that_declares_another_namespace_is_closed_and_leaves_nothing() {
    run_isolated_child("redeclare_pin_child");
}

#[tokio::test]
#[ignore = "isolated subprocess invoked by a_pinned_session_that_declares_another_namespace_is_closed_and_leaves_nothing"]
async fn redeclare_pin_child() {
    let Some(h) = boot_pinned("").await else {
        return;
    };
    let mut ws = raw_client_with(h.proxy_port, "Bearer pinned").await;
    ws.send(Message::text(
        json!({ "type": "registerfunction", "id": "probe::redeclare" }).to_string(),
    ))
    .await
    .unwrap();
    // The row must exist first, or the last assertion holds for a function
    // that never registered. Read on the catalog: the engine acks a
    // registration (`functionregistered`) only to a worker that declared
    // `observe.v1`, and this one declares nothing.
    let mut held = rows(&h.support, "probe::redeclare").await;
    for _ in 0..50 {
        if !held.is_empty() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
        held = rows(&h.support, "probe::redeclare").await;
    }
    assert_eq!(held, ["laptop"]);
    let rejected = request(
        &mut ws,
        json!({ "type": "invokefunction", "function_id": "engine::workers::register",
            "invocation_id": "55555555-5555-5555-5555-555555555555",
            "data": { "namespace": "orders", "name": "old-bridge" } }),
        |v| v["type"] == "registrationrejected",
    )
    .await;
    // `orders` is a grant key: accepted before the pin.
    assert_eq!(rejected["code"], "NAMESPACE_FORBIDDEN");
    let closed = timeout(Duration::from_secs(5), async {
        while let Some(Ok(m)) = ws.next().await {
            if m.is_close() {
                return true;
            }
        }
        false // EOF or a read error is not the Close the proxy owes
    })
    .await;
    assert!(matches!(closed, Ok(true)), "no Close frame: {closed:?}");
    // The engine drops the registration when the proxy drops the connection.
    let mut left = rows(&h.support, "probe::redeclare").await;
    for _ in 0..50 {
        if left.is_empty() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
        left = rows(&h.support, "probe::redeclare").await;
    }
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn a_pinned_session_cannot_host_the_middleware() {
    run_isolated_child("pinned_middleware_child");
}

#[tokio::test]
#[ignore = "isolated subprocess invoked by a_pinned_session_cannot_host_the_middleware"]
async fn pinned_middleware_child() {
    // The refusal reads the id the registration hook returns, and the hook's
    // own id is refused too.
    let rbac_extra = "  on_function_registration_function_id: test::on-register\n";
    let Some(h) = boot_pinned(rbac_extra).await else {
        return;
    };
    let mut ws = raw_client_with(h.proxy_port, "Bearer pinned").await;
    for id in [
        "test::middleware",
        "probe::evil",
        "test::on-register",
        "probe::ok",
    ] {
        ws.send(Message::text(
            json!({ "type": "registerfunction", "id": id }).to_string(),
        ))
        .await
        .unwrap();
    }
    sleep(Duration::from_millis(2500)).await;
    // The hook answered: it keeps this one.
    assert_eq!(rows(&h.support, "probe::ok").await, ["laptop"]);
    // The support worker's own, nothing in `laptop`.
    for id in ["test::middleware", "test::on-register"] {
        assert_eq!(rows(&h.support, id).await, ["default"], "{id}");
    }
    assert!(rows(&h.support, "probe::evil").await.is_empty());
}

#[test]
fn a_reload_that_renames_the_middleware_closes_a_pinned_session() {
    run_isolated_child("pinned_reload_child");
}

#[tokio::test]
#[ignore = "isolated subprocess invoked by a_reload_that_renames_the_middleware_closes_a_pinned_session"]
async fn pinned_reload_child() {
    let Some(h) = boot_pinned("").await else {
        return;
    };
    // Admitted while the middleware is `test::middleware`: this id is free.
    let mut ws = raw_client_with(h.proxy_port, "Bearer pinned").await;
    ws.send(Message::text(
        json!({ "type": "registerfunction", "id": "probe::next-mw" }).to_string(),
    ))
    .await
    .unwrap();
    let mut held = rows(&h.support, "probe::next-mw").await;
    for _ in 0..50 {
        if !held.is_empty() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
        held = rows(&h.support, "probe::next-mw").await;
    }
    assert_eq!(held, ["laptop"]);

    // The operator renames the middleware to the id the session holds.
    let mut value = h
        .support
        .call(call("configuration::get", json!({ "id": "rbac-proxy" })))
        .await
        .expect("configuration::get")["value"]
        .clone();
    value["middleware_function_id"] = json!("probe::next-mw");
    h.support
        .call(call(
            "configuration::set",
            json!({ "id": "rbac-proxy", "value": value }),
        ))
        .await
        .expect("configuration::set");

    // The session is closed, and the engine drops what it held with it.
    let closed = timeout(Duration::from_secs(5), async {
        while let Some(Ok(m)) = ws.next().await {
            if m.is_close() {
                return true;
            }
        }
        false
    })
    .await;
    assert!(
        matches!(closed, Ok(true)),
        "the pinned session stayed open: {closed:?}"
    );
    let mut left = rows(&h.support, "probe::next-mw").await;
    for _ in 0..50 {
        if left.is_empty() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
        left = rows(&h.support, "probe::next-mw").await;
    }
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn a_pinned_worker_keeps_the_metadata_it_declares() {
    run_isolated_child("pinned_metadata_child");
}

#[tokio::test]
#[ignore = "isolated subprocess invoked by a_pinned_worker_keeps_the_metadata_it_declares"]
async fn pinned_metadata_child() {
    let Some(h) = boot_pinned("").await else {
        return;
    };
    // The proxy's own declaration has no runtime and no version, and the
    // engine writes both on every declaration: it must run before the
    // worker's. Twenty workers, because a race goes the right way by chance.
    // Each sends a `pid`: `engine::workers::list` hides a worker without one.
    let mut clients = Vec::new();
    for n in 0..20 {
        let mut ws = raw_client_with(h.proxy_port, "Bearer pinned").await;
        ws.send(Message::text(
            json!({ "type": "invokefunction", "function_id": "engine::workers::register",
                "action": { "type": "void" },
                "data": { "runtime": "rust", "version": "0.26.0", "name": format!("meta-{n}"), "pid": std::process::id() } })
            .to_string(),
        ))
        .await
        .unwrap();
        clients.push(ws);
    }
    let mut workers = Vec::new();
    for _ in 0..50 {
        let list = h
            .support
            .call(call("engine::workers::list", json!({})))
            .await
            .expect("engine::workers::list on the engine");
        workers = list["workers"]
            .as_array()
            .expect("workers")
            .iter()
            .filter(|w| {
                w["name"]
                    .as_str()
                    .is_some_and(|n| n.starts_with("rbac-proxy:laptop::meta-"))
            })
            .cloned()
            .collect();
        if workers.len() == clients.len() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(workers.len(), clients.len(), "{workers:?}");
    for w in &workers {
        assert_eq!(
            (w["runtime"].as_str(), w["version"].as_str()),
            (Some("rust"), Some("0.26.0")),
            "{w}"
        );
    }
}

#[test]
fn an_sdk_worker_under_a_pin_serves_its_functions_and_observers() {
    run_isolated_child("pinned_sdk_child");
}

#[tokio::test]
#[ignore = "isolated subprocess invoked by an_sdk_worker_under_a_pin_serves_its_functions_and_observers"]
async fn pinned_sdk_child() {
    let Some(h) = boot_pinned("").await else {
        return;
    };
    let mut headers = HashMap::new();
    headers.insert("authorization".to_string(), "Bearer pinned".to_string());
    let sdk = register_worker(
        &format!("ws://127.0.0.1:{}", h.proxy_port),
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                name: "pinned-sdk".to_string(),
                ..WorkerMetadata::default()
            }),
            headers: Some(headers),
            namespace: Some("laptop".to_string()),
            ..InitOptions::default()
        },
    );
    sdk.register_function(
        "probe::hello",
        RegisterFunction::new_async(|_input: Value| async move {
            Ok::<Value, Error>(json!({ "hello": "laptop" }))
        }),
    );

    let mut answer = Err(Error::NotConnected);
    for _ in 0..50 {
        answer = h
            .support
            .call(call("probe::hello", json!({})).namespace("laptop"))
            .await;
        if answer.is_ok() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(answer.expect("probe::hello in laptop")["hello"], "laptop");

    let observer = h.support.observe(
        Source::function("probe::hello").namespace("laptop"),
        "api::sink",
    );
    timeout(Duration::from_secs(5), observer.ready())
        .await
        .expect("the observer did not become active in time")
        .expect("an observer of probe::hello in laptop");

    // The SDK declares itself after its registrations: until then the engine
    // lists the first segment of the id as the worker name.
    let mut row = Value::Null;
    for _ in 0..50 {
        row = function_rows(&h.support, "probe::hello")
            .await
            .into_iter()
            .find(|f| f["namespace"] == "laptop")
            .unwrap_or_default();
        if row["worker_name"] == "rbac-proxy:laptop::pinned-sdk" {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(row["worker_name"], "rbac-proxy:laptop::pinned-sdk", "{row}");

    let workers = h
        .support
        .call(call("engine::workers::list", json!({})))
        .await
        .expect("engine::workers::list on the engine");
    assert!(
        workers["workers"]
            .as_array()
            .expect("workers")
            .iter()
            .any(|w| w["name"] == "rbac-proxy:laptop::pinned-sdk"),
        "{workers}"
    );
}
