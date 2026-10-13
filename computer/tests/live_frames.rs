//! Live end-to-end check of the frame pipeline against a running engine:
//! the real `computer` binary, a fake guest-executor desktop, and consumer
//! workers bound to `computer::frame-changed`. Opt-in (needs an engine):
//!
//! ```sh
//! COMPUTER_LIVE_III_URL=ws://127.0.0.1:49651 COMPUTER_LIVE_DESKTOP_PORT=49652 \
//!   cargo test --test live_frames -- --ignored --nocapture
//! ```
//!
//! The engine needs only `iii-worker-manager` and `configuration`; it must
//! NOT run iii-stream (the test asserts `stream::set` is absent).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

const FRAME_BYTES: usize = 96 * 1024;

/// A desktop behind the guest-executor wire: answers `get_screen_size` and
/// `screenshot` (a distinct ~96 KB JPEG-looking image each time), accepts
/// every input command.
async fn fake_desktop(port: u16) {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind fake desktop port");
    tokio::spawn(async move {
        let mut shot = 0u64;
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let Ok(mut ws) = tokio_tungstenite::accept_async(tcp).await else {
                continue;
            };
            shot += 1_000;
            let mut n = shot;
            tokio::spawn(async move {
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let req: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                    let reply = match req["command"].as_str() {
                        Some("get_screen_size") => {
                            json!({ "success": true, "size": { "width": 1280, "height": 800 } })
                        }
                        Some("screenshot") => {
                            n += 1;
                            let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
                            bytes.extend(std::iter::repeat_n((n % 251) as u8, FRAME_BYTES));
                            use base64::Engine;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                            json!({ "success": true, "image_data": b64 })
                        }
                        _ => json!({ "success": true }),
                    };
                    if ws.send(Message::Text(reply.to_string())).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
}

type Seen = Arc<Mutex<Vec<Value>>>;

/// A consumer worker with one handler per viewer; `delay` makes it slow.
fn viewer(iii: &IIIClient, function_id: &str, delay: Duration) -> Seen {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    iii.register_function(
        function_id,
        RegisterFunction::new_async(move |payload: Value| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push(payload);
                tokio::time::sleep(delay).await;
                Ok::<_, Error>(Value::Null)
            }
        })
        .description("live test viewer"),
    );
    seen
}

fn bind(iii: &IIIClient, function_id: &str, config: Value) -> iii_sdk::trigger::Trigger {
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "computer::frame-changed".to_string(),
        function_id: function_id.to_string(),
        config,
        metadata: Some(json!({ "viewer": function_id })),
        namespace: None,
        trigger_namespace: None,
    })
    .expect("register frame-changed binding")
}

/// The engine keeps one live worker per (namespace, name): name each client.
fn client(url: &str, name: &str) -> IIIClient {
    register_worker(
        url,
        InitOptions {
            metadata: Some(iii_sdk::runtime::WorkerMetadata {
                name: format!("{name}-{}", std::process::id()),
                ..Default::default()
            }),
            ..InitOptions::default()
        },
    )
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, String> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(20_000),
    })
    .await
    .map_err(|e| e.to_string())
}

async fn wait_for(what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn seqs(seen: &Seen) -> Vec<u64> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|p| p["change"] == "updated")
        .map(|p| p["frame_seq"].as_u64().unwrap())
        .collect()
}

struct KillOnDrop(std::process::Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a live engine: set COMPUTER_LIVE_III_URL"]
async fn live_frames_without_iii_stream() {
    let Ok(url) = std::env::var("COMPUTER_LIVE_III_URL") else {
        eprintln!("COMPUTER_LIVE_III_URL unset; skipping");
        return;
    };
    let desktop_port: u16 = std::env::var("COMPUTER_LIVE_DESKTOP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(49652);
    fake_desktop(desktop_port).await;

    let _worker = KillOnDrop(
        std::process::Command::new(env!("CARGO_BIN_EXE_computer"))
            .env("III_URL", &url)
            .env("RUST_LOG", "info,computer=debug")
            .spawn()
            .expect("spawn computer worker"),
    );
    let iii = client(&url, "computer-live-caller");

    // The engine runs without iii-stream.
    let err = call(&iii, "stream::set", json!({})).await.unwrap_err();
    assert!(
        err.contains("not_found") || err.contains("not found"),
        "{err}"
    );
    eprintln!("[live] stream::set on this engine -> {err}");

    // Wait for the worker's surface.
    let start = Instant::now();
    loop {
        if call(&iii, "computer::sessions::list", json!({}))
            .await
            .is_ok()
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "worker never came up"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let endpoint = format!("ws://127.0.0.1:{desktop_port}");
    let c1 = call(
        &iii,
        "computer::sessions::start",
        json!({ "endpoint": endpoint }),
    )
    .await
    .expect("start c1")["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let c2 = call(
        &iii,
        "computer::sessions::start",
        json!({ "endpoint": endpoint }),
    )
    .await
    .expect("start c2")["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    eprintln!("[live] sessions {c1}, {c2}");

    // A frame stored before anyone binds: only the initial read shows it.
    call(
        &iii,
        "computer::screencast::start",
        json!({ "session_id": c1 }),
    )
    .await
    .unwrap();
    let start = Instant::now();
    let initial = loop {
        let f = call(&iii, "computer::frame", json!({ "session_id": c1 }))
            .await
            .unwrap();
        if f["frame"].is_string() {
            break f;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "no initial frame"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let b64_len = initial["frame"].as_str().unwrap().len();
    eprintln!(
        "[live] initial read: seq {} epoch {} ({b64_len} base64 chars)",
        initial["frame_seq"], initial["epoch"]
    );

    // Viewers: A follows c1, B follows c2, S follows c1 slowly (300 ms per
    // notification, ~4.5 frames produced per delivery at 15 fps).
    let consumer = client(&url, "computer-live-viewers");
    let a = viewer(&consumer, "live::viewer-a", Duration::ZERO);
    let b = viewer(&consumer, "live::viewer-b", Duration::ZERO);
    let s = viewer(&consumer, "live::viewer-slow", Duration::from_millis(300));
    let _ta = bind(&consumer, "live::viewer-a", json!({ "session_id": c1 }));
    let _tb = bind(&consumer, "live::viewer-b", json!({ "session_id": c2 }));
    let ts = bind(&consumer, "live::viewer-slow", json!({ "session_id": c1 }));

    wait_for("A notifications", Duration::from_secs(10), || {
        seqs(&a).len() >= 10
    })
    .await;
    let note = a.lock().unwrap().last().cloned().unwrap();
    assert_eq!(note["session_id"], c1.as_str());
    assert_eq!(note["bytes"], (FRAME_BYTES + 4) as u64);
    assert!(note.get("frame").is_none() && note.get("data").is_none());
    let note_len = note.to_string().len();
    eprintln!("[live] notification ({note_len} bytes): {note}");
    assert!(note_len < 512);
    // Notify, then fetch: the read returns that frame or a newer one.
    let since = initial["frame_seq"].as_u64().unwrap();
    let fresh = call(
        &iii,
        "computer::frame",
        json!({ "session_id": c1, "since_frame": since }),
    )
    .await
    .unwrap();
    assert!(fresh["frame"].is_string());
    assert!(fresh["frame_seq"].as_u64().unwrap() >= note["frame_seq"].as_u64().unwrap());
    let a_seqs = seqs(&a);
    assert!(
        a_seqs.windows(2).all(|w| w[0] < w[1]),
        "A out of order: {a_seqs:?}"
    );
    // Filter by session: B has heard nothing (c2 is not casting yet).
    assert!(b.lock().unwrap().is_empty(), "B heard c1");

    call(
        &iii,
        "computer::screencast::start",
        json!({ "session_id": c2 }),
    )
    .await
    .unwrap();
    wait_for("B notifications", Duration::from_secs(10), || {
        seqs(&b).len() >= 3
    })
    .await;
    assert!(b
        .lock()
        .unwrap()
        .iter()
        .all(|p| p["session_id"] == c2.as_str()));
    assert!(a
        .lock()
        .unwrap()
        .iter()
        .all(|p| p["session_id"] == c1.as_str()));

    // Slow consumer: coalesced, newest-first, no backlog.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let fast = seqs(&a);
    let slow = seqs(&s);
    eprintln!(
        "[live] over the run: fast viewer {} notifications, slow viewer {} (last seq fast {:?}, slow {:?})",
        fast.len(),
        slow.len(),
        fast.last(),
        slow.last()
    );
    assert!(
        slow.windows(2).all(|w| w[0] < w[1]),
        "slow out of order: {slow:?}"
    );
    assert!(
        slow.len() * 2 < fast.len(),
        "slow viewer was not coalesced: {} vs {}",
        slow.len(),
        fast.len()
    );
    ts.unregister();

    // Cleanup on session end: c1 stops -> `cleared`, the frame is gone.
    let stopped = call(
        &iii,
        "computer::sessions::stop",
        json!({ "session_id": c1 }),
    )
    .await
    .unwrap();
    assert_eq!(stopped["was_running"], true);
    wait_for("A cleared", Duration::from_secs(10), || {
        a.lock()
            .unwrap()
            .last()
            .is_some_and(|p| p["change"] == "cleared")
    })
    .await;
    let gone = call(&iii, "computer::frame", json!({ "session_id": c1 }))
        .await
        .unwrap_err();
    assert!(gone.contains("unknown session"), "{gone}");
    let after_stop = a.lock().unwrap().len();

    // Screencast stop on c2 -> `cleared`, frame dropped, session alive.
    call(
        &iii,
        "computer::screencast::stop",
        json!({ "session_id": c2 }),
    )
    .await
    .unwrap();
    wait_for("B cleared", Duration::from_secs(10), || {
        b.lock()
            .unwrap()
            .last()
            .is_some_and(|p| p["change"] == "cleared")
    })
    .await;
    let empty = call(&iii, "computer::frame", json!({ "session_id": c2 }))
        .await
        .unwrap();
    assert!(empty["frame"].is_null() && empty["active"] == false);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        a.lock().unwrap().len(),
        after_stop,
        "A heard frames after stop"
    );
    call(
        &iii,
        "computer::sessions::stop",
        json!({ "session_id": c2 }),
    )
    .await
    .unwrap();
    eprintln!("[live] PASS");
    consumer.shutdown_async().await;
    iii.shutdown_async().await;
}
