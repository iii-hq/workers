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

    Some(Harness { iii, worker })
}

/// Call `editor::diff` until it resolves or `budget` runs out.
///
/// A `function_not_found` means the worker has not finished booting, which is
/// worth retrying; anything else is a real failure and is returned at once.
async fn await_function(
    client: &iii_sdk::IIIClient,
    budget: Duration,
) -> Option<serde_json::Value> {
    let deadline = std::time::Instant::now() + budget;
    let mut last: Option<String> = None;
    while std::time::Instant::now() < deadline {
        let call = client.trigger(TriggerRequest {
            function_id: "editor::diff".into(),
            payload: json!({
                "before": "a\nb\nc\n",
                "after": "a\nB\nc\n",
                "path": "sample.txt",
            }),
            action: None,
            timeout_ms: Some(5_000),
        });
        match timeout(Duration::from_secs(10), call).await {
            Ok(Ok(value)) => return Some(value),
            Ok(Err(e)) => {
                let text = e.to_string();
                if !text.contains("not found") {
                    panic!("editor::diff failed: {text}");
                }
                last = Some(text);
            }
            Err(_) => last = Some("trigger timed out".to_string()),
        }
        sleep(Duration::from_millis(250)).await;
    }
    eprintln!("gave up waiting for editor::diff: {last:?}");
    None
}

#[tokio::test]
async fn diff_round_trips_over_the_bus() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` binary not on PATH");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());

    // Poll rather than sleep for a fixed interval. The worker registers its
    // functions only after `configuration::register` and `fetch_config` have
    // both come back, and those retry with backoff on a cold engine, so any
    // constant chosen here would be either flaky or needlessly slow.
    let result = await_function(&client, Duration::from_secs(30))
        .await
        .expect("editor::diff never became callable");

    assert_eq!(result["identical"], false);
    assert_eq!(result["added"], 1);
    assert_eq!(result["removed"], 1);
    assert!(result["patch"]
        .as_str()
        .expect("patch is a string")
        .contains("+B"));

    client.shutdown_async().await;
}
