//! End-to-end: an iii engine and the worker binary, driven through `iii-sdk`
//! as a client: the lifecycle (create -> claim -> status -> release ->
//! remove), and an observer of `worktree::on-created` filtered on
//! `input.repo_path`.
//!
//! The engine is, in order:
//! - `III_ENGINE_WS_URL`: an engine that is already running;
//! - `III_ENGINE_BIN`, or an `iii` 0.26+ on PATH: spawned here with an
//!   engine-only config (worker manager and configuration store) on
//!   `III_TEST_ENGINE_PORT` or an ephemeral port.
//!
//! Without one the test skips; `III_E2E_REQUIRE=1` turns the skip into a
//! failure. The engine hosts no `state::*` functions, so the test registers
//! in-memory ones when the engine has none.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iii_sdk::{
    register_worker, CallRequest, Deliver, IIIClient, InitOptions, RegisterFunction, Source,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

/// The engine the test talks to; a spawned one is stopped on drop.
struct Engine {
    url: String,
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

fn require() -> bool {
    std::env::var("III_E2E_REQUIRE").as_deref() == Ok("1")
}

/// `iii` on PATH, when it reports 0.26 or later: an older engine does not
/// speak the observe-v2 protocol this worker's SDK uses.
fn engine_on_path() -> Option<PathBuf> {
    let bin = which::which("iii").ok()?;
    let out = Command::new(&bin).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let version = text.split_whitespace().last()?;
    let mut parts = version.split('.').map(|p| p.parse::<u64>().ok());
    let (major, minor) = (parts.next()??, parts.next()??);
    (major > 0 || minor >= 26).then_some(bin)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("bind an ephemeral port")
}

async fn engine() -> Option<Engine> {
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

struct Worker {
    child: Child,
    log: PathBuf,
}

impl Worker {
    fn log_tail(&self) -> String {
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

fn spawn_worker(url: &str, dir: &Path) -> Worker {
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

fn git(dir: &Path, args: &[&str]) {
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

fn request(function_id: &str, payload: Value, timeout_ms: u64) -> CallRequest {
    CallRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(timeout_ms),
    }
}

async fn call(client: &IIIClient, function_id: &str, payload: Value) -> Value {
    timeout(
        Duration::from_secs(15),
        client.call(request(function_id, payload, 10_000)),
    )
    .await
    .unwrap_or_else(|_| panic!("{function_id} timed out"))
    .unwrap_or_else(|e| panic!("{function_id} failed: {e}"))
}

async fn registered(client: &IIIClient, function_id: &str) -> bool {
    client
        .call(request("engine::functions::list", json!({}), 5_000))
        .await
        .map(|listed| listed.to_string().contains(&format!("\"{function_id}\"")))
        .unwrap_or(false)
}

/// In-memory `state::get/set/delete/list`, for an engine without a state
/// worker.
fn register_state(client: &IIIClient) {
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

/// A function that forwards every payload it is called with to a channel.
fn recorder(client: &IIIClient, function_id: &str) -> mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = mpsc::unbounded_channel();
    client.register_function(
        function_id,
        RegisterFunction::new(move |payload: Value| {
            let _ = tx.send(payload);
            Ok::<_, iii_sdk::Error>(json!({ "ok": true }))
        }),
    );
    rx
}

#[tokio::test]
async fn end_to_end_lifecycle_via_iii_sdk() {
    let Some(engine) = engine().await else {
        assert!(
            !require(),
            "III_E2E_REQUIRE=1 but no engine: set III_ENGINE_WS_URL or III_ENGINE_BIN"
        );
        eprintln!("skipping: no iii engine (set III_ENGINE_WS_URL or III_ENGINE_BIN)");
        return;
    };

    let client = register_worker(&engine.url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    while client
        .call(request("engine::workers::list", json!({}), 2_000))
        .await
        .is_err()
    {
        assert!(
            Instant::now() < deadline,
            "the engine at {} never answered",
            engine.url
        );
        sleep(Duration::from_millis(250)).await;
    }
    if !registered(&client, "state::get").await {
        register_state(&client);
    }

    let worker_dir = tempfile::tempdir().unwrap();
    let worker = spawn_worker(&engine.url, worker_dir.path());
    let deadline = Instant::now() + Duration::from_secs(60);
    while !registered(&client, "worktree::create").await {
        assert!(
            Instant::now() < deadline,
            "worktree::create never registered with the engine; worker log:\n{}",
            worker.log_tail()
        );
        sleep(Duration::from_millis(250)).await;
    }

    let repo_dir = tempfile::tempdir().unwrap();
    let repo = repo_dir.path().canonicalize().unwrap().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "initial"]);
    let repo_path = repo.to_string_lossy().to_string();

    // One observer of this repository's creations, and one of another
    // repository's, which must receive nothing.
    let mut created_events = recorder(&client, "e2e::worktree-created");
    let mut other_events = recorder(&client, "e2e::other-repo-created");
    let this_repo = client.observe(
        Source::function("worktree::on-created")
            .filters(json!({ "input.repo_path": { "$eq": repo_path } }))
            .deliver(Deliver::Output),
        "e2e::worktree-created",
    );
    let other_repo = client.observe(
        Source::function("worktree::on-created")
            .filters(json!({ "input.repo_path": { "$eq": "/no/such/repo" } }))
            .deliver(Deliver::Output),
        "e2e::other-repo-created",
    );
    for observer in [&this_repo, &other_repo] {
        timeout(Duration::from_secs(15), observer.ready())
            .await
            .expect("the observer of worktree::on-created never became active")
            .expect("the observer of worktree::on-created was rejected");
    }

    let created = call(
        &client,
        "worktree::create",
        json!({ "repo_path": repo_path }),
    )
    .await;
    let worktree_id = created["worktree_id"]
        .as_str()
        .expect("worktree_id")
        .to_string();
    let path = created["path"].as_str().expect("path").to_string();
    assert!(Path::new(&path).is_dir());

    let event = timeout(Duration::from_secs(10), created_events.recv())
        .await
        .expect("worktree::on-created never reached its observer")
        .expect("recorder channel closed");
    assert_eq!(event["worktree_id"], worktree_id.as_str());
    assert_eq!(event["repo_path"], repo_path.as_str());
    assert_eq!(event["path"], path.as_str());

    let claimed = call(
        &client,
        "worktree::claim",
        json!({ "worktree_id": worktree_id, "session_id": "e2e" }),
    )
    .await;
    assert_eq!(claimed["claimed"], true);

    let status = call(
        &client,
        "worktree::status",
        json!({ "worktree_id": worktree_id }),
    )
    .await;
    assert_eq!(status["clean"], true);
    assert_eq!(status["lifecycle"], "claimed");

    let listed = call(&client, "worktree::list", json!({ "session_id": "e2e" })).await;
    assert_eq!(listed["worktrees"].as_array().unwrap().len(), 1);

    let released = call(
        &client,
        "worktree::release",
        json!({ "worktree_id": worktree_id, "session_id": "e2e" }),
    )
    .await;
    assert_eq!(released["released"], true);

    let removed = call(
        &client,
        "worktree::remove",
        json!({ "worktree_id": worktree_id }),
    )
    .await;
    assert_eq!(removed["removed"], true);
    assert!(!Path::new(&path).exists());

    assert!(
        other_events.try_recv().is_err(),
        "an observer filtered on another repository received this one's creation"
    );
    // Exactly one creation reached the matching observer.
    assert!(created_events.try_recv().is_err());

    for observer in [this_repo, other_repo] {
        observer.dispose().await.expect("dispose observer");
    }
    drop(worker);
    client.shutdown_async().await;
}
