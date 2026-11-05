//! `stories::on-changed`'s deliver hook against an iii engine: each observer
//! gets the event narrowed to its args, an observer whose filters do not
//! match or whose narrowing is empty gets nothing, and the caller gets the
//! event back.
//!
//! The engine is `III_ENGINE_WS_URL` when set; otherwise `III_ENGINE_BIN`
//! (or an `iii` 0.26+ on PATH) is spawned with an engine-only config on
//! `III_TEST_ENGINE_PORT` or an ephemeral port. Without one the test skips;
//! `III_E2E_REQUIRE=1` turns the skip into a failure.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{CallRequest, IIIClient, InitOptions, RegisterFunction, Source, register_worker};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

struct Engine {
    url: String,
    child: Option<Child>,
    dir: Option<PathBuf>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// `iii` on PATH, when it reports 0.26 or later.
fn engine_on_path() -> Option<PathBuf> {
    let out = Command::new("iii").arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text
        .split_whitespace()
        .last()?
        .split('.')
        .map(|p| p.parse::<u64>().ok());
    let (major, minor) = (parts.next()??, parts.next()??);
    (major > 0 || minor >= 26).then(|| PathBuf::from("iii"))
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("bind an ephemeral port")
}

async fn engine() -> Option<Engine> {
    if let Ok(url) = std::env::var("III_ENGINE_WS_URL") {
        return Some(Engine {
            url,
            child: None,
            dir: None,
        });
    }
    let bin = std::env::var_os("III_ENGINE_BIN")
        .map(PathBuf::from)
        .or_else(engine_on_path)?;
    let port = std::env::var("III_TEST_ENGINE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(free_port);
    let dir =
        std::env::temp_dir().join(format!("stories-on-changed-{}-{port}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create engine dir");
    let config = dir.join("config.yaml");
    std::fs::write(
        &config,
        format!(
            "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: {}\n",
            dir.join("config").display()
        ),
    )
    .expect("write engine config");
    let child = Command::new(&bin)
        .arg("--config")
        .arg(&config)
        .arg("--no-update-check")
        .env("III_TELEMETRY_ENABLED", "false")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn iii engine");
    let engine = Engine {
        url: format!("ws://127.0.0.1:{port}"),
        child: Some(child),
        dir: Some(dir),
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "the engine never listened on {port}"
        );
        sleep(Duration::from_millis(100)).await;
    }
    Some(engine)
}

/// A connection named `name`: two connections of one process need names of
/// their own.
async fn connect(url: &str, name: &str) -> IIIClient {
    let client = register_worker(
        url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: name.to_string(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let answer = client
            .call(CallRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(2_000),
            })
            .await;
        match answer {
            Ok(_) => return client,
            Err(error) => assert!(
                Instant::now() < deadline,
                "the engine at {url} never answered: {error}"
            ),
        }
        sleep(Duration::from_millis(250)).await;
    }
}

/// A function that forwards every payload it is called with to a channel.
fn recorder(client: &IIIClient, function_id: &str) -> mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = mpsc::unbounded_channel();
    client.register_function(
        function_id,
        RegisterFunction::new(move |payload: Value| {
            let _ = tx.send(payload);
            Ok::<_, iii_sdk::Error>(json!({ "ok": true }))
        }),
    );
    rx
}

/// A `Change` as the handler serializes it, so the event round-trips exactly.
fn change(id: &str, project: &str, kind: &str) -> Value {
    json!({
        "id": id,
        "project": project,
        "title": id,
        "file": format!("{id}.stories.tsx"),
        "kind": kind,
        "files": [],
        "states": [],
    })
}

fn event() -> Value {
    let line =
        json!({ "key": "worktree", "kind": "worktree", "label": "working tree", "built_at": "t" });
    json!({
        "workspace": "ws",
        "line": line,
        "base": line,
        "summary": { "direct": 1, "indirect": 1, "new": 0, "removed": 0, "unchanged": 0 },
        "changes": [change("button", "app", "direct"), change("badge", "admin", "indirect")],
        "at": "t",
    })
}

#[tokio::test]
async fn each_observer_receives_the_event_narrowed_to_its_args() {
    let Some(engine) = engine().await else {
        assert!(
            std::env::var("III_E2E_REQUIRE").as_deref() != Ok("1"),
            "III_E2E_REQUIRE=1 but no engine: set III_ENGINE_WS_URL or III_ENGINE_BIN"
        );
        eprintln!("skipping: no iii engine (set III_ENGINE_WS_URL or III_ENGINE_BIN)");
        return;
    };

    let host = Arc::new(connect(&engine.url, "stories").await);
    iii_stories::events::register(&host);

    let consumer = connect(&engine.url, "stories-test-consumer").await;
    let mut whole = recorder(&consumer, "test::stories-whole");
    let mut admin = recorder(&consumer, "test::stories-admin");
    let mut nothing_new = recorder(&consumer, "test::stories-new");
    let mut other_ws = recorder(&consumer, "test::stories-other-ws");
    let observers = [
        consumer.observe(
            Source::function("stories::on-changed").args(json!({ "workspace": "ws" })),
            "test::stories-whole",
        ),
        consumer.observe(
            Source::function("stories::on-changed").args(json!({ "project": "admin" })),
            "test::stories-admin",
        ),
        // Narrowing leaves nothing: the event has no `new` change.
        consumer.observe(
            Source::function("stories::on-changed").args(json!({ "kinds": ["new"] })),
            "test::stories-new",
        ),
        // Its filters do not match the call.
        consumer.observe(
            Source::function("stories::on-changed")
                .filters(json!({ "input.workspace": { "$eq": "other" } })),
            "test::stories-other-ws",
        ),
    ];
    for observer in &observers {
        timeout(Duration::from_secs(15), observer.ready())
            .await
            .expect("an observer of stories::on-changed never became active")
            .expect("an observer of stories::on-changed was rejected");
    }

    let returned = timeout(
        Duration::from_secs(15),
        host.call(CallRequest {
            function_id: "stories::on-changed".into(),
            payload: event(),
            action: None,
            timeout_ms: Some(10_000),
        }),
    )
    .await
    .expect("stories::on-changed timed out")
    .expect("stories::on-changed failed");
    assert_eq!(returned, event(), "the caller gets the event back");

    let got = timeout(Duration::from_secs(10), whole.recv())
        .await
        .expect("the workspace observer received nothing")
        .unwrap();
    assert_eq!(got["changes"].as_array().unwrap().len(), 2);
    assert_eq!(got["summary"]["direct"], 1);
    assert_eq!(got["summary"]["indirect"], 1);

    let got = timeout(Duration::from_secs(10), admin.recv())
        .await
        .expect("the project observer received nothing")
        .unwrap();
    let changes = got["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["project"], "admin");
    assert_eq!(got["summary"]["direct"], 0);
    assert_eq!(got["summary"]["indirect"], 1);

    // The deliver hook made every call before it returned; give the engine
    // time to route anything that should not have been sent.
    sleep(Duration::from_millis(500)).await;
    assert!(
        nothing_new.try_recv().is_err(),
        "an empty narrowing was delivered"
    );
    assert!(
        other_ws.try_recv().is_err(),
        "an observer whose filters do not match was delivered"
    );
    assert!(whole.try_recv().is_err(), "one call, one delivery");
    assert!(admin.try_recv().is_err(), "one call, one delivery");

    for observer in observers {
        observer.dispose().await.expect("dispose observer");
    }
    consumer.shutdown_async().await;
    host.shutdown_async().await;
}
