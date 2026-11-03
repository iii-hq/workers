//! The fleet watcher through `sandbox-code-runner::on-event`'s observer
//! hooks, against an iii engine: nothing polls `sandbox::list` while the
//! function is unobserved, the first observer starts the watcher and
//! receives the opening fleet snapshot, and disposing the last observer
//! stops it.
//!
//! The engine is `III_ENGINE_WS_URL` when set; otherwise `III_ENGINE_BIN`
//! (or an `iii` 0.26+ on PATH) is spawned with an engine-only config on
//! `III_TEST_ENGINE_PORT` or an ephemeral port. Without one the test skips;
//! `III_E2E_REQUIRE=1` turns the skip into a failure. The sandbox daemon is
//! an in-process stand-in for the worker's `Engine` seam.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, IIIClient, InitOptions, RegisterFunction, Source,
};
use sandbox_code_runner::engine::{CallResult, Engine, ProxyHandler, UnregisterFn};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

/// One watcher read every 2 s; this is longer than one interval.
const LONGER_THAN_ONE_READ: Duration = Duration::from_millis(2_600);

struct EngineProcess {
    url: String,
    child: Option<Child>,
    dir: Option<PathBuf>,
}

impl Drop for EngineProcess {
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

async fn engine() -> Option<EngineProcess> {
    if let Ok(url) = std::env::var("III_ENGINE_WS_URL") {
        return Some(EngineProcess {
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
    let dir = std::env::temp_dir().join(format!(
        "sandbox-code-runner-fleet-{}-{port}",
        std::process::id()
    ));
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
    let engine = EngineProcess {
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

/// The sandbox daemon as the watcher reaches it: `sandbox::list` answers one
/// sandbox, and every read is counted.
#[derive(Default)]
struct Daemon {
    lists: AtomicUsize,
}

impl Daemon {
    fn lists(&self) -> usize {
        self.lists.load(Ordering::SeqCst)
    }
}

impl Engine for Daemon {
    fn call(&self, fn_id: String, _: Value, _: u64) -> BoxFuture<'static, CallResult> {
        let answer = if fn_id == "sandbox::list" {
            self.lists.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "sandboxes": [{ "sandbox_id": "sb-1", "stopped": false }] }))
        } else {
            Err(format!("function_not_found: {fn_id}"))
        };
        Box::pin(async move { answer })
    }

    fn register(&self, _: String, _: Option<String>, _: ProxyHandler) -> UnregisterFn {
        Box::new(|| {})
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

#[tokio::test]
async fn observers_of_on_event_start_and_stop_the_fleet_watcher() {
    let Some(engine) = engine().await else {
        assert!(
            std::env::var("III_E2E_REQUIRE").as_deref() != Ok("1"),
            "III_E2E_REQUIRE=1 but no engine: set III_ENGINE_WS_URL or III_ENGINE_BIN"
        );
        eprintln!("skipping: no iii engine (set III_ENGINE_WS_URL or III_ENGINE_BIN)");
        return;
    };

    let host = Arc::new(connect(&engine.url, "sandbox-code-runner").await);
    let daemon = Arc::new(Daemon::default());
    let _emitter = sandbox_code_runner::events::register_event_function(&host, daemon.clone());

    let consumer = connect(&engine.url, "sandbox-code-runner-test-consumer").await;
    let mut fleet = recorder(&consumer, "test::fleet");
    let mut everything = recorder(&consumer, "test::every-event");

    sleep(Duration::from_millis(300)).await;
    assert_eq!(
        daemon.lists(),
        0,
        "nothing reads the daemon while unobserved"
    );

    // The first observer starts the watcher and receives its opening read.
    let first = consumer.observe(
        Source::function("sandbox-code-runner::on-event")
            .filters(json!({ "input.kind": { "$eq": "fleet" } }))
            .deliver(Deliver::Output),
        "test::fleet",
    );
    timeout(Duration::from_secs(15), first.ready())
        .await
        .expect("the observer of on-event never became active")
        .expect("the observer of on-event was rejected");
    let snapshot = timeout(Duration::from_secs(10), fleet.recv())
        .await
        .expect("the first observer never received the opening fleet snapshot")
        .unwrap();
    assert_eq!(snapshot["kind"], "fleet");
    assert_eq!(snapshot["daemon_absent"], false);
    assert_eq!(snapshot["sandboxes"][0]["sandbox_id"], "sb-1");
    assert!(daemon.lists() >= 1);

    // A second observer keeps it running when the first leaves.
    let second = consumer.observe(
        Source::function("sandbox-code-runner::on-event").deliver(Deliver::Output),
        "test::every-event",
    );
    timeout(Duration::from_secs(15), second.ready())
        .await
        .expect("the second observer never became active")
        .expect("the second observer was rejected");
    first.dispose().await.expect("dispose the first observer");
    let before = daemon.lists();
    sleep(LONGER_THAN_ONE_READ).await;
    assert!(
        daemon.lists() > before,
        "the watcher stopped while an observer was left"
    );

    // The last observer's removal stops it.
    second.dispose().await.expect("dispose the second observer");
    sleep(Duration::from_millis(500)).await;
    let stopped_at = daemon.lists();
    sleep(LONGER_THAN_ONE_READ).await;
    assert_eq!(
        daemon.lists(),
        stopped_at,
        "the watcher kept reading the daemon after the last observer left"
    );
    // The fleet is unchanged, so nothing reached either target after the
    // opening snapshot.
    assert!(fleet.try_recv().is_err());
    assert!(everything.try_recv().is_err());

    consumer.shutdown_async().await;
    host.shutdown_async().await;
}
