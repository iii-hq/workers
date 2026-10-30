//! The bus harness the end-to-end tests share: an iii engine (an existing
//! one or a spawned one), the worker binary, in-memory `state::*` functions
//! for an engine without a state worker, and call helpers.
//!
//! The engine is, in order:
//! - `III_ENGINE_WS_URL`: an engine that is already running;
//! - `III_ENGINE_BIN`, or an `iii` 0.26+ on PATH: spawned here with an
//!   engine-only config (worker manager and configuration store) on
//!   `III_TEST_ENGINE_PORT` or an ephemeral port.
//!
//! Without one a test skips; `III_E2E_REQUIRE=1` turns the skip into a
//! failure.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iii_sdk::{CallRequest, IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

/// The engine the test talks to; a spawned one is stopped on drop.
pub struct Engine {
    pub url: String,
    child: Option<Child>,
    _dir: Option<tempfile::TempDir>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn require() -> bool {
    std::env::var("III_E2E_REQUIRE").as_deref() == Ok("1")
}

/// `iii` on PATH, when it reports 0.26 or later: an older engine does not
/// speak the observe-v2 protocol this worker's SDK uses.
pub fn engine_on_path() -> Option<PathBuf> {
    let bin = which::which("iii").ok()?;
    let out = Command::new(&bin).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let version = text.split_whitespace().last()?;
    let mut parts = version.split('.').map(|p| p.parse::<u64>().ok());
    let (major, minor) = (parts.next()??, parts.next()??);
    (major > 0 || minor >= 26).then_some(bin)
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("bind an ephemeral port")
}

pub async fn engine() -> Option<Engine> {
    if let Ok(url) = std::env::var("III_ENGINE_WS_URL") {
        return Some(Engine {
            url,
            child: None,
            _dir: None,
        });
    }
    let bin = std::env::var_os("III_ENGINE_BIN")
        .map(PathBuf::from)
        .or_else(engine_on_path)?;
    let port = std::env::var("III_TEST_ENGINE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(free_port);
    let dir = tempfile::tempdir().expect("create engine tempdir");
    let config = dir.path().join("config.yaml");
    std::fs::write(
        &config,
        format!(
            "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: {}\n",
            dir.path().join("config").display()
        ),
    )
    .expect("write engine config");
    let child = Command::new(&bin)
        .arg("--config")
        .arg(&config)
        .arg("--no-update-check")
        .env("III_TELEMETRY_ENABLED", "false")
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn iii engine");
    let engine = Engine {
        url: format!("ws://127.0.0.1:{port}"),
        child: Some(child),
        _dir: Some(dir),
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "the engine at {bin:?} never listened on {port}"
        );
        sleep(Duration::from_millis(100)).await;
    }
    Some(engine)
}

pub struct Worker {
    child: Child,
    log: PathBuf,
}

impl Worker {
    pub fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(40)..].join("\n")
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn spawn_worker(url: &str, dir: &Path) -> Worker {
    // Seed the config so managed worktrees land inside the tempdir; a config
    // id of its own keeps a shared engine's `worktree` entry out of the run.
    let seed = dir.join("seed.yaml");
    std::fs::write(
        &seed,
        format!("worktree_root: {}\n", dir.join("wts").display()),
    )
    .expect("write seed config");
    let log = dir.join("worker.log");
    let out = std::fs::File::create(&log).expect("create worker log");
    let err = out.try_clone().expect("clone worker log");
    let config_name = format!("worktree-e2e-{}-{}", std::process::id(), free_port());
    let child = Command::new(env!("CARGO_BIN_EXE_worktree"))
        .arg("--url")
        .arg(url)
        .arg("--config")
        .arg(&seed)
        .env("III_CONFIG_NAME", config_name)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .expect("spawn worktree worker");
    Worker { child, log }
}

pub fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

pub fn request(function_id: &str, payload: Value, timeout_ms: u64) -> CallRequest {
    CallRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(timeout_ms),
    }
}

pub async fn call(client: &IIIClient, function_id: &str, payload: Value) -> Value {
    timeout(
        Duration::from_secs(15),
        client.call(request(function_id, payload, 10_000)),
    )
    .await
    .unwrap_or_else(|_| panic!("{function_id} timed out"))
    .unwrap_or_else(|e| panic!("{function_id} failed: {e}"))
}

pub async fn registered(client: &IIIClient, function_id: &str) -> bool {
    client
        .call(request("engine::functions::list", json!({}), 5_000))
        .await
        .map(|listed| listed.to_string().contains(&format!("\"{function_id}\"")))
        .unwrap_or(false)
}

/// In-memory `state::get/set/delete/list`, for an engine without a state
/// worker.
pub fn register_state(client: &IIIClient) {
    type Scopes = Arc<Mutex<HashMap<String, HashMap<String, Value>>>>;
    let scopes: Scopes = Arc::default();
    let text = |v: &Value, key: &str| v[key].as_str().unwrap_or_default().to_string();

    let store = scopes.clone();
    client.register_function(
        "state::get",
        RegisterFunction::new(move |req: Value| {
            let scopes = store.lock().unwrap();
            let value = scopes
                .get(&text(&req, "scope"))
                .and_then(|scope| scope.get(&text(&req, "key")))
                .cloned();
            Ok::<_, iii_sdk::Error>(value.unwrap_or(Value::Null))
        }),
    );
    let store = scopes.clone();
    client.register_function(
        "state::set",
        RegisterFunction::new(move |req: Value| {
            store
                .lock()
                .unwrap()
                .entry(text(&req, "scope"))
                .or_default()
                .insert(text(&req, "key"), req["value"].clone());
            Ok::<_, iii_sdk::Error>(json!({ "ok": true }))
        }),
    );
    let store = scopes.clone();
    client.register_function(
        "state::delete",
        RegisterFunction::new(move |req: Value| {
            if let Some(scope) = store.lock().unwrap().get_mut(&text(&req, "scope")) {
                scope.remove(&text(&req, "key"));
            }
            Ok::<_, iii_sdk::Error>(json!({ "ok": true }))
        }),
    );
    let store = scopes;
    client.register_function(
        "state::list",
        RegisterFunction::new(move |req: Value| {
            let values: Vec<Value> = store
                .lock()
                .unwrap()
                .get(&text(&req, "scope"))
                .map(|scope| scope.values().cloned().collect())
                .unwrap_or_default();
            Ok::<_, iii_sdk::Error>(Value::Array(values))
        }),
    );
}

/// The http worker binary, `III_HTTP_BIN` (build it with
/// `cargo build --manifest-path ../http/Cargo.toml`; it is `http` in that
/// target directory).
pub fn http_bin() -> Option<PathBuf> {
    std::env::var_os("III_HTTP_BIN").map(PathBuf::from)
}

/// The http worker listening on 127.0.0.1:`port`, under a configuration id
/// of its own (`III_CONFIG_NAME`), so a seed with this port is what it
/// loads.
pub fn spawn_http(bin: &Path, url: &str, dir: &Path, port: u16) -> Worker {
    let seed = dir.join("http.yaml");
    std::fs::write(&seed, format!("port: {port}\nhost: 127.0.0.1\n")).expect("write http seed");
    let log = dir.join("http.log");
    let out = std::fs::File::create(&log).expect("create http log");
    let err = out.try_clone().expect("clone http log");
    let child = Command::new(bin)
        .arg("--url")
        .arg(url)
        .arg("--config")
        .arg(&seed)
        .env(
            "III_CONFIG_NAME",
            format!("http-e2e-{}-{port}", std::process::id()),
        )
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .expect("spawn http worker");
    Worker { child, log }
}

/// `POST http://127.0.0.1:{port}{path}` with a JSON `body`: the response's
/// status and its body as JSON (`null` when empty). `None` when nothing
/// answers.
pub async fn post(port: u16, path: &str, body: Value) -> Option<(u16, Value)> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || post_blocking(port, &path, &body.to_string()))
        .await
        .expect("the HTTP request task panicked")
}

fn post_blocking(port: u16, path: &str, body: &str) -> Option<(u16, Value)> {
    use std::io::{Read, Write};

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .ok()?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut content = raw[split + 4..].to_vec();
    let status: u16 = head.split_whitespace().nth(1)?.parse().ok()?;
    let chunked = head.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("transfer-encoding:") && line.contains("chunked")
    });
    if chunked {
        content = dechunk(&content)?;
    }
    let body = if content.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&content)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&content).to_string()))
    };
    Some((status, body))
}

/// A `Transfer-Encoding: chunked` body, joined.
fn dechunk(mut raw: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = raw.windows(2).position(|w| w == b"\r\n")?;
        let size_line = String::from_utf8_lossy(&raw[..eol]).to_string();
        let size = usize::from_str_radix(size_line.split(';').next()?.trim(), 16).ok()?;
        raw = &raw[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(raw.get(..size)?);
        raw = raw.get(size + 2..)?;
    }
}
