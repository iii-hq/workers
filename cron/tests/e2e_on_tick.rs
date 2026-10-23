//! `cron::on-tick` against a live engine, next to the `cron` trigger type.
//!
//! Each test boots the worker in-process on its own connection and consumes
//! it from another. On an engine with observe, observers and a binding on the
//! same expression share one job; on an engine without it, `observe` fails
//! with `UNSUPPORTED` and the binding works as before.

mod common;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{Error, IIIClient, ObserverHandle, RegisterFunction, Source};
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

fn observe(iii: &IIIClient, args: Value, target: &str) -> ObserverHandle {
    iii.observe(Source::function("cron::on-tick").args(args), target)
}

/// The scheduled times seen by one target, failing on a duplicate: one tick
/// per period per target.
fn scheduled_times(received: &Received, field: impl Fn(&Value) -> &Value) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    for payload in received.lock().iter() {
        let at = field(payload)["scheduled_time"]
            .as_str()
            .unwrap_or_else(|| panic!("no scheduled_time in {payload}"))
            .to_string();
        assert!(seen.insert(at.clone()), "delivered twice: {at}");
    }
    seen
}

#[tokio::test]
#[serial]
async fn observers_and_a_binding_share_one_job_per_expression() {
    let Some(owner) = engine::connect_fresh_named("cron-e2e-share-owner").await else {
        return;
    };
    let boot = iii_cron::boot::start(owner.clone(), iii_cron::config::CronConfig::default())
        .await
        .expect("cron worker should boot");
    let Some(consumer) = engine::connect_fresh_named("cron-e2e-share-consumer").await else {
        return;
    };
    let jobs = || async { boot.scheduler.read().await.job_count().await };

    let bound = recorder(&consumer, "e2e::tick-bound");
    let observed = recorder(&consumer, "e2e::tick-observed");
    let even = recorder(&consumer, "e2e::tick-even");

    // Deprecated in iii-sdk 0.25: the old consumer path, served by the
    // `cron` trigger type on both engines.
    #[allow(deprecated)]
    let binding = consumer
        .register_trigger(RegisterTriggerInput::new(
            "cron".to_string(),
            "e2e::tick-bound".to_string(),
            json!({ "expression": EVERY_SECOND }),
        ))
        .expect("trigger registration");

    let every = observe(
        &consumer,
        json!({ "expression": EVERY_SECOND }),
        "e2e::tick-observed",
    );
    if consumer.engine_supports_observe() != Some(true) {
        // An engine without observe: the function is a plain one, `observe`
        // fails cleanly, and the binding works as before.
        let err = settled(&every).await.unwrap_err();
        assert_eq!(remote_code(&err), Some("UNSUPPORTED"), "{err}");
        common::wait_for_fires(&bound.fires, 2).await;
        assert_eq!(jobs().await, 1);
        scheduled_times(&bound, |p| p);
        binding.unregister();
        boot.shutdown().await;
        consumer.shutdown_async().await;
        owner.shutdown_async().await;
        return;
    }
    settled(&every)
        .await
        .expect("observer on an every-second schedule");
    let even_handle = observe(
        &consumer,
        json!({ "expression": EVEN_SECONDS }),
        "e2e::tick-even",
    );
    settled(&even_handle)
        .await
        .expect("observer on even seconds");
    assert_eq!(jobs().await, 2, "one job per distinct expression");

    tokio::time::sleep(Duration::from_millis(4500)).await;

    // The binding keeps its pre-0.25 payload.
    let bound_times = scheduled_times(&bound, |p| p);
    for payload in bound.lock().iter() {
        assert_eq!(payload["trigger"], "cron");
        assert!(payload["job_id"].as_str().is_some_and(|s| !s.is_empty()));
    }
    // Observers get the tick in the envelope's input, without job_id.
    let observed_times = scheduled_times(&observed, |p| &p["input"]);
    for payload in observed.lock().iter() {
        assert_eq!(payload["input"]["expression"], EVERY_SECOND);
        assert_eq!(payload["input"]["trigger"], "cron");
        assert!(payload["input"].get("job_id").is_none(), "{payload}");
        assert_eq!(payload["observer_id"], every.id());
    }
    assert!(observed_times.len() >= 3, "{observed_times:?}");
    assert!(bound_times.len() >= 3, "{bound_times:?}");
    assert!(
        observed_times.intersection(&bound_times).count() >= 2,
        "the binding and the observer are served by the same fires"
    );
    let even_times = scheduled_times(&even, |p| &p["input"]);
    assert!(!even_times.is_empty());
    for payload in even.lock().iter() {
        assert_eq!(payload["input"]["expression"], EVEN_SECONDS);
    }
    for at in &even_times {
        let at = chrono::DateTime::parse_from_rfc3339(at).unwrap();
        assert_eq!(chrono::Timelike::second(&at) % 2, 0, "{at}");
    }

    // The observer leaves: the binding keeps the job, the observer hears no more.
    every.dispose().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (seen, fired) = (observed.count(), bound.count());
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(observed.count(), seen, "a disposed observer hears nothing");
    assert!(bound.count() > fired, "the binding keeps firing");
    assert_eq!(jobs().await, 2, "the binding keeps the every-second job");

    // The engine tells the owner after `dispose()` resolves.
    even_handle.dispose().await.unwrap();
    wait_for_jobs(&jobs, 1, "the last observer of an expression stops its job").await;

    binding.unregister();
    wait_for_jobs(&jobs, 0, "the last binding stops the job").await;

    boot.shutdown().await;
    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn observer_args_are_validated_like_the_trigger_config() {
    let Some(owner) = engine::connect_fresh_named("cron-e2e-args-owner").await else {
        return;
    };
    let boot = iii_cron::boot::start(owner.clone(), iii_cron::config::CronConfig::default())
        .await
        .expect("cron worker should boot");
    let Some(consumer) = engine::connect_fresh_named("cron-e2e-args-consumer").await else {
        return;
    };
    if consumer.engine_supports_observe() != Some(true) {
        boot.shutdown().await;
        consumer.shutdown_async().await;
        owner.shutdown_async().await;
        return;
    }
    let _target = recorder(&consumer, "e2e::tick-never");

    for args in [
        json!({}),
        json!({ "expression": 5 }),
        json!({ "expression": EVERY_SECOND, "condition_function_id": "e2e::cond" }),
    ] {
        let handle = observe(&consumer, args.clone(), "e2e::tick-never");
        let err = settled(&handle).await.unwrap_err();
        assert_eq!(remote_code(&err), Some("INVALID_ARGS"), "{args}: {err}");
    }
    let handle = observe(
        &consumer,
        json!({ "expression": "not a cron" }),
        "e2e::tick-never",
    );
    let err = settled(&handle).await.unwrap_err();
    assert_eq!(remote_code(&err), Some("INVALID_CRON_EXPRESSION"), "{err}");
    assert!(
        err.to_string()
            .contains("invalid cron expression 'not a cron'"),
        "{err}"
    );
    assert_eq!(boot.scheduler.read().await.job_count().await, 0);

    boot.shutdown().await;
    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn invoking_on_tick_directly_is_refused() {
    let Some(owner) = engine::connect_fresh_named("cron-e2e-direct-owner").await else {
        return;
    };
    let boot = iii_cron::boot::start(owner.clone(), iii_cron::config::CronConfig::default())
        .await
        .expect("cron worker should boot");
    let Some(consumer) = engine::connect_fresh_named("cron-e2e-direct-consumer").await else {
        return;
    };
    let err = consumer
        .trigger(TriggerRequest {
            function_id: "cron::on-tick".to_string(),
            payload: json!({
                "trigger": "cron",
                "expression": EVERY_SECOND,
                "scheduled_time": "2026-10-20T00:00:00+00:00",
                "actual_time": "2026-10-20T00:00:00+00:00",
            }),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .expect_err("a forged tick must be refused");
    assert_eq!(remote_code(&err), Some("FORBIDDEN"), "{err}");

    boot.shutdown().await;
    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}
