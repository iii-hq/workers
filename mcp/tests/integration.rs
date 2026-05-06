//! End-to-end test: spawn `iii` engine + the worker binary, drive the bridge
//! via HTTP. Self-skips when `iii` is not on PATH.
//!
//! Boots:
//!   1. `iii --use-default-config` (engine on default ws://127.0.0.1:49134, http://127.0.0.1:3000)
//!   2. `mcp --url ws://127.0.0.1:49134 --config ./config.yaml`
//!
//! Posts JSON-RPC `initialize` and `tools/list` to `http://127.0.0.1:3000/mcp`,
//! parses the SSE frame, asserts:
//!   - `protocolVersion == "2025-06-18"`
//!   - `tools/list.result.tools` is an array
//!
//! Refuses to fail when the engine is unavailable -- prints a skip notice and
//! returns 0, mirroring binary-worker.md Pattern A.

use std::net::SocketAddr;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::time::sleep;

const ENGINE_WS: &str = "ws://127.0.0.1:49134";
const ENGINE_HTTP: &str = "http://127.0.0.1:3000/mcp";
const HTTP_PROBE_ADDR: &str = "127.0.0.1:3000";
const WS_PROBE_ADDR: &str = "127.0.0.1:49134";

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

async fn tcp_listening(addr: &str) -> bool {
    let target: SocketAddr = match addr.parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    TcpStream::connect(target).await.is_ok()
}

async fn wait_for_tcp(addr: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if tcp_listening(addr).await {
            return true;
        }
        sleep(Duration::from_millis(200)).await;
    }
    false
}

async fn boot() -> Option<Harness> {
    let iii_bin = which::which("iii").ok()?;

    // If port 49134 is already in use we can't safely spawn a fresh engine;
    // skip rather than collide with an existing developer setup.
    if tcp_listening(WS_PROBE_ADDR).await {
        return None;
    }

    let iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    if !wait_for_tcp(WS_PROBE_ADDR, Duration::from_secs(10)).await {
        return None;
    }
    if !wait_for_tcp(HTTP_PROBE_ADDR, Duration::from_secs(10)).await {
        return None;
    }

    let worker_bin = env!("CARGO_BIN_EXE_mcp");
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let config_path = format!("{manifest_dir}/config.yaml");

    let worker = Command::new(worker_bin)
        .args(["--url", ENGINE_WS, "--config", &config_path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Give the worker a moment to register the http trigger with the engine.
    sleep(Duration::from_millis(2000)).await;

    Some(Harness { iii, worker })
}

/// Strip the SSE envelope and return the JSON-RPC payload as a `Value`.
fn parse_sse(body: &str) -> Value {
    let line = body
        .lines()
        .find_map(|l| l.strip_prefix("data: "))
        .expect("expected `data: ` line in SSE response");
    serde_json::from_str(line).expect("data line must be valid JSON-RPC")
}

async fn post_jsonrpc(client: &reqwest::Client, body: Value) -> reqwest::Response {
    client
        .post(ENGINE_HTTP)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .body(body.to_string())
        .send()
        .await
        .expect("POST /mcp")
}

#[tokio::test]
async fn end_to_end_initialize_and_tools_list() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` engine not reachable on default ports");
        return;
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("build http client");

    let initialize = post_jsonrpc(
        &client,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    )
    .await;
    assert!(initialize.status().is_success(), "initialize HTTP status");
    let ct = initialize
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or_default().to_string())
        .unwrap_or_default();
    assert!(
        ct.starts_with("text/event-stream"),
        "expected SSE content-type, got {ct}"
    );
    let body = initialize.text().await.expect("initialize body");
    let rpc = parse_sse(&body);
    assert_eq!(rpc["jsonrpc"], "2.0");
    assert_eq!(rpc["id"], 1);
    assert_eq!(rpc["result"]["protocolVersion"], "2025-06-18");
    assert!(rpc["result"]["capabilities"]["tools"].is_object());

    let tools_list = post_jsonrpc(
        &client,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
    assert!(tools_list.status().is_success());
    let body = tools_list.text().await.expect("tools/list body");
    let rpc = parse_sse(&body);
    assert_eq!(rpc["jsonrpc"], "2.0");
    assert_eq!(rpc["id"], 2);
    assert!(
        rpc["result"]["tools"].is_array(),
        "expected tools to be an array, got {rpc:#}"
    );
}

#[tokio::test]
async fn notifications_return_202() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` engine not reachable on default ports");
        return;
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("build http client");

    let resp = post_jsonrpc(
        &client,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 202, "notification must return 202");
}
