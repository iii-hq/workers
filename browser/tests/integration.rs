//! End-to-end test: spawn the `iii` engine + the worker binary, drive the
//! `browser::*` surface via iii-sdk as a client. Self-skips when `iii` or a
//! Chromium executable is absent, so CI hosts without either stay green.

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

fn chromium_present() -> bool {
    let known = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ];
    known.iter().any(|p| std::path::Path::new(p).exists())
        || which::which("google-chrome").is_ok()
        || which::which("chromium").is_ok()
        || which::which("chromium-browser").is_ok()
}

async fn boot() -> Option<Harness> {
    let iii_bin = which::which("iii").ok()?;
    if !chromium_present() {
        return None;
    }

    let mut iii = Command::new(&iii_bin)
        .arg("--use-default-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let worker = match Command::new(env!("CARGO_BIN_EXE_browser"))
        .arg("--url")
        .arg(ENGINE_WS)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(w) => w,
        Err(_) => {
            // The Harness Drop that reaps `iii` never runs (it was never
            // constructed), so clean up the already-started engine here.
            let _ = iii.kill();
            let _ = iii.wait();
            return None;
        }
    };

    sleep(Duration::from_millis(1500)).await;

    Some(Harness { iii, worker })
}

#[tokio::test]
async fn session_lifecycle_console_and_snapshot() {
    let Some(_h) = boot().await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };

    let client = register_worker(ENGINE_WS, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let call = |function_id: &str, payload: serde_json::Value, timeout_ms: u64| {
        let client = &client;
        let function_id = function_id.to_string();
        async move {
            timeout(
                Duration::from_secs(30),
                client.trigger(TriggerRequest {
                    function_id,
                    payload,
                    action: None,
                    timeout_ms: Some(timeout_ms),
                }),
            )
            .await
            .expect("trigger timed out")
            .expect("trigger failed")
        }
    };

    // start a session on about:blank; console traffic is injected below via
    // browser::evaluate, so no outbound navigation is needed
    let started = call("browser::sessions::start", json!({}), 25_000).await;
    let session_id = started["session_id"]
        .as_str()
        .expect("session_id in start response")
        .to_string();

    // list shows the session
    let listed = call("browser::sessions::list", json!({}), 10_000).await;
    let ids: Vec<&str> = listed["sessions"]
        .as_array()
        .expect("sessions array")
        .iter()
        .filter_map(|s| s["session_id"].as_str())
        .collect();
    assert!(ids.contains(&session_id.as_str()), "{ids:?}");

    // evaluate runs in the page
    let evaluated = call(
        "browser::evaluate",
        json!({ "session_id": session_id, "expression": "1 + 2" }),
        15_000,
    )
    .await;
    assert_eq!(evaluated["ok"], true, "{evaluated}");
    assert_eq!(evaluated["value"], 3, "{evaluated}");

    // console capture sees a console.log emitted now
    let _ = call(
        "browser::evaluate",
        json!({ "session_id": session_id, "expression": "console.error('boom-marker'); true" }),
        15_000,
    )
    .await;
    sleep(Duration::from_millis(500)).await;
    let console = call(
        "browser::console::read",
        json!({ "session_id": session_id, "pattern": "boom-marker" }),
        10_000,
    )
    .await;
    let entries = console["entries"].as_array().expect("entries array");
    assert!(
        entries.iter().any(|e| e["level"] == "error"),
        "console did not capture the marker: {console}"
    );

    // snapshot returns a tree
    let snapshot = call(
        "browser::snapshot",
        json!({ "session_id": session_id }),
        15_000,
    )
    .await;
    assert!(snapshot["tree"].is_string(), "{snapshot}");

    // dom tree gives element refs
    let dom = call(
        "browser::dom::read",
        json!({ "session_id": session_id }),
        15_000,
    )
    .await;
    let body_ref = dom["root"]["children"]
        .as_array()
        .and_then(|kids| {
            kids.iter()
                .flat_map(|k| {
                    std::iter::once(k).chain(k["children"].as_array().into_iter().flatten())
                })
                .find(|n| n["tag"] == "body")
        })
        .and_then(|n| n["ref"].as_str())
        .expect("body node in dom tree")
        .to_string();

    // computed styles for the body, then a live inline edit round-trips
    let styles = call(
        "browser::styles::read",
        json!({ "session_id": session_id, "ref": body_ref }),
        15_000,
    )
    .await;
    assert!(
        styles["properties"]
            .as_array()
            .is_some_and(|p| !p.is_empty()),
        "{styles}"
    );
    let written = call(
        "browser::styles::write",
        json!({
            "session_id": session_id,
            "ref": body_ref,
            "property": "background-color",
            "value": "rgb(1, 2, 3)"
        }),
        15_000,
    )
    .await;
    assert!(
        written["inline_style"]
            .as_str()
            .unwrap_or_default()
            .contains("background-color"),
        "{written}"
    );

    // history reload keeps the session alive
    let reloaded = call(
        "browser::history",
        json!({ "session_id": session_id, "action": "reload" }),
        20_000,
    )
    .await;
    assert_eq!(reloaded["ok"], true, "{reloaded}");

    // stop is idempotent
    let stopped = call(
        "browser::sessions::stop",
        json!({ "session_id": session_id }),
        15_000,
    )
    .await;
    assert_eq!(stopped["was_running"], true, "{stopped}");
    let stopped_again = call(
        "browser::sessions::stop",
        json!({ "session_id": session_id }),
        15_000,
    )
    .await;
    assert_eq!(stopped_again["was_running"], false, "{stopped_again}");

    client.shutdown_async().await;
}
