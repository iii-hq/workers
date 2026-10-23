//! Adversarial checks on the native viewer's surface (the iii app's
//! `RemoteScreen`): hostile `viewer_id`s, lease and scale extremes,
//! `frame_density` caps, rapid resizes against the fresh-frame rule, `act`
//! with `ghost_cursor: false` and out-of-range coordinates, history walks
//! over `pushState` and `about:blank`, `browser::frame` cursors, and
//! sessions closed or asleep mid-operation. Each test boots its own `iii`
//! engine and worker, like `native_viewer.rs`, and self-skips when `iii` or
//! a Chromium is absent. Tests that reproduce a defect are ignored with the
//! finding number; the rest pass on the current code.

#![allow(dead_code)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
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
        // SIGTERM first so the worker closes its Chromium (see
        // integration.rs).
        // SAFETY: plain libc call on the child pid we spawned.
        unsafe {
            libc::kill(self.worker.id() as i32, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
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

/// The worker's own resolver decides, so a machine whose only Chromium is a
/// Playwright or downloaded copy runs these tests too.
fn chromium_present() -> bool {
    browser::chromium::resolve_executable(&browser::config::WorkerConfig::default()).is_some()
}

/// Boot an engine and the worker, the worker seeded with `seed` (YAML
/// under `browser:`). None when `iii` or a Chromium is missing.
async fn boot(seed: &str) -> Option<Harness> {
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
        "browser-native-viewer-{}.yaml",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(
        &config_path,
        format!(
            "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n"
        ),
    )
    .ok()?;
    let data_dir = std::env::temp_dir().join(format!(
        "browser-native-viewer-data-{}",
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

    let seed_path = data_dir.join("browser-seed.yaml");
    std::fs::write(&seed_path, format!("browser:\n{seed}")).ok()?;
    let worker_log = std::env::var_os("BROWSER_TEST_WORKER_LOG")
        .and_then(|path| std::fs::File::create(path).ok());
    let mut worker_cmd = Command::new(env!("CARGO_BIN_EXE_browser"));
    worker_cmd
        .arg("--url")
        .arg(&engine_ws)
        .arg("--config")
        .arg(&seed_path)
        .env("III_COMPOSE_DIR", &data_dir);
    match worker_log {
        Some(file) => {
            let err = file
                .try_clone()
                .ok()
                .map(Stdio::from)
                .unwrap_or(Stdio::null());
            worker_cmd
                .env(
                    "RUST_LOG",
                    std::env::var("BROWSER_TEST_WORKER_RUST_LOG")
                        .unwrap_or_else(|_| "debug".into()),
                )
                .stdout(file)
                .stderr(err)
        }
        None => worker_cmd.stdout(Stdio::null()).stderr(Stdio::null()),
    };
    let worker = match worker_cmd.spawn() {
        Ok(worker) => worker,
        Err(_) => {
            let _ = iii.kill();
            let _ = iii.wait();
            let _ = std::fs::remove_file(config_path);
            return None;
        }
    };
    sleep(Duration::from_millis(3000)).await;
    Some(Harness {
        iii,
        worker,
        engine_ws,
        config_path,
        data_dir,
    })
}

async fn connect(h: &Harness) -> IIIClient {
    let client = register_worker(&h.engine_ws, InitOptions::default());
    sleep(Duration::from_millis(500)).await;
    client
}

async fn try_call(client: &IIIClient, function_id: &str, payload: Value) -> Result<Value, String> {
    timeout(
        Duration::from_secs(60),
        client.trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(45_000),
        }),
    )
    .await
    .map_err(|_| format!("{function_id} timed out"))?
    .map_err(|e| format!("{function_id} failed: {e}"))
}

async fn call(client: &IIIClient, function_id: &str, payload: Value) -> Value {
    try_call(client, function_id, payload)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
}

/// A loopback HTTP server answering every request with `html`; lives as
/// long as the test process.
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

/// Repaints every 50 ms, so the screencast keeps producing frames.
const ANIMATED_PAGE: &str = "<!doctype html><html><head><meta name=viewport \
    content='width=device-width,initial-scale=1'></head><body style='margin:0'>\
    <div id=c style='height:100vh;background:red'></div><script>let n=0;\
    setInterval(()=>{n++;c.style.background=n%2?'red':'blue';c.textContent=n},50)\
    </script></body></html>";

/// Logs the input events it sees into `window.__events`; a fixed red
/// square at the top left is the click target.
const INPUT_PAGE: &str = "<!doctype html><html><head><meta name=viewport \
    content='width=device-width,initial-scale=1'><style>body{margin:0;height:3000px}\
    #t{position:fixed;left:0;top:0;width:200px;height:200px;background:#c33}</style>\
    </head><body><div id=t></div><script>window.__events=[];for(const type of \
    ['touchstart','touchend','pointerdown','pointerup','mousedown','mouseup','click',\
    'dblclick']){document.addEventListener(type,e=>window.__events.push(type+\
    (e.pointerType?':'+e.pointerType:'')),true)}</script></body></html>";

/// A document far larger than any viewport in both directions.
const WIDE_PAGE: &str = "<!doctype html><html><head><style>html{scroll-behavior:auto}\
    body{margin:0;width:4000px;height:4000px;background:linear-gradient(135deg,#fff,#06b)}\
    </style></head><body></body></html>";

/// Nothing on it ever repaints by itself: a tall striped page, no timers.
const STATIC_PAGE: &str = "<!doctype html><html><head><meta name=viewport \
    content='width=device-width'><style>body{margin:0}div{height:300px}</style></head>\
    <body><div style='background:#b91c1c'></div><div style='background:#1d4ed8'></div>\
    <div style='background:#047857'></div><div style='background:#a16207'></div>\
    <div style='background:#6d28d9'></div><div style='background:#0f766e'></div>\
    <div style='background:#be185d'></div><div style='background:#334155'></div>\
    </body></html>";

/// Text, a gradient and a running counter: what a phone shows most, and
/// what a denser frame makes legible.
const TEXT_PAGE: &str = "<!doctype html><html><head><meta name=viewport \
    content='width=device-width,initial-scale=1'><style>body{margin:0;font:16px/1.5 \
    sans-serif;color:#222}header{padding:16px;background:linear-gradient(90deg,#0b6,#06b);\
    color:#fff;font-size:22px}p{margin:12px 16px}</style></head><body><header>iii browser \
    <span id=n>0</span></header><p>Lorem ipsum dolor sit amet, consectetur adipiscing elit, \
    sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim \
    veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo \
    consequat.</p><p>Duis aute irure dolor in reprehenderit in voluptate velit esse cillum \
    dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in \
    culpa qui officia deserunt mollit anim id est laborum.</p><p>Sed ut perspiciatis unde \
    omnis iste natus error sit voluptatem accusantium doloremque laudantium, totam rem \
    aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae \
    dicta sunt explicabo.</p><p>Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut \
    odit aut fugit, sed quia consequuntur magni dolores eos qui ratione voluptatem sequi \
    nesciunt.</p><script>let k=0;setInterval(()=>{n.textContent=++k},50)</script></body>\
    </html>";

async fn open(client: &IIIClient, url: &str) -> String {
    let started = call(client, "browser::sessions::start", json!({ "url": url })).await;
    started["session_id"]
        .as_str()
        .expect("session_id")
        .to_string()
}

async fn evaluate(client: &IIIClient, session_id: &str, expression: &str) -> Value {
    let out = call(
        client,
        "browser::evaluate",
        json!({ "session_id": session_id, "expression": expression }),
    )
    .await;
    assert_eq!(out["ok"], true, "{expression}: {out}");
    out["value"].clone()
}

async fn screencast_active(client: &IIIClient, session_id: &str) -> bool {
    call(
        client,
        "browser::frame",
        json!({ "session_id": session_id }),
    )
    .await["active"]
        == true
}

/// Pixel size of a base64 JPEG/PNG.
fn image_size(base64: &str) -> (u32, u32) {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .expect("base64 frame");
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .expect("image format")
        .into_dimensions()
        .expect("image dimensions")
}

/// A frame newer than `after` whose JPEG is `want` pixels (±1 for the
/// rounding of the cap), within a few seconds. Returns its payload.
async fn frame_sized(client: &IIIClient, session_id: &str, after: u64, want: (u32, u32)) -> Value {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut last = (0, 0);
    loop {
        let frame = call(
            client,
            "browser::frame",
            json!({ "session_id": session_id }),
        )
        .await;
        if let (Some(data), Some(seq)) = (frame["frame"].as_str(), frame["frame_seq"].as_u64()) {
            if seq > after {
                last = image_size(data);
                if last.0.abs_diff(want.0) <= 1 && last.1.abs_diff(want.1) <= 1 {
                    return frame;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "no {}x{} frame after seq {after}; the newest was {}x{}",
            want.0,
            want.1,
            last.0,
            last.1
        );
        sleep(Duration::from_millis(100)).await;
    }
}

fn frame_seq(frame: &Value) -> u64 {
    frame["frame_seq"].as_u64().expect("frame_seq")
}


/// `browser::frame` for a viewer, with an optional `since_frame` cursor.
async fn frame_now(client: &IIIClient, session_id: &str, since: Option<Value>) -> Value {
    let mut payload = json!({ "session_id": session_id });
    if let Some(since) = since {
        payload["since_frame"] = since;
    }
    call(client, "browser::frame", payload).await
}

/// A frame newer than `after` whose JPEG is `want` pixels (±1), within
/// `within`. The error names the newest size seen.
async fn await_frame(
    client: &IIIClient,
    session_id: &str,
    after: u64,
    want: (u32, u32),
    within: Duration,
) -> Result<Value, String> {
    let deadline = Instant::now() + within;
    let mut last = (0, 0);
    let mut payload = (0u64, 0u64);
    loop {
        let frame = frame_now(client, session_id, None).await;
        if let (Some(data), Some(seq)) = (frame["frame"].as_str(), frame["frame_seq"].as_u64()) {
            if seq > after {
                last = image_size(data);
                payload = (
                    frame["width"].as_u64().unwrap_or(0),
                    frame["height"].as_u64().unwrap_or(0),
                );
                if last.0.abs_diff(want.0) <= 1 && last.1.abs_diff(want.1) <= 1 {
                    return Ok(frame);
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "no {}x{} frame after seq {after} within {within:?}; the newest (seq > {after}) has JPEG {}x{} and payload {}x{}",
                want.0, want.1, last.0, last.1, payload.0, payload.1
            ));
        }
        sleep(Duration::from_millis(50)).await;
    }
}

fn seq_of(frame: &Value) -> u64 {
    frame["frame_seq"].as_u64().unwrap_or(0)
}

async fn resize_to(client: &IIIClient, session_id: &str, width: i64, height: i64) -> Value {
    call(
        client,
        "browser::resize",
        json!({ "session_id": session_id, "width": width, "height": height }),
    )
    .await
}

/// Whether the tab itself is awake (`browser::sessions::list`).
async fn tab_active(client: &IIIClient, session_id: &str) -> bool {
    let listed = call(client, "browser::sessions::list", json!({})).await;
    listed["sessions"]
        .as_array()
        .and_then(|tabs| tabs.iter().find(|t| t["session_id"] == session_id))
        .map(|t| t["active"] == true)
        .unwrap_or(false)
}

/// Viewer ids that must be refused, and the ones that must be held apart.
/// Empty and whitespace-only ids are refused; ids are counted in characters
/// after trimming; distinct ids stay distinct; the console's anonymous
/// viewer and a named viewer called "anonymous" hold separate slots.
#[tokio::test]
async fn hostile_viewer_ids_are_refused_or_held_exactly() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;
    let start = |viewer: &str| json!({ "session_id": id, "viewer_id": viewer, "lease_ms": 20_000 });

    for bad in ["", " ", "\t\n", "\u{a0}"] {
        let r = try_call(&client, "browser::screencast::start", start(bad)).await;
        assert!(r.is_err(), "viewer_id {bad:?} was accepted: {r:?}");
    }
    let too_long = "v".repeat(129);
    let r = try_call(&client, "browser::screencast::start", start(&too_long)).await;
    assert!(r.is_err(), "a 129-character viewer_id was accepted");
    assert!(!screencast_active(&client, &id).await, "a refused viewer took the screencast");

    // Accepted: padding is trimmed away, 128 multi-byte characters fit.
    let padded = format!("  {}  ", "v".repeat(128));
    let accented = "é".repeat(128);
    let emoji = "🎥".repeat(128);
    for viewer in [padded.as_str(), accented.as_str(), emoji.as_str()] {
        call(&client, "browser::screencast::start", start(viewer)).await;
    }
    // A zero-width space is not whitespace: a different viewer.
    let zws = "android-a\u{200b}";
    call(&client, "browser::screencast::start", start("android-a")).await;
    call(&client, "browser::screencast::start", start(zws)).await;
    call(&client, "browser::screencast::start", start("anonymous")).await;
    call(&client, "browser::screencast::start", json!({ "session_id": id })).await;

    for viewer in [padded.as_str(), accented.as_str(), emoji.as_str(), "android-a", "anonymous"] {
        call(
            &client,
            "browser::screencast::stop",
            json!({ "session_id": id, "viewer_id": viewer }),
        )
        .await;
    }
    assert!(
        screencast_active(&client, &id).await,
        "stopping the other viewers released the zero-width-space viewer's or the console's hold"
    );
    call(&client, "browser::screencast::stop", json!({ "session_id": id })).await;
    assert!(
        screencast_active(&client, &id).await,
        "the zero-width-space viewer still holds the screencast"
    );
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": zws }),
    )
    .await;
    assert!(!screencast_active(&client, &id).await, "a viewer's hold leaked");
    client.shutdown_async().await;
}

/// `lease_ms` is held to 5 s..120 s: zero lapses at the 5 s floor, a huge
/// value does not overflow into an instant lapse, and a negative value is
/// refused outright.
#[tokio::test]
async fn lease_extremes_are_held_to_the_bounds() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;

    let refused = try_call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "neg", "lease_ms": -1 }),
    )
    .await;
    assert!(refused.is_err(), "a negative lease was accepted: {refused:?}");
    assert!(!screencast_active(&client, &id).await);

    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "zero", "lease_ms": 0 }),
    )
    .await;
    let began = Instant::now();
    let mut released = None;
    while began.elapsed() < Duration::from_secs(12) {
        if !screencast_active(&client, &id).await {
            released = Some(began.elapsed());
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }
    let released = released.expect("a zero lease never lapsed");
    assert!(
        released >= Duration::from_millis(4_500) && released <= Duration::from_millis(7_500),
        "a zero lease lapsed after {released:?}, outside the 5 s floor + 2 s reaper"
    );

    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "max", "lease_ms": u64::MAX }),
    )
    .await;
    sleep(Duration::from_secs(8)).await;
    assert!(
        screencast_active(&client, &id).await,
        "a u64::MAX lease lapsed before its 120 s cap"
    );
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "max" }),
    )
    .await;
    assert!(!screencast_active(&client, &id).await);
    client.shutdown_async().await;
}

/// `scale` is clamped to 0.5..3 and held to `frame_density`. Infinite,
/// negative, zero and absent scales all resolve to a sane density; renewals
/// move the stream both ways.
#[tokio::test]
async fn scale_extremes_are_clamped_and_renewals_move_both_ways() {
    let Some(h) = boot("  frame_density: 3\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;
    resize_to(&client, &id, 400, 800).await;
    let start = |scale: Value| {
        json!({ "session_id": id, "viewer_id": "scales", "lease_ms": 20_000, "scale": scale })
    };

    call(&client, "browser::screencast::start", start(json!(1e308))).await;
    let frame = await_frame(&client, &id, 0, (1200, 2400), Duration::from_secs(10))
        .await
        .unwrap_or_else(|e| panic!("scale 1e308: {e}"));
    assert_eq!(frame["width"], 400, "the payload left CSS pixels");

    call(&client, "browser::screencast::start", start(json!(-5.0))).await;
    let small = await_frame(&client, &id, seq_of(&frame), (200, 400), Duration::from_secs(10))
        .await
        .unwrap_or_else(|e| panic!("scale -5: {e}"));

    call(&client, "browser::screencast::start", start(json!(0.0))).await;
    await_frame(&client, &id, seq_of(&small), (200, 400), Duration::from_secs(10))
        .await
        .unwrap_or_else(|e| panic!("scale 0: {e}"));

    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "scales", "lease_ms": 20_000 }),
    )
    .await;
    let back = await_frame(&client, &id, seq_of(&small), (400, 800), Duration::from_secs(10))
        .await
        .unwrap_or_else(|e| panic!("an absent scale is 1x: {e}"));
    assert_eq!(back["width"], 400);
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "scales" }),
    )
    .await;
    client.shutdown_async().await;
}

/// The largest cap the worker can ask for (`frame_density` 3, a 4000x2000
/// viewport, scale 3) still streams: a 12000x6000 frame arrives, its payload
/// still 4000x2000 CSS pixels.
#[tokio::test]
async fn frame_density_three_at_the_largest_viewport_still_streams() {
    let Some(h) = boot("  frame_density: 3\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;
    resize_to(&client, &id, 4000, 2000).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "huge", "scale": 3.0 }),
    )
    .await;
    let frame = await_frame(&client, &id, 0, (12000, 6000), Duration::from_secs(40))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        (frame["width"].as_u64(), frame["height"].as_u64()),
        (Some(4000), Some(2000)),
        "the payload left CSS pixels"
    );
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "huge" }),
    )
    .await;
    client.shutdown_async().await;
}

/// `browser::resize` clamps the viewport to 200..4000 and the device scale
/// factor to 0.5..3 and answers the size it applied. Negative and fractional
/// sizes are refused, not wrapped.
#[tokio::test]
async fn resize_extremes_clamp_and_answer_what_they_applied() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(STATIC_PAGE)).await;

    for (w, h2, want_w, want_h) in [
        (0i64, 0i64, 200u64, 200u64),
        (1, 99_999, 200, 4000),
        (4_294_967_295, 4_294_967_295, 4000, 4000),
    ] {
        let out = resize_to(&client, &id, w, h2).await;
        assert_eq!(
            (out["width"].as_u64(), out["height"].as_u64()),
            (Some(want_w), Some(want_h)),
            "{w}x{h2}: {out}"
        );
    }
    for bad in [
        json!({ "session_id": id, "width": -1, "height": 600 }),
        json!({ "session_id": id, "width": 412.7, "height": 600 }),
    ] {
        let r = try_call(&client, "browser::resize", bad.clone()).await;
        assert!(r.is_err(), "{bad} was accepted: {r:?}");
    }
    for (dpr, want) in [(0.0f64, 0.5f64), (-3.0, 0.5), (1e9, 3.0)] {
        call(
            &client,
            "browser::resize",
            json!({ "session_id": id, "width": 412, "height": 600, "device_scale_factor": dpr }),
        )
        .await;
        let seen = evaluate(&client, &id, "window.devicePixelRatio").await;
        assert_eq!(seen.as_f64(), Some(want), "device_scale_factor {dpr}");
    }
    client.shutdown_async().await;
}

/// Resizes fired back to back, sampled by `browser::frame` while they land.
/// Every frame's JPEG must match the size its payload names, `frame_seq`
/// never goes back, one seq never names two sizes, and the last size arrives.
/// The sizes include same-aspect pairs (412x860 and 824x1720), which an
/// aspect-only check cannot tell apart.
#[tokio::test]
async fn rapid_resizes_publish_no_mismatched_frame_and_no_decreasing_seq() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(STATIC_PAGE)).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "rapid" }),
    )
    .await;
    let sizes: [(i64, i64); 10] = [
        (412, 860),
        (824, 1720),
        (412, 860),
        (400, 800),
        (800, 1600),
        (412, 600),
        (1000, 700),
        (412, 860),
        (824, 1720),
        (300, 600),
    ];
    let mut seen: Vec<(u64, u64, u64, (u32, u32))> = Vec::new();
    for (w, h2) in sizes {
        resize_to(&client, &id, w, h2).await;
        for _ in 0..5 {
            let frame = frame_now(&client, &id, None).await;
            if let Some(data) = frame["frame"].as_str() {
                seen.push((
                    seq_of(&frame),
                    frame["width"].as_u64().unwrap_or(0),
                    frame["height"].as_u64().unwrap_or(0),
                    image_size(data),
                ));
            }
            sleep(Duration::from_millis(60)).await;
        }
    }
    let mut last_seq = 0;
    let mut size_of_seq: std::collections::HashMap<u64, (u64, u64)> = Default::default();
    for (seq, pw, ph, (jw, jh)) in &seen {
        assert!(
            jw.abs_diff(*pw as u32) <= 1 && jh.abs_diff(*ph as u32) <= 1,
            "frame seq {seq} names {pw}x{ph} but its JPEG is {jw}x{jh}"
        );
        assert!(*seq >= last_seq, "frame_seq went back: {last_seq} then {seq}");
        last_seq = *seq;
        let named = size_of_seq.entry(*seq).or_insert((*pw, *ph));
        assert_eq!(*named, (*pw, *ph), "frame seq {seq} named two sizes");
    }
    await_frame(&client, &id, 0, (300, 600), Duration::from_secs(3))
        .await
        .unwrap_or_else(|e| panic!("the last resize never showed: {e}"));
    client.shutdown_async().await;
}

/// On a page that never repaints, every resize brings a frame of the new
/// size within 1.5 s: Chromium's own frame or the worker's capture (D28).
#[tokio::test]
#[ignore = "finding 1: a resize on a static page publishes a frame whose payload names the new viewport but whose JPEG is the old size, and no correct frame follows within 1.5 s (intermittent)"]
async fn a_resize_over_a_static_page_shows_within_one_and_a_half_seconds() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(STATIC_PAGE)).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "timed" }),
    )
    .await;
    let mut seq = seq_of(&await_frame(&client, &id, 0, (1280, 800), Duration::from_secs(8)).await.expect("first frame"));
    for (w, h2) in [(412, 600), (412, 860), (800, 700), (419, 337), (300, 900)] {
        let before = seq;
        resize_to(&client, &id, w, h2).await;
        let shown = Instant::now();
        let frame = await_frame(
            &client,
            &id,
            before,
            (w as u32, h2 as u32),
            Duration::from_millis(1_500),
        )
        .await
        .unwrap_or_else(|e| panic!("{w}x{h2}: {e}"));
        eprintln!("{w}x{h2} shown after {} ms", shown.elapsed().as_millis());
        seq = seq_of(&frame);
    }
    client.shutdown_async().await;
}

/// `frame_density` 2 with a viewer at scale 2 on a phone (dpr 2.625): each
/// resize streams a frame twice the CSS viewport, its payload in CSS pixels,
/// whether Chromium or the worker's capture supplies it.
#[tokio::test]
async fn density_two_frames_track_each_phone_resize() {
    let Some(h) = boot("  frame_density: 2\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(STATIC_PAGE)).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "hd", "scale": 2.0 }),
    )
    .await;
    for (w, h2) in [(412i64, 860i64), (300, 600), (1000, 700)] {
        let before = seq_of(&frame_now(&client, &id, None).await);
        call(
            &client,
            "browser::resize",
            json!({ "session_id": id, "width": w, "height": h2,
                    "device_scale_factor": 2.625, "mobile": true }),
        )
        .await;
        let frame = await_frame(
            &client,
            &id,
            before,
            ((w * 2) as u32, (h2 * 2) as u32),
            Duration::from_secs(8),
        )
        .await
        .unwrap_or_else(|e| panic!("{w}x{h2} at density 2: {e}"));
        assert_eq!(
            (frame["width"].as_u64(), frame["height"].as_u64()),
            (Some(w as u64), Some(h2 as u64)),
            "the payload left CSS pixels"
        );
    }
    client.shutdown_async().await;
}

/// `click_count` is not bounded: a count of a million has to come back
/// within 20 s, or it is a worker stuck dispatching clicks.
#[tokio::test]
#[ignore = "finding 2: act click_count is unbounded; a count of 1000000 is still dispatching after 20 s"]
async fn act_click_count_is_bounded() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(INPUT_PAGE)).await;
    let out = timeout(
        Duration::from_secs(20),
        try_call(
            &client,
            "browser::act",
            json!({ "session_id": id, "action": "click", "x": 50, "y": 60, "click_count": 1_000_000 }),
        ),
    )
    .await;
    let answered = out.unwrap_or_else(|_| {
        panic!("a click_count of 1000000 was still dispatching after 20 s")
    });
    answered.unwrap_or_else(|e| panic!("{e}"));
}

/// With `ghost_cursor: false` no pointer action draws the agent's cursor:
/// click (left and right), hover, drag and scroll, on the desktop and as a
/// phone tap. A positive control (the default) still draws it. A wrong-typed
/// `ghost_cursor` is refused.
#[tokio::test]
async fn ghost_cursor_false_never_injects_for_any_pointer_action() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(INPUT_PAGE)).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "ghost" }),
    )
    .await;
    let cursor = "!!document.getElementById('iii-ghost-cursor')";
    let quiet = [
        json!({ "session_id": id, "action": "click", "x": 50, "y": 60, "ghost_cursor": false }),
        json!({ "session_id": id, "action": "click", "x": 50, "y": 60, "button": "right", "ghost_cursor": false }),
        json!({ "session_id": id, "action": "hover", "x": 80, "y": 90, "ghost_cursor": false }),
        json!({ "session_id": id, "action": "drag", "x": 20, "y": 30, "x2": 300, "y2": 400, "ghost_cursor": false }),
        json!({ "session_id": id, "action": "scroll", "x": 50, "y": 60, "delta_y": 40, "ghost_cursor": false }),
    ];
    for action in &quiet {
        call(&client, "browser::act", action.clone()).await;
        assert_eq!(evaluate(&client, &id, cursor).await, false, "{action} drew the agent cursor");
    }

    call(
        &client,
        "browser::resize",
        json!({ "session_id": id, "width": 412, "height": 915, "device_scale_factor": 2.625,
                "mobile": true }),
    )
    .await;
    for action in [
        json!({ "session_id": id, "action": "click", "x": 50, "y": 60, "ghost_cursor": false }),
        json!({ "session_id": id, "action": "drag", "x": 20, "y": 30, "x2": 200, "y2": 300, "ghost_cursor": false }),
    ] {
        call(&client, "browser::act", action.clone()).await;
        assert_eq!(evaluate(&client, &id, cursor).await, false, "phone {action} drew the cursor");
    }

    let wrong_type = try_call(
        &client,
        "browser::act",
        json!({ "session_id": id, "action": "click", "x": 50, "y": 60, "ghost_cursor": "no" }),
    )
    .await;
    assert!(wrong_type.is_err(), "a string ghost_cursor was accepted: {wrong_type:?}");

    call(
        &client,
        "browser::act",
        json!({ "session_id": id, "action": "hover", "x": 80, "y": 90 }),
    )
    .await;
    assert_eq!(evaluate(&client, &id, cursor).await, true, "the default lost its agent cursor");
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "ghost" }),
    )
    .await;
    client.shutdown_async().await;
}

/// Coordinates far outside the viewport, negative, or huge wheel deltas must
/// each return (ok or a refusal) and leave the worker answering: a page
/// evaluation still works afterwards and no call hangs.
#[tokio::test]
#[ignore = "finding 4: a scroll with a wheel delta of 1e300 fails with a CDP request timeout after about 30 s"]
async fn act_coordinates_outside_the_viewport_return_and_leave_the_worker_answering() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(INPUT_PAGE)).await;
    let cases = [
        json!({ "session_id": id, "action": "click", "x": -1e6, "y": -1e6 }),
        json!({ "session_id": id, "action": "click", "x": 1e7, "y": 1e7 }),
        json!({ "session_id": id, "action": "hover", "x": -5, "y": -5 }),
        json!({ "session_id": id, "action": "drag", "x": -10, "y": -10, "x2": 1e6, "y2": 1e6 }),
        json!({ "session_id": id, "action": "scroll", "x": -1, "y": -1, "delta_y": 1e300 }),
        json!({ "session_id": id, "action": "scroll", "x": 5, "y": 5, "delta_x": -1e300, "delta_y": 1e300 }),
    ];
    for case in cases {
        let out = try_call(&client, "browser::act", case.clone()).await;
        assert!(
            !format!("{out:?}").contains("timed out"),
            "{case} did not return: {out:?}"
        );
    }
    assert_eq!(evaluate(&client, &id, "1 + 1").await, 2, "the worker stopped answering");
    client.shutdown_async().await;
}

/// `pushState` entries are real history: back and forward walk them in
/// order, and the page after the last back is never `about:blank`.
#[tokio::test]
async fn history_walks_pushstate_entries_in_order_and_never_blank() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let a = serve_html(STATIC_PAGE);
    let b = serve_html(INPUT_PAGE);
    let a_one = format!("{a}one");
    let id = open(&client, &a).await;
    let pushed = evaluate(&client, &id, "history.pushState({}, '', '/one'); location.href").await;
    assert_eq!(pushed, a_one.as_str(), "pushState did not change the url");
    call(
        &client,
        "browser::navigate",
        json!({ "session_id": id, "url": b }),
    )
    .await;
    let steps: [(&str, bool, &str); 6] = [
        ("back", true, a_one.as_str()),
        ("back", true, a.as_str()),
        ("back", false, a.as_str()),
        ("forward", true, a_one.as_str()),
        ("forward", true, b.as_str()),
        ("forward", false, b.as_str()),
    ];
    for (action, moved, url) in steps {
        let out = call(
            &client,
            "browser::history",
            json!({ "session_id": id, "action": action }),
        )
        .await;
        assert_eq!(
            (out["moved"].as_bool(), out["url"].as_str()),
            (Some(moved), Some(url)),
            "{action}: {out}"
        );
    }
    client.shutdown_async().await;
}

/// A page that sends itself to `about:blank` is one entry Back and Forward
/// step over: from B, Back lands on A and Forward on B, never on the blank.
#[tokio::test]
async fn history_steps_over_a_blank_the_page_sent_itself_to() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let a = serve_html(STATIC_PAGE);
    let b = serve_html(INPUT_PAGE);
    let id = open(&client, &a).await;
    evaluate(&client, &id, "location.href = 'about:blank'; true").await;
    sleep(Duration::from_millis(1_000)).await;
    call(
        &client,
        "browser::navigate",
        json!({ "session_id": id, "url": b }),
    )
    .await;
    let steps: [(&str, bool, &str); 4] = [
        ("back", true, a.as_str()),
        ("forward", true, b.as_str()),
        ("back", true, a.as_str()),
        ("back", false, a.as_str()),
    ];
    for (action, moved, url) in steps {
        let out = call(
            &client,
            "browser::history",
            json!({ "session_id": id, "action": action }),
        )
        .await;
        assert_eq!(
            (out["moved"].as_bool(), out["url"].as_str()),
            (Some(moved), Some(url)),
            "{action}: {out}"
        );
    }
    client.shutdown_async().await;
}

/// Back and Forward in quick succession over a history with a blank in it
/// never answer `about:blank` and never fail.
#[tokio::test]
async fn rapid_history_moves_over_a_blank_never_show_it() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let a = serve_html(STATIC_PAGE);
    let b = serve_html(INPUT_PAGE);
    let c = serve_html(ANIMATED_PAGE);
    let id = open(&client, &a).await;
    evaluate(&client, &id, "location.href = 'about:blank'; true").await;
    sleep(Duration::from_millis(1_000)).await;
    for url in [&b, &c] {
        call(
            &client,
            "browser::navigate",
            json!({ "session_id": id, "url": url }),
        )
        .await;
    }
    let order = [
        "back", "back", "back", "forward", "forward", "forward", "back", "forward", "back", "back",
    ];
    for action in order {
        let out = call(
            &client,
            "browser::history",
            json!({ "session_id": id, "action": action }),
        )
        .await;
        let url = out["url"].as_str().unwrap_or_default();
        assert!(
            url == a || url == b || url == c,
            "{action} answered {url:?}, not one of the pages"
        );
    }
    client.shutdown_async().await;
}

/// A tab that slept between two pages is woken, and Back returns to the
/// earlier page it remembers (the tab's own stack, not Chromium's).
#[tokio::test]
async fn back_after_the_tab_slept_returns_to_the_remembered_page() {
    let Some(h) = boot("  inactive_after_ms: 2500\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let a = serve_html(STATIC_PAGE);
    let b = serve_html(INPUT_PAGE);
    let id = open(&client, &a).await;
    call(
        &client,
        "browser::navigate",
        json!({ "session_id": id, "url": b }),
    )
    .await;
    sleep(Duration::from_millis(4_000)).await;
    assert!(!tab_active(&client, &id).await, "the idle tab did not sleep");
    let out = call(
        &client,
        "browser::history",
        json!({ "session_id": id, "action": "back" }),
    )
    .await;
    assert_eq!(
        (out["moved"].as_bool(), out["url"].as_str()),
        (Some(true), Some(a.as_str())),
        "{out}"
    );
    client.shutdown_async().await;
}

/// `since_frame` at the current seq answers with no frame at all, a seq far
/// ahead or u64::MAX answers with the newest frame, a null cursor answers
/// with the newest frame, and negative, non-numeric or fractional cursors
/// are refused.
#[tokio::test]
async fn since_frame_extremes_answer_or_refuse_without_hanging() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(STATIC_PAGE)).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "since" }),
    )
    .await;
    let first = await_frame(&client, &id, 0, (1280, 800), Duration::from_secs(8))
        .await
        .expect("first frame");
    let seq = seq_of(&first);

    let same = frame_now(&client, &id, Some(json!(seq))).await;
    assert!(
        same["frame"].is_null() || seq_of(&same) > seq,
        "the current seq sent its frame again: {}",
        seq_of(&same)
    );
    for cursor in [json!(seq + 1000), json!(u64::MAX), Value::Null] {
        let out = frame_now(&client, &id, Some(cursor.clone())).await;
        assert!(!out["frame"].is_null(), "cursor {cursor} got no frame: {out}");
    }
    for bad in [json!(-1), json!("abc"), json!(1.5)] {
        let r = try_call(
            &client,
            "browser::frame",
            json!({ "session_id": id, "since_frame": bad.clone() }),
        )
        .await;
        assert!(r.is_err(), "since_frame {bad} was accepted: {r:?}");
    }
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "since" }),
    )
    .await;
    client.shutdown_async().await;
}

/// A session stopped under a live viewer: the viewer's stop still answers,
/// the frame of the gone session is not active, and a later start answers
/// (refused or woken), never a hang.
#[tokio::test]
async fn a_viewer_of_a_closed_session_gets_answers_not_hangs() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "orphan" }),
    )
    .await;
    call(&client, "browser::sessions::stop", json!({ "session_id": id })).await;

    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "orphan" }),
    )
    .await;
    let frame = try_call(&client, "browser::frame", json!({ "session_id": id })).await;
    if let Ok(frame) = &frame {
        assert_ne!(frame["active"], true, "a closed session reports an active screencast");
    }
    let start = try_call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "orphan" }),
    )
    .await;
    assert!(
        !format!("{start:?}").contains("timed out"),
        "a start on a closed session hung: {start:?}"
    );
    if start.is_ok() {
        call(
            &client,
            "browser::screencast::stop",
            json!({ "session_id": id, "viewer_id": "orphan" }),
        )
        .await;
    }
    client.shutdown_async().await;
}

/// Resizes and a session stop fired together: every call returns within 40 s.
#[tokio::test]
async fn a_stop_during_a_resize_storm_returns() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "storm" }),
    )
    .await;
    let mut tasks = Vec::new();
    for i in 0..15i64 {
        let client = client.clone();
        let id = id.clone();
        tasks.push(tokio::spawn(async move {
            try_call(
                &client,
                "browser::resize",
                json!({ "session_id": id, "width": 300 + i * 20, "height": 600 + i * 10 }),
            )
            .await
        }));
    }
    let stop = {
        let client = client.clone();
        let id = id.clone();
        tokio::spawn(async move {
            try_call(&client, "browser::sessions::stop", json!({ "session_id": id })).await
        })
    };
    let all = timeout(Duration::from_secs(40), async {
        for task in tasks {
            let _ = task.await;
        }
        let _ = stop.await;
    })
    .await;
    assert!(all.is_ok(), "the resize and stop storm did not return within 40 s");
    client.shutdown_async().await;
}

async fn storm_viewer(client: IIIClient, id: String, viewer: String) -> Result<(), String> {
    for _ in 0..3 {
        try_call(
            &client,
            "browser::screencast::start",
            json!({ "session_id": id, "viewer_id": viewer, "lease_ms": 20_000 }),
        )
        .await?;
    }
    try_call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": viewer }),
    )
    .await
    .map(|_| ())
}

async fn storm_anonymous(client: IIIClient, id: String) -> Result<(), String> {
    try_call(&client, "browser::screencast::start", json!({ "session_id": id })).await?;
    try_call(&client, "browser::screencast::stop", json!({ "session_id": id }))
        .await
        .map(|_| ())
}

/// Thirty named viewers, each renewing three times and stopping, run
/// alongside ten anonymous start/stop pairs. Afterwards nothing holds the
/// screencast, and a fresh anonymous viewer still starts and stops it.
#[tokio::test]
async fn a_viewer_storm_leaves_no_consumer_behind() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;
    let mut tasks = Vec::new();
    for v in 0..30 {
        tasks.push(tokio::spawn(storm_viewer(client.clone(), id.clone(), format!("storm-{v}"))));
    }
    for _ in 0..10 {
        tasks.push(tokio::spawn(storm_anonymous(client.clone(), id.clone())));
    }
    for task in tasks {
        task.await
            .expect("storm task")
            .unwrap_or_else(|e| panic!("a storm call failed: {e}"));
    }
    assert!(
        !screencast_active(&client, &id).await,
        "a storm left a consumer holding the screencast"
    );
    call(&client, "browser::screencast::start", json!({ "session_id": id })).await;
    assert!(screencast_active(&client, &id).await);
    call(&client, "browser::screencast::stop", json!({ "session_id": id })).await;
    assert!(!screencast_active(&client, &id).await);
    client.shutdown_async().await;
}

/// A lease lapsing in the same instant as the viewer's own stop, three times
/// over: the screencast is released once, never leaked, and the next viewer
/// starts and stops cleanly each round.
#[tokio::test]
async fn a_lease_lapsing_as_its_stop_arrives_releases_once() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;
    for round in 0..3u64 {
        call(
            &client,
            "browser::screencast::start",
            json!({ "session_id": id, "viewer_id": "race", "lease_ms": 5_000 }),
        )
        .await;
        sleep(Duration::from_millis(5_000 + round * 150)).await;
        call(
            &client,
            "browser::screencast::stop",
            json!({ "session_id": id, "viewer_id": "race" }),
        )
        .await;
        let deadline = Instant::now() + Duration::from_secs(2);
        while screencast_active(&client, &id).await {
            assert!(Instant::now() < deadline, "round {round}: a lapsed-and-stopped viewer leaked the screencast");
            sleep(Duration::from_millis(100)).await;
        }
        call(&client, "browser::screencast::start", json!({ "session_id": id })).await;
        assert!(screencast_active(&client, &id).await, "round {round}: the console could not start");
        call(&client, "browser::screencast::stop", json!({ "session_id": id })).await;
        assert!(!screencast_active(&client, &id).await, "round {round}: the console's stop left it running");
    }
    client.shutdown_async().await;
}

/// Two named viewers at scales 2 and 1.5 on a `frame_density` 2 tab: the
/// largest drives the stream; each leave drops it to the next, and the last
/// leave returns to CSS-sized frames.
#[tokio::test]
async fn the_largest_scale_drives_the_stream_and_each_leave_drops_it() {
    let Some(h) = boot("  frame_density: 2\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;
    resize_to(&client, &id, 400, 800).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "A", "scale": 2.0 }),
    )
    .await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "B", "scale": 1.5 }),
    )
    .await;
    let both = await_frame(&client, &id, 0, (800, 1600), Duration::from_secs(8))
        .await
        .unwrap_or_else(|e| panic!("two viewers: {e}"));
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "A" }),
    )
    .await;
    let mid = await_frame(&client, &id, seq_of(&both), (600, 1200), Duration::from_secs(8))
        .await
        .unwrap_or_else(|e| panic!("after A left: {e}"));
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "B" }),
    )
    .await;
    // Nobody watches, so Chromium sends nothing: the cast is off until the
    // next viewer joins, which then streams at the density it asks for.
    assert!(!screencast_active(&client, &id).await, "B's stop left the screencast running");
    call(&client, "browser::screencast::start", json!({ "session_id": id })).await;
    await_frame(&client, &id, seq_of(&mid), (400, 800), Duration::from_secs(8))
        .await
        .unwrap_or_else(|e| panic!("after B left and the console joined: {e}"));
    client.shutdown_async().await;
}

/// The session-status badge describes the watching human. A named viewer
/// leaving while the console still watches must not take the badge away.
#[tokio::test]
#[ignore = "finding 5: stopping a named viewer removes the session badge while the console still watches"]
async fn the_badge_stays_while_a_console_still_watches() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;
    call(&client, "browser::screencast::start", json!({ "session_id": id })).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "badge" }),
    )
    .await;
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "badge" }),
    )
    .await;
    assert!(screencast_active(&client, &id).await, "the console's hold went");
    assert_eq!(
        evaluate(&client, &id, "!!document.getElementById('iii-session-badge')").await,
        true,
        "the badge left while the console still watches"
    );
    client.shutdown_async().await;
}

/// A tab that idles to sleep between viewers is woken by the next start,
/// which answers ok and holds the screencast, round after round.
#[tokio::test]
#[ignore = "finding 3: a tab woken by a viewer stays awake after the viewer stops, past its 2.5 s idle timer"]
async fn a_viewer_joining_a_tab_asleep_from_idle_wakes_it() {
    let Some(h) = boot("  inactive_after_ms: 2500\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let id = open(&client, &serve_html(ANIMATED_PAGE)).await;
    for round in 0..4 {
        call(
            &client,
            "browser::screencast::start",
            json!({ "session_id": id, "viewer_id": "idle" }),
        )
        .await;
        call(
            &client,
            "browser::screencast::stop",
            json!({ "session_id": id, "viewer_id": "idle" }),
        )
        .await;
        let deadline = Instant::now() + Duration::from_secs(8);
        while tab_active(&client, &id).await {
            assert!(
                Instant::now() < deadline,
                "round {round}: the tab stayed awake 8 s after its viewer stopped (idle 2.5 s)"
            );
            sleep(Duration::from_millis(250)).await;
        }
        let started = try_call(
            &client,
            "browser::screencast::start",
            json!({ "session_id": id, "viewer_id": "idle" }),
        )
        .await;
        assert!(started.is_ok(), "round {round}: a start on a sleeping tab failed: {started:?}");
        assert!(screencast_active(&client, &id).await, "round {round}: the woken tab is not streaming");
    }
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "idle" }),
    )
    .await;
    client.shutdown_async().await;
}
