//! `shell::on-job-finished` across a reconnect of the observing worker,
//! against a live engine.
//!
//! The consumer reaches the engine through an in-test TCP proxy, so the test
//! can drop its connection and let it come back. iii removes the consumer's
//! observers from the host when its connection drops and adds them again
//! when it reattaches; the host replays a retained completion to each
//! observer it adds again after the observer's job finished. The engine is
//! `III_ENGINE_WS_URL` (default `ws://127.0.0.1:49134`); without one the test
//! skips, and with `III_E2E_REQUIRE` set it fails instead.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ide::config::ShellConfig;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, Error, IIIClient, InitOptions, Observed, ObserverHandle,
    ObserverOptions, RegisterFunction, Source,
};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";
const ON_JOB_FINISHED: &str = "shell::on-job-finished";

/// How long a target is given to receive what it is owed, and the
/// consumer to drop or reattach.
const DELIVERY: Duration = Duration::from_secs(20);
/// How long a target that is owed nothing more is watched for.
const QUIET: Duration = Duration::from_millis(1500);

fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

/// The engine's `host:port`, from its WebSocket URL.
fn engine_addr() -> String {
    let url = ws_url();
    let rest = url
        .strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"))
        .unwrap_or(&url);
    rest.split('/').next().unwrap_or(rest).to_string()
}

/// A connection to `url` under its own worker name, once the engine
/// answers; `None` (skip) without an engine unless `III_E2E_REQUIRE` is set.
async fn connect(url: &str, name: &str) -> Option<Arc<IIIClient>> {
    let iii = Arc::new(register_worker(
        url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: format!("{name}-{}", uuid::Uuid::new_v4()),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let probe = iii
            .call(CallRequest {
                function_id: "engine::workers::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        if probe.is_ok() {
            return Some(iii);
        }
    }
    iii.shutdown_async().await;
    assert!(
        std::env::var("III_E2E_REQUIRE").is_err(),
        "no iii engine reachable at {}: start one or set III_ENGINE_WS_URL",
        ws_url()
    );
    eprintln!(
        "[skip] no iii engine reachable at {}; set III_E2E_REQUIRE=1 to fail instead",
        ws_url()
    );
    None
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

/// What one target received, in order.
#[derive(Default)]
struct Received(Mutex<Vec<Value>>);

impl Received {
    fn all(&self) -> Vec<Value> {
        self.0.lock().unwrap().clone()
    }

    /// Waits until `done` holds for what arrived so far.
    async fn wait_until(&self, what: &str, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + DELIVERY;
        loop {
            let all = self.all();
            if done(&all) {
                return all;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{what}: still waiting, received {all:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// Registers `id` on `iii`; every payload it is called with is recorded.
fn recorder(iii: &IIIClient, id: &str) -> Arc<Received> {
    let received = Arc::new(Received::default());
    let sink = received.clone();
    iii.register_function(
        id,
        RegisterFunction::new(move |payload: Value| {
            sink.0.lock().unwrap().push(payload);
            Ok::<_, Error>(json!({ "ok": true }))
        }),
    );
    received
}

/// Observes `source` into `target` with `options` and waits until the
/// observer is active.
async fn observe(
    iii: &IIIClient,
    source: Source,
    target: &str,
    options: ObserverOptions,
) -> ObserverHandle {
    let handle = iii.observe_with(source, target, options);
    tokio::time::timeout(Duration::from_secs(10), handle.ready())
        .await
        .unwrap_or_else(|_| panic!("observer of {target} did not settle"))
        .unwrap_or_else(|e| panic!("observer of {target} failed: {e}"));
    handle
}

/// Waits until the host holds an observer of the function (`true`) or none.
async fn wait_observed(host: &IIIClient, observed: bool) {
    let deadline = tokio::time::Instant::now() + DELIVERY;
    while host.is_observed(ON_JOB_FINISHED) != observed {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the host still {} an observer",
            if observed { "lacks" } else { "holds" }
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Starts `command` as a host background job named `job_id` and waits until
/// its record is final.
async fn run_job(iii: &IIIClient, job_id: &str, command: &str) {
    let cfg = Arc::new(ShellConfig::default());
    let request = serde_json::from_value(json!({ "command": command, "job_id": job_id })).unwrap();
    ide::functions::exec_bg::handle(cfg, iii.clone(), request)
        .await
        .unwrap();
    let handle = ide::jobs::get(job_id).await.expect("the job is retained");
    tokio::time::timeout(Duration::from_secs(10), async {
        while !handle.lock().await.finalized {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("job {job_id} did not finish"));
}

fn job(job_id: &str) -> Source {
    Source::function(ON_JOB_FINISHED).args(json!({ "job_id": job_id }))
}

/// The event an `Observed` envelope carries.
fn envelope_output(payload: &Value) -> Value {
    let envelope: Observed<Value, Value> = serde_json::from_value(payload.clone())
        .unwrap_or_else(|e| panic!("not an Observed envelope ({e}): {payload}"));
    envelope.output.expect("the envelope carries the output")
}

/// Each observer of a job receives its completion once each time iii adds
/// it. When the consumer's connection drops and comes back, an observer
/// that took its job's completion live receives it again from the replay,
/// and an observer whose job ended while the consumer was away receives it
/// from the replay for the first time. An observer with `max_fires: 1` ended
/// with its first delivery and receives nothing more.
#[tokio::test]
async fn on_job_finished_replays_to_an_observer_added_again_after_a_reconnect() {
    let Some(owner) = connect(&ws_url(), "ide-e2e-reconnect-owner").await else {
        return;
    };
    ide::job_events::register_job_finished(&owner);
    let proxy = Proxy::start(engine_addr()).await;
    let Some(consumer) = connect(&proxy.url(), "ide-e2e-reconnect-consumer").await else {
        owner.shutdown_async().await;
        return;
    };
    let run = uuid::Uuid::new_v4();
    let (before, during) = (
        format!("e2e-before-drop-{run}"),
        format!("e2e-during-drop-{run}"),
    );

    let took = recorder(&consumer, "e2e::reconnect::took");
    let once = recorder(&consumer, "e2e::reconnect::once");
    let away = recorder(&consumer, "e2e::reconnect::away");
    let took_handle = observe(
        &consumer,
        job(&before),
        "e2e::reconnect::took",
        ObserverOptions::default(),
    )
    .await;
    observe(
        &consumer,
        job(&before).deliver(Deliver::Output),
        "e2e::reconnect::once",
        ObserverOptions::default().max_fires(1),
    )
    .await;
    observe(
        &consumer,
        job(&during).deliver(Deliver::Output),
        "e2e::reconnect::away",
        ObserverOptions::default(),
    )
    .await;

    // `before` ends while the consumer is connected: the live call.
    run_job(&owner, &before, "true").await;
    took.wait_until("the live completion", |all| !all.is_empty())
        .await;
    once.wait_until("the live completion, once", |all| !all.is_empty())
        .await;

    // The connection drops: iii removes the consumer's observers from the
    // host, and `during` ends while they are away.
    proxy.cut();
    wait_observed(&owner, false).await;
    run_job(&owner, &during, "true").await;
    tokio::time::sleep(QUIET).await;
    assert!(away.all().is_empty(), "away: {:?}", away.all());

    // The consumer reattaches: iii adds its observers again, and the host
    // replays each retained completion.
    proxy.restore();
    took.wait_until("the replay after the reconnect", |all| all.len() >= 2)
        .await;
    away.wait_until("the replay of the job that ended while away", |all| {
        !all.is_empty()
    })
    .await;
    tokio::time::sleep(QUIET).await;

    let deliveries = took.all();
    assert_eq!(deliveries.len(), 2, "took: {deliveries:?}");
    for delivery in &deliveries {
        assert_eq!(delivery["observer_id"], took_handle.id());
        assert_eq!(envelope_output(delivery)["job_id"], before.as_str());
    }
    assert_eq!(
        envelope_output(&deliveries[0]),
        envelope_output(&deliveries[1]),
        "the replay carries the completion the live call did"
    );
    let first = away.all();
    assert_eq!(first.len(), 1, "away: {first:?}");
    assert_eq!(first[0]["job_id"], during.as_str());
    assert_eq!(first[0]["status"], "finished");
    let once = once.all();
    assert_eq!(once.len(), 1, "max_fires 1: {once:?}");
    assert_eq!(once[0]["job_id"], before.as_str());

    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}
