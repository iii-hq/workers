//! End-to-end test (binary-worker.md §9 pattern A): spawn the `iii` engine
//! and the `storage` worker as subprocesses, drive the worker through
//! `iii-sdk` as a client, then tear both down. **Self-skips** when `iii` is
//! not on `PATH` so CI hosts without the engine still pass.
//!
//! What we exercise: a `storage::putObject` round-trip against the bundled
//! rustfs `local` backend (no cloud credentials needed), then a `getObject`
//! to confirm the bytes round-trip.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;
use tokio::time::{sleep, timeout};

const ENGINE_WS: &str = "ws://127.0.0.1:49134";

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

    // The default `config.yaml` declares a `local` (rustfs-backed) bucket so
    // the worker has something useful to serve out of the box. Without a
    // rustfs binary, the worker errors during startup before registering
    // functions — self-skip rather than fail.
    if std::env::var("RUSTFS_BIN").ok().is_none() && which::which("rustfs").is_err() {
        return None;
    }

    let iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let worker_bin = env!("CARGO_BIN_EXE_storage");
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let config_path = format!("{manifest_dir}/config.yaml");

    let worker = Command::new(worker_bin)
        .args(["--url", ENGINE_WS, "--config", &config_path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    Some(Harness { iii, worker })
}

/// Poll `storage::putObject` until the worker has registered, or give up.
/// Returns the last response value (or panics with the last error). The worker
/// spawns rustfs and waits for the sidecar to become healthy before
/// registering functions, so there's a non-trivial readiness window.
async fn put_when_ready(client: &iii_sdk::III, payload: serde_json::Value) -> serde_json::Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut last_err: Option<String> = None;
    while std::time::Instant::now() < deadline {
        match timeout(
            Duration::from_secs(5),
            client.trigger(TriggerRequest {
                function_id: "storage::putObject".into(),
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
                    panic!("putObject failed: {msg}");
                }
                last_err = Some(msg);
            }
            Err(_) => last_err = Some("trigger timed out".into()),
        }
        sleep(Duration::from_millis(500)).await;
    }
    panic!(
        "worker never registered storage::putObject within 30s; last error: {}",
        last_err.unwrap_or_else(|| "<none>".into())
    );
}

#[tokio::test]
async fn put_then_get_round_trips_via_rustfs() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` and/or `rustfs` (or $RUSTFS_BIN) not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let body = b"hello from integration.rs";
    let body_b64 = B64.encode(body);

    let put = put_when_ready(
        &client,
        json!({
            "bucket": "scratch",
            "key": "integration/hello.txt",
            "body": body_b64,
            "content_type": "text/plain"
        }),
    )
    .await;

    assert!(put.is_object(), "putObject should return an object");

    let get = timeout(
        Duration::from_secs(10),
        client.trigger(TriggerRequest {
            function_id: "storage::getObject".into(),
            payload: json!({
                "bucket": "scratch",
                "key": "integration/hello.txt"
            }),
            action: None,
            timeout_ms: Some(8_000),
        }),
    )
    .await
    .expect("getObject timed out")
    .expect("getObject failed");

    let got_b64 = get["body"]
        .as_str()
        .expect("getObject response should have base64 `body`");
    let got = B64.decode(got_b64).expect("response body is valid base64");
    assert_eq!(got, body, "round-tripped body must match");

    client.shutdown_async().await;
}
