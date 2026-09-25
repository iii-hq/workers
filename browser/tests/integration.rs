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

/// Which engine a lane boots the worker on. `Lightpanda` needs the binary
/// on PATH or `BROWSER_TEST_LIGHTPANDA=<path>`; the lane self-skips
/// otherwise.
#[derive(Clone, Copy, PartialEq)]
enum Lane {
    Chromium,
    Lightpanda,
}

fn lightpanda_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BROWSER_TEST_LIGHTPANDA") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    which::which("lightpanda").ok()
}

/// A loopback HTTP server answering every request with `html`, for engines
/// that refuse `file://` (Lightpanda). Lives as long as the test process.
fn serve_html(html: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                html.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}/")
}

async fn boot() -> Option<Harness> {
    boot_lane(Lane::Chromium).await
}

async fn boot_lane(lane: Lane) -> Option<Harness> {
    let iii_bin = which::which("iii").ok()?;
    // The worker seeds its configuration from `--config` when no
    // configuration worker is around (the case here): the Lightpanda lane
    // points `engine`/`executable` at the binary through it.
    let seed = match lane {
        Lane::Chromium => {
            if !chromium_present() {
                return None;
            }
            None
        }
        Lane::Lightpanda => {
            let bin = lightpanda_binary()?;
            Some(format!(
                "browser:\n  engine: lightpanda\n  executable: {}\n",
                serde_json::to_string(&bin.to_string_lossy()).ok()?
            ))
        }
    };

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

    // The worker keeps its profile and tab list under `data_dir`, relative
    // to the compose dir: give each run a fresh one so tests never share a
    // Chromium profile or inherit another run's tabs. The engine runs from
    // it too: its builtin configuration store writes `./config/<id>.yaml`
    // under its own cwd, and a value one lane registers (the Lightpanda seed)
    // would otherwise outlive the run in the package dir and be handed to
    // every later boot ahead of that boot's own seed.
    let data_dir = std::env::temp_dir().join(format!(
        "browser-integration-data-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&data_dir).ok()?;

    let mut iii = Command::new(&iii_bin)
        .args(["--config", config_path.to_str()?, "--no-update-check"])
        .current_dir(&data_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;
    // `BROWSER_TEST_WORKER_LOG=<file>` captures the worker's debug log for
    // a failing run; by default it stays quiet.
    let worker_log = std::env::var_os("BROWSER_TEST_WORKER_LOG")
        .and_then(|path| std::fs::File::create(path).ok());
    let mut worker_cmd = Command::new(env!("CARGO_BIN_EXE_browser"));
    worker_cmd
        .arg("--url")
        .arg(&engine_ws)
        .env("III_COMPOSE_DIR", &data_dir);
    if let Some(seed) = seed {
        let seed_path = data_dir.join("browser-seed.yaml");
        std::fs::write(&seed_path, seed).ok()?;
        worker_cmd.arg("--config").arg(&seed_path);
    }
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

/// The Lightpanda engine lane: the DOM half of the `browser::*` surface
/// over `lightpanda serve` — a tab opens on a local http page, reads back
/// as a snapshot with named controls, clicks by ref, runs JS, captures its
/// console, screenshots (a text-only PNG), and the doctor names the engine.
/// The screencast is not there and says so. Skips unless the binary is on
/// PATH or `BROWSER_TEST_LIGHTPANDA` points at it.
#[tokio::test]
async fn lightpanda_engine_drives_the_dom_surface() {
    let Some(h) = boot_lane(Lane::Lightpanda).await else {
        eprintln!("skipping: `iii` or lightpanda not available");
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

    let doctor = call("browser::doctor", json!({}), 10_000).await;
    assert_eq!(doctor["engine"], "lightpanda", "{doctor}");
    assert_eq!(doctor["ok"], true, "{doctor}");
    assert_eq!(doctor["browser_running"], false, "{doctor}");

    let url = serve_html(
        "<!doctype html><title>Lightpanda lane</title><h1 id=\"t\">hello from lightpanda</h1>\
         <button id=\"b\" onclick=\"document.title='clicked'\">go</button>\
         <script>console.error('lightpanda-marker')</script>",
    );
    let started = call("browser::sessions::start", json!({ "url": url }), 25_000).await;
    let session_id = started["session_id"]
        .as_str()
        .expect("session_id in start response")
        .to_string();
    assert!(started["error"].is_null(), "{started}");
    assert_eq!(started["headless"], true, "{started}");

    let evaluated = call(
        "browser::evaluate",
        json!({ "session_id": session_id, "expression": "document.title" }),
        15_000,
    )
    .await;
    assert_eq!(evaluated["value"], "Lightpanda lane", "{evaluated}");

    let snapshot = call(
        "browser::snapshot",
        json!({ "session_id": session_id }),
        15_000,
    )
    .await;
    let tree = snapshot["tree"].as_str().expect("tree");
    assert!(tree.contains("hello from lightpanda"), "{tree}");

    // act by ref: Lightpanda names the button from its contents
    let button_ref = tree
        .lines()
        .find(|line| line.trim_start().starts_with("- button \"go\""))
        .and_then(|line| line.split("[ref=").nth(1))
        .and_then(|rest| rest.split(']').next())
        .expect("named button ref in snapshot")
        .to_string();
    let acted = call(
        "browser::act",
        json!({ "session_id": session_id, "action": "click", "ref": button_ref }),
        15_000,
    )
    .await;
    assert_eq!(acted["ok"], true, "{acted}");
    let title = call(
        "browser::evaluate",
        json!({ "session_id": session_id, "expression": "document.title" }),
        15_000,
    )
    .await;
    assert_eq!(title["value"], "clicked", "{title}");

    let console = call(
        "browser::console::read",
        json!({ "session_id": session_id, "pattern": "lightpanda-marker" }),
        10_000,
    )
    .await;
    assert!(
        console["entries"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|e| e["level"] == "error")),
        "console did not capture the marker: {console}"
    );

    // Lightpanda has no rasterizer: jpeg is refused, so the worker asks for
    // the text-only png instead of failing the call.
    let shot = call(
        "browser::screenshot",
        json!({ "session_id": session_id }),
        20_000,
    )
    .await;
    assert!(
        shot["content"]
            .as_array()
            .is_some_and(|blocks| !blocks.is_empty()),
        "{shot}"
    );
    assert_eq!(shot["content"][0]["mime"], "image/png", "{shot}");

    // and no screencast at all: the live view falls back to screenshots
    let screencast = client
        .trigger(TriggerRequest {
            function_id: "browser::screencast::start".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await;
    assert!(
        screencast.is_err(),
        "screencast unexpectedly started: {screencast:?}"
    );

    let doctor = call("browser::doctor", json!({}), 10_000).await;
    assert_eq!(doctor["browser_running"], true, "{doctor}");

    let stopped = call(
        "browser::sessions::stop",
        json!({ "session_id": session_id }),
        15_000,
    )
    .await;
    assert_eq!(stopped["was_running"], true, "{stopped}");
    // The last live tab closing ends the lightpanda process.
    sleep(Duration::from_millis(500)).await;
    let doctor = call("browser::doctor", json!({}), 10_000).await;
    assert_eq!(doctor["browser_running"], false, "{doctor}");

    client.shutdown_async().await;
}

/// Form fixture for the element table and the act guards: a text field, a
/// native select, a Save button that records both values in the title, a
/// disabled button, a password field, and a hidden full-page shield that
/// covers everything once shown.
const FORM_HTML: &str = r#"<!doctype html><title>form</title>
<style>#shield{position:fixed;inset:0;background:rgba(0,0,0,.3)}</style>
<form onsubmit="event.preventDefault()">
<label>Title <input id="summary" value="old"></label>
<label>Type <select id="kind"><option>Feature</option><option value="bug">Bug</option></select></label>
<button type="button" id="save" onclick="document.title='saved:'+document.getElementById('summary').value+':'+document.getElementById('kind').value">Save</button>
<button type="button" disabled>Archive</button>
<input type="password" aria-label="Secret" value="x">
</form>
<div id="shield" hidden></div>"#;

fn element<'a>(table: &'a serde_json::Value, label: &str) -> &'a serde_json::Value {
    table["elements"]
        .as_array()
        .expect("elements array")
        .iter()
        .find(|e| e["label"] == label)
        .unwrap_or_else(|| panic!("no element labelled {label:?}: {table}"))
}

#[tokio::test]
async fn elements_table_guards_select_and_replace() {
    let Some(h) = boot().await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = register_worker(&h.engine_ws, InitOptions::default());
    sleep(Duration::from_millis(500)).await;
    let try_call = |function_id: &str, payload: serde_json::Value| {
        let client = &client;
        let function_id = function_id.to_string();
        async move {
            timeout(
                Duration::from_secs(30),
                client.trigger(TriggerRequest {
                    function_id,
                    payload,
                    action: None,
                    timeout_ms: Some(20_000),
                }),
            )
            .await
            .expect("trigger timed out")
        }
    };
    let call = |function_id: &'static str, payload: serde_json::Value| {
        let pending = try_call(function_id, payload);
        async move { pending.await.expect("trigger failed") }
    };

    let url = serve_html(FORM_HTML);
    let started = call("browser::sessions::start", json!({ "url": url })).await;
    let sid = started["session_id"]
        .as_str()
        .expect("session_id")
        .to_string();
    let eval = |expression: &'static str| {
        let payload = json!({ "session_id": sid, "expression": expression });
        async move { call("browser::evaluate", payload).await["value"].clone() }
    };

    // one read: controls with refs and operations, disabled ones left out,
    // password values never exposed
    let table = call("browser::elements", json!({ "session_id": sid })).await;
    let title = element(&table, "Title");
    assert_eq!(title["value"], "old", "{table}");
    assert_eq!(title["operations"], json!(["type", "click"]), "{table}");
    let kind = element(&table, "Type");
    assert_eq!(kind["operations"], json!(["select"]), "{table}");
    assert_eq!(kind["options"], json!(["Feature", "Bug"]), "{table}");
    assert_eq!(element(&table, "Secret")["value"], "(set)", "{table}");
    assert!(
        !table["elements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["label"] == "Archive"),
        "a disabled button is not actionable: {table}"
    );
    let (title_ref, kind_ref, save_ref) = (
        title["ref"].as_str().unwrap().to_string(),
        kind["ref"].as_str().unwrap().to_string(),
        element(&table, "Save")["ref"].as_str().unwrap().to_string(),
    );

    // type by ref replaces the value; empty text clears it
    call(
        "browser::act",
        json!({ "session_id": sid, "action": "type", "ref": title_ref, "text": "" }),
    )
    .await;
    assert_eq!(eval("summary.value").await, "");
    let typed = call(
        "browser::act",
        json!({ "session_id": sid, "action": "type", "ref": title_ref, "text": "Hello" }),
    )
    .await;
    assert!(
        typed["detail"].as_str().unwrap().contains("replaced"),
        "{typed}"
    );
    assert_eq!(eval("summary.value").await, "Hello");

    // native select by label
    let selected = call(
        "browser::act",
        json!({ "session_id": sid, "action": "select", "ref": kind_ref, "option": "Bug" }),
    )
    .await;
    assert_eq!(selected["detail"], "selected 'Bug'", "{selected}");
    assert_eq!(eval("kind.value").await, "bug");

    // a covered button refuses the click instead of clicking the cover
    eval("shield.hidden = false").await;
    let covered = try_call(
        "browser::act",
        json!({ "session_id": sid, "action": "click", "ref": save_ref }),
    )
    .await
    .expect_err("click on a covered element must be refused");
    assert!(
        covered.to_string().contains("covered by div#shield"),
        "{covered}"
    );
    eval("shield.hidden = true").await;
    call(
        "browser::act",
        json!({ "session_id": sid, "action": "click", "ref": save_ref }),
    )
    .await;
    assert_eq!(eval("document.title").await, "saved:Hello:bug");

    // n refs work wherever refs do
    let dom = call(
        "browser::dom::read",
        json!({ "session_id": sid, "ref": save_ref }),
    )
    .await;
    assert_eq!(dom["root"]["tag"], "button", "{dom}");

    // snapshot refs still act (and pass the same guard)
    let snap = call("browser::snapshot", json!({ "session_id": sid })).await;
    let e_ref = snap["tree"]
        .as_str()
        .unwrap()
        .lines()
        .find(|l| l.contains("button \"Save\""))
        .and_then(|l| l.split("[ref=").nth(1))
        .and_then(|rest| rest.split(']').next())
        .expect("Save in snapshot")
        .to_string();
    eval("document.title = 'x'").await;
    call(
        "browser::act",
        json!({ "session_id": sid, "action": "click", "ref": e_ref }),
    )
    .await;
    assert_eq!(eval("document.title").await, "saved:Hello:bug");

    // navigation kills n refs
    call(
        "browser::navigate",
        json!({ "session_id": sid, "url": url }),
    )
    .await;
    let stale = try_call(
        "browser::act",
        json!({ "session_id": sid, "action": "click", "ref": save_ref }),
    )
    .await
    .expect_err("an n ref from the previous document must not resolve");
    assert!(stale.to_string().contains("unknown ref"), "{stale}");

    call("browser::sessions::stop", json!({ "session_id": sid })).await;
    client.shutdown_async().await;
}

/// A scripted `judge::evaluate`: select Bug, type the title, click Save,
/// then DONE once the page title says saved. Answers every head the way a
/// real judge does, but only the chosen operation's head matters.
fn scripted_judge(request: &serde_json::Value) -> serde_json::Value {
    let evaluation = &request["evaluations"][0];
    let state = &evaluation["state"];
    let questions = &evaluation["questions"];
    let pick = |question: &str, choice: &str| {
        let probabilities: serde_json::Map<String, serde_json::Value> = questions[question]
            ["criteria"]
            .as_object()
            .expect("choice criteria")
            .keys()
            .map(|k| (k.clone(), json!(if k == choice { 1.0 } else { 0.0 })))
            .collect();
        json!({ "type": "choice", "choice": choice, "probabilities": probabilities, "confidence": 0.9 })
    };
    let field = |label: &str| {
        state["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["label"] == label)
            .cloned()
            .unwrap_or_else(|| panic!("no {label} in {state}"))
    };
    let (kind, title, save) = (field("Type"), field("Title"), field("Save"));
    let r = |e: &serde_json::Value| e["ref"].as_str().unwrap().to_string();
    let (operation, head) = if state["page"]["title"]
        .as_str()
        .unwrap()
        .starts_with("saved")
    {
        ("DONE", None)
    } else if kind["value"] != "Bug" {
        ("SELECT", Some(("select_target", format!("{}:2", r(&kind)))))
    } else if title["value"] != "Crash on save" {
        ("TYPE_TEXT", Some(("type_text_target", r(&title))))
    } else {
        ("CLICK", Some(("click_target", r(&save))))
    };
    let mut answers = serde_json::Map::new();
    answers.insert("operation".into(), pick("operation", operation));
    if let Some((question, target)) = head {
        answers.insert(question.into(), pick(question, &target));
    }
    json!({
        "status": "ok",
        "model": "scripted",
        "results": { evaluation["id"].as_str().unwrap(): { "answers": answers } },
        "stats": { "attempts": 1, "requests": 1, "questions": answers.len(),
                   "input_tokens": 0, "output_tokens": 0, "elapsed_ms": 1, "usage_complete": false },
    })
}

#[tokio::test]
async fn run_drives_a_form_with_the_judge_and_asks_for_missing_text() {
    let Some(h) = boot().await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = register_worker(&h.engine_ws, InitOptions::default());
    let requests = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
    {
        let requests = requests.clone();
        client.register_function(
            "judge::evaluate",
            RegisterFunction::new_async(move |request: serde_json::Value| {
                let requests = requests.clone();
                async move {
                    let reply = scripted_judge(&request);
                    requests.lock().unwrap().push(request);
                    Ok::<_, iii_sdk::errors::Error>(reply)
                }
            }),
        );
    }
    sleep(Duration::from_millis(800)).await;
    let call = |function_id: &'static str, payload: serde_json::Value| {
        let client = &client;
        async move {
            timeout(
                Duration::from_secs(60),
                client.trigger(TriggerRequest {
                    function_id: function_id.into(),
                    payload,
                    action: None,
                    timeout_ms: Some(50_000),
                }),
            )
            .await
            .expect("trigger timed out")
            .expect("trigger failed")
        }
    };
    let url = serve_html(FORM_HTML);
    let started = call("browser::sessions::start", json!({ "url": url })).await;
    let sid = started["session_id"].as_str().unwrap().to_string();
    let goal = "File a Bug with the given title and save it.";

    // no inputs: the select happens, then the run hands the text back
    let first = call("browser::run", json!({ "session_id": sid, "goal": goal })).await;
    assert_eq!(first["status"], "needs_text", "{first}");
    assert_eq!(first["steps"].as_array().unwrap().len(), 1, "{first}");
    assert_eq!(first["steps"][0]["operation"], "SELECT", "{first}");
    assert_eq!(first["steps"][0]["option"], "Bug", "{first}");
    assert_eq!(first["needs_text"]["label"], "Title", "{first}");

    // with the text: type, click, done — one call
    let n = requests.lock().unwrap().len();
    // this call comes from a session whose judge is semif: every judge
    // request of the run must name it
    let semif = opentelemetry::baggage::BaggageExt::current_with_baggage(vec![
        opentelemetry::KeyValue::new(judge_contract::PROVIDER_BAGGAGE_KEY, "semif"),
    ]);
    let second = opentelemetry::context::FutureExt::with_context(
        call(
            "browser::run",
            json!({ "session_id": sid, "goal": goal, "inputs": { "title": "Crash on save" } }),
        ),
        semif,
    )
    .await;
    assert_eq!(second["status"], "done", "{second}");
    let ops: Vec<&str> = second["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["operation"].as_str().unwrap())
        .collect();
    assert_eq!(ops, ["TYPE_TEXT", "CLICK"], "{second}");
    assert_eq!(
        second["page"]["title"], "saved:Crash on save:bug",
        "{second}"
    );

    let requests = requests.lock().unwrap().clone();
    // the session's provider rides every request of that run, and only it
    assert!(requests[..n].iter().all(|r| r.get("provider").is_none()));
    assert!(requests[n..].iter().all(|r| r["provider"] == "semif"));
    // input values never reach the judge (until the page itself shows them)
    assert!(!requests[n]["evaluations"][0]["state"]
        .to_string()
        .contains("Crash on save"));
    // each request asks the operation plus one head per offered operation
    let heads: Vec<&String> = requests[n]["evaluations"][0]["questions"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(
        heads,
        [
            "click_target",
            "operation",
            "select_target",
            "type_text_target"
        ]
    );
    call("browser::sessions::stop", json!({ "session_id": sid })).await;
    client.shutdown_async().await;
}

#[tokio::test]
async fn run_without_a_judge_returns_the_page_and_touches_nothing() {
    let Some(h) = boot().await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = register_worker(&h.engine_ws, InitOptions::default());
    sleep(Duration::from_millis(500)).await;
    let call = |function_id: &'static str, payload: serde_json::Value| {
        let client = &client;
        async move {
            timeout(
                Duration::from_secs(30),
                client.trigger(TriggerRequest {
                    function_id: function_id.into(),
                    payload,
                    action: None,
                    timeout_ms: Some(20_000),
                }),
            )
            .await
            .expect("trigger timed out")
            .expect("trigger failed")
        }
    };
    let started = call(
        "browser::sessions::start",
        json!({ "url": serve_html(FORM_HTML) }),
    )
    .await;
    let sid = started["session_id"].as_str().unwrap().to_string();
    let run = json!({ "session_id": sid, "goal": "save the form" });

    let first = call("browser::run", run.clone()).await;
    assert_eq!(first["status"], "judge_unavailable", "{first}");
    assert!(first["steps"].as_array().unwrap().is_empty(), "{first}");
    assert_eq!(first["page"]["title"], "form", "{first}");
    assert!(
        first["page"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["label"] == "Save"),
        "the page table comes back so the caller can act: {first}"
    );
    // the outage pauses the judge: the next run does not wait on the bus
    let second = call("browser::run", run).await;
    assert!(
        second["reason"].as_str().unwrap().contains("paused"),
        "{second}"
    );
    call("browser::sessions::stop", json!({ "session_id": sid })).await;
    client.shutdown_async().await;
}

/// Like `serve_html`, but a request for `/slow` answers after 1.2 s: a login
/// endpoint a submit waits on.
fn serve_html_with_slow_api(html: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let (kind, body) =
                    if request.starts_with("GET /slow") || request.starts_with("POST /slow") {
                        std::thread::sleep(Duration::from_millis(1200));
                        ("application/json", r#"{"ok":false}"#)
                    } else {
                        ("text/html; charset=utf-8", html)
                    };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            });
        }
    });
    format!("http://{addr}/")
}

/// A login whose submit hides the form behind "Entrando…" until the API
/// answers, then shows the error — fields labelled in Portuguese.
const LOGIN_HTML: &str = r#"<!doctype html><title>BeQuali</title>
<form id="f" onsubmit="event.preventDefault()">
<label>E-mail <input type="email" name="user_email"></label>
<label>Senha <input type="password" name="pw"></label>
<button type="button" id="go">Entrar</button>
</form>
<p id="msg"></p>
<script>
document.getElementById('go').onclick = async () => {
  const f = document.getElementById('f'), msg = document.getElementById('msg');
  f.hidden = true; msg.textContent = 'Entrando…';
  await fetch('/slow', { method: 'POST' });
  f.hidden = false; msg.textContent = 'E-mail ou senha inválidos';
};
</script>"#;

/// A judge that, like the live one, calls the goal done as soon as the
/// login was submitted: fills both fields, clicks Entrar, then DONE.
fn eager_login_judge(request: &serde_json::Value) -> serde_json::Value {
    let evaluation = &request["evaluations"][0];
    let state = &evaluation["state"];
    let questions = &evaluation["questions"];
    let pick = |question: &str, choice: &str| {
        let probabilities: serde_json::Map<String, serde_json::Value> = questions[question]
            ["criteria"]
            .as_object()
            .expect("choice criteria")
            .keys()
            .map(|k| (k.clone(), json!(if k == choice { 1.0 } else { 0.0 })))
            .collect();
        json!({ "type": "choice", "choice": choice, "probabilities": probabilities, "confidence": 0.9 })
    };
    let text = state["page"]["text"].as_str().unwrap_or_default();
    let field = |label: &str| {
        state["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["label"] == label)
            .cloned()
    };
    let mut answers = serde_json::Map::new();
    let submitted = text.contains("Entrando") || text.contains("inválidos");
    let target = if submitted {
        None
    } else if field("E-mail").is_some_and(|e| e.get("value").is_none()) {
        Some(("TYPE_TEXT", "type_text_target", field("E-mail").unwrap()))
    } else if field("Senha").is_some_and(|e| e.get("value").is_none()) {
        Some(("TYPE_TEXT", "type_text_target", field("Senha").unwrap()))
    } else {
        Some(("CLICK", "click_target", field("Entrar").unwrap()))
    };
    match target {
        None => {
            answers.insert("operation".into(), pick("operation", "DONE"));
        }
        Some((operation, head, element)) => {
            answers.insert("operation".into(), pick("operation", operation));
            if questions.get(head).is_some() {
                answers.insert(head.into(), pick(head, element["ref"].as_str().unwrap()));
            }
        }
    }
    json!({
        "status": "ok",
        "model": "scripted",
        "results": { evaluation["id"].as_str().unwrap(): { "answers": answers } },
        "stats": { "attempts": 1, "requests": 1, "questions": answers.len(),
                   "input_tokens": 0, "output_tokens": 0, "elapsed_ms": 1, "usage_complete": false },
    })
}

#[tokio::test]
async fn run_waits_for_the_submit_it_triggered_and_matches_inputs_by_field() {
    let Some(h) = boot().await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = register_worker(&h.engine_ws, InitOptions::default());
    client.register_function(
        "judge::evaluate",
        RegisterFunction::new_async(|request: serde_json::Value| async move {
            Ok::<_, iii_sdk::errors::Error>(eager_login_judge(&request))
        }),
    );
    sleep(Duration::from_millis(800)).await;
    let call = |function_id: &'static str, payload: serde_json::Value| {
        let client = &client;
        async move {
            timeout(
                Duration::from_secs(60),
                client.trigger(TriggerRequest {
                    function_id: function_id.into(),
                    payload,
                    action: None,
                    timeout_ms: Some(50_000),
                }),
            )
            .await
            .expect("trigger timed out")
            .expect("trigger failed")
        }
    };
    let started = call(
        "browser::sessions::start",
        json!({ "url": serve_html_with_slow_api(LOGIN_HTML) }),
    )
    .await;
    let sid = started["session_id"].as_str().unwrap().to_string();

    // `email` / `password` reach "E-mail" / "Senha" through the label and
    // the field type; the run ends on the login's answer, not on "Entrando…"
    let run = call(
        "browser::run",
        json!({ "session_id": sid, "goal": "Log in.",
                "inputs": { "email": "a@b.test", "password": "pw" } }),
    )
    .await;
    assert_eq!(run["status"], "done", "{run}");
    let ops: Vec<&str> = run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["operation"].as_str().unwrap())
        .collect();
    assert_eq!(ops, ["TYPE_TEXT", "TYPE_TEXT", "CLICK"], "{run}");
    let text = run["page"]["text"].as_str().unwrap();
    assert!(text.contains("E-mail ou senha inválidos"), "{run}");
    assert!(!text.contains("Entrando"), "{run}");
    assert_eq!(run["page"]["busy"], false, "{run}");

    call("browser::sessions::stop", json!({ "session_id": sid })).await;
    client.shutdown_async().await;
}
