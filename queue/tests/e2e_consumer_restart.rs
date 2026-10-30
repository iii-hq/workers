//! A `queue::on-message` consumer whose process dies and comes back under the
//! same worker name and observer id: the engine suspends its observer
//! (`observerremoved` reaches the queue worker, whose consumer stops), the
//! subscriber's queue keeps receiving the topic for `subscriber_retention_ms`,
//! and the restarted process, re-observing the id within the engine's grace
//! period, receives the unacked backlog and every message published while it
//! was away, each once. Back within the grace period but after a shorter
//! retention, it receives the backlog and what the queue received within the
//! retention, and not what was published after it.
//!
//! Each test starts its own engine from `III_ENGINE_BIN`, on
//! `III_TEST_ENGINE_PORT + 2` (+ 3 for the second) or, unset, on a free
//! ephemeral port, and runs the queue worker in-process. Each consumer is this
//! test binary run again as [`consumer_process`] in a child process, so it can
//! be killed with SIGKILL (no shutdown, no reattach). Without `III_ENGINE_BIN`
//! the tests skip; with it set, an engine that does not come up fails them.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_queue::boot::BootHandle;
use iii_queue::config::QueueConfig;
use iii_queue::functions::{list_topics, PUBLISH_FN_ID};
use iii_queue::on_message::ON_MESSAGE_FN;
use iii_queue::subscribers::Subscribers;
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, IIIClient, InitOptions, ObserverOptions, RegisterFunction,
    Source, WorkerIdentityMode,
};
use serde_json::{json, Value};
use tokio::time::Instant;
use uuid::Uuid;

/// Prefix of the lines a consumer process reports on stdout.
const EVENT_PREFIX: &str = "QUEUE_E2E_EVENT ";
const CONSUMER_ENV: &str = "QUEUE_E2E_CONSUMER";
const TARGET_FN: &str = "e2e::restart::process";

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

/// Start an engine with the default `observers.observer_grace_ms` (30 s).
/// Calls through an observer end after 5 s, so a delivery the dead consumer
/// held fails in time for the test. `None` (skip) without `III_ENGINE_BIN`.
async fn spawn_engine(port_offset: u16) -> Option<SpawnedEngine> {
    let Some(bin) = std::env::var_os("III_ENGINE_BIN") else {
        eprintln!("[skip] III_ENGINE_BIN is not set: this test starts its own engine");
        return None;
    };
    let port = engine_port(port_offset);
    let dir = std::env::temp_dir().join(format!("queue-consumer-restart-{}", Uuid::new_v4()));
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
  activation_timeout_ms: 5000
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

async fn connect(url: &str, namespace: &str, name: &str) -> Arc<IIIClient> {
    let iii = Arc::new(register_worker(
        url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: name.to_string(),
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

/// What a consumer process is told, through its environment.
struct ConsumerSpec {
    url: String,
    namespace: String,
    worker_name: String,
    observer_id: String,
    topic: String,
    /// The target never answers a message with `n` at or above this.
    hang_from: Option<i64>,
}

/// A consumer process and the events it reported: `{ "ev": "ready", "id",
/// "state" }` once its observer is answered, `{ "ev": "msg", "n" }` per call
/// of its target.
struct Consumer {
    child: Child,
    events: Arc<Mutex<Vec<Value>>>,
}

impl Consumer {
    fn start(spec: &ConsumerSpec) -> Self {
        let mut command = Command::new(std::env::current_exe().expect("test binary"));
        command
            .args([
                "--exact",
                "consumer_process",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CONSUMER_ENV, "1")
            .env("QUEUE_E2E_URL", &spec.url)
            .env("QUEUE_E2E_NAMESPACE", &spec.namespace)
            .env("QUEUE_E2E_WORKER_NAME", &spec.worker_name)
            .env("QUEUE_E2E_OBSERVER_ID", &spec.observer_id)
            .env("QUEUE_E2E_TOPIC", &spec.topic)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(hang_from) = spec.hang_from {
            command.env("QUEUE_E2E_HANG_FROM", hang_from.to_string());
        }
        let mut child = command.spawn().expect("start the consumer process");
        let events: Arc<Mutex<Vec<Value>>> = Arc::default();
        let stdout = child.stdout.take().expect("consumer stdout");
        let recorded = events.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                // The harness's own `test consumer_process ... ` has no line
                // break, so an event can follow it on the same line.
                if let Some((_, event)) = line.split_once(EVENT_PREFIX) {
                    if let Ok(event) = serde_json::from_str::<Value>(event) {
                        recorded.lock().unwrap().push(event);
                    }
                }
            }
        });
        Self { child, events }
    }

    /// Its observer's answer: the `ready` or `rejected` event.
    fn ready(&self) -> Option<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .find(|event| event["ev"] == "ready" || event["ev"] == "rejected")
            .cloned()
    }

    /// The `n` of each message its target was called with, in call order.
    fn messages(&self) -> Vec<i64> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event["ev"] == "msg")
            .filter_map(|event| event["n"].as_i64())
            .collect()
    }

    /// SIGKILL: the process ends without a shutdown or a reattach.
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        self.kill();
    }
}

fn counts(messages: &[i64]) -> HashMap<i64, usize> {
    let mut counts = HashMap::new();
    for n in messages {
        *counts.entry(*n).or_default() += 1;
    }
    counts
}

/// Whether `done` holds within `within`.
async fn eventually(within: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    while !done() {
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    true
}

async fn wait_until(what: &str, within: Duration, done: impl FnMut() -> bool) {
    assert!(
        eventually(within, done).await,
        "timed out waiting for {what}"
    );
}

async fn subscribers_of(subscribers: &Subscribers, topic: &str) -> usize {
    subscribers
        .registrations()
        .await
        .iter()
        .filter(|registration| registration.spec.queue == topic)
        .count()
}

async fn wait_for_subscribers(subscribers: &Subscribers, topic: &str, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while subscribers_of(subscribers, topic).await != expected {
        assert!(
            Instant::now() < deadline,
            "expected {expected} subscribers of {topic}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn publish(iii: &IIIClient, namespace: &str, topic: &str, n: i64) {
    iii.call(
        CallRequest {
            function_id: PUBLISH_FN_ID.to_string(),
            payload: json!({ "topic": topic, "data": { "n": n } }),
            action: None,
            timeout_ms: Some(5_000),
        }
        .namespace(namespace),
    )
    .await
    .expect("publish");
}

/// The queue worker in-process with `config`, a publisher, and one topic,
/// in a namespace of their own on `engine`.
struct Setup {
    url: String,
    namespace: String,
    topic: String,
    worker: Arc<IIIClient>,
    publisher: Arc<IIIClient>,
    boot: BootHandle,
}

impl Setup {
    async fn start(engine: &SpawnedEngine, config: QueueConfig) -> Self {
        let namespace = format!("queue-consumer-restart-{}", Uuid::new_v4().simple());
        let worker = connect(&engine.url, &namespace, &format!("{namespace}-queue")).await;
        let publisher = connect(&engine.url, &namespace, &format!("{namespace}-publisher")).await;
        let boot = iii_queue::boot::start(worker.clone(), config)
            .await
            .expect("queue worker should boot");
        Self {
            url: engine.url.clone(),
            topic: format!("restart-{}", Uuid::new_v4().simple()),
            namespace,
            worker,
            publisher,
            boot,
        }
    }

    /// A consumer under this setup's consumer worker name.
    fn spec(&self, observer_id: &str, hang_from: Option<i64>) -> ConsumerSpec {
        ConsumerSpec {
            url: self.url.clone(),
            namespace: self.namespace.clone(),
            worker_name: format!("{}-consumer", self.namespace),
            observer_id: observer_id.to_string(),
            topic: self.topic.clone(),
            hang_from,
        }
    }

    async fn publish(&self, n: i64) {
        publish(&self.publisher, &self.namespace, &self.topic, n).await;
    }

    /// Whether the builtin adapter routes the topic to any subscriber queue.
    async fn topic_routed(&self) -> bool {
        list_topics(self.boot.adapter.clone())
            .await
            .expect("list topics")
            .iter()
            .any(|info| info.name == self.topic)
    }

    async fn shutdown(self) {
        self.boot.shutdown().await;
        self.publisher.shutdown_async().await;
        self.worker.shutdown_async().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restarted_consumer_gets_its_backlog_and_what_was_published_while_away() {
    let Some(engine) = spawn_engine(2).await else {
        return;
    };
    let setup = Setup::start(&engine, QueueConfig::default()).await;
    let (namespace, topic, publisher, boot) = (
        &setup.namespace,
        &setup.topic,
        &setup.publisher,
        &setup.boot,
    );
    let observer_id = format!("restart-consumer-{}", Uuid::new_v4().simple());
    let first = setup.spec(&observer_id, Some(3));

    // A takes 1 and 2 (acked) and holds 3, 4 and 5 unanswered.
    let mut a = Consumer::start(&first);
    wait_until("A's observer", Duration::from_secs(30), || {
        a.ready().is_some()
    })
    .await;
    let ready = a.ready().unwrap();
    assert_eq!(ready["id"], json!(observer_id), "{ready}");
    assert_eq!(ready["state"], json!("active"), "{ready}");
    wait_for_subscribers(&boot.subscribers, topic, 1).await;
    for n in [1, 2] {
        publish(publisher, namespace, topic, n).await;
    }
    wait_until("A acks 1 and 2", Duration::from_secs(10), || {
        a.messages().len() >= 2
    })
    .await;
    for n in [3, 4, 5] {
        publish(publisher, namespace, topic, n).await;
    }
    wait_until("A holds 3, 4 and 5", Duration::from_secs(10), || {
        let got = a.messages();
        [3, 4, 5].iter().all(|n| got.contains(n))
    })
    .await;

    // SIGKILL: the engine suspends A's observer and withdraws it from the
    // queue worker, whose consumer for it stops.
    a.kill();
    wait_for_subscribers(&boot.subscribers, topic, 0).await;

    // Published while no consumer serves the subscriber.
    for n in [6, 7, 8] {
        publish(publisher, namespace, topic, n).await;
    }

    // B: a new process under the same worker name re-observes the same id
    // within the engine's grace period.
    let b = Consumer::start(&setup.spec(&observer_id, None));
    wait_until("B's observer", Duration::from_secs(30), || {
        b.ready().is_some()
    })
    .await;
    let ready = b.ready().unwrap();
    assert_eq!(ready["id"], json!(observer_id), "{ready}");
    assert_eq!(ready["state"], json!("active"), "{ready}");

    // The backlog (3, 4, 5) and what was published while A was away (6, 7,
    // 8).
    let expected = [3, 4, 5, 6, 7, 8];
    let received = eventually(Duration::from_secs(30), || {
        let got = b.messages();
        expected.iter().all(|n| got.contains(n))
    })
    .await;
    assert!(
        received,
        "B received {:?}, expected each of {expected:?}",
        b.messages()
    );
    // Past the retry backoff: nothing arrives twice.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let got = counts(&b.messages());
    for n in expected {
        assert_eq!(
            got.get(&n),
            Some(&1),
            "message {n} once: {:?}",
            b.messages()
        );
    }
    assert!(
        !got.contains_key(&1) && !got.contains_key(&2),
        "A acked 1 and 2: {:?}",
        b.messages()
    );

    publish(publisher, namespace, topic, 9).await;
    wait_until("B receives 9", Duration::from_secs(10), || {
        b.messages().contains(&9)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(b.messages().len(), expected.len() + 1, "{:?}", b.messages());

    drop(b);
    setup.shutdown().await;
}

/// `subscriber_retention_ms` shorter than the engine's grace period (30 s):
/// a consumer back after the retention, within the grace period, takes its
/// id back and receives its backlog and what its queue received within the
/// retention; a message published after the retention went to the bare
/// topic, not to its queue.
#[tokio::test(flavor = "multi_thread")]
async fn a_consumer_back_after_the_retention_misses_what_was_published_after_it() {
    let Some(engine) = spawn_engine(3).await else {
        return;
    };
    let setup = Setup::start(
        &engine,
        QueueConfig {
            subscriber_retention_ms: 5_000,
            ..QueueConfig::default()
        },
    )
    .await;
    let observer_id = format!("restart-consumer-{}", Uuid::new_v4().simple());

    // A acks 1 and holds 2 unanswered.
    let mut a = Consumer::start(&setup.spec(&observer_id, Some(2)));
    wait_until("A's observer", Duration::from_secs(30), || {
        a.ready().is_some()
    })
    .await;
    assert_eq!(a.ready().unwrap()["state"], json!("active"));
    wait_for_subscribers(&setup.boot.subscribers, &setup.topic, 1).await;
    for n in [1, 2] {
        setup.publish(n).await;
    }
    wait_until("A holds 2", Duration::from_secs(10), || {
        a.messages().contains(&2)
    })
    .await;

    a.kill();
    let killed_at = Instant::now();
    wait_for_subscribers(&setup.boot.subscribers, &setup.topic, 0).await;
    // Within the retention: the subscriber's queue receives it.
    setup.publish(3).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while setup.topic_routed().await {
        assert!(
            Instant::now() < deadline,
            "the retention did not end: the topic still routes to the queue"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // After the retention: the topic routes to no queue.
    setup.publish(4).await;

    // B re-observes the id after the retention, within the grace period.
    let b = Consumer::start(&setup.spec(&observer_id, None));
    wait_until("B's observer", Duration::from_secs(30), || {
        b.ready().is_some()
    })
    .await;
    let ready = b.ready().unwrap();
    assert_eq!(ready["id"], json!(observer_id), "{ready}");
    assert_eq!(ready["state"], json!("active"), "{ready}");
    assert!(
        killed_at.elapsed() < Duration::from_secs(30),
        "B re-observed {:?} after the kill, past the engine's grace period",
        killed_at.elapsed()
    );
    wait_for_subscribers(&setup.boot.subscribers, &setup.topic, 1).await;

    setup.publish(5).await;
    let received = eventually(Duration::from_secs(30), || {
        let got = b.messages();
        [2, 3, 5].iter().all(|n| got.contains(n))
    })
    .await;
    assert!(
        received,
        "B received {:?}, expected 2, 3 and 5",
        b.messages()
    );
    // Past the retry backoff: nothing arrives twice, and 4 never arrives.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let mut got = b.messages();
    got.sort_unstable();
    assert_eq!(got, vec![2, 3, 5], "B received {:?}", b.messages());

    drop(a);
    drop(b);
    setup.shutdown().await;
}

/// The consumer [`Consumer::start`] runs: this test binary, in a child
/// process, with [`CONSUMER_ENV`] set. It observes `queue::on-message` for
/// the topic under the given observer id and reports on stdout until it is
/// killed. Run by itself (without [`CONSUMER_ENV`]) it returns at once.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a consumer process of the tests above, run by them"]
async fn consumer_process() {
    if std::env::var_os(CONSUMER_ENV).is_none() {
        return;
    }
    let var = |key: &str| std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set"));
    let report = |event: Value| {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{EVENT_PREFIX}{event}");
        let _ = stdout.flush();
    };
    let hang_from = std::env::var("QUEUE_E2E_HANG_FROM")
        .ok()
        .and_then(|value| value.parse::<i64>().ok());

    let iii = connect(
        &var("QUEUE_E2E_URL"),
        &var("QUEUE_E2E_NAMESPACE"),
        &var("QUEUE_E2E_WORKER_NAME"),
    )
    .await;
    iii.register_function(
        TARGET_FN,
        RegisterFunction::new_async(move |message: Value| async move {
            let n = message["n"].as_i64().unwrap_or_default();
            report(json!({ "ev": "msg", "n": n }));
            if hang_from.is_some_and(|from| n >= from) {
                std::future::pending::<()>().await;
            }
            Ok::<_, Error>(json!({ "ok": true }))
        }),
    );
    let observer = iii.observe_with(
        Source::function(ON_MESSAGE_FN).args(json!({
            "topic": var("QUEUE_E2E_TOPIC"),
            "max_retries": 5,
            "backoff_ms": 200,
        })),
        TARGET_FN,
        ObserverOptions::default().id(var("QUEUE_E2E_OBSERVER_ID")),
    );
    match tokio::time::timeout(Duration::from_secs(20), observer.ready()).await {
        Ok(Ok(())) => report(json!({
            "ev": "ready",
            "id": observer.id(),
            "state": format!("{:?}", observer.state()).to_lowercase(),
        })),
        Ok(Err(error)) => report(json!({ "ev": "rejected", "error": error.to_string() })),
        Err(_) => report(json!({ "ev": "rejected", "error": "observe not answered" })),
    }
    std::future::pending::<()>().await;
}
