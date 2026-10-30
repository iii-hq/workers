//! A killed github worker, started again under the same worker name, keeps
//! its queue observer's identity: it observes under a stable id
//! (`github::webhooks::queue_observer_id`), so the new process's observe
//! replaces the observer the killed one left suspended, and the engine and
//! the queue worker each hold one observer for it, under that id. The jobs
//! the killed process left unacked wait in the durable queue of its consumer
//! group, and reach the new process once each.
//!
//! Runs the `github` binary against a real engine (`III_ENGINE_BIN`), with
//! the contract mock of the queue worker and the http worker on connections
//! of this test. Skips when `III_ENGINE_BIN` is not set.

#[path = "support/webhook_mocks.rs"]
#[allow(dead_code)]
mod mocks;
#[path = "support/real_engine.rs"]
mod real_engine;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_http::{
    boot,
    config::{RestApiConfig, WebhookListenerConfig},
};
use iii_sdk::{Deliver, Error, IIIClient, RegisterFunction, Source};
use mocks::Queue;
use real_engine::{call, RealEngine};
use serde_json::{json, Value};

const PROCESS: &str = "github::webhooks::process";
const RECORDER: &str = "test::processed";
/// The github worker's observers this test follows.
const QUEUE_ON_MESSAGE: (&str, &str) = ("queue::on-message", PROCESS);
const HTTP_ON_REQUEST: (&str, &str) = ("http::on-request", "github::webhooks::receive");
const RECORDED: (&str, &str) = (PROCESS, RECORDER);

/// The github binary, started with webhooks enabled.
struct Github {
    child: Child,
    log: PathBuf,
}

impl Github {
    fn start(engine: &RealEngine, dir: &Path, run: usize) -> Self {
        let log = dir.join(format!("github-{run}.log"));
        let out = std::fs::File::create(&log).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_github"))
            .arg("--config")
            .arg(dir.join("config.yaml"))
            .current_dir(dir)
            .env("III_URL", &engine.url)
            .env("III_TELEMETRY_ENABLED", "false")
            .env("OTEL_ENABLED", "false")
            .env("RUST_LOG", "info")
            .env_remove("III_WORKER_NAME")
            .env_remove("III_NAMESPACE")
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .spawn()
            .unwrap();
        Self { child, log }
    }

    /// SIGKILL: the process never says goodbye to the engine.
    fn kill(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Github {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The seed config of the github binary: webhooks enabled, the offline gh
/// fixture, the store in `dir`.
fn write_config(dir: &Path) {
    let gh_state = dir.join("gh-state.json");
    std::fs::write(
        &gh_state,
        json!({"prs":{}, "hook":null, "calls":[]}).to_string(),
    )
    .unwrap();
    let gh = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/integration-gh.py");
    let config = json!({
        "gh_executable": gh,
        "token": gh_state,
        "webhooks": {
            "enabled": true,
            "storage_path": dir.join("store/store.sqlite3"),
        },
    });
    std::fs::write(
        dir.join("config.yaml"),
        serde_yaml::to_string(&config).unwrap(),
    )
    .unwrap();
}

/// The observers of `function_id` with target `target`, as the engine lists
/// them (the engine observes `queue::on-message` itself too).
async fn observers(iii: &IIIClient, function_id: &str, target: &str) -> Vec<Value> {
    let listed = call(iii, "engine::observers::list", json!({}))
        .await
        .expect("engine::observers::list");
    listed["observers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["function_id"] == function_id && o["target"]["function_id"] == target)
        .cloned()
        .collect()
}

/// Waits until `ready` holds for the observers of `function_id` with target
/// `target`; panics with them and the github log when it does not within
/// 30 s.
async fn wait_observers(
    iii: &IIIClient,
    (function_id, target): (&str, &str),
    github: &Github,
    ready: impl Fn(&[Value]) -> bool,
) -> Vec<Value> {
    let mut last = Vec::new();
    for _ in 0..600 {
        last = observers(iii, function_id, target).await;
        if ready(&last) {
            return last;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "observers of {function_id} not ready: {last:#?}\ngithub log:\n{}",
        github.log()
    );
}

async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what} within 30 s"));
}

fn ids(observers: &[Value]) -> Vec<&str> {
    observers.iter().filter_map(|o| o["id"].as_str()).collect()
}

/// The ids of the github observers the queue worker holds.
fn held(queue: &Queue) -> Vec<String> {
    queue
        .observers
        .values()
        .iter()
        .filter(|obs| obs.target() == PROCESS)
        .map(|obs| obs.id().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restarted_worker_keeps_its_queue_observer_id_and_gets_its_unacked_jobs_once() {
    let Some(engine) = RealEngine::start().await else {
        eprintln!("III_ENGINE_BIN is not set; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path());
    let topic = github::webhooks::WebhookConfig::default().queue;
    let stable = github::webhooks::queue_observer_id(None, github::WORKER_NAME, &topic);

    // The queue worker (its contract mock) and the http worker.
    let queue_host = engine.client("queue").await;
    let queue = Queue::default();
    queue.register(&queue_host);
    let http_host = engine.client("http").await;
    let http = boot::start(
        http_host.clone(),
        RestApiConfig {
            host: "127.0.0.1".into(),
            port: 0,
            webhook_listener: Some(WebhookListenerConfig {
                host: "127.0.0.1".into(),
                port: 0,
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // Every job github::webhooks::process handles, as its input.
    let recorder = engine.client("recorder").await;
    let processed: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = processed.clone();
    recorder.register_function(
        RECORDER,
        RegisterFunction::new(move |input: Value| {
            let job = input["job_id"].as_str().unwrap_or_default().to_owned();
            seen.lock().unwrap().push(job);
            Ok::<_, Error>(Value::Null)
        }),
    );
    let _recording = recorder.observe(Source::function(PROCESS).deliver(Deliver::Input), RECORDER);
    let count = |job: &str| {
        processed
            .lock()
            .unwrap()
            .iter()
            .filter(|j| *j == job)
            .count()
    };

    let mut first = Github::start(&engine, dir.path(), 1);
    let listed = wait_observers(&queue_host, QUEUE_ON_MESSAGE, &first, |o| {
        o.len() == 1 && o[0]["state"] == "active"
    })
    .await;
    assert_eq!(ids(&listed), [stable.as_str()], "{listed:#?}");
    assert_eq!(
        listed[0]["worker"]["worker_name"],
        github::WORKER_NAME,
        "{listed:#?}"
    );
    wait_observers(&queue_host, HTTP_ON_REQUEST, &first, |o| {
        o.len() == 1 && o[0]["state"] == "active"
    })
    .await;
    wait_observers(&queue_host, RECORDED, &first, |o| {
        o.len() == 1 && o[0]["state"] == "active"
    })
    .await;

    // A job the first process handles and acks.
    let publish = |job: &'static str| {
        let iii = queue_host.clone();
        let topic = topic.clone();
        async move {
            call(
                &iii,
                "iii::durable::publish",
                json!({ "topic": topic, "data": { "job_id": job } }),
            )
            .await
            .unwrap();
        }
    };
    publish("acked-before").await;
    queue.drain(&queue_host).await.unwrap();
    assert_eq!(queue.len(), 0);
    wait_until("the first process handles a job", || {
        count("acked-before") == 1
    })
    .await;

    // Jobs the first process never acks: it is killed with them queued.
    let backlog = ["left-1", "left-2", "left-3"];
    for job in backlog {
        publish(job).await;
    }
    assert_eq!(queue.len(), backlog.len());
    first.kill();

    // The engine keeps its observer suspended for the grace period and
    // withdraws it from the queue worker, which keeps the backlog.
    wait_observers(&queue_host, QUEUE_ON_MESSAGE, &first, |o| {
        o.len() == 1 && o[0]["state"] == "suspended"
    })
    .await;
    wait_until("the queue worker lets the observer go", || {
        held(&queue).is_empty()
    })
    .await;
    queue.drain(&queue_host).await.unwrap();
    assert_eq!(
        queue.len(),
        backlog.len(),
        "nothing consumes the backlog meanwhile"
    );

    // The same worker, started again: its observe replaces the suspended
    // observer at once, so the engine lists one observer for it, within the
    // grace period, and the queue worker holds the new one under the same id.
    let second = Github::start(&engine, dir.path(), 2);
    let listed = wait_observers(&queue_host, QUEUE_ON_MESSAGE, &second, |o| {
        o.iter().any(|o| o["state"] == "active")
    })
    .await;
    assert_eq!(ids(&listed), [stable.as_str()], "{listed:#?}");
    assert_eq!(listed[0]["state"], "active", "{listed:#?}");
    wait_until("the queue worker holds the new observer", || {
        held(&queue) == [stable.clone()]
    })
    .await;
    // The route is free for the new process within the grace period: the
    // engine withdrew the killed process's observer from the http worker.
    wait_observers(&queue_host, HTTP_ON_REQUEST, &second, |o| {
        o.iter().any(|o| o["state"] == "active")
    })
    .await;
    wait_observers(&queue_host, RECORDED, &second, |o| {
        o.len() == 1 && o[0]["state"] == "active"
    })
    .await;

    // The backlog, kept in the durable queue of the consumer group, reaches
    // the new process, once each.
    queue.drain(&queue_host).await.unwrap();
    assert_eq!(queue.len(), 0);
    for job in backlog {
        wait_until(&format!("the new process handles {job}"), || {
            count(job) >= 1
        })
        .await;
    }
    queue.drain(&queue_host).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let processed = processed.lock().unwrap().clone();
    for job in backlog.iter().chain(&["acked-before"]) {
        assert_eq!(
            processed.iter().filter(|j| j == job).count(),
            1,
            "{job} handled once: {processed:?}"
        );
    }

    drop(second);
    http.shutdown().await;
    for client in [queue_host, http_host, recorder] {
        client.shutdown_async().await;
    }
}
