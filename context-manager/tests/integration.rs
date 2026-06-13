//! End-to-end test: spawn the `iii` engine + the worker binary as
//! subprocesses, then drive the worker through `iii-sdk` as a client.
//! Self-skips when `iii` is not on PATH.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::{register_worker, IIIError, InitOptions, TriggerRequest};
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

    let iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let worker_bin = env!("CARGO_BIN_EXE_context-manager");
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

#[tokio::test]
async fn end_to_end_via_iii_sdk() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` binary not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());

    // Poll until the worker's functions are routable (registration
    // flows over its own connection; no fixed sleep).
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let payload = json!({
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }],
                       "timestamp": 1 }],
        "model": { "id": "any-model" }
    });
    let result = loop {
        let attempt = timeout(
            Duration::from_secs(5),
            client.trigger(TriggerRequest {
                function_id: "context::count_tokens".into(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(4_000),
            }),
        )
        .await;
        match attempt {
            Ok(Ok(v)) => break v,
            Ok(Err(IIIError::NotConnected)) | Err(_) => {}
            Ok(Err(_)) => {}
        }
        assert!(
            std::time::Instant::now() < deadline,
            "context::count_tokens did not become routable within 20s"
        );
        sleep(Duration::from_millis(250)).await;
    };

    assert_eq!(result["estimator"], "heuristic");
    assert!(result["tokens"].as_u64().is_some_and(|t| t > 0));

    client.shutdown_async().await;
}
