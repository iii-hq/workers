//! The REST routes through the http worker, end to end: an iii engine, the
//! http worker (`III_HTTP_BIN`) on `III_TEST_HTTP_PORT` or an ephemeral
//! port, and the worker binary, whose `http::on-request` observers route
//! `POST /worktree/{list,get,status,validate}` to `worktree::rest`. Each
//! route answers with its function's result as the response body, a `W###`
//! error with its status and `{ error: { code, message } }`, and a path with
//! no route with the http worker's 404.
//!
//! The engine and the worker come from the bus harness (tests/bus). Without
//! an engine or `III_HTTP_BIN` the test skips; `III_E2E_REQUIRE=1` turns the
//! skip into a failure. It also skips against an engine that already hosts
//! `http::on-request`, whose server it cannot address.

mod bus;

use std::path::Path;
use std::time::{Duration, Instant};

use bus::{
    call, engine, free_port, git, http_bin, post, register_state, registered, request, require,
    spawn_http, spawn_worker,
};
use iii_sdk::{register_worker, InitOptions};
use serde_json::{json, Value};
use tokio::time::sleep;

fn skip(reason: &str) {
    assert!(!require(), "III_E2E_REQUIRE=1 but {reason}");
    eprintln!("skipping: {reason}");
}

#[tokio::test]
async fn read_routes_answer_through_the_http_worker() {
    let Some(http) = http_bin() else {
        return skip("no http worker binary: set III_HTTP_BIN");
    };
    let Some(engine) = engine().await else {
        return skip("no engine: set III_ENGINE_WS_URL or III_ENGINE_BIN");
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
    if registered(&client, "http::on-request").await {
        client.shutdown_async().await;
        return skip("the engine already hosts http::on-request");
    }
    if !registered(&client, "state::get").await {
        register_state(&client);
    }

    let dir = tempfile::tempdir().unwrap();
    let port = std::env::var("III_TEST_HTTP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(free_port);
    let http = spawn_http(&http, &engine.url, dir.path(), port);
    let worker = spawn_worker(&engine.url, dir.path());

    // The routes are active once the http worker has added them: until then
    // the server is down, or answers 404 for a path it has no route for.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Some((200, body)) = post(port, "/worktree/list", json!({})).await {
            assert!(body["worktrees"].is_array(), "{body}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "POST /worktree/list never answered 200.\nworker log:\n{}\nhttp log:\n{}",
            worker.log_tail(),
            http.log_tail()
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

    let created = call(
        &client,
        "worktree::create",
        json!({ "repo_path": repo_path }),
    )
    .await;
    let worktree_id = created["worktree_id"].as_str().expect("worktree_id");
    let path = created["path"].as_str().expect("path");

    let (status, body) = post(port, "/worktree/list", json!({ "repo_path": repo_path }))
        .await
        .expect("POST /worktree/list");
    assert_eq!(status, 200, "{body}");
    let listed = body["worktrees"].as_array().expect("worktrees");
    assert_eq!(listed.len(), 1, "{body}");
    assert_eq!(listed[0]["worktree_id"], worktree_id);

    let (status, body) = post(port, "/worktree/get", json!({ "worktree_id": worktree_id }))
        .await
        .expect("POST /worktree/get");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["worktree_id"], worktree_id);
    assert_eq!(body["path"], path);
    assert_eq!(body["repo_path"], repo_path.as_str());

    let (status, body) = post(
        port,
        "/worktree/status",
        json!({ "worktree_id": worktree_id }),
    )
    .await
    .expect("POST /worktree/status");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["worktree_id"], worktree_id);
    assert_eq!(body["clean"], true);

    let (status, body) = post(port, "/worktree/validate", json!({ "path": path }))
        .await
        .expect("POST /worktree/validate");
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["valid"], true);
    assert_eq!(body["managed"], true);
    assert_eq!(body["worktree_id"], worktree_id);

    let (status, body) = post(port, "/worktree/get", json!({ "worktree_id": "wt_absent" }))
        .await
        .expect("POST /worktree/get");
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "W200");

    let (status, body) = post(port, "/worktree/status", json!({}))
        .await
        .expect("POST /worktree/status");
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "W001");

    let (status, body) = post(port, "/worktree/validate", json!({ "path": "relative" }))
        .await
        .expect("POST /worktree/validate");
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "W111");

    // Only the read surface has routes.
    let (status, _) = post(
        port,
        "/worktree/remove",
        json!({ "worktree_id": worktree_id }),
    )
    .await
    .expect("POST /worktree/remove");
    assert_eq!(status, 404);
    assert!(Path::new(path).is_dir(), "a route removed the worktree");

    let removed = call(
        &client,
        "worktree::remove",
        json!({ "worktree_id": worktree_id }),
    )
    .await;
    assert_eq!(removed["removed"], Value::Bool(true));

    drop(worker);
    drop(http);
    client.shutdown_async().await;
}
