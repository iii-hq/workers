//! End-to-end test (binary-worker.md §9 pattern A): spawn the `iii` engine
//! and the `coder` worker as subprocesses, drive the worker through
//! `iii-sdk` as a client, then tear both down. **Self-skips** when `iii`
//! is not on `PATH` so CI hosts without the engine still pass.
//!
//! A single test runs both scenarios sequentially because the engine
//! binds a fixed loopback port and parallel test cases would race for it.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;
use tempfile::TempDir;
use tokio::time::{sleep, timeout};

const ENGINE_WS: &str = "ws://127.0.0.1:49134";

struct Harness {
    iii: Child,
    worker: Child,
    /// Kept alive so the temp base persists for the worker's lifetime.
    base: TempDir,
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

    let mut iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let base = tempfile::tempdir().ok()?;
    let cfg_path = base.path().join("coder-config.yaml");
    let yaml = format!(
        "base_path: {}\nnon_accessible_globs:\n  - \"**/.env\"\n",
        base.path().display()
    );
    std::fs::write(&cfg_path, yaml).ok()?;

    let worker_bin = env!("CARGO_BIN_EXE_coder");
    let worker = match Command::new(worker_bin)
        .args([
            "--url",
            ENGINE_WS,
            "--config",
            cfg_path.to_str().expect("utf-8 cfg path"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(w) => w,
        Err(_) => {
            let _ = iii.kill();
            let _ = iii.wait();
            return None;
        }
    };

    Some(Harness { iii, worker, base })
}

/// Poll `function_id` with `payload` until the worker has registered, or
/// give up. Returns the response (or panics with the last error).
async fn call_when_ready(
    client: &iii_sdk::III,
    function_id: &str,
    payload: serde_json::Value,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut last_err: Option<String> = None;
    while std::time::Instant::now() < deadline {
        match timeout(
            Duration::from_secs(5),
            client.trigger(TriggerRequest {
                function_id: function_id.into(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(4_000),
            }),
        )
        .await
        {
            Ok(Ok(v)) => return v,
            Ok(Err(e)) => {
                let msg = format!("{e:?}");
                if !msg.contains("function_not_found") && !msg.contains("not found") {
                    panic!("{function_id} failed: {msg}");
                }
                last_err = Some(msg);
            }
            Err(_) => last_err = Some("trigger timed out".into()),
        }
        sleep(Duration::from_millis(500)).await;
    }
    panic!(
        "worker never registered {function_id} within 30s; last error: {}",
        last_err.unwrap_or_else(|| "<none>".into())
    );
}

#[tokio::test]
async fn full_lifecycle_via_iii_sdk() {
    let Some(h) = boot().await else {
        eprintln!("skipping: `iii` not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    // 1. Create a file via the worker.
    let create = call_when_ready(
        &client,
        "coder::create-file",
        json!({
            "files": [{
                "path": "hello.txt",
                "content": "hello world\nsecond line\n",
                "mode": "0644",
                "parents": true,
                "overwrite": false
            }]
        }),
    )
    .await;
    let create_results = create["results"].as_array().expect("create results array");
    assert_eq!(create_results.len(), 1);
    assert_eq!(
        create_results[0]["success"], true,
        "create failed: {:?}",
        create_results[0]["error"]
    );

    // 2. Read it back.
    let read = client
        .trigger(TriggerRequest {
            function_id: "coder::read-file".into(),
            payload: json!({ "path": "hello.txt" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("read-file");
    assert_eq!(read["content"], "hello world\nsecond line\n");
    assert_eq!(read["size"], 24);

    // 3. Update — replace line 2.
    let updated = client
        .trigger(TriggerRequest {
            function_id: "coder::update-file".into(),
            payload: json!({
                "files": [{
                    "path": "hello.txt",
                    "ops": [
                        { "op": "update_lines", "from_line": 2, "to_line": 2, "content": "REPLACED" }
                    ]
                }]
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("update-file");
    assert_eq!(updated["results"][0]["success"], true);
    let after = client
        .trigger(TriggerRequest {
            function_id: "coder::read-file".into(),
            payload: json!({ "path": "hello.txt" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("read after update");
    assert_eq!(after["content"], "hello world\nREPLACED\n");

    // 4. list-folder shows the file.
    let list = client
        .trigger(TriggerRequest {
            function_id: "coder::list-folder".into(),
            payload: json!({ "path": ".", "page": 1 }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("list-folder");
    let entries = list["entries"].as_array().expect("entries array");
    assert!(entries.iter().any(|e| e["name"] == "hello.txt"));

    // 5. tree returns the file as a child of root.
    let tree = client
        .trigger(TriggerRequest {
            function_id: "coder::tree".into(),
            payload: json!({ "path": "." }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("tree");
    let root = &tree["root"];
    let kids = root["children"].as_array().expect("root children");
    assert!(kids.iter().any(|k| k["name"] == "hello.txt"));

    // 6. search finds the content.
    let search = client
        .trigger(TriggerRequest {
            function_id: "coder::search".into(),
            payload: json!({ "query": "REPLACED", "regex": false }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("search");
    let content_matches = search["content_matches"]
        .as_array()
        .expect("content matches");
    // Response paths are canonical-absolute (decision D2-eng); the worker
    // canonicalized its root, so anchor the expectation the same way.
    let expected_match_path = std::fs::canonicalize(h.base.path())
        .expect("canonicalize harness base")
        .join("hello.txt")
        .display()
        .to_string();
    assert!(content_matches
        .iter()
        .any(|m| m["path"] == expected_match_path.as_str() && m["line"] == 2));

    // 7. Non-accessible glob blocks reads even though list-folder shows it.
    std::fs::write(h.base.path().join(".env"), "API_KEY=secret\n").unwrap();
    let list_with_env = client
        .trigger(TriggerRequest {
            function_id: "coder::list-folder".into(),
            payload: json!({ "path": ".", "page": 1 }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("list-folder after .env");
    let env_entry = list_with_env["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == ".env")
        .expect(".env should be listed");
    assert_eq!(env_entry["non_accessible"], true);

    let read_env = client
        .trigger(TriggerRequest {
            function_id: "coder::read-file".into(),
            payload: json!({ "path": ".env" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    let err_msg = match read_env {
        Ok(_) => panic!("read of .env must be rejected"),
        Err(e) => format!("{e:?}"),
    };
    assert!(err_msg.contains("C211"), "got: {err_msg}");

    // 8. delete-file removes the file.
    let del = client
        .trigger(TriggerRequest {
            function_id: "coder::delete-file".into(),
            payload: json!({ "paths": ["hello.txt"] }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("delete-file");
    assert_eq!(del["results"][0]["success"], true);
    assert_eq!(del["results"][0]["removed"], true);

    client.shutdown_async().await;
}
