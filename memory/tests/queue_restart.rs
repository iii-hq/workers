//! The extraction consumer across restarts, against a real engine. The
//! worker observes `queue::on-message` under a stable id
//! (`memory::hooks::extraction_observer_id`), so:
//!
//! - a killed worker, started again under the same worker name, gets the
//!   extraction jobs it left unacked, once each: the new process's `observe`
//!   replaces the observer the killed one left suspended, and the queue
//!   worker hands the new process the backlog;
//! - while another worker holds the id, the worker observes again until the
//!   id is free, 5 s after its first refusal and 10 s after its second (the
//!   refused observer's dispose, which the SDK waits 5 s on for an id the
//!   engine refused, runs during the wait).
//!
//! Runs the `memory` binary against `III_ENGINE_WS_URL` (default
//! `ws://127.0.0.1:49134`), with a contract mock of the queue worker
//! (`queue::on-message`, as queue/src/on_message.rs serves it) and
//! `session::messages` / `state::get` on connections of this test. Skips
//! when no engine answers; `III_E2E_REQUIRE=1` makes that a failure.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, Call, CallRequest, Deliver, Error, IIIClient, InitOptions, Match,
    ObserveRejection, Observer, ObserverOptions, RegisterFunction, Source,
};
use memory::hooks::{extraction_observer_id, EXTRACTION_QUEUE, EXTRACT_JOB_FN, WORKER_NAME};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

const ON_MESSAGE: &str = "queue::on-message";
const RECORDER: &str = "memory-restart-test::extracted";
const HOLDER: &str = "memory-restart-test::held";

/// One memory process at a time per engine: the worker's functions and its
/// name have one live holder each.
static ONE_WORKER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".to_string())
}

/// A client named `<name>-<pid>` once the engine answers, or `None` (skip)
/// when it never does.
async fn connect(url: &str, name: &str) -> Option<Arc<IIIClient>> {
    let iii = Arc::new(register_worker(
        url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: format!("{name}-{}", std::process::id()),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    for _ in 0..20 {
        let probe = iii
            .call(CallRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        if probe.is_ok() {
            return Some(iii);
        }
        sleep(Duration::from_millis(250)).await;
    }
    iii.shutdown_async().await;
    assert!(
        std::env::var("III_E2E_REQUIRE").as_deref() != Ok("1"),
        "III_E2E_REQUIRE=1 but no engine answers at {url}"
    );
    eprintln!("skipping: no iii engine at {url}; set III_ENGINE_WS_URL");
    None
}

/// The memory binary, its store and log in `dir`.
struct Memory {
    child: Child,
    log: PathBuf,
}

impl Memory {
    /// Starts the worker; `worker_name` is the name its supervisor gives it
    /// (`III_WORKER_NAME`), `None` for its own. Its configuration entry is
    /// one of this test process's own, seeded from `dir`.
    fn start(url: &str, dir: &Path, run: usize, worker_name: Option<&str>) -> Self {
        let config_name = format!(
            "{}-test-{}",
            worker_name.unwrap_or(WORKER_NAME),
            std::process::id()
        );
        let log = dir.join(format!("memory-{run}.log"));
        let out = std::fs::File::create(&log).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_memory"));
        command
            .arg("--config")
            .arg(dir.join("config.yaml"))
            .current_dir(dir)
            .env("III_URL", url)
            .env("III_TELEMETRY_ENABLED", "false")
            .env("OTEL_ENABLED", "false")
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1")
            .env("III_CONFIG_NAME", config_name)
            .env_remove("III_NAMESPACE")
            .env_remove("III_COMPOSE_DIR")
            .stdout(out.try_clone().unwrap())
            .stderr(out);
        match worker_name {
            Some(name) => command.env("III_WORKER_NAME", name),
            None => command.env_remove("III_WORKER_NAME"),
        };
        Self {
            child: command.spawn().unwrap(),
            log,
        }
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

impl Drop for Memory {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The seed config: the store under `dir`, no embeddings.
fn write_config(dir: &Path) {
    std::fs::write(
        dir.join("config.yaml"),
        format!(
            "data_dir: {}\nembeddings_enabled: false\n",
            dir.join("data").display()
        ),
    )
    .unwrap();
}

/// The queue worker's `queue::on-message` contract: each observer is a
/// subscriber whose durable backlog, filed under its id, outlives it; a
/// message is a call carrying `metadata.subscriber`, matched by key, and the
/// deliver hook hands it to that observer's target, whose result acks it and
/// whose error leaves it queued.
#[derive(Clone, Default)]
struct Queue {
    observers: Arc<Mutex<Vec<Observer>>>,
    /// Each subscriber's topic and backlog, by observer id.
    backlogs: Arc<Mutex<BTreeMap<String, Backlog>>>,
}

/// A subscriber's topic and the messages waiting for it.
type Backlog = (String, VecDeque<Value>);

impl Queue {
    fn register(&self, iii: &Arc<IIIClient>) {
        let added = self.clone();
        let removed = self.clone();
        let engine = iii.clone();
        iii.on_observer_added_for(ON_MESSAGE, move |obs: Observer| {
            let added = added.clone();
            async move {
                let topic = obs.args_value()["topic"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                added
                    .backlogs
                    .lock()
                    .unwrap()
                    .entry(obs.id().to_owned())
                    .or_insert_with(|| (topic, VecDeque::new()));
                added.observers.lock().unwrap().push(obs);
                Ok::<(), ObserveRejection>(())
            }
        })
        .on_observer_removed_for(ON_MESSAGE, move |obs: Observer| {
            let removed = removed.clone();
            async move {
                removed
                    .observers
                    .lock()
                    .unwrap()
                    .retain(|o| o.id() != obs.id());
            }
        })
        .on_observer_match_for(
            ON_MESSAGE,
            Match::keys(
                |obs| obs.id().into(),
                |call| {
                    call.metadata
                        .as_ref()
                        .and_then(|m| m["subscriber"].as_str())
                        .into()
                },
            ),
        )
        .on_observer_deliver_for(
            ON_MESSAGE,
            move |call: Call, observers: Vec<Observer>| {
                let engine = engine.clone();
                async move {
                    let Some(obs) = observers.first() else {
                        return Err(Error::from(ObserveRejection::new(
                            "OBSERVER_NOT_ACTIVE",
                            "the subscriber is not active",
                        )));
                    };
                    engine
                        .call(
                            CallRequest::observer(obs, call.input).timeout(Duration::from_secs(10)),
                        )
                        .await
                }
            },
        );
        iii.register_function(
            ON_MESSAGE,
            RegisterFunction::new(|message: Value| Ok::<_, Error>(message))
                .description("The queue worker's contract, for the restart test")
                .args_format_value(json!({ "type": "object" }))
                .private(),
        );
    }

    /// Ids of the extraction topic's observers the engine announced and has
    /// not withdrawn (the engine observes other topics of its own).
    fn held(&self) -> Vec<String> {
        self.observers
            .lock()
            .unwrap()
            .iter()
            .filter(|o| o.args_value()["topic"] == EXTRACTION_QUEUE)
            .map(|o| o.id().to_owned())
            .collect()
    }

    /// Publishes `data` to every subscriber of `topic`, present or gone.
    fn publish(&self, topic: &str, data: Value) {
        for (subscribed, backlog) in self.backlogs.lock().unwrap().values_mut() {
            if subscribed == topic {
                backlog.push_back(data.clone());
            }
        }
    }

    /// Messages waiting, consumed or not.
    fn len(&self) -> usize {
        self.backlogs
            .lock()
            .unwrap()
            .values()
            .map(|(_, backlog)| backlog.len())
            .sum()
    }

    /// Delivers the backlog of every subscriber that has an observer, one
    /// message at a time; a backlog nobody consumes stays. A failed delivery
    /// is put back and fails the drain.
    async fn drain(&self, iii: &IIIClient) -> Result<(), Error> {
        for _ in 0..100 {
            let next = {
                let held = self.held();
                let mut backlogs = self.backlogs.lock().unwrap();
                backlogs.iter_mut().find_map(|(id, (_, backlog))| {
                    if !held.contains(id) {
                        return None;
                    }
                    Some((id.clone(), backlog.pop_front()?))
                })
            };
            let Some((id, message)) = next else {
                return Ok(());
            };
            let call = CallRequest::new(ON_MESSAGE, message.clone())
                .metadata(json!({ "subscriber": id }))
                .timeout(Duration::from_secs(10));
            if let Err(error) = iii.call(call).await {
                if let Some((_, backlog)) = self.backlogs.lock().unwrap().get_mut(&id) {
                    backlog.push_front(message);
                }
                return Err(error);
            }
        }
        panic!("the drain exceeded its budget");
    }
}

/// The siblings an extraction job with `skip: true` reads: no cursor, no
/// messages, so the job steps nowhere and succeeds.
fn register_siblings(iii: &IIIClient) {
    iii.register_function(
        "state::get",
        RegisterFunction::new(|_: Value| Ok::<_, Error>(Value::Null)),
    );
    iii.register_function(
        "session::messages",
        RegisterFunction::new(|_: Value| Ok::<_, Error>(json!({ "messages": [] }))),
    );
}

/// Every job `memory::extract-job` handled, by its session id.
fn record_jobs(iii: &IIIClient) -> Arc<Mutex<Vec<String>>> {
    let handled: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = handled.clone();
    iii.register_function(
        RECORDER,
        RegisterFunction::new(move |input: Value| {
            let job = input["session_id"].as_str().unwrap_or_default().to_owned();
            seen.lock().unwrap().push(job);
            Ok::<_, Error>(Value::Null)
        }),
    );
    let _ = iii.observe(
        Source::function(EXTRACT_JOB_FN).deliver(Deliver::Input),
        RECORDER,
    );
    handled
}

fn job(session_id: &str) -> Value {
    json!({ "session_id": session_id, "skip": true })
}

/// The observers of `function_id` with target `target`, as the engine lists
/// them.
async fn observers(iii: &IIIClient, function_id: &str, target: &str) -> Vec<Value> {
    let listed = iii
        .call(CallRequest::new(
            "engine::observers::list",
            json!({ "function_id": function_id }),
        ))
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

/// Waits until `ready` holds for those observers; panics with them and the
/// worker's log when it does not within 30 s.
async fn wait_observers(
    iii: &IIIClient,
    (function_id, target): (&str, &str),
    memory: &Memory,
    ready: impl Fn(&[Value]) -> bool,
) -> Vec<Value> {
    let mut last = Vec::new();
    for _ in 0..600 {
        last = observers(iii, function_id, target).await;
        if ready(&last) {
            return last;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "observers of {function_id} -> {target} not ready: {last:#?}\nmemory log:\n{}",
        memory.log()
    );
}

/// The only one of them, active.
fn one_active(observers: &[Value]) -> bool {
    observers.len() == 1 && observers[0]["state"] == "active"
}

async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    timeout(Duration::from_secs(30), async {
        while !ready() {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting until {what}"));
}

fn count(handled: &Mutex<Vec<String>>, job: &str) -> usize {
    handled.lock().unwrap().iter().filter(|j| *j == job).count()
}

/// The extraction consumer's `observing` and `observer refused` lines in a
/// worker log, in order, each with its timestamp.
fn consumer_events(log: &str) -> Vec<(&'static str, chrono::DateTime<chrono::FixedOffset>)> {
    log.lines()
        .filter(|line| line.contains(&format!("function_id=\"{ON_MESSAGE}\"")))
        .filter_map(|line| {
            let event = if line.contains(" memory: observing ") {
                "observing"
            } else if line.contains(" memory: observer refused ") {
                "refused"
            } else {
                return None;
            };
            let at = chrono::DateTime::parse_from_rfc3339(line.split_whitespace().next()?).ok()?;
            Some((event, at))
        })
        .collect()
}

const CONSUMER: (&str, &str) = (ON_MESSAGE, EXTRACT_JOB_FN);
const RECORDED: (&str, &str) = (EXTRACT_JOB_FN, RECORDER);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_killed_worker_gets_its_unacked_extraction_jobs_once_after_a_restart() {
    let _one = ONE_WORKER.lock().await;
    let url = ws_url();
    let Some(host) = connect(&url, "memory-restart-queue").await else {
        return;
    };
    let recorder = connect(&url, "memory-restart-recorder")
        .await
        .expect("a second client connects");
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path());
    let stable = extraction_observer_id(None, WORKER_NAME, EXTRACTION_QUEUE);
    assert_eq!(stable, "memory:queue:memory-extraction");

    let queue = Queue::default();
    queue.register(&host);
    register_siblings(&host);
    let handled = record_jobs(&recorder);

    let mut first = Memory::start(&url, dir.path(), 1, None);
    let listed = wait_observers(&host, CONSUMER, &first, one_active).await;
    assert_eq!(listed[0]["id"], stable.as_str(), "{listed:#?}");
    assert_eq!(
        listed[0]["worker"]["worker_name"], WORKER_NAME,
        "{listed:#?}"
    );
    wait_until("the queue holds the consumer", || {
        queue.held() == [stable.clone()]
    })
    .await;
    wait_observers(&host, RECORDED, &first, one_active).await;

    // A job the first process handles and acks.
    queue.publish(EXTRACTION_QUEUE, job("acked-before"));
    queue
        .drain(&host)
        .await
        .expect("the first process acks the job");
    assert_eq!(queue.len(), 0);
    wait_until("the first process handles a job", || {
        count(&handled, "acked-before") == 1
    })
    .await;

    // Jobs the first process never acks: it is killed with them queued.
    let backlog = ["left-1", "left-2", "left-3"];
    for session in backlog {
        queue.publish(EXTRACTION_QUEUE, job(session));
    }
    assert_eq!(queue.len(), backlog.len());
    first.kill();

    // The engine keeps the observer suspended for its grace period and
    // withdraws it from the queue worker, which keeps the backlog.
    wait_observers(&host, CONSUMER, &first, |o| {
        o.len() == 1 && o[0]["state"] == "suspended"
    })
    .await;
    wait_until("the queue lets the consumer go", || queue.held().is_empty()).await;
    queue.drain(&host).await.unwrap();
    assert_eq!(
        queue.len(),
        backlog.len(),
        "nothing consumes the backlog meanwhile"
    );

    // The same worker, started again: its observe replaces the suspended
    // observer, and the queue holds the new one under the same id.
    let second = Memory::start(&url, dir.path(), 2, None);
    let listed = wait_observers(&host, CONSUMER, &second, one_active).await;
    assert_eq!(listed[0]["id"], stable.as_str(), "{listed:#?}");
    // Its first observe took the id: no refusal waited out the grace period.
    assert!(
        !second.log().contains("observer refused"),
        "memory log:\n{}",
        second.log()
    );
    wait_until("the queue holds the new consumer", || {
        queue.held() == [stable.clone()]
    })
    .await;
    wait_observers(&host, RECORDED, &second, one_active).await;

    // The backlog reaches the new process, once each.
    queue
        .drain(&host)
        .await
        .expect("the new process acks the backlog");
    assert_eq!(queue.len(), 0);
    for session in backlog {
        wait_until(&format!("the new process handles {session}"), || {
            count(&handled, session) >= 1
        })
        .await;
    }
    queue.drain(&host).await.unwrap();
    sleep(Duration::from_millis(500)).await;
    for session in backlog.iter().chain(&["acked-before"]) {
        assert_eq!(
            count(&handled, session),
            1,
            "{session} handled once: {:?}",
            handled.lock().unwrap()
        );
    }

    drop(second);
    recorder.shutdown_async().await;
    host.shutdown_async().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_consumer_observes_again_on_its_schedule_until_another_worker_lets_its_id_go() {
    let _one = ONE_WORKER.lock().await;
    let url = ws_url();
    let Some(host) = connect(&url, "memory-held-queue").await else {
        return;
    };
    let holder = connect(&url, "memory-held-holder")
        .await
        .expect("a second client connects");
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path());
    // A worker name of its own: the other test's killed process may still
    // hold `memory:queue:memory-extraction` for its grace period.
    let name = format!("memory-held-{}", std::process::id());
    let stable = extraction_observer_id(None, &name, EXTRACTION_QUEUE);

    let queue = Queue::default();
    queue.register(&host);
    register_siblings(&host);
    let handled = record_jobs(&host);

    // Another worker observes the topic under the worker's id first.
    holder.register_function(
        HOLDER,
        RegisterFunction::new(|_: Value| Ok::<_, Error>(Value::Null)),
    );
    let held = holder.observe_with(
        Source::function(ON_MESSAGE).args(json!({ "topic": EXTRACTION_QUEUE })),
        HOLDER,
        ObserverOptions::default().id(stable.clone()),
    );
    timeout(Duration::from_secs(15), held.ready())
        .await
        .expect("the holder's observer becomes active")
        .expect("the holder's observer is accepted");

    // The worker's observe is refused while the id is held.
    let memory = Memory::start(&url, dir.path(), 1, Some(&name));
    wait_until("the worker's observe is refused", || {
        let log = memory.log();
        log.contains("observer refused") && log.contains("OBSERVER_ID_CONFLICT")
    })
    .await;
    assert_eq!(queue.held(), std::slice::from_ref(&stable));
    wait_observers(&host, (ON_MESSAGE, HOLDER), &memory, one_active).await;
    // It observes again 5 s later, and is refused again.
    wait_until("the worker's second observe is refused", || {
        consumer_events(&memory.log())
            .iter()
            .filter(|(event, _)| *event == "refused")
            .count()
            >= 2
    })
    .await;

    // Once the holder lets the id go, the worker observes again (10 s after
    // the second refusal) and gets it.
    held.dispose().await.expect("dispose");
    let listed = wait_observers(&host, CONSUMER, &memory, one_active).await;
    let events = consumer_events(&memory.log());
    let order: Vec<&str> = events.iter().map(|(event, _)| *event).collect();
    assert_eq!(
        order,
        ["observing", "refused", "observing", "refused", "observing"],
        "memory log:\n{}",
        memory.log()
    );
    // Each wait is the schedule's, from the refusal to the next observe: the
    // refused observer's dispose (5 s in the SDK for an id the engine
    // refused) runs during it and adds nothing.
    for (refusal, wait_ms) in [(1, 5_000), (3, 10_000)] {
        let waited = (events[refusal + 1].1 - events[refusal].1).num_milliseconds();
        assert!(
            (wait_ms - 50..wait_ms + 900).contains(&waited),
            "observed again {waited} ms after refusal {}, not {wait_ms} ms\nmemory log:\n{}",
            refusal / 2 + 1,
            memory.log()
        );
    }
    assert_eq!(listed[0]["id"], stable.as_str(), "{listed:#?}");
    assert_eq!(
        listed[0]["worker"]["worker_name"],
        name.as_str(),
        "{listed:#?}"
    );
    wait_until("the queue holds the worker's consumer", || {
        queue.held() == [stable.clone()]
    })
    .await;
    wait_observers(&host, RECORDED, &memory, one_active).await;

    // The subscription's backlog (filed under the id) reaches the worker.
    queue.publish(EXTRACTION_QUEUE, job("after-the-holder"));
    queue.drain(&host).await.expect("the worker acks the job");
    wait_until("the worker handles the job", || {
        count(&handled, "after-the-holder") == 1
    })
    .await;

    drop(memory);
    holder.shutdown_async().await;
    host.shutdown_async().await;
}
