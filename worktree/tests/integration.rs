//! End-to-end: an iii engine and the worker binary, driven through `iii-sdk`
//! as a client: the lifecycle (create -> claim -> status -> release ->
//! remove), and an observer of `worktree::on-created` filtered on
//! `input.repo_path`.
//!
//! The engine and the worker come from the bus harness (tests/bus): without
//! an engine the test skips; `III_E2E_REQUIRE=1` turns the skip into a
//! failure. The engine hosts no `state::*` functions, so the test registers
//! in-memory ones when the engine has none.

mod bus;

use std::path::Path;
use std::time::{Duration, Instant};

use bus::{call, engine, git, register_state, registered, request, require, spawn_worker};
use iii_sdk::{register_worker, Deliver, IIIClient, InitOptions, RegisterFunction, Source};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

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
