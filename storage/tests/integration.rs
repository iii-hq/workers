//! End-to-end test (docs/sops/binary-worker.md §9 pattern A): spawn the `iii` engine
//! and the `storage` worker as subprocesses, drive the worker through
//! `iii-sdk` as a client, then tear both down. **Self-skips** when neither
//! `III_ENGINE_BIN` nor an `iii` binary on `PATH` is available.
//!
//! What we exercise: a `storage::putObject` round-trip against the native
//! local backend (no cloud credentials or sidecar needed), then a `getObject`
//! to confirm the bytes round-trip.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;
use tokio::time::{sleep, timeout};

struct Harness {
    iii: Child,
    worker: Child,
    url: String,
    _directory: tempfile::TempDir,
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
    let iii_bin = std::env::var_os("III_ENGINE_BIN")
        .map(PathBuf::from)
        .or_else(|| which::which("iii").ok())?;
    let port = TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
    let directory = tempfile::tempdir().ok()?;
    let engine_config_path = directory.path().join("engine.yaml");
    std::fs::write(
        &engine_config_path,
        format!("workers:\n  - name: iii-worker-manager\n    config:\n      port: {port}\n"),
    )
    .ok()?;
    let url = format!("ws://127.0.0.1:{port}");

    let mut iii = Command::new(&iii_bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&engine_config_path)
        .current_dir(directory.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let worker_bin = env!("CARGO_BIN_EXE_storage");
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let config_path = format!("{manifest_dir}/config.yaml");

    // If the worker fails to spawn, kill the engine before returning so we
    // don't leak the iii subprocess into the next test (or out of the
    // process entirely). Without this, a transient spawn failure could
    // leave a stray engine bound to the test port.
    let worker = match Command::new(worker_bin)
        .args(["--url", &url, "--config", &config_path])
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

    Some(Harness {
        iii,
        worker,
        url,
        _directory: directory,
    })
}

/// Poll `storage::putObject` until the worker has registered, or give up.
/// Returns the last response value (or panics with the last error). The worker
/// initializes its local data directory before registering functions, so
/// there can still be a short readiness window.
async fn put_when_ready(
    client: &iii_sdk::IIIClient,
    payload: serde_json::Value,
) -> serde_json::Value {
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
async fn put_then_get_round_trips_via_native_local_storage() {
    let Some(harness) = boot().await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };

    let client = register_worker(&harness.url, InitOptions::default());
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

    let get: serde_json::Value = timeout(
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

    let got_b64 = get["body_base64"]
        .as_str()
        .expect("getObject response should have base64 `body_base64`");
    let got = B64.decode(got_b64).expect("response body is valid base64");
    assert_eq!(got, body, "round-tripped body must match");

    client.shutdown_async().await;
}
