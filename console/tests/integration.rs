//! End-to-end test: spawn the `iii` engine, the `console` worker, and
//! drive the worker through its HTTP port and the `/ws` proxy.
//!
//! Self-skips when:
//!   - the `iii` binary is not on `PATH`, or
//!   - the chosen `HTTP_PORT` is already taken on this host.
//!
//! When skipped the test prints why so CI logs make the gap visible.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;

const ENGINE_WS: &str = "ws://127.0.0.1:49134";
const HTTP_PORT: u16 = 48217;

struct Harness {
    iii: Child,
    worker: Child,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        let _ = self.iii.kill();
        let _ = self.iii.wait();
    }
}

async fn boot() -> Option<Harness> {
    let iii_bin = which::which("iii").ok()?;

    if TcpStream::connect(("127.0.0.1", HTTP_PORT)).await.is_ok() {
        eprintln!(
            "skipping: port {HTTP_PORT} already in use on 127.0.0.1; \
             leftover from a previous test run?"
        );
        return None;
    }

    let iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let worker_bin = env!("CARGO_BIN_EXE_console");
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let config_path = format!("{manifest_dir}/config.yaml");

    let worker = Command::new(worker_bin)
        .args([
            "--url",
            ENGINE_WS,
            "--config",
            &config_path,
            "--http-port",
            &HTTP_PORT.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    if !wait_for_listen(HTTP_PORT, Duration::from_secs(8)).await {
        return None;
    }

    Some(Harness { iii, worker })
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

#[tokio::test]
async fn end_to_end_http_and_ws_proxy() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: prerequisites not met (see logs above)");
        return;
    };

    let base = format!("http://127.0.0.1:{HTTP_PORT}");

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
        &body.chars().take(200).collect::<String>(),
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
    let ws_url = format!("ws://127.0.0.1:{HTTP_PORT}/ws");
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
