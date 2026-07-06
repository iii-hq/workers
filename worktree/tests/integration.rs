//! End-to-end: spawn the `iii` engine and the worker binary, then drive the
//! lifecycle (create -> claim -> status -> release -> remove) through
//! `iii-sdk` as a client. Self-skips when `iii` is not on PATH.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

const ENGINE_WS: &str = "ws://127.0.0.1:49134";

struct Harness {
    iii: Child,
    worker: Child,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        let _ = self.iii.kill();
        let _ = self.iii.wait();
    }
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

async fn boot() -> Option<Harness> {
    // The only reason to self-skip: no `iii` engine on PATH. Every failure
    // past this point is a real test-environment fault and must fail loudly.
    let iii_bin = which::which("iii").ok()?;

    // Isolate the engine's data dir (configuration store, state) in a
    // tempdir so the seed applies and nothing leaks between runs.
    let engine_dir = tempfile::tempdir().expect("create engine tempdir");
    let worker_dir = tempfile::tempdir().expect("create worker tempdir");

    let iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .current_dir(engine_dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn iii engine");

    sleep(Duration::from_millis(1200)).await;

    // Seed the config so managed worktrees land inside the tempdir.
    let seed_path = worker_dir.path().join("seed.yaml");
    std::fs::write(
        &seed_path,
        format!(
            "worktree_root: {}\n",
            worker_dir.path().join("wts").display()
        ),
    )
    .expect("write seed config");

    let worker_bin = env!("CARGO_BIN_EXE_worktree");
    let worker = Command::new(worker_bin)
        .arg("--url")
        .arg(ENGINE_WS)
        .arg("--config")
        .arg(&seed_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn worktree worker");

    sleep(Duration::from_millis(2000)).await;

    Some(Harness {
        iii,
        worker,
        _dirs: (engine_dir, worker_dir),
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
/// of racing it with a fixed sleep.
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
        if let Ok(v) = listed {
            if v.to_string().contains("worktree::create") {
                return;
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    panic!("worktree::create never registered with the engine");
}

#[tokio::test]
async fn end_to_end_lifecycle_via_iii_sdk() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` binary not on PATH");
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

    let client = register_worker(ENGINE_WS, InitOptions::default());
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
