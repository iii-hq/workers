//! `storage::on-object-created` / `storage::on-object-deleted` delivery
//! through an isolated real engine (`III_ENGINE_BIN`, on an ephemeral
//! 127.0.0.1 port; the test skips when it is not set). The functions and
//! their hooks are registered in-process with `events::source::register`, and
//! events go through `EngineDispatcher`, as the notifications sources send
//! them: an event reaches only the observers of its bucket, and its call
//! answers `{ acked }`, false when a target returns `{ ack: false }`, fails
//! or runs past its `handler_timeout_ms`.

use std::collections::HashSet;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iii_sdk::runtime::{IIIConnectionState, WorkerMetadata};
use iii_sdk::{
    register_worker, Call, CallRequest, IIIClient, InitOptions, RegisterFunction, Source,
};
use serde_json::{json, Value};
use storage::events::dispatcher::{EngineDispatcher, EventDispatcher};
use storage::events::normalize::{EventKind, ObjectEventNormalized};
use storage::events::object_created::ON_OBJECT_CREATED_FN;
use storage::events::object_deleted::ON_OBJECT_DELETED_FN;
use storage::events::source::{self, WiredBuckets};
use tokio::sync::RwLock;

struct Engine {
    child: Child,
    url: String,
    _root: tempfile::TempDir,
}

impl Engine {
    async fn start() -> Option<Self> {
        let binary = PathBuf::from(std::env::var_os("III_ENGINE_BIN")?);
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("engine.yaml");
        std::fs::write(
            &config,
            format!(
                "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n"
            ),
        )
        .unwrap();
        let child = Command::new(binary)
            .args(["--no-update-check", "--config"])
            .arg(&config)
            .current_dir(root.path())
            .env("III_TELEMETRY_ENABLED", "false")
            .env("OTEL_ENABLED", "false")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let engine = Self {
            child,
            url: format!("ws://127.0.0.1:{port}"),
            _root: root,
        };
        let probe = engine.client("engine-probe").await;
        tokio::time::timeout(Duration::from_secs(30), async {
            while call(&probe, "engine::functions::list", json!({}))
                .await
                .is_err()
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the engine answers");
        probe.shutdown_async().await;
        Some(engine)
    }

    async fn client(&self, name: &str) -> IIIClient {
        let iii = register_worker(
            &self.url,
            InitOptions {
                metadata: Some(WorkerMetadata {
                    name: name.into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        tokio::time::timeout(Duration::from_secs(30), async {
            while iii.get_connection_state() != IIIConnectionState::Connected {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("connected to the engine");
        iii
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, iii_sdk::Error> {
    iii.call(CallRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
}

async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what}"));
}

fn object(kind: EventKind, bucket: &str, key: &str) -> ObjectEventNormalized {
    ObjectEventNormalized {
        bucket: bucket.into(),
        key: key.into(),
        size: 3,
        etag: "etag".into(),
        content_type: Some("text/plain".into()),
        created_at: None,
        deleted_at: None,
        event_kind: kind,
        raw_event_id: format!("raw-{key}"),
    }
}

/// How the `uploads` target answers.
#[derive(Clone, Copy)]
enum Answer {
    Null,
    Ack,
    Nack,
    Fail,
    Slow,
}

type Log = Arc<Mutex<Vec<Value>>>;

fn keys(log: &Log) -> Vec<String> {
    log.lock()
        .unwrap()
        .iter()
        .map(|event| event["key"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_event_reaches_its_bucket_and_answers_whether_its_targets_acked() {
    let Some(engine) = Engine::start().await else {
        eprintln!("skipping: set III_ENGINE_BIN to run against a real engine");
        return;
    };
    let iii = engine.client("storage").await;
    let wired: WiredBuckets = Arc::new(RwLock::new(HashSet::from([
        "uploads".to_string(),
        "archive".to_string(),
    ])));
    source::register(&iii, wired, Arc::new(RwLock::new(())));
    let consumer = engine.client("consumer").await;

    let answer = Arc::new(Mutex::new(Answer::Null));
    let uploads: Log = Arc::default();
    let (log, mode) = (uploads.clone(), answer.clone());
    consumer.register_function(
        "consumer::on-upload",
        RegisterFunction::new_async(move |event: Value| {
            let (log, mode) = (log.clone(), mode.clone());
            async move {
                log.lock().unwrap().push(event);
                let mode = *mode.lock().unwrap();
                match mode {
                    Answer::Null => Ok(Value::Null),
                    Answer::Ack => Ok(json!({ "ack": true })),
                    Answer::Nack => Ok(json!({ "ack": false })),
                    Answer::Fail => Err(iii_sdk::Error::Handler("thumbnailer down".into())),
                    Answer::Slow => {
                        tokio::time::sleep(Duration::from_millis(1_500)).await;
                        Ok(Value::Null)
                    }
                }
            }
        }),
    );
    let archive: Log = Arc::default();
    let log = archive.clone();
    consumer.register_function(
        "consumer::on-archive",
        RegisterFunction::new(move |event: Value| {
            log.lock().unwrap().push(event);
            Ok::<_, iii_sdk::Error>(Value::Null)
        }),
    );
    let observers = [
        consumer.observe(
            Source::function(ON_OBJECT_CREATED_FN)
                .args(json!({ "bucket": "uploads", "handler_timeout_ms": 500 })),
            "consumer::on-upload",
        ),
        consumer.observe(
            Source::function(ON_OBJECT_CREATED_FN).args(json!({ "bucket": "archive" })),
            "consumer::on-archive",
        ),
        consumer.observe(
            Source::function(ON_OBJECT_DELETED_FN).args(json!({ "bucket": "uploads" })),
            "consumer::on-upload",
        ),
    ];
    for observer in &observers {
        tokio::time::timeout(Duration::from_secs(10), observer.ready())
            .await
            .expect("observer answered")
            .expect("a wired bucket admits its observer");
    }
    // A bucket without a wired notifications source refuses its observer.
    let refused = consumer.observe(
        Source::function(ON_OBJECT_CREATED_FN).args(json!({ "bucket": "elsewhere" })),
        "consumer::on-archive",
    );
    let error = tokio::time::timeout(Duration::from_secs(10), refused.ready())
        .await
        .expect("observer answered")
        .unwrap_err();
    assert!(error.to_string().contains("CONFIG_ERROR"), "{error}");
    let probe = |bucket: &str| Call {
        input: json!({ "bucket": bucket }),
        ..Call::default()
    };
    until("the storage worker sees the observers", || {
        iii.is_observed_for(ON_OBJECT_CREATED_FN, &probe("uploads"))
            && iii.is_observed_for(ON_OBJECT_CREATED_FN, &probe("archive"))
            && iii.is_observed_for(ON_OBJECT_DELETED_FN, &probe("uploads"))
    })
    .await;
    assert!(!iii.is_observed_for(ON_OBJECT_DELETED_FN, &probe("archive")));

    let dispatcher = EngineDispatcher::new(iii.clone());
    let created = |bucket: &str, key: &str| object(EventKind::Created, bucket, key);

    // Each bucket's observers get its events, and only those.
    assert!(dispatcher.dispatch(created("uploads", "a.txt")).await);
    assert!(dispatcher.dispatch(created("archive", "b.txt")).await);
    assert!(
        dispatcher
            .dispatch(object(EventKind::Deleted, "uploads", "c.txt"))
            .await
    );
    assert_eq!(keys(&uploads), ["a.txt", "c.txt"]);
    assert_eq!(keys(&archive), ["b.txt"]);
    assert_eq!(uploads.lock().unwrap()[0]["bucket"], "uploads");
    assert!(uploads.lock().unwrap()[1]["deleted_at"].is_string());

    // The answer is acked only when every target acked.
    *answer.lock().unwrap() = Answer::Ack;
    assert!(dispatcher.dispatch(created("uploads", "ack.txt")).await);
    *answer.lock().unwrap() = Answer::Nack;
    assert!(!dispatcher.dispatch(created("uploads", "nack.txt")).await);
    *answer.lock().unwrap() = Answer::Fail;
    assert!(!dispatcher.dispatch(created("uploads", "fail.txt")).await);
    *answer.lock().unwrap() = Answer::Slow;
    let started = Instant::now();
    assert!(!dispatcher.dispatch(created("uploads", "slow.txt")).await);
    assert!(
        started.elapsed() < Duration::from_millis(1_500),
        "the handler timeout bounds the call: {:?}",
        started.elapsed()
    );
    assert_eq!(
        keys(&uploads),
        ["a.txt", "c.txt", "ack.txt", "nack.txt", "fail.txt", "slow.txt"]
    );

    // An event of a bucket nobody observes is acked without a call.
    assert!(dispatcher.dispatch(created("quiet", "d.txt")).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(keys(&archive), ["b.txt"]);
    assert_eq!(uploads.lock().unwrap().len(), 6);

    consumer.shutdown_async().await;
    iii.shutdown_async().await;
}
