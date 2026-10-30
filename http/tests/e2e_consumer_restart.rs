//! A consumer that serves a route by observing `http::on-request` under a
//! stable observer id, against a running engine.
//!
//! When the consumer's connection goes away, the engine suspends its
//! observer and withdraws it from the http worker (`observerremoved`), which
//! frees the route. When the consumer comes back under its worker name within
//! the grace period and observes again under the same id, the engine replaces
//! the suspended observer and announces the new one (`observeradded`): the
//! http worker files the route again, and requests reach the new process.
//!
//! The consumer is a child process (this test binary running the ignored
//! [`consumer_process`] test), so the test kills it with SIGKILL.

mod common;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::{engine, worker};
use iii_http::on_request::ON_REQUEST_FN;
use iii_http::types::HttpRequest;
use iii_sdk::errors::Error;
use iii_sdk::protocol::observe_codes::OBSERVER_ID_CONFLICT;
use iii_sdk::{ObserverOptions, RegisterFunction, Source};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::sync::mpsc;

/// `<worker name>|<observer id>|<generation>`: set on the consumer process.
const CONSUMER_ENV: &str = "HTTP_E2E_CONSUMER";
const ROUTE_PATH: &str = "/restart/orders";
const TARGET: &str = "test::restart::orders";
/// What the consumer prints once its observer is active, or rejected.
const READY: &str = "consumer-ready";
const REJECTED: &str = "consumer-rejected";

/// The route's observer args.
fn route() -> Source {
    Source::function(ON_REQUEST_FN).args(json!({ "method": "GET", "path": ROUTE_PATH }))
}

/// The consumer: connects under its worker name, registers [`TARGET`]
/// (answering with its generation), observes the route under the observer
/// id, reports the outcome on stdout and serves until it is killed. Without
/// [`CONSUMER_ENV`] it does nothing.
#[tokio::test]
#[ignore = "the consumer process of a_consumer_restarted_under_its_worker_name_gets_its_route_back"]
async fn consumer_process() {
    let Ok(spec) = std::env::var(CONSUMER_ENV) else {
        return;
    };
    let mut parts = spec.splitn(3, '|');
    let (Some(name), Some(observer_id), Some(generation)) =
        (parts.next(), parts.next(), parts.next())
    else {
        panic!("{CONSUMER_ENV} is <worker name>|<observer id>|<generation>, got {spec}");
    };
    let generation: u64 = generation.parse().expect("generation is a number");
    let iii = engine::connect_named(name)
        .await
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
    let handle = iii.observe_with(route(), TARGET, ObserverOptions::default().id(observer_id));
    match tokio::time::timeout(Duration::from_secs(10), handle.ready()).await {
        Ok(Ok(())) => println!("{READY}"),
        Ok(Err(error)) => println!("{REJECTED} {error}"),
        Err(_) => println!("{REJECTED} not active within 10 s"),
    }
    std::future::pending::<()>().await;
}

/// A running consumer process. Dropping it kills the process.
struct Consumer {
    child: Child,
    lines: mpsc::UnboundedReceiver<String>,
}

impl Consumer {
    fn spawn(name: &str, observer_id: &str, generation: u64) -> Self {
        let mut child = Command::new(std::env::current_exe().expect("the test binary"))
            .args([
                "consumer_process",
                "--exact",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CONSUMER_ENV, format!("{name}|{observer_id}|{generation}"))
            .env("III_ENGINE_WS_URL", engine::ws_url())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn the consumer process");
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

#[tokio::test]
#[serial]
async fn a_consumer_restarted_under_its_worker_name_gets_its_route_back() {
    let Some(owner) = engine::connect_fresh().await else {
        return;
    };
    let boot = worker::start_http_worker(owner.clone()).await;
    let run = uuid::Uuid::new_v4();
    let name = format!("http-e2e-restart-consumer-{run}");
    let observer_id = format!("http-e2e-restart-orders-{run}");
    let url = format!("http://{}{ROUTE_PATH}", boot.local_addr);

    let mut first = Consumer::spawn(&name, &observer_id, 1);
    first
        .ready()
        .await
        .expect("the http worker accepts the consumer's route");
    common::wait_for_route(&boot.routes, "GET", ROUTE_PATH).await;
    assert_eq!(get(&url).await, (200, Some(1)), "the first process serves");

    // Killed: the engine suspends its observer and withdraws it from the
    // http worker, which frees the route.
    first.kill();
    common::wait_for_no_route(&boot.routes, "GET", ROUTE_PATH).await;
    assert_eq!(
        get(&url).await.0,
        404,
        "no route while the consumer is away"
    );

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
        route(),
        "test::restart::other",
        ObserverOptions::default().id(observer_id.clone()),
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
    let mut second = Consumer::spawn(&name, &observer_id, 2);
    second
        .ready()
        .await
        .expect("the restarted consumer observes again under its id");
    common::wait_for_route(&boot.routes, "GET", ROUTE_PATH).await;
    assert_eq!(
        boot.routes
            .read()
            .await
            .get(&observer_id)
            .map(|route| route.http_path.clone()),
        Some(ROUTE_PATH.to_string()),
        "the route is filed under the consumer's stable id"
    );
    assert_eq!(get(&url).await, (200, Some(2)), "the new process serves");

    second.kill();
    common::wait_for_no_route(&boot.routes, "GET", ROUTE_PATH).await;
    boot.shutdown().await;
    owner.shutdown_async().await;
}
