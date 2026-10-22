//! What a native viewer (the iii app's `RemoteScreen`) needs from the live
//! view, end to end against a real Chromium: named viewers holding the
//! screencast through a lease that lapses when the app vanishes, sharper
//! frames at a viewer's `scale` under `frame_density`, phone emulation the
//! page can actually see (touch, an Android user agent), and horizontal
//! scrolling. Each test boots its own `iii` engine and worker, like
//! `integration.rs`, and self-skips when `iii` or a Chromium is absent. The
//! Chromium is the one the worker itself would launch (`resolve_executable`:
//! a system install, the managed download, or a Playwright/Puppeteer copy).

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
            worker_cmd.env("RUST_LOG", "debug").stdout(file).stderr(err)
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

/// Repeating a named start only renews it: one stop releases it, however
/// many starts came first. A named viewer and the console's anonymous one
/// hold the screencast independently, and a stop for an id that holds
/// nothing releases nothing.
#[tokio::test]
async fn a_named_viewer_is_idempotent_and_independent_of_anonymous_ones() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let url = serve_html(ANIMATED_PAGE);
    let id = open(&client, &url).await;

    let start = |viewer: &str| json!({ "session_id": id, "viewer_id": viewer, "lease_ms": 20_000, "scale": 1.0 });
    for _ in 0..3 {
        let out = call(&client, "browser::screencast::start", start("android-a")).await;
        assert_eq!(out["ok"], true, "{out}");
    }
    assert!(screencast_active(&client, &id).await);
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "android-a" }),
    )
    .await;
    assert!(
        !screencast_active(&client, &id).await,
        "three starts by one viewer must be released by its one stop"
    );

    // The console (anonymous) and a named viewer together.
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id }),
    )
    .await;
    call(&client, "browser::screencast::start", start("android-b")).await;
    // Unknown id, then a stray anonymous stop: neither may take the named
    // viewer's slot.
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "nobody" }),
    )
    .await;
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id }),
    )
    .await;
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id }),
    )
    .await;
    assert!(
        screencast_active(&client, &id).await,
        "a stray anonymous stop took the named viewer's slot"
    );
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "android-b" }),
    )
    .await;
    assert!(!screencast_active(&client, &id).await);

    // An unusable id is refused before anything is held.
    let refused = try_call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "x".repeat(129) }),
    )
    .await;
    assert!(refused.is_err(), "{refused:?}");
    assert!(!screencast_active(&client, &id).await);

    call(
        &client,
        "browser::sessions::stop",
        json!({ "session_id": id }),
    )
    .await;
    client.shutdown_async().await;
}

/// A viewer that vanishes without a stop (an app killed or sent to the
/// background) holds the tab only until its lease lapses: the reaper drops
/// it within lease + 2 s, the screencast stops, and the tab, unwatched
/// again, goes to sleep on its idle timer. A renewed lease does not lapse.
#[tokio::test]
async fn a_lapsed_lease_stops_pinning_the_tab() {
    let Some(h) = boot("  inactive_after_ms: 2500\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let url = serve_html(ANIMATED_PAGE);
    let id = open(&client, &url).await;
    let lease = json!({ "session_id": id, "viewer_id": "android-gone", "lease_ms": 5_000 });

    // Renewed every 2 s for 8 s (longer than the lease): still held, and the
    // tab never slept although its idle time is 2.5 s.
    for _ in 0..4 {
        call(&client, "browser::screencast::start", lease.clone()).await;
        sleep(Duration::from_millis(2000)).await;
    }
    assert!(
        screencast_active(&client, &id).await,
        "a renewed lease lapsed"
    );

    // The app is gone: no more renewals, no stop.
    call(&client, "browser::screencast::start", lease.clone()).await;
    let renewed = Instant::now();
    let mut released = None;
    while renewed.elapsed() < Duration::from_secs(12) {
        if !screencast_active(&client, &id).await {
            released = Some(renewed.elapsed());
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }
    let released = released.expect("the lapsed viewer still holds the screencast after 12 s");
    assert!(
        released >= Duration::from_millis(4_500),
        "released after {released:?}, before the 5 s lease ran out"
    );
    assert!(
        released <= Duration::from_millis(7_500),
        "released after {released:?}, later than lease + 2 s (+0.5 s slack)"
    );

    // Unwatched now, the tab sleeps on its 2.5 s idle timer.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let listed = call(&client, "browser::sessions::list", json!({})).await;
        let tab = listed["sessions"]
            .as_array()
            .and_then(|tabs| tabs.iter().find(|t| t["session_id"] == id.as_str()))
            .cloned()
            .expect("tab listed");
        if tab["active"] == false {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the tab stayed awake after its viewer lapsed: {tab}"
        );
        sleep(Duration::from_millis(250)).await;
    }

    // A start after the lapse wakes the tab and joins afresh.
    call(&client, "browser::screencast::start", lease.clone()).await;
    assert!(screencast_active(&client, &id).await);
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "android-gone" }),
    )
    .await;
    assert!(!screencast_active(&client, &id).await);
    client.shutdown_async().await;
}

/// With `frame_density: 2`, a named viewer at `scale` 2 gets frames at
/// twice the viewport's CSS pixels while the payload's `width`/`height` stay
/// CSS; the console's anonymous viewer alone gets 1x; when the named viewer
/// leaves, the cast restarts at 1x. Screenshots and `act` coordinates do
/// not move: every tab keeps a device scale factor of 1.
#[tokio::test]
async fn a_viewer_scale_sharpens_frames_under_frame_density() {
    let Some(h) = boot("  frame_density: 2\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let url = serve_html(INPUT_PAGE);
    let id = open(&client, &url).await;

    // A screenshot of the fresh 1280x800 tab is 1280x800, not 2560x1600.
    let shot = call(
        &client,
        "browser::screenshot",
        json!({ "session_id": id, "format": "png" }),
    )
    .await;
    let png = shot["content"][0]["data"].as_str().expect("png data");
    assert_eq!(image_size(png), (1280, 800), "screenshot size moved");
    assert_eq!(evaluate(&client, &id, "window.devicePixelRatio").await, 1);
    // A click by coordinates lands where the CSS pixels say.
    call(
        &client,
        "browser::act",
        json!({ "session_id": id, "action": "click", "x": 100, "y": 100 }),
    )
    .await;
    let events = evaluate(&client, &id, "window.__events.join(' ')").await;
    assert!(
        events.as_str().unwrap_or_default().contains("click"),
        "the click missed the target: {events}"
    );

    // The page repaints so frames keep coming.
    evaluate(
        &client,
        &id,
        "setInterval(()=>{document.getElementById('t').style.background=\
         '#'+Math.floor(Math.random()*0xffffff).toString(16).padStart(6,'0')},50); true",
    )
    .await;
    call(
        &client,
        "browser::resize",
        json!({ "session_id": id, "width": 412, "height": 915 }),
    )
    .await;

    // The console alone: 1x.
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id }),
    )
    .await;
    let frame = frame_sized(&client, &id, 0, (412, 915)).await;
    assert_eq!(
        (frame["width"].as_u64(), frame["height"].as_u64()),
        (Some(412), Some(915))
    );

    // A phone at scale 2 joins: 2x, the payload still in CSS pixels.
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "android-hd", "scale": 2.0 }),
    )
    .await;
    let sharp = frame_sized(&client, &id, frame_seq(&frame), (824, 1830)).await;
    assert_eq!(
        (sharp["width"].as_u64(), sharp["height"].as_u64()),
        (Some(412), Some(915)),
        "the frame payload left CSS pixels: {}",
        sharp["width"]
    );

    // A renewal asking for 3 is held to frame_density (2): no change.
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "android-hd", "scale": 3.0 }),
    )
    .await;
    let held = frame_sized(&client, &id, frame_seq(&sharp), (824, 1830)).await;

    // The phone leaves: back to 1x for the console.
    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id, "viewer_id": "android-hd" }),
    )
    .await;
    frame_sized(&client, &id, frame_seq(&held), (412, 915)).await;

    // Still CSS-sized for everything else, at the new viewport.
    let shot = call(
        &client,
        "browser::screenshot",
        json!({ "session_id": id, "format": "png" }),
    )
    .await;
    let png = shot["content"][0]["data"].as_str().expect("png data");
    assert_eq!(image_size(png), (412, 915), "screenshot size moved");

    call(
        &client,
        "browser::screencast::stop",
        json!({ "session_id": id }),
    )
    .await;
    assert!(!screencast_active(&client, &id).await);
    client.shutdown_async().await;
}

/// At the default `frame_density` (1) a named viewer's `scale` above 1
/// cannot sharpen anything: frames stay CSS-sized. Below 1 it shrinks them
/// (less to send), unless the console also watches.
#[tokio::test]
async fn frame_density_one_keeps_css_frames_and_a_small_scale_shrinks_them() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let url = serve_html(ANIMATED_PAGE);
    let id = open(&client, &url).await;
    call(
        &client,
        "browser::resize",
        json!({ "session_id": id, "width": 400, "height": 800 }),
    )
    .await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "android-hd", "scale": 2.5 }),
    )
    .await;
    let frame = frame_sized(&client, &id, 0, (400, 800)).await;
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id, "viewer_id": "android-hd", "scale": 0.5 }),
    )
    .await;
    let small = frame_sized(&client, &id, frame_seq(&frame), (200, 400)).await;
    assert_eq!(small["width"], 400, "{}", small["width"]);
    // The console joins: never below 1x for it.
    call(
        &client,
        "browser::screencast::start",
        json!({ "session_id": id }),
    )
    .await;
    frame_sized(&client, &id, frame_seq(&small), (400, 800)).await;
    client.shutdown_async().await;
}

/// `resize` with `mobile: true` makes the page a phone: touch points,
/// `(pointer: coarse)`, an Android Chrome user agent of this Chromium's
/// major version with client hints, and an `act` click that arrives as a
/// tap (touch events, then the click). `mobile: false` brings back the
/// desktop page it had. A fit that is only grown under several viewers
/// keeps the mode.
#[tokio::test]
async fn mobile_resize_emulates_a_phone_the_page_can_see() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let url = serve_html(INPUT_PAGE);
    let id = open(&client, &url).await;

    let probe = "JSON.stringify({touch: navigator.maxTouchPoints, \
         coarse: matchMedia('(pointer: coarse)').matches, ua: navigator.userAgent, \
         platform: navigator.platform, mobile: navigator.userAgentData ? \
         navigator.userAgentData.mobile : null, uaPlatform: navigator.userAgentData ? \
         navigator.userAgentData.platform : null, brands: navigator.userAgentData ? \
         navigator.userAgentData.brands.map(b => b.brand + '/' + b.version).join(',') : null, \
         width: innerWidth, dpr: devicePixelRatio})";
    let read = |value: Value| -> Value {
        serde_json::from_str(value.as_str().expect("probe string")).expect("probe json")
    };
    let desktop = read(evaluate(&client, &id, probe).await);
    eprintln!("desktop page sees: {desktop}");
    let desktop_ua = desktop["ua"].as_str().expect("ua").to_string();
    let major = desktop_ua
        .split("Chrome/")
        .nth(1)
        .and_then(|v| v.split('.').next())
        .expect("chrome major")
        .to_string();
    assert!(!desktop_ua.contains("Android"), "{desktop_ua}");

    let resized = call(
        &client,
        "browser::resize",
        json!({ "session_id": id, "width": 412, "height": 915,
                "device_scale_factor": 2.625, "mobile": true, "fit": true }),
    )
    .await;
    assert_eq!(resized["mobile"], true, "{resized}");
    assert_eq!(resized["mode_changed"], true, "{resized}");
    let again = call(
        &client,
        "browser::resize",
        json!({ "session_id": id, "width": 412, "height": 915,
                "device_scale_factor": 2.625, "mobile": true, "fit": true }),
    )
    .await;
    assert_eq!(again["mode_changed"], false, "{again}");

    // The user agent reaches the page on its next load.
    call(
        &client,
        "browser::history",
        json!({ "session_id": id, "action": "reload" }),
    )
    .await;
    let phone = read(evaluate(&client, &id, probe).await);
    eprintln!("phone page sees: {phone}");
    assert!(phone["touch"].as_i64().unwrap_or(0) > 0, "{phone}");
    assert_eq!(phone["coarse"], true, "{phone}");
    let ua = phone["ua"].as_str().unwrap_or_default();
    assert!(ua.contains("Android"), "{ua}");
    assert!(ua.contains("Mobile Safari"), "{ua}");
    assert!(
        ua.contains(&format!("Chrome/{major}.")),
        "{ua} vs major {major}"
    );
    assert_eq!(phone["mobile"], true, "{phone}");
    assert_eq!(phone["uaPlatform"], "Android", "{phone}");
    assert_eq!(phone["width"], 412, "{phone}");

    // A click is a tap: touch events first, then the click.
    call(
        &client,
        "browser::act",
        json!({ "session_id": id, "action": "click", "x": 50, "y": 60 }),
    )
    .await;
    let events = evaluate(&client, &id, "window.__events.join(' ')").await;
    let events = events.as_str().unwrap_or_default().to_string();
    eprintln!("phone click events: {events}");
    let at = |name: &str| events.split(' ').position(|e| e.starts_with(name));
    let (Some(start), Some(end), Some(click)) = (at("touchstart"), at("touchend"), at("click"))
    else {
        panic!("a phone click must arrive as touchstart, touchend, click: {events}");
    };
    assert!(start < end && end < click, "{events}");
    // A double click is two taps.
    evaluate(&client, &id, "window.__events = []; true").await;
    call(
        &client,
        "browser::act",
        json!({ "session_id": id, "action": "click", "x": 50, "y": 60, "click_count": 2 }),
    )
    .await;
    let events = evaluate(&client, &id, "window.__events.join(' ')").await;
    let events = events.as_str().unwrap_or_default().to_string();
    eprintln!("phone double click events: {events}");
    assert_eq!(events.matches("touchstart").count(), 2, "{events}");

    // Several viewers: a fit may grow the viewport but not flip the mode.
    for _ in 0..2 {
        call(
            &client,
            "browser::screencast::start",
            json!({ "session_id": id }),
        )
        .await;
    }
    let grown = call(
        &client,
        "browser::resize",
        json!({ "session_id": id, "width": 1000, "height": 915, "fit": true }),
    )
    .await;
    assert_eq!(
        (grown["width"].as_u64(), grown["mobile"].as_bool()),
        (Some(1000), Some(true)),
        "{grown}"
    );
    assert_eq!(grown["mode_changed"], false, "{grown}");
    for _ in 0..2 {
        call(
            &client,
            "browser::screencast::stop",
            json!({ "session_id": id }),
        )
        .await;
    }

    // Back to desktop: the page it had before.
    let back = call(
        &client,
        "browser::resize",
        json!({ "session_id": id, "width": 1280, "height": 800 }),
    )
    .await;
    assert_eq!(back["mobile"], false, "{back}");
    assert_eq!(back["mode_changed"], true, "{back}");
    call(
        &client,
        "browser::history",
        json!({ "session_id": id, "action": "reload" }),
    )
    .await;
    let restored = read(evaluate(&client, &id, probe).await);
    eprintln!("desktop again: {restored}");
    assert_eq!(restored["ua"], desktop["ua"], "{restored}");
    assert_eq!(restored["touch"], desktop["touch"], "{restored}");
    assert_eq!(restored["coarse"], desktop["coarse"], "{restored}");
    assert_eq!(restored["mobile"], desktop["mobile"], "{restored}");
    call(
        &client,
        "browser::act",
        json!({ "session_id": id, "action": "click", "x": 50, "y": 60 }),
    )
    .await;
    let events = evaluate(&client, &id, "window.__events.join(' ')").await;
    let events = events.as_str().unwrap_or_default().to_string();
    eprintln!("desktop click events: {events}");
    assert!(events.contains("mousedown"), "{events}");
    assert!(events.contains("click"), "{events}");
    assert!(!events.contains("touchstart"), "{events}");
    client.shutdown_async().await;
}

/// `act scroll` with `delta_x` moves the page sideways (and only sideways
/// when `delta_y` is left out); `delta_y` still scrolls down.
#[tokio::test]
async fn act_scroll_moves_sideways_with_delta_x() {
    let Some(h) = boot("  max_sessions: 4\n").await else {
        eprintln!("skipping: `iii` or Chromium not available");
        return;
    };
    let client = connect(&h).await;
    let url = serve_html(WIDE_PAGE);
    let id = open(&client, &url).await;

    let scrolled = |client: &IIIClient, id: &str, want: (i64, i64)| {
        let client = client.clone();
        let id = id.to_string();
        async move {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let at = evaluate(&client, &id, "[scrollX, scrollY]").await;
                let at = (
                    at[0].as_f64().unwrap_or(-1.0),
                    at[1].as_f64().unwrap_or(-1.0),
                );
                if (at.0 - want.0 as f64).abs() < 1.0 && (at.1 - want.1 as f64).abs() < 1.0 {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "scrolled to {at:?}, wanted {want:?}"
                );
                sleep(Duration::from_millis(100)).await;
            }
        }
    };

    let out = call(
        &client,
        "browser::act",
        json!({ "session_id": id, "action": "scroll", "x": 400, "y": 400, "delta_x": 300 }),
    )
    .await;
    assert_eq!(out["ok"], true, "{out}");
    scrolled(&client, &id, (300, 0)).await;
    call(
        &client,
        "browser::act",
        json!({ "session_id": id, "action": "scroll", "x": 400, "y": 400,
                "delta_x": -100, "delta_y": 250 }),
    )
    .await;
    scrolled(&client, &id, (200, 250)).await;
    client.shutdown_async().await;
}

/// Not a check: the cost of sharp frames, for the `frame_density` decision.
/// Streams a text page at 412x915 CSS pixels from a browser launched at
/// density 1 and one at density 2 (a viewer at scale 2), and prints the
/// frame size and the bytes per frame. Run it by name with `--ignored
/// --nocapture`.
#[tokio::test]
#[ignore]
async fn measure_frame_bytes_at_density_one_and_two() {
    for density in [1.0f64, 2.0] {
        let Some(h) = boot(&format!("  frame_density: {density}\n")).await else {
            eprintln!("skipping: `iii` or Chromium not available");
            return;
        };
        let client = connect(&h).await;
        let url = serve_html(TEXT_PAGE);
        let id = open(&client, &url).await;
        call(
            &client,
            "browser::resize",
            json!({ "session_id": id, "width": 412, "height": 915,
                    "device_scale_factor": 2.625, "mobile": true, "fit": true }),
        )
        .await;
        call(
            &client,
            "browser::screencast::start",
            json!({ "session_id": id, "viewer_id": "android-measure", "scale": 2.0 }),
        )
        .await;
        let want = ((412.0 * density) as u32, (915.0 * density) as u32);
        let mut seq = frame_seq(&frame_sized(&client, &id, 0, want).await);
        let mut sizes = Vec::new();
        let began = Instant::now();
        let first_seq = seq;
        while sizes.len() < 40 {
            let frame = frame_sized(&client, &id, seq, want).await;
            seq = frame_seq(&frame);
            let data = frame["frame"].as_str().expect("frame");
            sizes.push(
                base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .expect("base64")
                    .len(),
            );
        }
        let elapsed = began.elapsed().as_secs_f64();
        sizes.sort_unstable();
        let mean = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
        eprintln!(
            "frame_density {density}: {}x{} JPEG, {} frames sampled, bytes/frame mean {:.0} \
             median {} min {} max {}, ~{:.1} frames/s pushed",
            want.0,
            want.1,
            sizes.len(),
            mean,
            sizes[sizes.len() / 2],
            sizes[0],
            sizes[sizes.len() - 1],
            (seq - first_seq) as f64 / elapsed
        );
        client.shutdown_async().await;
        drop(h);
    }
}
