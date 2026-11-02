//! End-to-end test: spawn a private `iii` engine and the `console` worker,
//! then drive the worker through its HTTP port and the `/ws` proxy.
//!
//! Both processes run on free ports with a throwaway `HOME` and working
//! directory, so a developer's own engine (and `~/.iii`) is never touched.
//!
//! Self-skips, printing why, when the `iii` binary is not on `PATH` or the
//! stack fails to come up.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

/// Kills and reaps the child on drop, so every early return or panic
/// cleans up instead of orphaning the process.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Harness {
    // Fields drop in order: the worker, then its engine, then the tempdir.
    _worker: Reaped,
    _iii: Reaped,
    _dir: tempfile::TempDir,
    engine_ws: String,
    http_port: u16,
    rebound_http_port: u16,
    config_id: String,
}

/// Distinct free ports: every listener stays bound until all are picked.
fn free_ports<const N: usize>() -> [u16; N] {
    [(); N]
        .map(|_| std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port"))
        .map(|listener| listener.local_addr().expect("local addr").port())
}

/// Spawn `command` with a private `HOME` and working directory.
fn spawn_isolated(mut command: Command, dir: &Path) -> Option<Reaped> {
    command
        .current_dir(dir)
        .env("HOME", dir.join("home"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(Reaped)
        .inspect_err(|error| eprintln!("skipping: spawn failed: {error}"))
        .ok()
}

async fn boot() -> Option<Harness> {
    let iii_bin = which::which("iii").ok()?;
    let [engine_port, http_port, rebound_http_port] = free_ports();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("home")).expect("private HOME");
    let engine_config = dir.path().join("engine.yaml");
    std::fs::write(
        &engine_config,
        format!(
            r#"workers:
  - name: iii-worker-manager
    config:
      port: {engine_port}
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: "{}"
      ttl_seconds: 0
"#,
            dir.path().join("configuration").display()
        ),
    )
    .expect("write engine config");

    let mut command = Command::new(&iii_bin);
    command
        .args(["--no-update-check", "--config"])
        .arg(&engine_config);
    let iii = spawn_isolated(command, dir.path())?;
    // The configuration functions register before the engine starts listening.
    if !wait_for_listen(engine_port, Duration::from_secs(30)).await {
        eprintln!("skipping: the iii engine never listened on {engine_port}");
        return None;
    }

    let engine_ws = format!("ws://127.0.0.1:{engine_port}");
    let config_path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml");
    let config_id = format!("console-integration-{}", Uuid::new_v4());
    let mut command = Command::new(env!("CARGO_BIN_EXE_ade"));
    command
        .args(["--url", &engine_ws, "--config", config_path])
        .args([
            "--http-host",
            "127.0.0.1",
            "--http-port",
            &http_port.to_string(),
        ])
        .env("III_CONFIG_NAME", &config_id)
        .env("III_COMPOSE_DIR", dir.path());
    let worker = spawn_isolated(command, dir.path())?;
    if !wait_for_listen(http_port, Duration::from_secs(8)).await {
        eprintln!("skipping: the console never listened on {http_port}");
        return None;
    }

    Some(Harness {
        _worker: worker,
        _iii: iii,
        _dir: dir,
        engine_ws,
        http_port,
        rebound_http_port,
        config_id,
    })
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

/// Runs the end-to-end body in a child with a private `HOME`: its SDK client
/// would otherwise write an `iii ps` marker under the real `~/.iii/run`.
#[test]
fn end_to_end_http_ws_proxy_and_configuration_rebind() {
    let home = tempfile::tempdir().expect("tempdir");
    let output = Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "end_to_end_child", "--ignored", "--nocapture"])
        .env("HOME", home.path())
        .env("ADE_E2E_CHILD", "1")
        .output()
        .expect("run the end-to-end child");
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
#[ignore = "isolated subprocess invoked by end_to_end_http_ws_proxy_and_configuration_rebind"]
async fn end_to_end_child() {
    // `--ignored` runs would otherwise use the real HOME; only the parent sets this.
    if std::env::var_os("ADE_E2E_CHILD").is_none() {
        eprintln!("skipping: runs only as the isolated child of end_to_end_http_ws_proxy_and_configuration_rebind");
        return;
    }
    let Some(harness) = boot().await else {
        eprintln!("skipping: prerequisites not met (see logs above)");
        return;
    };
    let (http_port, rebound_http_port) = (harness.http_port, harness.rebound_http_port);

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
    let client = register_worker(&harness.engine_ws, InitOptions::default());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let current = loop {
        match client
            .trigger(
                TriggerRequest {
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
                    std::time::Instant::now() < deadline,
                    "configuration::get never became callable: {error}"
                );
                sleep(Duration::from_millis(100)).await;
            }
        }
    };
    let mut value = current["value"].clone();
    value["http_port"] = serde_json::json!(rebound_http_port);
    client
        .trigger(
            TriggerRequest {
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
    let rebound = reqwest::get(format!("http://127.0.0.1:{rebound_http_port}/"))
        .await
        .expect("GET / on rebound port failed");
    assert!(rebound.status().is_success());

    let status = client
        .trigger(TriggerRequest {
            function_id: "console::status".to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("console::status failed after rebind");
    assert_eq!(status["http_port"], rebound_http_port);
    assert_eq!(
        status["url"],
        format!("http://127.0.0.1:{rebound_http_port}")
    );

    let old_deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if TcpStream::connect(("127.0.0.1", http_port)).await.is_err() {
            break;
        }
        assert!(
            std::time::Instant::now() < old_deadline,
            "original Console port kept accepting connections after rebind"
        );
        sleep(Duration::from_millis(100)).await;
    }

    client.shutdown_async().await;
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
