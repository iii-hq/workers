//! End-to-end: spawn a private `iii` engine, a `state` worker and the worker
//! binary, then drive the lifecycle (create -> claim -> status -> release ->
//! remove) through `iii-sdk` as a client.
//!
//! The engine listens on a free port and every process runs with a throwaway
//! `HOME` and working directory, so a developer's own engine (and `~/.iii`) is
//! never touched. Self-skips when `iii` is not on PATH or no `state` worker
//! binary is found (`III_STATE_BIN`, or a build under `../state/target`).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

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
    // Fields drop in order: the workers, then their engine, then the tempdir.
    _worker: Reaped,
    _state: Reaped,
    _iii: Reaped,
    _dir: tempfile::TempDir,
    engine_ws: String,
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

/// Spawn `command` with a private `HOME` and working directory.
fn spawn_isolated(mut command: Command, dir: &Path, what: &str) -> Reaped {
    command
        .current_dir(dir)
        .env("HOME", dir.join("home"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(Reaped)
        .unwrap_or_else(|error| panic!("spawn {what}: {error}"))
}

/// The engine has no built-in `state`, and the lifecycle persists through it.
fn state_bin() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("III_STATE_BIN") {
        return Some(path.into());
    }
    let target = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../state/target"));
    ["debug", "release"]
        .map(|profile| target.join(profile).join("state"))
        .into_iter()
        .find(|path| path.is_file())
}

async fn wait_for_listen(port: u16, max: Duration) -> bool {
    let deadline = std::time::Instant::now() + max;
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    false
}

async fn boot() -> Option<Harness> {
    // The only reasons to self-skip: no `iii` engine on PATH, or no `state`
    // worker to run next to it. Every failure past this point is a real
    // test-environment fault and must fail loudly.
    let iii_bin = which::which("iii").ok()?;
    let Some(state_bin) = state_bin() else {
        eprintln!("skipping: no state worker (set III_STATE_BIN or build ../state)");
        return None;
    };
    let engine_port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("free port")
        .port();

    // Isolate the engine's data dir (configuration store, state) and every
    // HOME in a tempdir so the seed applies and nothing leaks between runs.
    let dir = tempfile::tempdir().expect("create tempdir");
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
    let state_config = dir.path().join("state.yaml");
    std::fs::write(
        &state_config,
        "adapter:\n  name: kv\n  config:\n    store_method: in_memory\n",
    )
    .expect("write state config");
    // Seed the config so managed worktrees land inside the tempdir.
    let seed_path = dir.path().join("seed.yaml");
    std::fs::write(
        &seed_path,
        format!("worktree_root: {}\n", dir.path().join("wts").display()),
    )
    .expect("write seed config");

    let mut command = Command::new(&iii_bin);
    command
        .args(["--no-update-check", "--config"])
        .arg(&engine_config);
    let iii = spawn_isolated(command, dir.path(), "iii engine");
    // The configuration functions register before the engine starts listening.
    assert!(
        wait_for_listen(engine_port, Duration::from_secs(30)).await,
        "the iii engine never listened on {engine_port}"
    );

    let engine_ws = format!("ws://127.0.0.1:{engine_port}");
    let mut command = Command::new(state_bin);
    command
        .args(["--url", &engine_ws, "--config"])
        .arg(&state_config);
    let state = spawn_isolated(command, dir.path(), "state worker");

    let mut command = Command::new(env!("CARGO_BIN_EXE_worktree"));
    command
        .args(["--url", &engine_ws, "--config"])
        .arg(&seed_path);
    let worker = spawn_isolated(command, dir.path(), "worktree worker");

    Some(Harness {
        _worker: worker,
        _state: state,
        _iii: iii,
        _dir: dir,
        engine_ws,
    })
}

async fn call(client: &iii_sdk::IIIClient, function_id: &str, payload: Value) -> Value {
    timeout(
        Duration::from_secs(15),
        client.trigger(TriggerRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(10_000),
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("{function_id} timed out"))
    .unwrap_or_else(|e| panic!("{function_id} failed: {e}"))
}

/// Worker boot is not instant (connect retry backoff plus the configuration
/// register/fetch round-trips), so wait for the surface to register instead
/// of racing it with a fixed sleep. `state::set` too: the lifecycle persists
/// through the state worker.
async fn wait_until_registered(client: &iii_sdk::IIIClient) {
    for _ in 0..40 {
        let listed = client
            .trigger(TriggerRequest {
                function_id: "engine::functions::list".into(),
                payload: Value::Object(Default::default()),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await;
        if let Ok(v) = listed.map(|v| v.to_string()) {
            if v.contains("worktree::create") && v.contains("state::set") {
                return;
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    panic!("worktree::create or state::set never registered with the engine");
}

/// Runs the end-to-end body in a child with a private `HOME`: its SDK client
/// would otherwise write an `iii ps` marker under the real `~/.iii/run`.
#[test]
fn end_to_end_lifecycle_via_iii_sdk() {
    let home = tempfile::tempdir().expect("tempdir");
    let output = Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "end_to_end_child", "--ignored", "--nocapture"])
        .env("HOME", home.path())
        .env("WORKTREE_E2E_CHILD", "1")
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
#[ignore = "isolated subprocess invoked by end_to_end_lifecycle_via_iii_sdk"]
async fn end_to_end_child() {
    // `--ignored` runs would otherwise use the real HOME; only the parent sets this.
    if std::env::var_os("WORKTREE_E2E_CHILD").is_none() {
        eprintln!("skipping: runs only as the isolated child of end_to_end_lifecycle_via_iii_sdk");
        return;
    }
    let Some(h) = boot().await else {
        eprintln!("skipping: prerequisites not met (see logs above)");
        return;
    };

    let repo_dir = tempfile::tempdir().unwrap();
    let repo = repo_dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "initial"]);

    let client = register_worker(&h.engine_ws, InitOptions::default());
    wait_until_registered(&client).await;

    let created = call(
        &client,
        "worktree::create",
        json!({ "repo_path": repo.to_string_lossy() }),
    )
    .await;
    let worktree_id = created["worktree_id"]
        .as_str()
        .expect("worktree_id")
        .to_string();
    let path = created["path"].as_str().expect("path").to_string();
    assert!(Path::new(&path).is_dir());

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

    client.shutdown_async().await;
}
