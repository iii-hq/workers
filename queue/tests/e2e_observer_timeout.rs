//! A `queue::on-message` target is held for the engine's
//! `observers.activation_timeout_ms`: the worker calls each target through its
//! observer, and the engine fails a call through an observer that has not
//! answered by then with `TIMEOUT`.
//!
//! Each test starts its own engine from `III_ENGINE_BIN` with a known
//! `observers.activation_timeout_ms`, on `III_TEST_ENGINE_PORT` (the second
//! test on the next port) or, unset, on a free ephemeral port. Without
//! `III_ENGINE_BIN` the tests skip; with it set, an engine that does not come
//! up fails them.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_queue::config::QueueConfig;
use iii_queue::functions::{DLQ_MESSAGES_FN_ID, PUBLISH_FN_ID};
use iii_queue::on_message::ON_MESSAGE_FN;
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, IIIClient, InitOptions, RegisterFunction, Source,
    WorkerIdentityMode,
};
use serde_json::{json, Value};
use tokio::time::Instant;
use uuid::Uuid;

/// How long the slow target takes to answer.
const TARGET_MS: u64 = 3_000;

/// An engine this test started; dropping it stops the process and removes
/// its directory.
struct SpawnedEngine {
    url: String,
    child: Child,
    dir: PathBuf,
}

impl Drop for SpawnedEngine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `III_TEST_ENGINE_PORT + offset`, else a free ephemeral port.
fn engine_port(offset: u16) -> u16 {
    if let Some(base) = std::env::var("III_TEST_ENGINE_PORT")
        .ok()
        .and_then(|port| port.parse::<u16>().ok())
    {
        return base + offset;
    }
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a free port")
        .port()
}

/// Start an engine whose `observers.activation_timeout_ms` is
/// `activation_timeout_ms`. `None` (skip) without `III_ENGINE_BIN`.
async fn spawn_engine(port_offset: u16, activation_timeout_ms: u64) -> Option<SpawnedEngine> {
    let Some(bin) = std::env::var_os("III_ENGINE_BIN") else {
        eprintln!("[skip] III_ENGINE_BIN is not set: this test starts its own engine");
        return None;
    };
    let port = engine_port(port_offset);
    let dir = std::env::temp_dir().join(format!("queue-observer-timeout-{}", Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("config")).expect("engine directory");
    let config = format!(
        r#"workers:
  - name: iii-worker-manager
    config:
      host: 127.0.0.1
      port: {port}
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: {config_dir}
observers:
  activation_timeout_ms: {activation_timeout_ms}
"#,
        config_dir = dir.join("config").display(),
    );
    let config_path = dir.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut file| file.write_all(config.as_bytes()))
        .expect("engine config");
    let child = Command::new(bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .env("III_TELEMETRY_ENABLED", "false")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start III_ENGINE_BIN");
    let engine = SpawnedEngine {
        url: format!("ws://127.0.0.1:{port}"),
        child,
        dir,
    };

    let probe = register_worker(&engine.url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let configuration_up = probe
            .call(CallRequest {
                function_id: "configuration::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(1_000),
            })
            .await
            .is_ok();
        if configuration_up {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the engine on {} did not come up",
            engine.url
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown_async().await;
    Some(engine)
}

async fn connect(url: &str, namespace: &str, role: &str) -> Arc<IIIClient> {
    let iii = Arc::new(register_worker(
        url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: format!("{namespace}-{role}"),
                ..WorkerMetadata::default()
            }),
            namespace: Some(namespace.to_string()),
            identity: WorkerIdentityMode::Explicit,
            ..InitOptions::default()
        },
    ));
    iii.wait_until_registered(Duration::from_secs(10))
        .await
        .expect("worker registered");
    iii
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value, namespace: &str) -> Value {
    iii.call(
        CallRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(5_000),
        }
        .namespace(namespace),
    )
    .await
    .expect("call should succeed")
}

/// Calls of the slow target: started, and answered.
#[derive(Default)]
struct Attempts {
    started: AtomicUsize,
    answered: AtomicUsize,
}

impl Attempts {
    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    fn answered(&self) -> usize {
        self.answered.load(Ordering::SeqCst)
    }
}

/// The queue worker and a consumer whose target takes [`TARGET_MS`] to
/// acknowledge, subscribed to a fresh topic with `max_retries: 2`; one
/// message published.
struct SlowSubscriber {
    namespace: String,
    topic: String,
    worker: Arc<IIIClient>,
    consumer: Arc<IIIClient>,
    boot: iii_queue::boot::BootHandle,
    observer: iii_sdk::ObserverHandle,
    attempts: Arc<Attempts>,
}

impl SlowSubscriber {
    async fn start(engine: &SpawnedEngine) -> Self {
        let namespace = format!("queue-observer-timeout-{}", Uuid::new_v4().simple());
        let worker = connect(&engine.url, &namespace, "queue").await;
        let consumer = connect(&engine.url, &namespace, "consumer").await;
        let boot = iii_queue::boot::start(worker.clone(), QueueConfig::default())
            .await
            .expect("queue worker should boot");

        let attempts = Arc::new(Attempts::default());
        let counted = attempts.clone();
        consumer.register_function(
            "e2e::slow::process",
            RegisterFunction::new_async(move |_message: Value| {
                let counted = counted.clone();
                async move {
                    counted.started.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(TARGET_MS)).await;
                    counted.answered.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, Error>(json!({ "ok": true }))
                }
            }),
        );
        let topic = format!("slow-{}", Uuid::new_v4().simple());
        let observer = consumer.observe(
            Source::function(ON_MESSAGE_FN)
                .args(json!({ "topic": topic, "max_retries": 2, "backoff_ms": 10 })),
            "e2e::slow::process",
        );
        tokio::time::timeout(Duration::from_secs(10), observer.ready())
            .await
            .expect("observer ready in time")
            .expect("observer active");
        call(
            &consumer,
            PUBLISH_FN_ID,
            json!({ "topic": topic, "data": { "n": 1 } }),
            &namespace,
        )
        .await;
        Self {
            namespace,
            topic,
            worker,
            consumer,
            boot,
            observer,
            attempts,
        }
    }

    async fn dead_letters(&self) -> Vec<Value> {
        call(
            &self.consumer,
            DLQ_MESSAGES_FN_ID,
            json!({ "queue": self.topic, "limit": 10 }),
            &self.namespace,
        )
        .await
        .as_array()
        .cloned()
        .unwrap_or_default()
    }

    async fn stop(self) {
        let _ = self.observer.dispose().await;
        self.boot.shutdown().await;
        self.consumer.shutdown_async().await;
        self.worker.shutdown_async().await;
    }
}

async fn wait_until(what: &str, within: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_target_slower_than_the_activation_timeout_is_retried_then_dead_lettered() {
    let Some(engine) = spawn_engine(0, 500).await else {
        return;
    };
    let run = SlowSubscriber::start(&engine).await;

    // Each delivery fails after 500 ms, long before the target answers
    // (TARGET_MS): only the activation timeout ends a call that early. The
    // retry starts while the first call is still running.
    let deadline = Instant::now() + Duration::from_secs(10);
    let dead = loop {
        let dead = run.dead_letters().await;
        if !dead.is_empty() {
            break dead;
        }
        assert!(
            Instant::now() < deadline,
            "the message never reached the DLQ"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(dead.len(), 1, "{dead:?}");
    assert_eq!(dead[0]["payload"], json!({ "n": 1 }), "{dead:?}");
    assert_eq!(run.attempts.started(), 2, "max_retries deliveries");
    assert_eq!(
        run.attempts.answered(),
        0,
        "both deliveries failed while their calls were still running"
    );

    // What the queue worker's deliver hook gets back: its call through the
    // subscriber's observer ends with TIMEOUT.
    let observer = run
        .worker
        .observers(ON_MESSAGE_FN)
        .into_iter()
        .find(|observer| observer.id() == run.observer.id())
        .expect("the subscriber is an active observer");
    let error = run
        .worker
        .call(
            CallRequest::observer(&observer, json!({ "n": 2 }))
                .timeout(Duration::from_millis(TARGET_MS * 2)),
        )
        .await
        .expect_err("a call through the observer outlives the activation timeout");
    assert!(
        matches!(&error, Error::Remote { code, .. } if code == "TIMEOUT"),
        "{error}"
    );

    // Every call runs to completion; an answer after TIMEOUT acks nothing.
    let attempts = run.attempts.clone();
    wait_until(
        "every call of the target to answer",
        Duration::from_millis(TARGET_MS * 2),
        || attempts.answered() == 3,
    )
    .await;
    assert_eq!(run.dead_letters().await.len(), 1);

    run.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_activation_timeout_above_the_target_s_duration_holds_it_until_it_answers() {
    let Some(engine) = spawn_engine(1, TARGET_MS * 3).await else {
        return;
    };
    let run = SlowSubscriber::start(&engine).await;

    let attempts = run.attempts.clone();
    wait_until(
        "the target to answer",
        Duration::from_millis(TARGET_MS * 3),
        || attempts.answered() == 1,
    )
    .await;
    // Past the backoff: an answered delivery is acked, never retried.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(run.attempts.started(), 1, "one delivery");
    assert!(run.dead_letters().await.is_empty(), "nothing dead-lettered");

    run.stop().await;
}
