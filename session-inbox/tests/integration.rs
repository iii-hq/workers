//! Smoke tests and optional engine-backed e2e (`iii` on PATH).

mod common;

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;
use tokio::time::{sleep, timeout};

const ENGINE_WS: &str = "ws://127.0.0.1:49134";

#[test]
fn function_ids_use_worker_namespace() {
    assert_eq!(session_inbox::PUSH_ID, "session-inbox::push");
    assert_eq!(session_inbox::DRAIN_ID, "session-inbox::drain");
    assert_eq!(session_inbox::PEEK_ID, "session-inbox::peek");
}

#[test]
fn inbox_key_uses_session_namespace() {
    assert_eq!(
        session_inbox::inbox_key("steering", "s1"),
        "session/s1/steering"
    );
}

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

    let worker_bin = common::session_inbox_executable();
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let config_path = format!("{manifest_dir}/config.yaml");

    let worker = Command::new(worker_bin)
        .args(["--url", ENGINE_WS, "--config", &config_path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(1500)).await;

    Some(Harness { iii, worker })
}

#[tokio::test]
async fn end_to_end_push_via_iii_sdk() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` binary not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let result = timeout(
        Duration::from_secs(10),
        client.trigger(TriggerRequest {
            function_id: "session-inbox::push".into(),
            payload: json!({
                "session_id": "s-e2e",
                "name": "steering",
                "item": { "role": "user" },
            }),
            action: None,
            timeout_ms: Some(5_000),
        }),
    )
    .await
    .expect("trigger timed out")
    .expect("trigger failed");

    assert_eq!(result["ok"], true);

    client.shutdown_async().await;
}
