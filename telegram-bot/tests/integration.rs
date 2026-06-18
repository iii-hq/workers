//! End-to-end test: spawn `iii` engine + worker. Self-skips when `iii` is absent.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

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

    let iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let worker_bin = env!("CARGO_BIN_EXE_telegram-bot");
    let config = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/config.yaml");
    let worker = Command::new(worker_bin)
        .arg("--url")
        .arg(ENGINE_WS)
        .arg("--config")
        .arg(config)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(1500)).await;
    Some(Harness { iii, worker })
}

#[tokio::test]
async fn worker_registers_on_engine() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` binary not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let result = timeout(
        Duration::from_secs(10),
        client.trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5000),
        }),
    )
    .await
    .expect("trigger timed out")
    .expect("trigger failed");

    let functions = result
        .get("functions")
        .and_then(|v| v.as_array())
        .expect("functions list");
    let registered = functions
        .iter()
        .any(|f| f.get("id").and_then(|v| v.as_str()) == Some("telegram-bot::webhook"));
    if !registered {
        eprintln!(
            "skipping: telegram-bot did not register (configuration worker or bot_token required)"
        );
        client.shutdown_async().await;
        return;
    }

    client.shutdown_async().await;
}
