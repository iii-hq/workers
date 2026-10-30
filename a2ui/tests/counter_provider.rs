//! The `counter_provider` example's `demo::on-counter-changed` rules against
//! a real engine: `onObserverAdded` rejects a counter outside 1-64 bytes with
//! `INVALID_ARGS`, takes one slot per observer id and rejects observers past
//! 64 with `OBSERVER_LIMIT`, and `onObserverRemoved` gives the slot back.
//!
//! The test spawns `III_ENGINE_BIN` (or `iii` on PATH) on an ephemeral port
//! and hosts the provider in process. Without an engine binary it skips;
//! `III_E2E_REQUIRE=1` makes that a failure.

#[path = "../examples/counter_provider/provider.rs"]
mod provider;

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, Error, IIIClient, InitOptions, ObserverHandle,
    RegisterFunction, Source,
};
use serde_json::{json, Value};

use provider::{MAX_OBSERVERS, ON_COUNTER_CHANGED_FN};

const SINK_FN: &str = "a2ui-counter-limits::sink";

struct Engine {
    url: String,
    child: std::process::Child,
    dir: PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn engine_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("III_ENGINE_BIN") {
        return Some(path.into());
    }
    let on_path = std::process::Command::new("iii")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    on_path.then(|| "iii".into())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("bind an ephemeral port")
        .port()
}

async fn spawn_engine() -> Option<Engine> {
    let Some(bin) = engine_bin() else {
        assert!(
            std::env::var("III_E2E_REQUIRE").as_deref() != Ok("1"),
            "III_E2E_REQUIRE=1 and no engine binary (set III_ENGINE_BIN)"
        );
        return None;
    };
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("a2ui-counter-{}-{port}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).expect("engine dir");
    let config = format!(
        "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: \"{}\"\n",
        dir.join("config").display()
    );
    let config_path = dir.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut file| file.write_all(config.as_bytes()))
        .expect("engine config");
    let child = std::process::Command::new(&bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .env("III_TELEMETRY_ENABLED", "false")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn the engine");
    let engine = Engine {
        url: format!("ws://127.0.0.1:{port}"),
        child,
        dir,
    };

    let probe = register_worker(&engine.url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(60);
    while probe
        .call(CallRequest {
            timeout_ms: Some(1_000),
            ..CallRequest::new("engine::workers::list", json!({}))
        })
        .await
        .is_err()
    {
        assert!(Instant::now() < deadline, "engine never became ready");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    probe.shutdown_async().await;
    Some(engine)
}

fn connect(url: &str, name: &str) -> Arc<IIIClient> {
    Arc::new(register_worker(
        url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                name: name.into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ))
}

/// `{ count, reserved }` from the provider: active observers, slots held.
async fn observer_count(iii: &IIIClient) -> (u64, u64) {
    let counts = iii
        .call(CallRequest {
            timeout_ms: Some(5_000),
            ..CallRequest::new("demo::observers::count", json!({}))
        })
        .await
        .unwrap();
    (
        counts["count"].as_u64().unwrap(),
        counts["reserved"].as_u64().unwrap(),
    )
}

async fn wait_for_count(iii: &IIIClient, expected: usize) {
    let expected = (expected as u64, expected as u64);
    let mut last = (0, 0);
    for _ in 0..100 {
        last = observer_count(iii).await;
        if last == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("provider (count, reserved) stayed {last:?}, expected {expected:?}");
}

fn observe(iii: &IIIClient, counter: &str) -> ObserverHandle {
    iii.observe(
        Source::function(ON_COUNTER_CHANGED_FN)
            .args(json!({ "counter": counter }))
            .deliver(Deliver::Output),
        SINK_FN,
    )
}

/// `Ok` when the observer became active, the rejection's code and message
/// otherwise.
async fn settle(handle: &ObserverHandle) -> Result<(), (String, String)> {
    match tokio::time::timeout(Duration::from_secs(30), handle.ready())
        .await
        .expect("observer settles in time")
    {
        Ok(()) => Ok(()),
        Err(Error::Remote { code, message, .. }) => Err((code, message)),
        Err(other) => panic!("unexpected observer error: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn counter_observers_are_validated_and_capped_per_observer_id() {
    let Some(engine) = spawn_engine().await else {
        eprintln!("skipping: no iii engine (set III_ENGINE_BIN or put `iii` on PATH)");
        return;
    };
    let host = connect(&engine.url, "a2ui-demo-provider");
    provider::register(&host);
    let iii = connect(&engine.url, "a2ui-counter-limits");
    iii.register_function(
        SINK_FN,
        RegisterFunction::new(|event: Value| Ok::<Value, Error>(event))
            .description("Target of the counter observers."),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while iii
        .call(CallRequest::new(
            "engine::functions::info",
            json!({ "function_id": ON_COUNTER_CHANGED_FN }),
        ))
        .await
        .is_err()
    {
        assert!(Instant::now() < deadline, "the provider never registered");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    wait_for_count(&iii, 0).await;

    // INVALID_ARGS: the provider's own check, past the args_format. A
    // rejected observer takes no slot.
    for counter in [String::new(), "x".repeat(65)] {
        let handle = observe(&iii, &counter);
        let (code, message) = settle(&handle).await.unwrap_err();
        assert_eq!(code, "INVALID_ARGS", "{message}");
        assert!(message.contains("counter must be 1-64 bytes"), "{message}");
        let _ = handle.dispose().await;
    }
    let longest = observe(&iii, &"x".repeat(64));
    settle(&longest).await.unwrap();
    wait_for_count(&iii, 1).await;
    longest.dispose().await.unwrap();
    wait_for_count(&iii, 0).await;

    // Observers added at the same time: exactly 64 take a slot, the rest are
    // rejected with OBSERVER_LIMIT.
    let extra = 16;
    let handles: Vec<ObserverHandle> = (0..MAX_OBSERVERS + extra)
        .map(|index| observe(&iii, &format!("c{index}")))
        .collect();
    let mut accepted = Vec::with_capacity(MAX_OBSERVERS);
    let mut rejected = Vec::with_capacity(extra);
    for handle in handles {
        match settle(&handle).await {
            Ok(()) => accepted.push(handle),
            Err((code, message)) => {
                assert_eq!(code, "OBSERVER_LIMIT", "{message}");
                assert!(message.contains("at most 64 observers"), "{message}");
                rejected.push(handle);
            }
        }
    }
    assert_eq!(accepted.len(), MAX_OBSERVERS);
    assert_eq!(rejected.len(), extra);
    wait_for_count(&iii, MAX_OBSERVERS).await;
    for handle in rejected {
        let _ = handle.dispose().await;
    }

    // A removed observer gives its slot back, and the slot is taken once.
    accepted.pop().unwrap().dispose().await.unwrap();
    wait_for_count(&iii, MAX_OBSERVERS - 1).await;
    let again = observe(&iii, "again");
    let over = observe(&iii, "over");
    settle(&again).await.unwrap();
    let (code, _) = settle(&over).await.unwrap_err();
    assert_eq!(code, "OBSERVER_LIMIT");
    accepted.push(again);
    let _ = over.dispose().await;
    wait_for_count(&iii, MAX_OBSERVERS).await;

    for handle in accepted {
        handle.dispose().await.unwrap();
    }
    wait_for_count(&iii, 0).await;
    iii.shutdown_async().await;
    host.shutdown_async().await;
    drop(engine);
}
