//! `cron::on-tick` against a live engine.
//!
//! Each test boots the worker in-process on its own connection and observes
//! the function from another.

mod common;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::{CallRequest, Deliver, Error, IIIClient, ObserverHandle, RegisterFunction, Source};
use serde_json::{json, Value};
use serial_test::serial;

use common::engine;

const EVERY_SECOND: &str = "* * * * * *";
const EVEN_SECONDS: &str = "*/2 * * * * *";

/// What a target received.
#[derive(Default)]
struct Received {
    payloads: Mutex<Vec<Value>>,
    fires: Arc<AtomicUsize>,
}

impl Received {
    fn count(&self) -> usize {
        self.fires.load(Ordering::SeqCst)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Value>> {
        self.payloads.lock().unwrap()
    }
}

/// Registers `id` on `iii` and records every payload it receives.
fn recorder(iii: &IIIClient, id: &str) -> Arc<Received> {
    let received = Arc::new(Received::default());
    let sink = received.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |payload: Value| {
            let sink = sink.clone();
            async move {
                sink.payloads.lock().unwrap().push(payload);
                sink.fires.fetch_add(1, Ordering::SeqCst);
                Ok::<_, Error>(json!({ "ok": true }))
            }
        }),
    );
    received
}

fn remote_code(error: &Error) -> Option<&str> {
    match error {
        Error::Remote { code, .. } => Some(code.as_str()),
        _ => None,
    }
}

async fn settled(handle: &ObserverHandle) -> Result<(), Error> {
    tokio::time::timeout(Duration::from_secs(10), handle.ready())
        .await
        .expect("observer neither became active nor failed within 10 s")
}

async fn wait_for_jobs<F, Fut>(jobs: &F, n: usize, what: &str)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = usize>,
{
    for _ in 0..50 {
        if jobs().await == n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{what}: still {} job(s), expected {n}", jobs().await);
}

fn on_tick(expression: &str) -> Source {
    Source::function("cron::on-tick").args(json!({ "expression": expression }))
}

/// The scheduled times seen by one target, failing on a duplicate: one tick
/// per fire per observer.
fn scheduled_times(received: &Received, tick: impl Fn(&Value) -> &Value) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    for payload in received.lock().iter() {
        let at = tick(payload)["scheduled_time"]
            .as_str()
            .unwrap_or_else(|| panic!("no scheduled_time in {payload}"))
            .to_string();
        assert!(seen.insert(at.clone()), "delivered twice: {at}");
    }
    seen
}

#[tokio::test]
#[serial]
async fn each_observer_receives_its_expressions_ticks_from_one_job_per_expression() {
    let Some(owner) = engine::connect_fresh_named("cron-e2e-share-owner").await else {
        return;
    };
    let boot = iii_cron::boot::start(owner.clone());
    let Some(consumer) = engine::connect_fresh_named("cron-e2e-share-consumer").await else {
        return;
    };
    let jobs = || async { boot.scheduler.job_count().await };

    let enveloped = recorder(&consumer, "e2e::tick-envelope");
    let output = recorder(&consumer, "e2e::tick-output");
    let even = recorder(&consumer, "e2e::tick-even");

    let every = consumer.observe(on_tick(EVERY_SECOND), "e2e::tick-envelope");
    settled(&every)
        .await
        .expect("observer on an every-second schedule");
    let every_output = consumer.observe(
        on_tick(EVERY_SECOND).deliver(Deliver::Output),
        "e2e::tick-output",
    );
    settled(&every_output)
        .await
        .expect("a second observer of the same expression");
    let even_handle = consumer.observe(
        on_tick(EVEN_SECONDS).deliver(Deliver::Output),
        "e2e::tick-even",
    );
    settled(&even_handle)
        .await
        .expect("observer on even seconds");
    assert_eq!(jobs().await, 2, "one job per distinct expression");

    tokio::time::sleep(Duration::from_millis(4500)).await;

    // The envelope carries the tick as the call's input and as its output.
    let enveloped_times = scheduled_times(&enveloped, |p| &p["input"]);
    for payload in enveloped.lock().iter() {
        assert_eq!(payload["function_id"], "cron::on-tick");
        assert_eq!(payload["observer_id"], every.id());
        assert_eq!(payload["input"]["expression"], EVERY_SECOND);
        assert_eq!(payload["output"], payload["input"], "{payload}");
    }
    // `deliver: output` hands the target the tick itself.
    let output_times = scheduled_times(&output, |p| p);
    for payload in output.lock().iter() {
        assert_eq!(payload["expression"], EVERY_SECOND);
        let mut fields: Vec<&String> = payload.as_object().unwrap().keys().collect();
        fields.sort();
        assert_eq!(fields, ["actual_time", "expression", "scheduled_time"]);
    }
    assert!(enveloped_times.len() >= 3, "{enveloped_times:?}");
    assert!(output_times.len() >= 3, "{output_times:?}");
    assert!(
        enveloped_times.intersection(&output_times).count() >= 2,
        "observers of one expression are served by the same fires"
    );
    let even_times = scheduled_times(&even, |p| p);
    assert!(!even_times.is_empty());
    for payload in even.lock().iter() {
        assert_eq!(payload["expression"], EVEN_SECONDS);
    }
    for at in &even_times {
        let at = chrono::DateTime::parse_from_rfc3339(at).unwrap();
        assert_eq!(chrono::Timelike::second(&at) % 2, 0, "{at}");
    }

    // One observer leaves: the other keeps the job, the one that left hears
    // no more.
    every.dispose().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (seen, kept) = (enveloped.count(), output.count());
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(enveloped.count(), seen, "a disposed observer hears nothing");
    assert!(output.count() > kept, "the other observer keeps ticking");
    assert_eq!(
        jobs().await,
        2,
        "the remaining observer keeps the every-second job"
    );

    // The engine tells the host after `dispose()` resolves.
    every_output.dispose().await.unwrap();
    wait_for_jobs(&jobs, 1, "the last observer of an expression stops its job").await;
    even_handle.dispose().await.unwrap();
    wait_for_jobs(&jobs, 0, "the last observer stops the last job").await;

    boot.shutdown().await;
    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn filters_select_ticks_within_an_expression() {
    let Some(owner) = engine::connect_fresh_named("cron-e2e-filters-owner").await else {
        return;
    };
    let boot = iii_cron::boot::start(owner.clone());
    let Some(consumer) = engine::connect_fresh_named("cron-e2e-filters-consumer").await else {
        return;
    };
    let all = recorder(&consumer, "e2e::tick-all");
    let none = recorder(&consumer, "e2e::tick-none");

    let every = consumer.observe(on_tick(EVERY_SECOND), "e2e::tick-all");
    settled(&every).await.expect("unfiltered observer");
    let filtered = consumer.observe(
        on_tick(EVERY_SECOND).filters(json!({ "input.expression": { "$eq": EVEN_SECONDS } })),
        "e2e::tick-none",
    );
    settled(&filtered).await.expect("filtered observer");

    common::wait_for_fires(&all.fires, 2).await;
    assert_eq!(
        none.count(),
        0,
        "filters that reject every tick deliver none"
    );

    every.dispose().await.unwrap();
    filtered.dispose().await.unwrap();
    boot.shutdown().await;
    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn observer_args_are_validated() {
    let Some(owner) = engine::connect_fresh_named("cron-e2e-args-owner").await else {
        return;
    };
    let boot = iii_cron::boot::start(owner.clone());
    let Some(consumer) = engine::connect_fresh_named("cron-e2e-args-consumer").await else {
        return;
    };
    let _target = recorder(&consumer, "e2e::tick-never");

    for args in [
        json!({}),
        json!({ "expression": 5 }),
        json!({ "expression": EVERY_SECOND, "condition_function_id": "e2e::cond" }),
    ] {
        let handle = consumer.observe(
            Source::function("cron::on-tick").args(args.clone()),
            "e2e::tick-never",
        );
        let err = settled(&handle).await.unwrap_err();
        assert_eq!(remote_code(&err), Some("INVALID_ARGS"), "{args}: {err}");
    }
    let handle = consumer.observe(on_tick("not a cron"), "e2e::tick-never");
    let err = settled(&handle).await.unwrap_err();
    assert_eq!(remote_code(&err), Some("INVALID_CRON_EXPRESSION"), "{err}");
    assert!(
        err.to_string()
            .contains("invalid cron expression 'not a cron'"),
        "{err}"
    );
    assert_eq!(boot.scheduler.job_count().await, 0);

    boot.shutdown().await;
    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn calling_on_tick_from_another_worker_is_forbidden() {
    let Some(owner) = engine::connect_fresh_named("cron-e2e-direct-owner").await else {
        return;
    };
    let boot = iii_cron::boot::start(owner.clone());
    let Some(consumer) = engine::connect_fresh_named("cron-e2e-direct-consumer").await else {
        return;
    };
    let err = consumer
        .call(CallRequest {
            timeout_ms: Some(5000),
            ..CallRequest::new(
                "cron::on-tick",
                json!({
                    "expression": EVERY_SECOND,
                    "scheduled_time": "2026-10-20T00:00:00+00:00",
                    "actual_time": "2026-10-20T00:00:00+00:00",
                }),
            )
        })
        .await
        .expect_err("a forged tick must be refused");
    assert_eq!(remote_code(&err), Some("FORBIDDEN"), "{err}");

    boot.shutdown().await;
    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

/// Waits until `cond` holds (up to about 10 s).
async fn wait_for<F, Fut>(what: &str, cond: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..100 {
        if cond().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{what}: timed out");
}

/// Two instances of the worker on separate connections. The second
/// registers `cron::on-tick` as a standby: while the first hosts it, only the
/// first runs jobs and each expression fires once per scheduled time. When
/// the first leaves, the second takes over the existing observers and their
/// ticks continue.
#[tokio::test]
#[serial]
async fn a_standby_instance_takes_over_the_observers_when_the_host_leaves() {
    let Some(first) = engine::connect_fresh_named("cron-e2e-standby-first").await else {
        return;
    };
    let first_boot = iii_cron::boot::start(first.clone());
    let Some(consumer) = engine::connect_fresh_named("cron-e2e-standby-consumer").await else {
        return;
    };
    let received = recorder(&consumer, "e2e::tick-standby");
    let handle = consumer.observe(on_tick(EVERY_SECOND), "e2e::tick-standby");
    settled(&handle)
        .await
        .expect("the first instance accepts the observer");
    assert!(first_boot.scheduler.retains(handle.id()).await);

    let Some(second) = engine::connect_fresh_named("cron-e2e-standby-second").await else {
        return;
    };
    let second_boot = iii_cron::boot::start(second.clone());

    // Past a few of the standby's registration attempts (one a second).
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert!(
        first_boot.scheduler.retains(handle.id()).await,
        "the host runs the observer's job"
    );
    assert_eq!(
        second_boot.scheduler.job_count().await,
        0,
        "the standby runs no job while the first instance hosts the function"
    );
    let before = scheduled_times(&received, |p| &p["input"]);
    assert!(before.len() >= 3, "{before:?}");

    // The host leaves: its process stops the function, then disconnects.
    first_boot.shutdown().await;
    first.shutdown_async().await;
    wait_for(
        "the standby takes over the observer and starts its job",
        || second_boot.scheduler.retains(handle.id()),
    )
    .await;

    let seen = received.count();
    common::wait_for_fires(&received.fires, seen + 2).await;
    // One tick per scheduled time across the handover, every one of them
    // through the observer the consumer created before it.
    let after = scheduled_times(&received, |p| &p["input"]);
    assert!(after.len() >= before.len() + 2, "{after:?}");
    for payload in received.lock().iter() {
        assert_eq!(payload["observer_id"], handle.id(), "{payload}");
        assert_eq!(payload["input"]["expression"], EVERY_SECOND);
    }

    handle.dispose().await.unwrap();
    wait_for("the last observer stops the standby's job", || async {
        !second_boot.scheduler.retains(handle.id()).await
    })
    .await;
    second_boot.shutdown().await;
    consumer.shutdown_async().await;
    second.shutdown_async().await;
}
