//! End-to-end tests: spawn the `iii` engine + the worker binary, drive the
//! `browser::*` surface via iii-sdk as a client. Self-skip when `iii` or a
//! Chromium executable is absent, so CI hosts without either stay green.
//! Each test boots its own engine on its own port and its own data dir, so
//! they can run in parallel and never share a Chromium profile.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{register_worker, InitOptions, RegisterFunction};
use serde_json::json;
use tokio::time::{sleep, timeout};

struct Harness {
    iii: Child,
    worker: Child,
    engine_ws: String,
    config_path: PathBuf,
    data_dir: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        // SIGTERM first: the worker then closes its Chromium. A plain kill
        // would orphan the browser process (and its profile lock) on every
        // failing run.
        // SAFETY: plain libc call on the child pid we spawned.
        unsafe {
            libc::kill(self.worker.id() as i32, libc::SIGTERM);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if matches!(self.worker.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        let _ = self.iii.kill();
        let _ = self.iii.wait();
        let _ = std::fs::remove_file(&self.config_path);
        let _ = std::fs::remove_dir_all(&self.data_dir);
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

    let port = TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
    let engine_ws = format!("ws://127.0.0.1:{port}");
    let config_path = std::env::temp_dir().join(format!(
        "browser-integration-{}.yaml",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(
        &config_path,
        format!(
            // No `modules:` key: the engine then injects its builtin workers
            // (streams among them); an empty list would leave `stream::set`
            // out and the live view silently dead.
            "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n"
        ),
    )
    .ok()?;

    let mut iii = Command::new(&iii_bin)
        .args(["--config", config_path.to_str()?, "--no-update-check"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    // The worker keeps its profile and tab list under `data_dir`, relative
    // to the compose dir: give each run a fresh one so tests never share a
    // Chromium profile or inherit another run's tabs.
    let data_dir = std::env::temp_dir().join(format!(
        "browser-integration-data-{}",
        uuid::Uuid::new_v4().simple()
    ));
    // `BROWSER_TEST_WORKER_LOG=<file>` captures the worker's debug log for
    // a failing run; by default it stays quiet.
    let worker_log = std::env::var_os("BROWSER_TEST_WORKER_LOG")
        .and_then(|path| std::fs::File::create(path).ok());
    let mut worker_cmd = Command::new(env!("CARGO_BIN_EXE_browser"));
    worker_cmd
        .arg("--url")
        .arg(&engine_ws)
        .env("III_COMPOSE_DIR", &data_dir);
    match worker_log {
        // tracing writes to stdout; keep stderr too for panics.
        Some(file) => {
            let err = file
                .try_clone()
                .ok()
                .map(Stdio::from)
                .unwrap_or(Stdio::null());
            worker_cmd.env("RUST_LOG", "debug").stdout(file).stderr(err)
        }
        None => worker_cmd.stdout(Stdio::null()).stderr(Stdio::null()),
    };
    let worker = match worker_cmd.spawn() {
        Ok(w) => w,
        Err(_) => {
            // The Harness Drop that reaps `iii` never runs (it was never
            // constructed), so clean up the already-started engine here.
            let _ = iii.kill();
            let _ = iii.wait();
            let _ = std::fs::remove_file(config_path);
            return None;
        }
    };

    // Boot includes three configuration::get retries with backoff when no
    // configuration worker is around, so registration lands ~1s in.
    sleep(Duration::from_millis(3000)).await;

    Some(Harness {
        iii,
        worker,
        engine_ws,
        config_path,
        data_dir,
    })
}

#[tokio::test]
async fn session_lifecycle_console_and_snapshot() {
    let Some(h) = boot().await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };

    let client = register_worker(&h.engine_ws, InitOptions::default());
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

    // doctor reports a usable environment (Chromium is present; we launched)
    let doctor = call("browser::doctor", json!({}), 10_000).await;
    assert_eq!(doctor["ok"], true, "{doctor}");
    assert!(doctor["chromium_path"].is_string(), "{doctor}");

    // attach is gated off by default (allow_attach false): both attach
    // functions must refuse rather than reach the real profile
    let attach = client
        .trigger(TriggerRequest {
            function_id: "browser::sessions::attach".into(),
            payload: json!({ "cdp_url": "http://127.0.0.1:9222" }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await;
    assert!(
        attach.is_err(),
        "attach must be refused when allow_attach is false: {attach:?}"
    );
    let tabs = client
        .trigger(TriggerRequest {
            function_id: "browser::tabs::list".into(),
            payload: json!({ "cdp_url": "http://127.0.0.1:9222" }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await;
    assert!(
        tabs.is_err(),
        "tabs::list must be refused when allow_attach is false: {tabs:?}"
    );
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

/// The browser model: an incognito tab is listed as such, a page that fails
/// to load is reported (not thrown) with the tab still usable, history moves
/// back across that, and clearing all browser data puts the profile away.
#[tokio::test]
async fn tabs_incognito_soft_errors_and_clear_browser_data() {
    let Some(h) = boot().await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };

    let client = register_worker(&h.engine_ws, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let call = |function_id: &str, payload: serde_json::Value, timeout_ms: u64| {
        let client = &client;
        let function_id = function_id.to_string();
        async move {
            timeout(
                Duration::from_secs(60),
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

    // A regular tab and a private one share the browser process, not data.
    let regular = call("browser::sessions::start", json!({}), 60_000).await;
    let regular_id = regular["session_id"].as_str().expect("id").to_string();
    assert_eq!(regular["incognito"], false, "{regular}");
    let private = call(
        "browser::sessions::start",
        json!({ "incognito": true }),
        60_000,
    )
    .await;
    let private_id = private["session_id"].as_str().expect("id").to_string();
    assert_eq!(private["incognito"], true, "{private}");

    let listed = call("browser::sessions::list", json!({}), 10_000).await;
    let tabs = listed["sessions"].as_array().expect("sessions array");
    let find = |id: &str| tabs.iter().find(|t| t["session_id"] == id).cloned();
    let regular_row = find(&regular_id).expect("regular tab listed");
    let private_row = find(&private_id).expect("private tab listed");
    assert_eq!(regular_row["incognito"], false, "{regular_row}");
    assert_eq!(regular_row["active"], true, "{regular_row}");
    assert_eq!(private_row["incognito"], true, "{private_row}");

    // Like a browser: a refused connection leaves Chromium's error page in
    // the tab and is reported, and the tab keeps working.
    let dead = call(
        "browser::navigate",
        json!({ "session_id": regular_id, "url": "http://127.0.0.1:9/" }),
        30_000,
    )
    .await;
    assert_eq!(dead["ok"], false, "{dead}");
    assert!(
        dead["error"]
            .as_str()
            .is_some_and(|e| e.starts_with("net::ERR_")),
        "{dead}"
    );
    let evaluated = call(
        "browser::evaluate",
        json!({ "session_id": regular_id, "expression": "location.href" }),
        15_000,
    )
    .await;
    assert_eq!(evaluated["ok"], true, "{evaluated}");

    // The worker's own back/forward stack: two local pages, then back.
    let pages_dir = std::env::temp_dir().join(format!(
        "browser-integration-pages-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&pages_dir).expect("pages dir");
    for name in ["one", "two"] {
        std::fs::write(
            pages_dir.join(format!("{name}.html")),
            format!("<!doctype html><title>{name}</title><h1>{name}</h1>"),
        )
        .expect("page file");
        let nav = call(
            "browser::navigate",
            json!({
                "session_id": regular_id,
                "url": format!("file://{}/{name}.html", pages_dir.display()),
            }),
            30_000,
        )
        .await;
        assert_eq!(nav["ok"], true, "{nav}");
    }
    let back = call(
        "browser::history",
        json!({ "session_id": regular_id, "action": "back" }),
        30_000,
    )
    .await;
    assert_eq!(back["moved"], true, "{back}");
    assert!(
        back["url"].as_str().unwrap_or_default().contains("one"),
        "{back}"
    );

    // Clearing everything closes the pages and the browser; the regular tab
    // stays listed asleep, the private one is gone for good.
    let cleared = call("browser::clear-browser-data", json!({}), 60_000).await;
    assert_eq!(cleared["ok"], true, "{cleared}");
    let listed = call("browser::sessions::list", json!({}), 10_000).await;
    let tabs = listed["sessions"].as_array().expect("sessions array");
    let regular_row = tabs
        .iter()
        .find(|t| t["session_id"] == regular_id)
        .expect("regular tab survives clearing");
    assert_eq!(regular_row["active"], false, "{regular_row}");
    assert!(
        !tabs.iter().any(|t| t["session_id"] == private_id),
        "private tab must not survive clearing: {listed}"
    );
    let doctor = call("browser::doctor", json!({}), 10_000).await;
    assert_eq!(doctor["browser_running"], false, "{doctor}");

    // Any call wakes the sleeping tab at the page it remembered.
    let woke = call(
        "browser::evaluate",
        json!({ "session_id": regular_id, "expression": "document.title" }),
        60_000,
    )
    .await;
    assert_eq!(woke["value"], "one", "{woke}");

    call(
        "browser::sessions::stop",
        json!({ "session_id": regular_id }),
        30_000,
    )
    .await;
    client.shutdown_async().await;
}

/// The live view: an animated page keeps producing screencast frames, they
/// reach the `browser:frames` stream a subscriber listens on, and opening
/// (and watching) a second tab does not freeze the first — the regression a
/// shared headless window caused, where only the newest tab still rendered.
#[tokio::test]
async fn live_frames_keep_flowing_across_tabs() {
    let Some(h) = boot().await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };

    let client = register_worker(&h.engine_ws, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let call = |function_id: &str, payload: serde_json::Value, timeout_ms: u64| {
        let client = &client;
        let function_id = function_id.to_string();
        async move {
            timeout(
                Duration::from_secs(60),
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

    // A page that repaints every 50ms, so Chromium emits a steady screencast.
    let pages_dir = std::env::temp_dir().join(format!(
        "browser-integration-anim-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&pages_dir).expect("pages dir");
    let page = pages_dir.join("anim.html");
    std::fs::write(
        &page,
        "<!doctype html><body style='margin:0'><div id=c style='height:100vh;background:red'>\
         </div><script>let n=0;setInterval(()=>{n++;c.style.background=n%2?'red':'blue';\
         c.textContent=n},50)</script></body>",
    )
    .expect("page file");
    let page_url = format!("file://{}", page.display());

    // A frame subscriber, the way the console watches a tab.
    let pushes = Arc::new(AtomicUsize::new(0));
    {
        let pushes = pushes.clone();
        client.register_function(
            "iii::browser-test::frames",
            RegisterFunction::new_async(move |_frame: serde_json::Value| {
                let pushes = pushes.clone();
                async move {
                    pushes.fetch_add(1, Ordering::Relaxed);
                    Ok::<_, iii_sdk::errors::Error>(json!({ "ok": true }))
                }
            }),
        );
    }

    let first = call(
        "browser::sessions::start",
        json!({ "url": page_url }),
        60_000,
    )
    .await;
    let first_id = first["session_id"].as_str().expect("id").to_string();
    client
        .register_trigger(RegisterTriggerInput::new(
            "browser::frame-event".to_string(),
            "iii::browser-test::frames".to_string(),
            json!({ "session_id": first_id }),
        ))
        .expect("frame trigger");
    call(
        "browser::screencast::start",
        json!({ "session_id": first_id }),
        30_000,
    )
    .await;
    sleep(Duration::from_millis(1500)).await;

    let frame_seq = |id: String| {
        let call = &call;
        async move {
            let frame = call("browser::frame", json!({ "session_id": id }), 10_000).await;
            assert_eq!(frame["active"], true, "{frame}");
            frame["frame_seq"].as_u64().expect("frame_seq")
        }
    };
    let before = frame_seq(first_id.clone()).await;
    sleep(Duration::from_millis(1000)).await;
    let after = frame_seq(first_id.clone()).await;
    assert!(
        after >= before + 8,
        "the pump stalled: frame_seq went {before} -> {after} in 1s"
    );
    let pushed = pushes.load(Ordering::Relaxed);
    assert!(
        pushed >= 8,
        "only {pushed} frames reached the frame subscriber"
    );

    // A second tab, watched too: the first must keep rendering.
    let second = call(
        "browser::sessions::start",
        json!({ "url": page_url }),
        60_000,
    )
    .await;
    let second_id = second["session_id"].as_str().expect("id").to_string();
    call(
        "browser::screencast::start",
        json!({ "session_id": second_id }),
        30_000,
    )
    .await;
    sleep(Duration::from_millis(500)).await;
    pushes.store(0, Ordering::Relaxed);
    let before = frame_seq(first_id.clone()).await;
    sleep(Duration::from_millis(1000)).await;
    let after = frame_seq(first_id.clone()).await;
    assert!(
        after >= before + 8,
        "the first tab froze once a second tab opened: frame_seq {before} -> {after}"
    );
    let pushed = pushes.load(Ordering::Relaxed);
    assert!(
        pushed >= 8,
        "only {pushed} frames of the first tab reached the subscriber after a second tab opened"
    );
    let second_seq = frame_seq(second_id.clone()).await;
    assert!(second_seq > 0, "second tab produced no frames");

    for id in [first_id, second_id] {
        call(
            "browser::sessions::stop",
            json!({ "session_id": id }),
            30_000,
        )
        .await;
    }
    let _ = std::fs::remove_dir_all(pages_dir);
    client.shutdown_async().await;
}
