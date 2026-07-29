//! End-to-end: spawn the `iii` engine and the worker, drive both through the
//! SDK as a client. Self-skips when `iii` is not on PATH, and — importantly —
//! when an engine is ALREADY listening.
//!
//! That second guard is not paranoia. A second engine cannot bind the port, so
//! it exits quietly, but the worker we spawn next connects to whatever is
//! there: the developer's live rig. It then re-registers `editor/page.js`
//! (same path ⇒ last writer wins, console-wide) and, when `Drop` kills it,
//! that Message-path trigger is GC'd — taking the real worker's console page
//! down with it. The test passes and the rig quietly loses its editor tab.
//!
//! Only `editor::diff` is exercised here, on purpose: it is the one function
//! that needs nothing but the worker itself. Everything else delegates to
//! `shell`, so testing it in this harness would be testing whether `shell` is
//! installed and jailed to the temp dir, which is `shell`'s own e2e job. The
//! delegation shape is covered by unit tests over the parsers instead.

use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
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

/// True when something already holds the engine port.
fn engine_already_running() -> bool {
    TcpStream::connect_timeout(
        &"127.0.0.1:49134".parse().expect("valid socket address"),
        Duration::from_millis(250),
    )
    .is_ok()
}

async fn boot() -> Option<Harness> {
    let iii_bin = which::which("iii").ok()?;

    if engine_already_running() {
        eprintln!(
            "skipping: an engine is already listening on 127.0.0.1:49134. \
             This test spawns its own worker, which would re-register the \
             editor console assets on that engine and remove them again on \
             teardown."
        );
        return None;
    }

    let iii = Command::new(&iii_bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    // `?` here would drop `iii` without killing it: `Harness` does not exist
    // yet, so nothing owns the engine until both children are spawned.
    let worker = match Command::new(env!("CARGO_BIN_EXE_editor"))
        .args(["--url", ENGINE_WS])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("failed to spawn the worker: {e}");
            let mut iii = iii;
            let _ = iii.kill();
            let _ = iii.wait();
            return None;
        }
    };

    sleep(Duration::from_millis(1500)).await;

    Some(Harness { iii, worker })
}

#[tokio::test]
async fn diff_round_trips_over_the_bus() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` binary not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let result = timeout(
        Duration::from_secs(10),
        client.trigger(TriggerRequest {
            function_id: "editor::diff".into(),
            payload: json!({
                "before": "a\nb\nc\n",
                "after": "a\nB\nc\n",
                "path": "sample.txt",
            }),
            action: None,
            timeout_ms: Some(5_000),
        }),
    )
    .await
    .expect("trigger timed out")
    .expect("trigger failed");

    assert_eq!(result["identical"], false);
    assert_eq!(result["added"], 1);
    assert_eq!(result["removed"], 1);
    assert!(result["patch"]
        .as_str()
        .expect("patch is a string")
        .contains("+B"));

    client.shutdown_async().await;
}
