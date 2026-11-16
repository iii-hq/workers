//! Engine-level checks of `workflow::on-run-completed`: the worker registers
//! it private with the event as request and response and `{run_id}` as its
//! observer args, its handler returns the event, an emitted event reaches the
//! observers that want it, and an observer of one run that already ended gets
//! it replayed through `CallRequest::observer`.
//!
//! The engine binary is `III_ENGINE_BIN`, or `iii` on PATH, started with an
//! engine-only config on an ephemeral port. A test worker stands in for the
//! state worker's `state::get` and `state::set`. Without an engine the tests skip;
//! `III_E2E_REQUIRE=1` makes a missing or unreachable engine a failure.

use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, Call, CallRequest, Capture, IIIClient, InitOptions, Observed, ObserverHandle,
    ObserverOptions, RegisterFunction, Source,
};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

use workflow::events::{
    emit_run_completed, EventDeliverer, RunCompletedEvent, ON_RUN_COMPLETED_FN,
};
use workflow::functions::{self, Deps, TickRequest};
use workflow::types::{NodeCheckpoint, NodeState, RunStatus, WorkflowRunRecord};

const TARGET_FN: &str = "test::workflow-run-failed";

struct Engine {
    url: String,
    child: Child,
    dir: PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn e2e_required() -> bool {
    std::env::var("III_E2E_REQUIRE").is_ok_and(|v| v == "1")
}

fn engine_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("III_ENGINE_BIN") {
        return Some(path.into());
    }
    let on_path = Command::new("iii")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    on_path.then(|| "iii".into())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

async fn call(client: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    client
        .call(CallRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(2000),
        })
        .await
}

async fn spawn_engine() -> Result<Engine, String> {
    let bin = engine_bin().ok_or("no engine binary (set III_ENGINE_BIN or put `iii` on PATH)")?;
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("workflow-it-{}-{port}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).map_err(|e| format!("create {dir:?}: {e}"))?;
    let config = format!(
        "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: \"{}\"\n",
        dir.join("config").display(),
    );
    let config_path = dir.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut f| f.write_all(config.as_bytes()))
        .map_err(|e| format!("write {config_path:?}: {e}"))?;

    let child = Command::new(&bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .env("III_TELEMETRY_ENABLED", "false")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {bin:?}: {e}"))?;
    let engine = Engine {
        url: format!("ws://127.0.0.1:{port}"),
        child,
        dir,
    };

    let probe = register_worker(&engine.url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(30);
    while call(&probe, "engine::workers::list", json!({}))
        .await
        .is_err()
    {
        if Instant::now() > deadline {
            probe.shutdown_async().await;
            return Err(format!("engine {bin:?} did not come up on {}", engine.url));
        }
        sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown_async().await;
    Ok(engine)
}

fn record(run_id: &str, status: RunStatus) -> WorkflowRunRecord {
    WorkflowRunRecord {
        run_id: run_id.to_string(),
        step: 3,
        status,
        abort: false,
        def_ref: run_id.to_string(),
        input: json!({}),
        nodes: BTreeMap::new(),
        fanout_src: BTreeMap::new(),
        result: (status == RunStatus::Completed).then(|| json!({ "answer": 42 })),
        result_error: (status == RunStatus::Failed).then(|| "node 'a': boom".to_string()),
        notify: None,
        caller_session_id: None,
        caller_fs_root: None,
        idem_key: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// Worker names are unique per namespace, so each client of this process
/// names itself.
fn named(name: &str) -> InitOptions {
    InitOptions {
        metadata: Some(WorkerMetadata {
            name: name.to_string(),
            ..WorkerMetadata::default()
        }),
        ..InitOptions::default()
    }
}

async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn run_completed_is_private_returns_the_event_and_reaches_matching_observers() {
    let engine = match spawn_engine().await {
        Ok(engine) => engine,
        Err(reason) if !e2e_required() => {
            eprintln!("skipping: {reason}");
            return;
        }
        Err(reason) => panic!("{reason}"),
    };

    // The workflow worker, registered the way main.rs does it.
    let host = Arc::new(register_worker(&engine.url, named("workflow")));
    let deps = Deps {
        iii: host.clone(),
        cfg: Arc::new(RwLock::new(Arc::new(
            workflow::config::WorkerConfig::default(),
        ))),
        locks: workflow::locks::WorkflowLocks::default(),
        events: host.clone(),
    };
    functions::register_all(&host, &deps);

    // Registered private, with the event as request and response.
    let observer = register_worker(&engine.url, named("workflow-test-observer"));
    let deadline = Instant::now() + Duration::from_secs(10);
    let info = loop {
        match call(
            &observer,
            "engine::functions::info",
            json!({ "function_id": ON_RUN_COMPLETED_FN }),
        )
        .await
        {
            Ok(info) => break info,
            Err(e) => {
                assert!(
                    Instant::now() < deadline,
                    "{ON_RUN_COMPLETED_FN} never registered: {e}"
                );
                sleep(Duration::from_millis(100)).await;
            }
        }
    };
    assert_eq!(info["private"], json!(true), "{info}");
    for schema in ["request_schema", "response_schema"] {
        assert!(
            info[schema]["properties"]["run_id"].is_object()
                && info[schema]["properties"]["status"].is_object(),
            "{schema} is the RunCompletedEvent: {info}"
        );
    }
    assert!(
        info["args_format"]["properties"]["run_id"].is_object(),
        "observer args are {{run_id}}: {info}"
    );
    // main.rs registers workflow::start only after the stamp-reply hook is bound.
    let start = call(
        &observer,
        "engine::functions::info",
        json!({ "function_id": "workflow::start" }),
    )
    .await;
    assert!(
        start.is_err(),
        "register_all registered workflow::start: {start:?}"
    );

    // Its handler returns the event it is called with.
    let event = json!({
        "run_id": "r-direct",
        "status": "completed",
        "result": { "answer": 42 },
        "result_error": null,
    });
    let returned = call(&host, ON_RUN_COMPLETED_FN, event.clone())
        .await
        .expect("the host calls its own function");
    assert_eq!(returned, event);

    // Unobserved: the emitter's gate is closed.
    assert!(!host.is_observed(ON_RUN_COMPLETED_FN));

    // An observer of failed runs only.
    let (tx, mut rx) = mpsc::unbounded_channel::<RunCompletedEvent>();
    observer.register_function(
        TARGET_FN,
        RegisterFunction::new(move |event: Observed<Value, RunCompletedEvent>| {
            if let Some(event) = event.output {
                let _ = tx.send(event);
            }
            Ok::<_, Error>(json!({}))
        }),
    );
    let handle = observer.observe(
        Source::function(ON_RUN_COMPLETED_FN)
            .filters(json!({ "input.status": { "$eq": "failed" } }))
            .capture([Capture::Output]),
        TARGET_FN,
    );
    timeout(Duration::from_secs(10), handle.ready())
        .await
        .expect("observer became active in time")
        .expect("observer active");
    wait_until("the host to see the observer", || {
        host.is_observed(ON_RUN_COMPLETED_FN)
    })
    .await;

    emit_run_completed(host.as_ref(), &record("r-ok", RunStatus::Completed)).await;
    emit_run_completed(host.as_ref(), &record("r-fail", RunStatus::Failed)).await;

    let received = timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("the failed run reached the observer")
        .expect("channel open");
    assert_eq!(
        received,
        RunCompletedEvent {
            run_id: "r-fail".into(),
            status: RunStatus::Failed,
            result: None,
            result_error: Some("node 'a': boom".into()),
            failed_nodes: vec![],
        }
    );
    // The completed run does not match the observer's filters.
    assert!(
        timeout(Duration::from_millis(500), rx.recv())
            .await
            .is_err(),
        "only the failed run is delivered"
    );

    handle.dispose().await.expect("dispose the observer");
    observer.shutdown_async().await;
    host.shutdown_async().await;
}

// ---------------------------------------------------------------------------
// One run's observer and its replay
// ---------------------------------------------------------------------------

/// How long a target is given to receive what it is owed.
const DELIVERY: Duration = Duration::from_secs(20);
/// How long a target owed nothing more is watched for.
const QUIET: Duration = Duration::from_millis(1500);

async fn engine_or_skip() -> Option<Engine> {
    match spawn_engine().await {
        Ok(engine) => Some(engine),
        Err(reason) if !e2e_required() => {
            eprintln!("skipping: {reason}");
            None
        }
        Err(reason) => panic!("{reason}"),
    }
}

/// The run records the stand-in `state::get` answers from.
type Runs = Arc<Mutex<HashMap<String, Value>>>;

/// A worker standing in for the state worker: run records in `runs`, and one
/// one-node definition for every run. These are the state calls an observer's
/// added hook, its replay and a finalizing tick of a run with no nodes make.
async fn fake_state(url: &str) -> (IIIClient, Runs) {
    let runs: Runs = Arc::default();
    let state = register_worker(url, named("workflow-test-state"));
    let store = runs.clone();
    state.register_function(
        "state::get",
        RegisterFunction::new(move |req: Value| {
            let found = match req["scope"].as_str() {
                Some(workflow::state::SCOPE_RUN) => req["key"]
                    .as_str()
                    .and_then(|k| store.lock().unwrap().get(k).cloned()),
                Some(workflow::state::SCOPE_DEF) => Some(json!({
                    "nodes": { "a": { "agent": { "model": "m" }, "input": { "from": "run_input" } } },
                    "output": { "from": "node:a" }
                })),
                _ => None,
            };
            Ok::<_, Error>(found.unwrap_or(Value::Null))
        }),
    );
    let store = runs.clone();
    state.register_function(
        "state::set",
        RegisterFunction::new(move |req: Value| {
            if let (Some(workflow::state::SCOPE_RUN), Some(key)) =
                (req["scope"].as_str(), req["key"].as_str())
            {
                store
                    .lock()
                    .unwrap()
                    .insert(key.to_string(), req["value"].clone());
            }
            Ok::<_, Error>(json!({}))
        }),
    );
    wait_registered(&state, "state::get").await;
    wait_registered(&state, "state::set").await;
    (state, runs)
}

fn put(runs: &Runs, record: &WorkflowRunRecord) {
    runs.lock()
        .unwrap()
        .insert(record.run_id.clone(), serde_json::to_value(record).unwrap());
}

/// The workflow worker, registered the way main.rs does it.
fn workflow_host(url: &str) -> (Arc<IIIClient>, Deps) {
    let host = Arc::new(register_worker(url, named("workflow")));
    let deps = Deps {
        iii: host.clone(),
        cfg: Arc::new(RwLock::new(Arc::new(
            workflow::config::WorkerConfig::default(),
        ))),
        locks: workflow::locks::WorkflowLocks::default(),
        events: host.clone(),
    };
    functions::register_all(&host, &deps);
    (host, deps)
}

async fn wait_registered(client: &IIIClient, function_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while call(
        client,
        "engine::functions::info",
        json!({ "function_id": function_id }),
    )
    .await
    .is_err()
    {
        assert!(Instant::now() < deadline, "{function_id} never registered");
        sleep(Duration::from_millis(100)).await;
    }
}

/// A completed run with one tolerated failure.
fn ended(run_id: &str) -> WorkflowRunRecord {
    let mut record = record(run_id, RunStatus::Completed);
    record.nodes.insert(
        "side".into(),
        NodeCheckpoint {
            state: NodeState::Failed,
            result_error: Some("boom".into()),
            ..NodeCheckpoint::default()
        },
    );
    record
}

/// What one target received, in order.
#[derive(Default)]
struct Received(Mutex<Vec<Value>>);

impl Received {
    fn all(&self) -> Vec<Value> {
        self.0.lock().unwrap().clone()
    }

    /// The events received for `run_id`.
    fn of(&self, run_id: &str) -> Vec<RunCompletedEvent> {
        self.all()
            .into_iter()
            .map(event_of)
            .filter(|event| event.run_id == run_id)
            .collect()
    }

    async fn wait_for(&self, what: &str, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + DELIVERY;
        loop {
            let all = self.all();
            if all.len() >= count {
                return all;
            }
            assert!(Instant::now() < deadline, "{what}: received {all:?}");
            sleep(Duration::from_millis(50)).await;
        }
    }
}

/// Registers `id` on `client`; every payload it is called with is recorded.
fn recorder(client: &IIIClient, id: &str) -> Arc<Received> {
    let received = Arc::new(Received::default());
    let sink = received.clone();
    client.register_function(
        id,
        RegisterFunction::new(move |payload: Value| {
            sink.0.lock().unwrap().push(payload);
            Ok::<_, Error>(json!({}))
        }),
    );
    received
}

/// The event a target got, decoded the way the harness target decodes it: the
/// output of an `Observed` envelope (a live delivery), else the payload as
/// sent through the observer (a replay) without the engine's caller stamp.
fn event_of(payload: Value) -> RunCompletedEvent {
    if let Ok(envelope) =
        serde_json::from_value::<Observed<Value, RunCompletedEvent>>(payload.clone())
    {
        return envelope.output.expect("a live delivery carries the output");
    }
    let mut event = payload;
    if let Some(object) = event.as_object_mut() {
        object.remove("_caller_worker_id");
    }
    serde_json::from_value(event).expect("a replay is the bare event")
}

fn is_envelope(payload: &Value) -> bool {
    payload.get("observer_id").is_some()
}

fn run(run_id: &str) -> Source {
    Source::function(ON_RUN_COMPLETED_FN).args(json!({ "run_id": run_id }))
}

/// Observes `source` into `target` and waits until it settles.
async fn observe(
    client: &IIIClient,
    source: Source,
    target: &str,
    options: ObserverOptions,
) -> Result<ObserverHandle, Error> {
    let handle = client.observe_with(source.capture([Capture::Output]), target, options);
    timeout(Duration::from_secs(10), handle.ready())
        .await
        .unwrap_or_else(|_| panic!("observer of {target} did not settle"))
        .map(|()| handle)
}

#[tokio::test]
async fn a_run_observer_gets_its_ended_run_replayed_and_others_get_nothing() {
    let Some(engine) = engine_or_skip().await else {
        return;
    };
    let (state, runs) = fake_state(&engine.url).await;
    let (host, _deps) = workflow_host(&engine.url);
    let observer = register_worker(&engine.url, named("workflow-test-observer"));
    wait_registered(&observer, ON_RUN_COMPLETED_FN).await;
    let done = ended("r-done");
    put(&runs, &done);
    put(&runs, &record("r-running", RunStatus::Running));

    // An observer of a running run: no replay, and it is not observed for another run.
    let other = recorder(&observer, "test::other-run");
    observe(
        &observer,
        run("r-running"),
        "test::other-run",
        ObserverOptions::default(),
    )
    .await
    .expect("an observer of a running run is accepted");
    let call_for = |run_id: &str| Call {
        input: json!({ "run_id": run_id }),
        ..Call::default()
    };
    wait_until("the host to see the run observer", || {
        host.is_observed_for(ON_RUN_COMPLETED_FN, &call_for("r-running"))
    })
    .await;
    assert!(!host.is_observed_for(ON_RUN_COMPLETED_FN, &call_for("r-done")));

    // A late observer with only `filters` on the run gets no replay.
    let filtered = recorder(&observer, "test::filtered");
    observe(
        &observer,
        Source::function(ON_RUN_COMPLETED_FN)
            .filters(json!({ "input.run_id": { "$eq": "r-done" } })),
        "test::filtered",
        ObserverOptions::default(),
    )
    .await
    .expect("an arg-less observer is accepted");
    // It takes every run's end, but observes no run for a waiting caller.
    wait_until("the host to see the every-run observer", || {
        host.is_observed_for(ON_RUN_COMPLETED_FN, &call_for("r-done"))
    })
    .await;
    assert!(!host.run_observed("r-done"));
    assert!(host.run_observed("r-running"));

    // An observer of the ended run gets it once, as the bare event.
    let late = recorder(&observer, "test::late");
    observe(
        &observer,
        run("r-done"),
        "test::late",
        ObserverOptions::default(),
    )
    .await
    .expect("an observer of an ended run is accepted");
    let replay = late.wait_for("the replay", 1).await;
    sleep(QUIET).await;
    assert_eq!(late.all().len(), 1, "one replay: {:?}", late.all());
    assert!(
        !is_envelope(&replay[0]),
        "a replay is the bare event: {replay:?}"
    );
    let expected = RunCompletedEvent::from(&done);
    assert_eq!(expected.failed_nodes, ["node 'side': boom"]);
    assert_eq!(event_of(replay[0].clone()), expected);
    assert!(filtered.all().is_empty(), "filters get no replay");
    assert!(
        other.all().is_empty(),
        "another run's observer gets nothing"
    );

    // A live delivery of the same end decodes to the same event.
    emit_run_completed(host.as_ref(), &done).await;
    let live = late.wait_for("the live delivery", 2).await;
    assert!(
        is_envelope(&live[1]),
        "a live delivery is the envelope: {live:?}"
    );
    assert_eq!(event_of(live[1].clone()), expected);
    filtered
        .wait_for("the live delivery to the filter", 1)
        .await;
    sleep(QUIET).await;
    assert!(
        other.all().is_empty(),
        "another run's observer gets nothing"
    );

    // Bad args and an unknown run are refused.
    let refusal = |source: Source| {
        let observer = &observer;
        async move {
            match observe(observer, source, "test::late", ObserverOptions::default()).await {
                Ok(_) => panic!("the observer was accepted"),
                Err(error) => error.to_string(),
            }
        }
    };
    let typo =
        refusal(Source::function(ON_RUN_COMPLETED_FN).args(json!({ "runid": "r-done" }))).await;
    assert!(typo.contains("INVALID_ARGS"), "{typo}");
    let unknown = refusal(run("r-missing")).await;
    assert!(unknown.contains("UNKNOWN_RUN"), "{unknown}");

    observer.shutdown_async().await;
    host.shutdown_async().await;
    state.shutdown_async().await;
}

#[tokio::test]
async fn a_run_that_ends_while_its_observer_is_added_reaches_it_once() {
    let Some(engine) = engine_or_skip().await else {
        return;
    };
    let (state, runs) = fake_state(&engine.url).await;
    let (host, deps) = workflow_host(&engine.url);
    let observer = Arc::new(register_worker(
        &engine.url,
        named("workflow-test-observer"),
    ));
    wait_registered(&observer, ON_RUN_COMPLETED_FN).await;
    let received = recorder(&observer, "test::race");

    let ids: Vec<String> = (0..20).map(|i| format!("r-race-{i}")).collect();
    let mut tasks = Vec::new();
    for (i, run_id) in ids.iter().enumerate() {
        // Aborted with no nodes: its next tick finalizes it `cancelled` without
        // a harness call.
        let mut running = record(run_id, RunStatus::Running);
        running.abort = true;
        put(&runs, &running);
        let (deps, observer) = (deps.clone(), observer.clone());
        let tick = TickRequest {
            run_id: run_id.clone(),
            step: running.step,
            wake: false,
        };
        let run_id = run_id.clone();
        tasks.push(tokio::spawn(async move {
            // The real tick: it holds the run lock from its load through the
            // emit to the persist.
            let finalize = async {
                // Staggered, so runs end before, while and after their observer lands.
                sleep(Duration::from_millis(3 * i as u64)).await;
                let ticked = functions::tick::handle(&deps, tick)
                    .await
                    .expect("the finalizing tick");
                assert!(!ticked.skipped);
            };
            // One-shot, like the harness wake: a plain observer can get the live
            // event and the replay when it lands during the final tick.
            let wake = observe(
                &observer,
                run(&run_id),
                "test::race",
                ObserverOptions::default().max_fires(1),
            );
            let ((), handle) = tokio::join!(finalize, wake);
            handle.expect("the run observer is accepted");
        }));
    }
    for task in tasks {
        task.await.expect("race task");
    }
    received.wait_for("every run's end", ids.len()).await;
    sleep(QUIET).await;
    for run_id in &ids {
        let events = received.of(run_id);
        assert_eq!(events.len(), 1, "{run_id}: {events:?}");
        assert_eq!(events[0].status, RunStatus::Cancelled);
        assert_eq!(runs.lock().unwrap()[run_id]["status"], "cancelled");
    }

    observer.shutdown_async().await;
    host.shutdown_async().await;
    state.shutdown_async().await;
}

/// A TCP proxy to the engine on an ephemeral port. `cut` closes every
/// proxied connection and refuses new ones until `restore`.
struct Proxy {
    port: u16,
    open: Arc<AtomicBool>,
    links: Arc<Mutex<Vec<JoinHandle<()>>>>,
    accept: JoinHandle<()>,
}

impl Proxy {
    async fn start(upstream: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let open = Arc::new(AtomicBool::new(true));
        let links: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::default();
        let accept = tokio::spawn({
            let (open, links) = (open.clone(), links.clone());
            async move {
                while let Ok((mut inbound, _)) = listener.accept().await {
                    if !open.load(Ordering::SeqCst) {
                        continue;
                    }
                    let upstream = upstream.clone();
                    let link = tokio::spawn(async move {
                        if let Ok(mut outbound) = TcpStream::connect(&upstream).await {
                            let _ =
                                tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                        }
                    });
                    links.lock().unwrap().push(link);
                }
            }
        });
        Self {
            port,
            open,
            links,
            accept,
        }
    }

    fn url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }

    fn cut(&self) {
        self.open.store(false, Ordering::SeqCst);
        for link in self.links.lock().unwrap().drain(..) {
            link.abort();
        }
    }

    fn restore(&self) {
        self.open.store(true, Ordering::SeqCst);
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.cut();
        self.accept.abort();
    }
}

/// iii removes the observing worker's observers from the host when its
/// connection drops and adds them again when it reattaches; each add replays.
#[tokio::test]
async fn a_run_observer_added_again_after_a_reconnect_is_replayed() {
    let Some(engine) = engine_or_skip().await else {
        return;
    };
    let (state, runs) = fake_state(&engine.url).await;
    let (host, _deps) = workflow_host(&engine.url);
    let done = ended("r-reconnect");
    put(&runs, &done);
    let proxy = Proxy::start(engine.url.trim_start_matches("ws://").to_string()).await;
    let consumer = register_worker(&proxy.url(), named("workflow-test-consumer"));
    wait_registered(&consumer, ON_RUN_COMPLETED_FN).await;
    let received = recorder(&consumer, "test::reconnect");
    observe(
        &consumer,
        run("r-reconnect"),
        "test::reconnect",
        ObserverOptions::default(),
    )
    .await
    .expect("observer accepted");
    received.wait_for("the first replay", 1).await;

    proxy.cut();
    wait_until("the host to drop the observer", || {
        !host.is_observed(ON_RUN_COMPLETED_FN)
    })
    .await;
    proxy.restore();
    received.wait_for("the replay after the reconnect", 2).await;
    sleep(QUIET).await;

    let events: Vec<RunCompletedEvent> = received.all().into_iter().map(event_of).collect();
    assert_eq!(events.len(), 2, "{events:?}");
    let expected = RunCompletedEvent::from(&done);
    assert!(events.iter().all(|event| *event == expected), "{events:?}");

    consumer.shutdown_async().await;
    host.shutdown_async().await;
    state.shutdown_async().await;
}
