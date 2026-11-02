//! End-to-end test: an `iii` engine, the `ade` worker, and a test client that
//! drives the worker through its HTTP port, the `/ws` proxy, its
//! configuration entry and its observable functions.
//!
//! The engine, in this order:
//!   - `III_ENGINE_BIN` (or `III_BIN`): that binary, started here on an
//!     ephemeral port with an engine-only config in a temp dir;
//!   - `III_ENGINE_WS_URL`: an engine already running there (one no other
//!     `ade` worker is connected to, since the test's worker hosts the
//!     `console::*` functions);
//!   - `iii` on `PATH`, started here, when it is 0.26 or later.
//!
//! The worker listens on ephemeral ports and keeps its data in the temp dir.
//! Without an engine the test prints why and skips; with `III_E2E_REQUIRE`
//! set it fails instead. A failed run keeps the temp dir (engine and worker
//! logs) and prints where it is.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use iii_sdk::runtime::{IIIConnectionState, WorkerIdentityMode, WorkerMetadata};
use iii_sdk::{register_worker, IIIClient, InitOptions, ObserverHandle, ObserverState};
use iii_sdk::{CallRequest, Deliver, Error, RegisterFunction, Source};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

/// The oldest engine (major, minor) the test runs against: the first with
/// observable functions.
const MIN_ENGINE: (u64, u64) = (0, 26);

enum EngineSource {
    Spawn(PathBuf),
    Running(String),
}

/// Which engine the test uses, or why there is none.
fn engine_source() -> Result<EngineSource, String> {
    for var in ["III_ENGINE_BIN", "III_BIN"] {
        if let Some(bin) = std::env::var_os(var).filter(|value| !value.is_empty()) {
            return Ok(EngineSource::Spawn(bin.into()));
        }
    }
    if let Some(url) = std::env::var("III_ENGINE_WS_URL")
        .ok()
        .filter(|value| !value.is_empty())
    {
        return Ok(EngineSource::Running(url));
    }
    let bin = which::which("iii").map_err(|_| {
        "no engine: set III_ENGINE_BIN, III_BIN or III_ENGINE_WS_URL, or put iii 0.26 or later on PATH"
            .to_string()
    })?;
    match engine_version(&bin) {
        Some(version) if version >= MIN_ENGINE => Ok(EngineSource::Spawn(bin)),
        Some((major, minor)) => Err(format!(
            "{} is iii {major}.{minor}; this test needs {}.{} or later (set III_ENGINE_BIN)",
            bin.display(),
            MIN_ENGINE.0,
            MIN_ENGINE.1,
        )),
        None => Err(format!("{} --version printed no version", bin.display())),
    }
}

/// `(major, minor)` from `iii --version` (`0.26.0` or `iii 0.26.0`).
fn engine_version(bin: &Path) -> Option<(u64, u64)> {
    let output = Command::new(bin).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text
        .split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = version.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("bind an ephemeral port")
        .port()
}

/// No engine to run against: skip, or fail under `III_E2E_REQUIRE`.
fn skip(reason: &str) -> Option<Harness> {
    if std::env::var_os("III_E2E_REQUIRE").is_some() {
        panic!("III_E2E_REQUIRE is set and the end-to-end test cannot run: {reason}");
    }
    eprintln!("skipping: {reason}");
    None
}

struct Harness {
    dir: PathBuf,
    engine: Option<Child>,
    worker: Child,
    engine_url: String,
    client: IIIClient,
    http_port: u16,
    rebound_http_port: u16,
    config_id: String,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.client.shutdown();
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        if let Some(engine) = self.engine.as_mut() {
            let _ = engine.kill();
            let _ = engine.wait();
        }
        if std::thread::panicking() {
            eprintln!("engine and worker logs kept in {}", self.dir.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// Start the engine named by `III_ENGINE_BIN` on an ephemeral port with an
/// engine-only config: the worker manager and the configuration worker.
fn spawn_engine(bin: &Path, dir: &Path) -> Result<(String, Child), String> {
    let port = free_port();
    let config = format!(
        "workers:
  - name: iii-worker-manager
    config:
      host: 127.0.0.1
      port: {port}
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: {dir}/configuration
",
        dir = dir.display(),
    );
    let config_path = dir.join("engine.yaml");
    std::fs::write(&config_path, config).map_err(|e| format!("write engine config: {e}"))?;
    let log = std::fs::File::create(dir.join("engine.log"))
        .map_err(|e| format!("create engine log: {e}"))?;
    let child = Command::new(bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(dir)
        .env("III_TELEMETRY_ENABLED", "false")
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .map_err(|e| format!("start {}: {e}", bin.display()))?;
    Ok((format!("ws://127.0.0.1:{port}"), child))
}

/// Resolves once the engine answers and its configuration worker serves
/// (a missing entry answers `NOT_FOUND`).
async fn engine_ready(client: &IIIClient, max: Duration) -> bool {
    let deadline = Instant::now() + max;
    while Instant::now() < deadline {
        let answer = client
            .call(CallRequest {
                function_id: "configuration::get".to_string(),
                payload: json!({ "id": "ade-integration-readiness-probe" }),
                action: None,
                timeout_ms: Some(1_000),
            })
            .await;
        match answer {
            Ok(_) => return true,
            Err(error) if error.to_string().to_ascii_uppercase().contains("NOT_FOUND") => {
                return true;
            }
            Err(_) => sleep(Duration::from_millis(200)).await,
        }
    }
    false
}

async fn boot() -> Option<Harness> {
    let source = match engine_source() {
        Ok(source) => source,
        Err(reason) => return skip(&reason),
    };
    let dir = std::env::temp_dir().join(format!("ade-integration-{}", Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("configuration")).expect("create the test dir");

    let (engine_url, engine) = match source {
        EngineSource::Running(url) => (url, None),
        EngineSource::Spawn(bin) => match spawn_engine(&bin, &dir) {
            Ok((url, child)) => (url, Some(child)),
            Err(reason) => {
                let _ = std::fs::remove_dir_all(&dir);
                return skip(&reason);
            }
        },
    };

    let worker_bin = env!("CARGO_BIN_EXE_ade");
    let config_path = format!("{}/config.yaml", env!("CARGO_MANIFEST_DIR"));
    let config_id = format!("console-integration-{}", Uuid::new_v4());
    let http_port = free_port();
    let rebound_http_port = free_port();
    let worker_log = std::fs::File::create(dir.join("ade.log")).expect("create the worker log");
    let worker = Command::new(worker_bin)
        .args([
            "--url",
            &engine_url,
            "--config",
            &config_path,
            "--http-port",
            &http_port.to_string(),
            "--http-host",
            "127.0.0.1",
        ])
        .current_dir(&dir)
        .env("III_CONFIG_NAME", &config_id)
        .env("III_COMPOSE_DIR", &dir)
        .stdout(Stdio::null())
        .stderr(worker_log)
        .spawn()
        .expect("start the ade worker");

    let client = register_worker(&engine_url, InitOptions::default());
    let harness = Harness {
        dir,
        engine,
        worker,
        engine_url,
        client,
        http_port,
        rebound_http_port,
        config_id,
    };
    if !engine_ready(&harness.client, Duration::from_secs(30)).await {
        let reason = format!("no engine answered at {}", harness.engine_url);
        drop(harness);
        return skip(&reason);
    }
    assert!(
        wait_for_listen(harness.http_port, Duration::from_secs(20)).await,
        "the ade worker never listened on {} (logs in {})",
        harness.http_port,
        harness.dir.display(),
    );
    Some(harness)
}

async fn wait_for_listen(port: u16, max: Duration) -> bool {
    let deadline = Instant::now() + max;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    false
}

/// The payload as the console sent it: a call through an observer carries
/// the `_caller_worker_id` stamp the engine adds to every call a worker
/// makes.
fn without_caller(mut value: Value) -> Value {
    if let Some(fields) = value.as_object_mut() {
        fields.remove("_caller_worker_id");
    }
    value
}

/// Register `id` as a target that forwards what it receives.
fn recording_target(client: &IIIClient, id: &str) -> UnboundedReceiver<Value> {
    let (tx, rx) = unbounded_channel();
    client.register_function(
        id,
        RegisterFunction::new(move |event: Value| {
            let _ = tx.send(without_caller(event));
            Ok::<_, Error>(json!({}))
        }),
    );
    rx
}

async fn next_event(rx: &mut UnboundedReceiver<Value>, what: &str) -> Value {
    timeout(Duration::from_secs(10), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
        .unwrap_or_else(|| panic!("channel closed waiting for {what}"))
}

/// Whether `event`, a `console::ui::on-assets-changed` event, is an `op`
/// about `path`.
fn is_asset_event(event: &Value, op: &str, path: &str) -> bool {
    let about = match op {
        "set" => &event["asset"]["path"],
        _ => &event["path"],
    };
    event["op"] == op && about == path
}

/// The next `console::ui::on-assets-changed` event of `op` about `path`,
/// skipping events about other assets (the console's own UI among them).
async fn asset_event(rx: &mut UnboundedReceiver<Value>, op: &str, path: &str) -> Value {
    loop {
        let event = next_event(rx, &format!("an assets `{op}` of {path}")).await;
        if is_asset_event(&event, op, path) {
            return event;
        }
    }
}

/// [`asset_event`] for a default observer: the next envelope whose output is
/// an `op` about `path`.
async fn asset_envelope(rx: &mut UnboundedReceiver<Value>, op: &str, path: &str) -> Value {
    loop {
        let envelope = next_event(rx, &format!("an assets `{op}` envelope of {path}")).await;
        if is_asset_event(&envelope["output"], op, path) {
            return envelope;
        }
    }
}

async fn active(observer: &ObserverHandle, what: &str) {
    timeout(Duration::from_secs(10), observer.ready())
        .await
        .unwrap_or_else(|_| panic!("{what} never became active"))
        .unwrap_or_else(|error| panic!("{what} was refused: {error}"));
}

/// Waits until the provider rejects `observer` and returns the code.
async fn rejected(observer: &ObserverHandle, what: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if observer.state() == ObserverState::Error {
            return observer
                .reason()
                .map(|reason| reason.code)
                .unwrap_or_default();
        }
        assert!(
            Instant::now() < deadline,
            "{what} was never rejected (state {:?})",
            observer.state()
        );
        sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn end_to_end_http_ws_proxy_and_configuration_rebind() {
    let Some(harness) = boot().await else {
        return;
    };
    let client = &harness.client;
    let http_port = harness.http_port;
    let rebound_http_port = harness.rebound_http_port;

    let base = format!("http://127.0.0.1:{http_port}");

    // 1. GET / -> 200, contains the SPA root.
    let res = reqwest::get(&base).await.expect("GET / failed");
    assert!(res.status().is_success(), "GET / returned {}", res.status());
    let ctype = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ctype.starts_with("text/html"),
        "Content-Type for / was {ctype:?}"
    );
    let body = res.text().await.expect("GET / body");
    assert!(
        body.contains("id=\"root\""),
        "GET / did not include the SPA mount point; body starts with: {}",
        body.chars().take(200).collect::<String>(),
    );

    // 2. Pluck the first asset href out of index.html and GET it.
    let asset_path = parse_first_asset(&body).expect("no asset link in index.html");
    let asset_url = resolve_asset_url(&base, &asset_path);
    let res = reqwest::get(&asset_url)
        .await
        .unwrap_or_else(|e| panic!("GET {asset_url} failed: {e}"));
    assert!(
        res.status().is_success(),
        "GET {asset_url} returned {}",
        res.status()
    );
    let cache = res
        .headers()
        .get(reqwest::header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        cache.contains("immutable"),
        "asset Cache-Control did not include 'immutable': {cache:?}"
    );

    // 3. GET /missing -> 404.
    let res = reqwest::get(format!("{base}/this/does/not/exist"))
        .await
        .expect("GET /missing failed");
    assert_eq!(res.status().as_u16(), 404, "GET /missing must 404");

    // 4. Open /ws and observe the engine on the other end. We send a
    //    syntactically-valid iii control frame and expect ANY frame
    //    back from the engine — that proves the proxy is forwarding
    //    bidirectionally. We don't care about the semantic response.
    let ws_url = format!("ws://127.0.0.1:{http_port}/ws");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .unwrap_or_else(|e| panic!("WS connect to {ws_url} failed: {e}"));

    let probe = serde_json::json!({
        "id": "console-int-test-1",
        "type": "ping",
    });
    ws.send(Message::Text(probe.to_string()))
        .await
        .expect("WS send failed");

    let frame = timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("WS recv timed out — proxy isn't forwarding");
    let frame = frame
        .expect("WS stream ended before any frame")
        .expect("WS recv error");
    match frame {
        Message::Text(_) | Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => {
            // Good — the engine sent something through the proxy.
        }
        Message::Close(_) | Message::Frame(_) => {
            panic!("proxy closed the WS without forwarding a real frame: {frame:?}");
        }
    }

    let _ = ws.close(None).await;

    // 5. Change the authoritative configuration entry and prove the same
    // process moves to the new listener: replacement serves, status updates,
    // and the original port stops accepting new connections.
    let deadline = Instant::now() + Duration::from_secs(10);
    let current = loop {
        match client
            .call(
                CallRequest {
                    function_id: "configuration::get".to_string(),
                    payload: serde_json::json!({ "id": harness.config_id }),
                    action: None,
                    timeout_ms: Some(2_000),
                }
                .namespace("default"),
            )
            .await
        {
            Ok(value) => break value,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "configuration::get never became callable: {error}"
                );
                sleep(Duration::from_millis(100)).await;
            }
        }
    };
    let mut value = current["value"].clone();
    value["http_port"] = json!(rebound_http_port);
    client
        .call(
            CallRequest {
                function_id: "configuration::set".to_string(),
                payload: serde_json::json!({
                    "id": harness.config_id,
                    "value": value,
                }),
                action: None,
                timeout_ms: Some(5_000),
            }
            .namespace("default"),
        )
        .await
        .expect("configuration::set failed");

    assert!(
        wait_for_listen(rebound_http_port, Duration::from_secs(8)).await,
        "rebound Console port never started listening"
    );
    let rebound_base = format!("http://127.0.0.1:{rebound_http_port}");
    let rebound = reqwest::get(format!("{rebound_base}/"))
        .await
        .expect("GET / on rebound port failed");
    assert!(rebound.status().is_success());

    let status = client
        .call(CallRequest {
            function_id: "console::status".to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("console::status failed after rebind");
    assert_eq!(status["http_port"], rebound_http_port);
    assert_eq!(status["url"], rebound_base);

    let old_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if TcpStream::connect(("127.0.0.1", http_port)).await.is_err() {
            break;
        }
        assert!(
            Instant::now() < old_deadline,
            "original Console port kept accepting connections after rebind"
        );
        sleep(Duration::from_millis(100)).await;
    }

    // 6. Observe console::workspace::on-changed: the console rings a new
    //    observer once it is active.
    let mut rung = recording_target(client, "integration::workspace-rung");
    let observer = client.observe(
        Source::function("console::workspace::on-changed").deliver(Deliver::Output),
        "integration::workspace-rung",
    );
    active(&observer, "the console::workspace::on-changed observer").await;
    // The ring is a call of the function for this observer: `deliver: output`
    // hands the target the function's output, the empty event.
    assert_eq!(next_event(&mut rung, "the workspace ring").await, json!({}));
    observer.dispose().await.expect("dispose failed");

    // 7. Injectable UI.
    ui_assets(client, &rebound_base).await;

    harness.client.shutdown_async().await;
}

/// `console::ui::on-assets-changed` and `console::ui::asset` against the
/// engine: a tab's observer receives the served set in a call for it alone
/// once it is active; an asset observer's content function receives
/// `{ path }` once the observer is active and its answer is served at
/// `/ui/<path>` and announced to every tab as a `set`; a later observer of
/// the path supersedes it; a failed fetch rejects the observer; removing the
/// asset's observer announces a `delete`.
async fn ui_assets(client: &IIIClient, base: &str) {
    const ASSET_FN: &str = "console::ui::asset";
    const ON_ASSETS_CHANGED_FN: &str = "console::ui::on-assets-changed";
    const PATH: &str = "integration/e2e.js";

    let mut fetches = {
        let (tx, rx) = unbounded_channel();
        client.register_function(
            "integration::ui-content",
            RegisterFunction::new(move |input: Value| {
                let _ = tx.send(without_caller(input));
                Ok::<_, Error>(json!({ "content": "export const version = 1\n" }))
            }),
        );
        rx
    };
    client.register_function(
        "integration::ui-content-next",
        RegisterFunction::new(|_: Value| {
            Ok::<_, Error>(json!({ "content": "export const version = 2\n" }))
        }),
    );
    client.register_function(
        "integration::ui-content-broken",
        RegisterFunction::new(|_: Value| {
            Err::<Value, _>(Error::Handler("no content for this path".to_string()))
        }),
    );
    let observe_asset = |kind: &str, path: &str, content_fn: &str| {
        client.observe(
            Source::function(ASSET_FN).args(json!({ "kind": kind, "path": path })),
            content_fn,
        )
    };

    // A tab: its first delivery is the sync, a call for its observer alone.
    let mut tab_events = recording_target(client, "integration::assets-tab");
    let tab = client.observe(
        Source::function(ON_ASSETS_CHANGED_FN).deliver(Deliver::Output),
        "integration::assets-tab",
    );
    active(&tab, "the tab's console::ui::on-assets-changed observer").await;
    let sync = next_event(&mut tab_events, "the tab's sync").await;
    assert_eq!(sync["op"], "sync", "a new tab observer was sent {sync}");
    assert!(sync["assets"].is_array(), "sync without assets: {sync}");

    // An asset: its content function receives `{ path }`; the content is
    // served and announced.
    let first = observe_asset("module", PATH, "integration::ui-content");
    active(&first, "the asset observer").await;
    assert_eq!(
        next_event(&mut fetches, "the content fetch").await,
        json!({ "path": PATH })
    );
    let set = asset_event(&mut tab_events, "set", PATH).await;
    assert_eq!(set["asset"]["kind"], "module");
    let first_hash = set["asset"]["hash"].clone();
    assert!(first_hash.is_string(), "set without a hash: {set}");
    let served = reqwest::get(format!("{base}/ui/{PATH}"))
        .await
        .expect("GET the asset");
    assert!(
        served.status().is_success(),
        "GET /ui/{PATH}: {}",
        served.status()
    );
    assert_eq!(served.text().await.unwrap(), "export const version = 1\n");

    // A tab that opens later gets the asset in its sync.
    let mut late_events = recording_target(client, "integration::assets-late-tab");
    let late = client.observe(
        Source::function(ON_ASSETS_CHANGED_FN).deliver(Deliver::Output),
        "integration::assets-late-tab",
    );
    active(&late, "the late tab's observer").await;
    let sync = next_event(&mut late_events, "the late tab's sync").await;
    assert_eq!(sync["op"], "sync");
    assert!(
        sync["assets"]
            .as_array()
            .is_some_and(|assets| assets.contains(&json!({
                "path": PATH,
                "kind": "module",
                "hash": first_hash,
            }))),
        "the late tab's sync does not name {PATH}: {sync}"
    );
    late.dispose().await.expect("dispose the late tab");

    // A later observer of the path supersedes the first. Its fetch is a call
    // for it alone: the first observer's content function is not called.
    let next = observe_asset("module", PATH, "integration::ui-content-next");
    active(&next, "the superseding asset observer").await;
    let set = asset_event(&mut tab_events, "set", PATH).await;
    assert_ne!(set["asset"]["hash"], first_hash, "content changed: {set}");
    assert!(
        fetches.try_recv().is_err(),
        "the fetch for the superseding observer reached the first one's content function"
    );
    assert_eq!(
        rejected(&first, "the superseded asset observer").await,
        "UI_ASSET_SUPERSEDED"
    );
    let served = reqwest::get(format!("{base}/ui/{PATH}"))
        .await
        .expect("GET the asset");
    assert_eq!(served.text().await.unwrap(), "export const version = 2\n");

    // A content function that fails rejects its observer.
    let broken = observe_asset(
        "script",
        "integration/broken.js",
        "integration::ui-content-broken",
    );
    assert_eq!(
        rejected(&broken, "the asset observer whose fetch fails").await,
        "UI_ASSET_FETCH_FAILED"
    );

    // Removing the asset's observer removes the asset.
    next.dispose().await.expect("dispose the asset observer");
    asset_event(&mut tab_events, "delete", PATH).await;
    let gone = reqwest::get(format!("{base}/ui/{PATH}"))
        .await
        .expect("GET the removed asset");
    assert_eq!(gone.status().as_u16(), 404);

    tab.dispose().await.expect("dispose the tab");
    let _ = first.dispose().await;
    let _ = broken.dispose().await;
}

const WORKSPACE_CHANGED_FN: &str = "console::workspace::on-changed";
const ASSETS_CHANGED_FN: &str = "console::ui::on-assets-changed";
const ASSET_FN: &str = "console::ui::asset";

/// How long a target stays quiet before the test concludes nothing more is
/// coming for it.
const QUIET: Duration = Duration::from_millis(750);

/// Waits [`QUIET`] and fails when `rx` received anything meanwhile.
async fn quiet(rx: &mut UnboundedReceiver<Value>, what: &str) {
    sleep(QUIET).await;
    if let Ok(event) = rx.try_recv() {
        panic!("{what}: {event}");
    }
}

/// The top-level keys of a delivered envelope, sorted.
fn keys_of(envelope: &Value) -> Vec<String> {
    let mut keys: Vec<String> = envelope
        .as_object()
        .unwrap_or_else(|| panic!("not an envelope: {envelope}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// `own` (a call for one observer) and `ordinary` (any other call of the
/// same function) reached a default-envelope observer in one shape.
fn same_envelope(own: &Value, ordinary: &Value, function_id: &str, observer_id: &str) {
    assert_eq!(keys_of(own), keys_of(ordinary), "{own:#}\n{ordinary:#}");
    for envelope in [own, ordinary] {
        assert_eq!(envelope["function_id"], function_id, "{envelope:#}");
        assert_eq!(envelope["observer_id"], observer_id, "{envelope:#}");
        assert!(envelope.get("output").is_some(), "{envelope:#}");
    }
}

/// Stores the layout: every store write rings `console::workspace::on-changed`.
async fn write_layout(client: &IIIClient) {
    client
        .call(CallRequest {
            function_id: "console::workspace::set".to_string(),
            payload: json!({ "value": {} }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("console::workspace::set failed");
}

/// Resolves once the console serves its own four assets, so no `set` of
/// them reaches a tab the test observes afterwards.
async fn console_assets_served(client: &IIIClient) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let manifest = client
            .call(CallRequest {
                function_id: "console::ui-manifest".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(2_000),
            })
            .await;
        let served = manifest.as_ref().map_or(0, |m| {
            m["assets"].as_array().map_or(0, |assets| {
                assets
                    .iter()
                    .filter(|a| {
                        a["path"]
                            .as_str()
                            .is_some_and(|p| p.starts_with("console/"))
                    })
                    .count()
            })
        });
        if served == 4 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the console never served its own assets: {manifest:?}"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

/// iii decision 52 against the engine: the call the console makes for one
/// new observer (the workspace ring, a tab's asset `sync`, an asset's
/// content fetch) reaches that observer alone, and reaches it in the shape
/// of every other call of the function: the envelope for a default
/// observer, the event for a `deliver: output` one.
#[tokio::test]
async fn a_call_for_one_observer_reaches_it_alone_in_the_shape_of_every_event() {
    let Some(harness) = boot().await else {
        return;
    };
    let client = &harness.client;
    console_assets_served(client).await;

    // console::workspace::on-changed. A takes the envelope, B the output.
    let mut ring_a = recording_target(client, "integration::ring-envelope");
    let mut ring_b = recording_target(client, "integration::ring-output");
    let a = client.observe(
        Source::function(WORKSPACE_CHANGED_FN),
        "integration::ring-envelope",
    );
    active(&a, "the envelope ring observer").await;
    let own_a = next_event(&mut ring_a, "A's own ring").await;
    let b = client.observe(
        Source::function(WORKSPACE_CHANGED_FN).deliver(Deliver::Output),
        "integration::ring-output",
    );
    active(&b, "the output ring observer").await;
    assert_eq!(next_event(&mut ring_b, "B's own ring").await, json!({}));
    quiet(&mut ring_a, "B's own ring reached A").await;
    quiet(&mut ring_b, "B got a second ring of its own").await;
    // A store write rings both, once each.
    write_layout(client).await;
    let ordinary_a = next_event(&mut ring_a, "A's ring after a write").await;
    assert_eq!(
        next_event(&mut ring_b, "B's ring after a write").await,
        json!({})
    );
    same_envelope(&own_a, &ordinary_a, WORKSPACE_CHANGED_FN, a.id());
    assert_eq!(own_a["output"], json!({}));
    assert_eq!(ordinary_a["output"], json!({}));
    quiet(&mut ring_a, "A got a second ring for one write").await;
    a.dispose().await.expect("dispose A");
    b.dispose().await.expect("dispose B");

    // console::ui::on-assets-changed. Tab A takes the envelope, tab B the
    // output.
    let mut tab_a = recording_target(client, "integration::tab-envelope");
    let mut tab_b = recording_target(client, "integration::tab-output");
    let a = client.observe(
        Source::function(ASSETS_CHANGED_FN),
        "integration::tab-envelope",
    );
    active(&a, "the envelope tab").await;
    let sync_a = next_event(&mut tab_a, "tab A's sync").await;
    assert_eq!(sync_a["output"]["op"], "sync", "{sync_a:#}");
    let b = client.observe(
        Source::function(ASSETS_CHANGED_FN).deliver(Deliver::Output),
        "integration::tab-output",
    );
    active(&b, "the output tab").await;
    let sync_b = next_event(&mut tab_b, "tab B's sync").await;
    assert_eq!(sync_b["op"], "sync", "{sync_b}");
    assert_eq!(sync_b["assets"], sync_a["output"]["assets"]);
    quiet(&mut tab_a, "tab B's sync reached tab A").await;

    // An asset change reaches both tabs; its fetch reaches its own content
    // function alone.
    const PATH: &str = "integration/shape.js";
    let mut fetches_one = recording_content(client, "integration::content-one", "1");
    let mut fetches_two = recording_content(client, "integration::content-two", "2");
    let one = client.observe(
        Source::function(ASSET_FN).args(json!({ "kind": "script", "path": PATH })),
        "integration::content-one",
    );
    active(&one, "the first asset observer").await;
    assert_eq!(
        next_event(&mut fetches_one, "the first fetch").await,
        json!({ "path": PATH })
    );
    let set_a = next_event(&mut tab_a, "tab A's set").await;
    assert_eq!(set_a["output"]["op"], "set", "{set_a:#}");
    assert_eq!(set_a["output"]["asset"]["path"], PATH);
    let set_b = next_event(&mut tab_b, "tab B's set").await;
    assert_eq!(set_b["op"], "set", "{set_b}");
    same_envelope(&sync_a, &set_a, ASSETS_CHANGED_FN, a.id());

    let two = client.observe(
        Source::function(ASSET_FN)
            .args(json!({ "kind": "script", "path": "integration/other.js" })),
        "integration::content-two",
    );
    active(&two, "the second asset observer").await;
    assert_eq!(
        next_event(&mut fetches_two, "the second fetch").await,
        json!({ "path": "integration/other.js" })
    );
    asset_event(&mut tab_b, "set", "integration/other.js").await;
    quiet(
        &mut fetches_one,
        "the second asset's fetch reached the first content function",
    )
    .await;

    for observer in [a, b, one, two] {
        let _ = observer.dispose().await;
    }
    harness.client.shutdown_async().await;
}

/// Register `id` as a content function answering `version`; it forwards
/// each request it receives.
fn recording_content(client: &IIIClient, id: &str, version: &str) -> UnboundedReceiver<Value> {
    let (tx, rx) = unbounded_channel();
    let content = format!("export const version = {version}\n");
    client.register_function(
        id,
        RegisterFunction::new(move |input: Value| {
            let _ = tx.send(without_caller(input));
            Ok::<_, Error>(json!({ "content": content }))
        }),
    );
    rx
}

/// A TCP proxy between a worker and the engine that can cut every
/// connection it carries; the worker reconnects through it and reattaches.
struct DropProxy {
    url: String,
    drop_all: broadcast::Sender<()>,
}

impl DropProxy {
    async fn start(engine_url: &str) -> Self {
        let upstream = engine_url
            .trim_start_matches("ws://")
            .trim_end_matches('/')
            .to_string();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind the proxy");
        let url = format!("ws://{}", listener.local_addr().expect("proxy address"));
        let (drop_all, _) = broadcast::channel(4);
        let dropper = drop_all.clone();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let Ok(server) = TcpStream::connect(&upstream).await else {
                    continue;
                };
                let mut dropped = dropper.subscribe();
                tokio::spawn(async move {
                    let (mut client_rx, mut client_tx) = client.into_split();
                    let (mut server_rx, mut server_tx) = server.into_split();
                    tokio::select! {
                        _ = tokio::io::copy(&mut client_rx, &mut server_tx) => {}
                        _ = tokio::io::copy(&mut server_rx, &mut client_tx) => {}
                        _ = dropped.recv() => {}
                    }
                });
            }
        });
        Self { url, drop_all }
    }

    /// Cuts every proxied connection.
    fn drop_connections(&self) {
        let _ = self.drop_all.send(());
    }
}

async fn connection(client: &IIIClient, connected: bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while (client.get_connection_state() == IIIConnectionState::Connected) != connected {
        assert!(Instant::now() < deadline, "{what}");
        sleep(Duration::from_millis(20)).await;
    }
}

/// An observer whose worker's connection drops is withdrawn from the
/// console and announced to it again when the worker reattaches (iii
/// decision 47); the console then sends it its current state again in a
/// call for it alone (decision 52): a tab gets a new `sync` and a new ring,
/// an asset's content is fetched again and the asset announced again. The
/// tab and the ring observer behind the proxy take the default envelope, and
/// the new `sync` and ring reach them in the envelope of every other call of
/// their function. A tab on another connection gets none of the calls for
/// those observers, and sees the asset deleted while its worker is away and
/// set again after.
#[tokio::test]
async fn an_observer_that_reattaches_gets_its_current_state_again() {
    let Some(harness) = boot().await else {
        return;
    };
    let client = &harness.client;
    console_assets_served(client).await;

    // A tab on the harness's own connection, which stays up.
    let mut steady = recording_target(client, "integration::steady-tab");
    let steady_tab = client.observe(
        Source::function(ASSETS_CHANGED_FN).deliver(Deliver::Output),
        "integration::steady-tab",
    );
    active(&steady_tab, "the steady tab").await;
    assert_eq!(
        next_event(&mut steady, "the steady tab's sync").await["op"],
        "sync"
    );
    let mut steady_ring = recording_target(client, "integration::steady-ring");
    let steady_ringer = client.observe(
        Source::function(WORKSPACE_CHANGED_FN).deliver(Deliver::Output),
        "integration::steady-ring",
    );
    active(&steady_ringer, "the steady ring observer").await;
    next_event(&mut steady_ring, "the steady ring observer's own ring").await;

    // A worker behind the proxy: a tab and a ring observer, both taking the
    // default envelope, and an asset.
    const PATH: &str = "integration/away.js";
    let proxy = DropProxy::start(&harness.engine_url).await;
    // Its own worker name: the engine admits one live worker per name.
    let away = register_worker(
        &proxy.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: format!("integration-away-{}", Uuid::new_v4()),
                ..WorkerMetadata::default()
            }),
            identity: WorkerIdentityMode::Explicit,
            ..InitOptions::default()
        },
    );
    let mut tab = recording_target(&away, "integration::away-tab");
    let mut ring = recording_target(&away, "integration::away-ring");
    let mut fetches = recording_content(&away, "integration::away-content", "1");
    let away_tab = away.observe(Source::function(ASSETS_CHANGED_FN), "integration::away-tab");
    let away_ringer = away.observe(
        Source::function(WORKSPACE_CHANGED_FN),
        "integration::away-ring",
    );
    active(&away_tab, "the tab behind the proxy").await;
    active(&away_ringer, "the ring observer behind the proxy").await;
    // Their first sync and ring. The shape of a new observer's call is
    // checked by a_call_for_one_observer_reaches_it_alone_in_the_shape_of_every_event.
    next_event(&mut tab, "the first sync").await;
    next_event(&mut ring, "the first ring").await;
    let asset = away.observe(
        Source::function(ASSET_FN).args(json!({ "kind": "script", "path": PATH })),
        "integration::away-content",
    );
    active(&asset, "the asset behind the proxy").await;
    assert_eq!(
        next_event(&mut fetches, "the first fetch").await,
        json!({ "path": PATH })
    );
    let set = asset_event(&mut steady, "set", PATH).await;
    let hash = set["asset"]["hash"].clone();
    // The tab behind the proxy sees the set too, in the envelope of every
    // call of the function.
    let away_set = asset_envelope(&mut tab, "set", PATH).await;
    assert_eq!(away_set["output"]["asset"]["hash"], hash, "{away_set:#}");

    // The worker's connection drops and comes back.
    proxy.drop_connections();
    connection(
        &away,
        false,
        "the worker behind the proxy never saw the drop",
    )
    .await;
    // While it is away its asset is withdrawn.
    asset_event(&mut steady, "delete", PATH).await;
    connection(&away, true, "the worker behind the proxy never reconnected").await;
    active(&away_tab, "the tab after it reattached").await;
    active(&asset, "the asset after it reattached").await;

    // Each observer behind the proxy gets its current state again, in the
    // envelope of every call of its function. A bare `sync` ends the wait
    // too, so the shape check names it.
    let sync = loop {
        let event = next_event(&mut tab, "the sync after the reattach").await;
        if event["output"]["op"] == "sync" || event["op"] == "sync" {
            break event;
        }
    };
    same_envelope(&sync, &away_set, ASSETS_CHANGED_FN, away_tab.id());
    assert!(sync["output"]["assets"].is_array(), "{sync:#}");
    let renewed_ring = next_event(&mut ring, "the ring after the reattach").await;
    assert_eq!(
        next_event(&mut fetches, "the fetch after the reattach").await,
        json!({ "path": PATH })
    );
    let set = asset_event(&mut steady, "set", PATH).await;
    assert_eq!(set["asset"]["hash"], hash, "the same content: {set}");

    // The steady observers got none of those calls.
    quiet(
        &mut steady_ring,
        "a ring for the reattached observer reached the steady one",
    )
    .await;
    while let Ok(event) = steady.try_recv() {
        assert_ne!(
            event["op"], "sync",
            "a sync for the reattached tab reached the steady one"
        );
    }

    // A store write rings both ring observers once; the one behind the proxy
    // receives it in the envelope of its renewed ring.
    write_layout(client).await;
    assert_eq!(
        next_event(&mut steady_ring, "the steady ring after a write").await,
        json!({})
    );
    let ordinary_ring = next_event(&mut ring, "the ring after a write").await;
    same_envelope(
        &renewed_ring,
        &ordinary_ring,
        WORKSPACE_CHANGED_FN,
        away_ringer.id(),
    );
    assert_eq!(renewed_ring["output"], json!({}), "{renewed_ring:#}");
    assert_eq!(ordinary_ring["output"], json!({}), "{ordinary_ring:#}");

    for observer in [steady_tab, steady_ringer] {
        let _ = observer.dispose().await;
    }
    away.shutdown_async().await;
    harness.client.shutdown_async().await;
}

/// A page cannot register a function the console hosts through `/ws`, and
/// what it registers reaches the engine stripped of event-source settings.
/// Self-contained: uses a fake engine, no `iii` binary needed.
#[tokio::test]
async fn ws_proxy_drops_console_provider_registrations() {
    use std::sync::Arc;

    // Fake engine: accept one WS connection and hand back the first frame
    // it receives.
    let engine_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake engine listener");
    let engine_addr = engine_listener.local_addr().expect("fake engine addr");
    let (frame_tx, frame_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (stream, _) = engine_listener.accept().await.expect("accept proxy dial");
        let mut ws = tokio_tungstenite::accept_async(stream)
            .await
            .expect("ws handshake from proxy");
        if let Some(Ok(Message::Text(text))) = ws.next().await {
            let _ = frame_tx.send(text.to_string());
        }
        while let Some(Ok(_)) = ws.next().await {}
    });

    let engine_url = Arc::new(format!("ws://{engine_addr}"));
    let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let (bound_tx, bound_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(ade::server::serve(
        0,
        ade::server::AppState::new(engine_url, None, None, None),
        shutdown_rx,
        Some(bound_tx),
    ));
    let addr = bound_rx.await.expect("server bound");
    let (mut ws, _) =
        tokio_tungstenite::connect_async(&format!("ws://127.0.0.1:{}/ws", addr.port()))
            .await
            .expect("connect to /ws proxy");

    let hijack = serde_json::json!({
        "type": "registerfunction",
        "id": "console::ui::asset",
        "private": true,
    });
    let page = serde_json::json!({
        "type": "registerfunction",
        "id": "iii::console::ui-assets::console-1",
        "private": true,
        "args_format": { "type": "object" },
    });
    ws.send(Message::Text(hijack.to_string()))
        .await
        .expect("send hijack");
    ws.send(Message::Text(page.to_string()))
        .await
        .expect("send page registration");

    let first = timeout(Duration::from_secs(5), frame_rx)
        .await
        .expect("the fake engine received nothing")
        .expect("fake engine closed");
    let first: serde_json::Value = serde_json::from_str(&first).expect("forwarded frame is JSON");
    assert_eq!(first["id"], "iii::console::ui-assets::console-1");
    assert!(first.get("private").is_none());
    assert!(first.get("args_format").is_none());
    assert_eq!(first["metadata"]["internal"], serde_json::json!(true));
    let _ = ws.close(None).await;
}

fn parse_first_asset(html: &str) -> Option<String> {
    for attr in ["src=\"", "href=\""] {
        let mut search_from = 0;
        while let Some(rel_start) = html[search_from..].find(attr) {
            let value_start = search_from + rel_start + attr.len();
            let after_attr = &html[value_start..];
            if let Some(end) = after_attr.find('"') {
                let path = &after_attr[..end];
                if path.contains("assets/") && (path.ends_with(".js") || path.ends_with(".css")) {
                    return Some(path.to_string());
                }
            }
            search_from = value_start;
        }
    }
    None
}

fn resolve_asset_url(base: &str, asset_path: &str) -> String {
    if asset_path.starts_with('/') {
        format!("{base}{asset_path}")
    } else {
        let base = base.trim_end_matches('/');
        let path = asset_path.strip_prefix("./").unwrap_or(asset_path);
        format!("{base}/{path}")
    }
}

#[test]
fn parse_first_asset_picks_script_or_link() {
    let html = r#"<html><head><script type="module" src="/assets/index-abc.js"></script><link href="/assets/index-def.css"/></head></html>"#;
    assert_eq!(
        parse_first_asset(html).as_deref(),
        Some("/assets/index-abc.js")
    );

    let html = r#"<html><link href="/assets/style-1.css"/></html>"#;
    assert_eq!(
        parse_first_asset(html).as_deref(),
        Some("/assets/style-1.css")
    );

    let html = r#"<html><head><script type="module" src="./assets/index-abc.js"></script><link href="./assets/index-def.css"/></head></html>"#;
    assert_eq!(
        parse_first_asset(html).as_deref(),
        Some("./assets/index-abc.js")
    );

    assert!(parse_first_asset("<html></html>").is_none());
}

#[test]
fn resolve_asset_url_joins_relative_paths() {
    assert_eq!(
        resolve_asset_url("http://127.0.0.1:3113", "/assets/index.js"),
        "http://127.0.0.1:3113/assets/index.js"
    );
    assert_eq!(
        resolve_asset_url("http://127.0.0.1:3113", "./assets/index.js"),
        "http://127.0.0.1:3113/assets/index.js"
    );
    assert_eq!(
        resolve_asset_url("http://127.0.0.1:3113/", "assets/index.js"),
        "http://127.0.0.1:3113/assets/index.js"
    );
}

/// Regression test: an engine response bigger than tungstenite's default
/// limits (16 MiB frame / 64 MiB message) must pass through the proxy
/// instead of killing the connection. Before the fix the proxy dialed the
/// engine with default limits, the read errored, and the browser socket
/// was closed — putting the console SPA into an endless reconnect loop.
/// Self-contained: uses a fake engine, no `iii` binary needed.
#[tokio::test]
async fn ws_proxy_forwards_oversized_engine_message() {
    use std::sync::Arc;

    const BIG_LEN: usize = 20 * 1024 * 1024; // > 16 MiB default frame cap

    // Fake engine: accept one WS connection, send one oversized text
    // message, then hold the connection open until the peer closes.
    let engine_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake engine listener");
    let engine_addr = engine_listener.local_addr().expect("fake engine addr");
    tokio::spawn(async move {
        let (stream, _) = engine_listener.accept().await.expect("accept proxy dial");
        let mut ws = tokio_tungstenite::accept_async(stream)
            .await
            .expect("ws handshake from proxy");
        ws.send(Message::Text("x".repeat(BIG_LEN)))
            .await
            .expect("send oversized message");
        while let Some(Ok(_)) = ws.next().await {}
    });

    // Console worker's HTTP server (port 0 = OS-assigned) fronting it.
    let engine_url = Arc::new(format!("ws://{engine_addr}"));
    let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let (bound_tx, bound_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(ade::server::serve(
        0,
        ade::server::AppState::new(engine_url, None, None, None),
        shutdown_rx,
        Some(bound_tx),
    ));
    let addr = bound_rx.await.expect("server bound");

    // Browser stand-in: browsers impose no message size limit, so the
    // test client must not either.
    let client_config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: None,
        max_frame_size: None,
        ..Default::default()
    };
    let (mut ws, _) = tokio_tungstenite::connect_async_with_config(
        &format!("ws://127.0.0.1:{}/ws", addr.port()),
        Some(client_config),
        false,
    )
    .await
    .expect("connect to /ws proxy");

    let msg = timeout(Duration::from_secs(15), ws.next())
        .await
        .expect("timed out waiting for proxied message")
        .expect("proxy closed the connection instead of forwarding")
        .expect("ws read error from proxy");
    match msg {
        Message::Text(t) => assert_eq!(t.len(), BIG_LEN),
        other => panic!("expected oversized text message, got {other:?}"),
    }
    let _ = ws.close(None).await;
}
