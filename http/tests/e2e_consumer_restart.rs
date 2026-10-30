//! A consumer that serves a route by observing `http::on-request` under a
//! stable observer id, against a running engine.
//!
//! When the consumer's connection goes away, the engine suspends its
//! observer and withdraws it from the http worker (`observerremoved`), which
//! frees the route. When the consumer comes back under its worker name within
//! the grace period and observes again under the same id, the engine replaces
//! the suspended observer and announces the new one (`observeradded`): the
//! http worker files the route again, and requests reach the new process.
//! The worker name comes from the SDK's worker name option or from
//! `III_WORKER_NAME`. Under the SDK's default name, `hostname:pid`, the
//! restarted process is another worker, and the engine refuses its observe
//! with `OBSERVER_ID_CONFLICT`.
//!
//! The consumer is a child process (this test binary running the ignored
//! [`consumer_process`] test), so the test kills it with SIGKILL.

mod common;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use common::{engine, worker};
use iii_http::boot::BootHandle;
use iii_http::on_request::ON_REQUEST_FN;
use iii_http::types::HttpRequest;
use iii_sdk::errors::Error;
use iii_sdk::protocol::observe_codes::OBSERVER_ID_CONFLICT;
use iii_sdk::{IIIClient, ObserverOptions, RegisterFunction, Source};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::sync::mpsc;

/// `<worker name>|<route path>|<observer id>|<generation>`: set on the
/// consumer process. An empty worker name connects under the SDK's default
/// name.
const CONSUMER_ENV: &str = "HTTP_E2E_CONSUMER";
const TARGET: &str = "test::restart::orders";
/// The variable a supervisor sets to the worker's name.
const WORKER_NAME_ENV: &str = "III_WORKER_NAME";
/// What the consumer prints once its observer is active, or rejected.
const READY: &str = "consumer-ready";
const REJECTED: &str = "consumer-rejected";

/// The route's observer args.
fn route(path: &str) -> Source {
    Source::function(ON_REQUEST_FN).args(json!({ "method": "GET", "path": path }))
}

/// The consumer: connects under its worker name, registers [`TARGET`]
/// (answering with its generation), observes the route under the observer
/// id, reports the outcome on stdout and serves until it is killed. Without
/// [`CONSUMER_ENV`] it does nothing.
#[tokio::test]
#[ignore = "the consumer process of the restart tests in this file"]
async fn consumer_process() {
    let Ok(spec) = std::env::var(CONSUMER_ENV) else {
        return;
    };
    let mut parts = spec.splitn(4, '|');
    let (Some(name), Some(path), Some(observer_id), Some(generation)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        panic!(
            "{CONSUMER_ENV} is <worker name>|<route path>|<observer id>|<generation>, got {spec}"
        );
    };
    let generation: u64 = generation.parse().expect("generation is a number");
    let iii = if name.is_empty() {
        engine::connect_default().await
    } else {
        engine::connect_named(name).await
    }
    .expect("the consumer connects to the engine");
    iii.register_function(
        TARGET,
        RegisterFunction::new_async(move |req: HttpRequest| async move {
            Ok::<Value, Error>(json!({
                "status_code": 200,
                "body": { "generation": generation, "path": req.path },
            }))
        }),
    );
    let handle = iii.observe_with(
        route(path),
        TARGET,
        ObserverOptions::default().id(observer_id),
    );
    match tokio::time::timeout(Duration::from_secs(10), handle.ready()).await {
        Ok(Ok(())) => println!("{READY}"),
        Ok(Err(error)) => println!("{REJECTED} {error}"),
        Err(_) => println!("{REJECTED} not active within 10 s"),
    }
    std::future::pending::<()>().await;
}

/// Where a consumer process gets its worker name.
enum WorkerName<'a> {
    /// The SDK's worker name option (`WorkerMetadata::name`).
    Option(&'a str),
    /// `III_WORKER_NAME`, with the SDK's default metadata.
    Env(&'a str),
    /// Neither: the SDK's default name, `hostname:pid`.
    Default,
}

/// A running consumer process. Dropping it kills the process.
struct Consumer {
    child: Child,
    lines: mpsc::UnboundedReceiver<String>,
}

impl Consumer {
    fn spawn(name: &WorkerName<'_>, path: &str, observer_id: &str, generation: u64) -> Self {
        let mut command = Command::new(std::env::current_exe().expect("the test binary"));
        command
            .args([
                "consumer_process",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("III_ENGINE_WS_URL", engine::ws_url())
            .env_remove(WORKER_NAME_ENV)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let option_name = match name {
            WorkerName::Option(name) => name,
            WorkerName::Env(name) => {
                command.env(WORKER_NAME_ENV, name);
                ""
            }
            WorkerName::Default => "",
        };
        command.env(
            CONSUMER_ENV,
            format!("{option_name}|{path}|{observer_id}|{generation}"),
        );
        let mut child = command.spawn().expect("spawn the consumer process");
        let stdout = child.stdout.take().expect("the consumer's stdout");
        let (tx, lines) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self { child, lines }
    }

    /// Waits for the consumer's observer: `Ok` once it is active, `Err` with
    /// the consumer's report when it is rejected or the process ends.
    async fn ready(&mut self) -> Result<(), String> {
        let wait = async {
            // libtest prints `test consumer_process ... ` before the
            // consumer's own line.
            while let Some(line) = self.lines.recv().await {
                if line.trim_end().ends_with(READY) {
                    return Ok(());
                }
                if let Some((_, reason)) = line.split_once(REJECTED) {
                    return Err(reason.trim().to_string());
                }
            }
            Err("the consumer process ended before its observer was active".to_string())
        };
        tokio::time::timeout(Duration::from_secs(30), wait)
            .await
            .unwrap_or_else(|_| Err("no answer from the consumer within 30 s".to_string()))
    }

    /// SIGKILL: the connection goes away without a word from the worker.
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

/// The status of `GET url` and the generation of the consumer that served it.
async fn get(url: &str) -> (u16, Option<u64>) {
    let resp = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .get(url)
        .send()
        .await
        .expect("the http worker answers");
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    (status, body["generation"].as_u64())
}

/// One test's http worker, route and stable observer id.
struct Run {
    owner: Arc<IIIClient>,
    boot: BootHandle,
    /// Unique per run, so no test meets another one's suspended observer.
    run: uuid::Uuid,
    path: String,
    observer_id: String,
    url: String,
}

impl Run {
    /// Starts the http worker. `None` when no engine is reachable (the test
    /// is skipped).
    async fn start() -> Option<Self> {
        let owner = engine::connect_fresh().await?;
        let boot = worker::start_http_worker(owner.clone()).await;
        let run = uuid::Uuid::new_v4();
        let path = format!("/restart/{run}/orders");
        let observer_id = format!("http-e2e-restart-orders-{run}");
        let url = format!("http://{}{path}", boot.local_addr);
        Some(Self {
            owner,
            boot,
            run,
            path,
            observer_id,
            url,
        })
    }

    fn spawn(&self, name: &WorkerName<'_>, generation: u64) -> Consumer {
        Consumer::spawn(name, &self.path, &self.observer_id, generation)
    }

    /// A first consumer process under `name` serves the route; then it is
    /// killed, and the engine withdraws its observer from the http worker,
    /// which frees the route.
    async fn serve_then_kill(&self, name: &WorkerName<'_>) {
        let mut first = self.spawn(name, 1);
        first
            .ready()
            .await
            .expect("the http worker accepts the consumer's route");
        common::wait_for_route(&self.boot.routes, "GET", &self.path).await;
        assert_eq!(
            get(&self.url).await,
            (200, Some(1)),
            "the first process serves"
        );

        first.kill();
        common::wait_for_no_route(&self.boot.routes, "GET", &self.path).await;
        assert_eq!(
            get(&self.url).await.0,
            404,
            "no route while the consumer is away"
        );
    }

    /// The restarted consumer (generation 2) serves the route, filed under
    /// the stable observer id.
    async fn assert_served_again(&self) {
        common::wait_for_route(&self.boot.routes, "GET", &self.path).await;
        assert_eq!(
            self.boot
                .routes
                .read()
                .await
                .get(&self.observer_id)
                .map(|route| route.http_path.clone()),
            Some(self.path.clone()),
            "the route is filed under the consumer's stable id"
        );
        assert_eq!(
            get(&self.url).await,
            (200, Some(2)),
            "the new process serves"
        );
    }

    async fn finish(self) {
        self.boot.shutdown().await;
        self.owner.shutdown_async().await;
    }
}

#[tokio::test]
#[serial]
async fn a_consumer_restarted_under_its_worker_name_gets_its_route_back() {
    let Some(run) = Run::start().await else {
        return;
    };
    let name = format!("http-e2e-restart-consumer-{}", run.run);
    let name = WorkerName::Option(&name);
    run.serve_then_kill(&name).await;

    // While the consumer is away its id stays taken for every other worker.
    let Some(other) = engine::connect_fresh().await else {
        return;
    };
    other.register_function(
        "test::restart::other",
        RegisterFunction::new_async(|_req: HttpRequest| async move {
            Ok::<Value, Error>(json!({ "status_code": 200 }))
        }),
    );
    let taken = other.observe_with(
        route(&run.path),
        "test::restart::other",
        ObserverOptions::default().id(run.observer_id.clone()),
    );
    let refused = tokio::time::timeout(Duration::from_secs(10), taken.ready())
        .await
        .expect("the engine answers within 10 s")
        .expect_err("another worker cannot take a suspended observer's id");
    match &refused {
        Error::Remote { code, .. } => assert_eq!(code, OBSERVER_ID_CONFLICT, "{refused}"),
        other => panic!("expected OBSERVER_ID_CONFLICT, got {other:?}"),
    }
    other.shutdown_async().await;

    // Back under its worker name within the grace period: observing again
    // under its id replaces the suspended observer, with no
    // HTTP_ROUTE_CONFLICT, and requests reach the new process.
    let mut second = run.spawn(&name, 2);
    second
        .ready()
        .await
        .expect("the restarted consumer observes again under its id");
    run.assert_served_again().await;

    second.kill();
    common::wait_for_no_route(&run.boot.routes, "GET", &run.path).await;
    run.finish().await;
}

/// `III_WORKER_NAME` (what `iii compose` and `iii worker` set) keeps the
/// worker name across restarts with the SDK's default metadata.
#[tokio::test]
#[serial]
async fn a_consumer_restarted_under_iii_worker_name_gets_its_route_back() {
    let Some(run) = Run::start().await else {
        return;
    };
    let name = format!("http-e2e-restart-managed-{}", run.run);
    let name = WorkerName::Env(&name);
    run.serve_then_kill(&name).await;

    let mut second = run.spawn(&name, 2);
    second
        .ready()
        .await
        .expect("the restarted consumer observes again under its id");
    run.assert_served_again().await;

    second.kill();
    common::wait_for_no_route(&run.boot.routes, "GET", &run.path).await;
    run.finish().await;
}

/// The SDK's default worker name, `hostname:pid`, changes with the process:
/// the restarted process is another worker, and its observe under the stable
/// id is refused until the grace period ends.
#[tokio::test]
#[serial]
async fn a_consumer_restarted_under_the_default_worker_name_gets_observer_id_conflict() {
    let Some(run) = Run::start().await else {
        return;
    };
    run.serve_then_kill(&WorkerName::Default).await;

    let mut second = run.spawn(&WorkerName::Default, 2);
    let refused = second
        .ready()
        .await
        .expect_err("a process under another worker name cannot take the suspended id");
    assert!(
        refused.contains(OBSERVER_ID_CONFLICT),
        "expected OBSERVER_ID_CONFLICT, got {refused}"
    );
    assert!(
        run.boot.routes.read().await.get(&run.observer_id).is_none(),
        "the http worker has no route for the refused observer"
    );
    assert_eq!(
        get(&run.url).await.0,
        404,
        "the restarted process does not serve the route"
    );

    second.kill();
    run.finish().await;
}
