#![cfg(feature = "rabbitmq")]
//! One app scaled out as several namespaces (a namespace per instance, each with its own queue
//! worker, all on one RabbitMQ vhost): its subscriptions form consumer groups (`group`) and its
//! function-queue jobs run in the worker that takes them (`run_jobs_in: worker`).
//! Needs an engine (`III_ENGINE_WS_URL`) and Docker; skips without them unless `III_E2E_REQUIRE`.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_queue::boot::BootHandle;
use iii_queue::config::{AdapterEntry, QueueConfig, RunJobsIn};
use iii_queue::functions::PUBLISH_FN_ID;
use iii_queue::TRIGGER_TYPE;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{IIIClient, InitOptions, RegisterFunction, TriggerAction, WorkerIdentityMode};
use lapin::options::QueueDeclareOptions;
use lapin::types::FieldTable;
use lapin::{Connection, ConnectionProperties};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::time::Instant;
use uuid::Uuid;

use common::{docker, engine};

struct Instance {
    namespace: String,
    iii: Arc<IIIClient>,
    boot: BootHandle,
    fires: Arc<AtomicUsize>,
}

impl Instance {
    async fn stop(self) {
        self.boot.shutdown().await;
        self.iii.shutdown_async().await;
    }
}

fn config(amqp_url: &str, queue: Option<&str>) -> QueueConfig {
    let mut config = QueueConfig {
        adapter: Some(AdapterEntry {
            name: "rabbitmq".to_string(),
            config: Some(json!({ "amqp_url": amqp_url })),
        }),
        run_jobs_in: RunJobsIn::Worker,
        ..QueueConfig::default()
    };
    if let Some(queue) = queue {
        config
            .queue_configs
            .insert(queue.to_string(), Default::default());
    }
    config
}

fn options(namespace: &str) -> InitOptions {
    InitOptions {
        metadata: Some(WorkerMetadata {
            name: namespace.to_string(),
            ..WorkerMetadata::default()
        }),
        namespace: Some(namespace.to_string()),
        identity: WorkerIdentityMode::Explicit,
        ..InitOptions::default()
    }
}

/// A instance: a queue worker booted in `namespace`, plus (when `function_id` is given) the app's
/// counting function, registered in the same namespace like any other worker of that instance.
async fn instance(
    namespace: &str,
    config: QueueConfig,
    function_id: Option<&str>,
) -> Option<Instance> {
    let iii = engine::connect_fresh_with_options(options(namespace)).await?;
    start_instance(iii, namespace, config, function_id).await
}

async fn start_instance(
    iii: Arc<IIIClient>,
    namespace: &str,
    config: QueueConfig,
    function_id: Option<&str>,
) -> Option<Instance> {
    let boot = iii_queue::boot::start(iii.clone(), config)
        .await
        .expect("queue worker should boot");
    let fires = Arc::new(AtomicUsize::new(0));
    if let Some(function_id) = function_id {
        let counter = fires.clone();
        iii.register_function(
            function_id,
            RegisterFunction::new_async(move |_payload: Value| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok::<Value, Error>(json!({ "ok": true }))
                }
            }),
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Some(Instance {
        namespace: namespace.to_string(),
        iii,
        boot,
        fires,
    })
}

fn subscribe(instance: &Instance, function_id: &str, topic: &str) {
    instance
        .iii
        .register_trigger(RegisterTriggerInput::new(
            TRIGGER_TYPE.to_string(),
            function_id.to_string(),
            json!({ "queue": topic, "group": function_id, "max_retries": 1, "backoff_ms": 5 }),
        ))
        .expect("register durable subscriber trigger");
}

async fn publish(instance: &Instance, topic: &str, value: &str) {
    instance
        .iii
        .trigger(
            TriggerRequest {
                function_id: PUBLISH_FN_ID.to_string(),
                payload: json!({ "topic": topic, "data": { "value": value } }),
                action: None,
                timeout_ms: Some(5_000),
            }
            .namespace(&instance.namespace),
        )
        .await
        .expect("publish should succeed");
}

/// `(messages, consumers)` of a queue, `None` when it does not exist.
async fn queue_counts(amqp_url: &str, queue: &str) -> Option<(u32, u32)> {
    let connection = Connection::connect(amqp_url, ConnectionProperties::default())
        .await
        .expect("connect to the broker");
    let channel = connection.create_channel().await.expect("open a channel");
    let declared = channel
        .queue_declare(
            queue,
            QueueDeclareOptions {
                passive: true,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await
        .ok()
        .map(|q| (q.message_count(), q.consumer_count()));
    let _ = connection.close(0, "").await;
    declared
}

fn total(instances: &[&Instance]) -> usize {
    instances
        .iter()
        .map(|r| r.fires.load(Ordering::SeqCst))
        .sum()
}

/// Waits `settle` for deliveries to stop arriving, then returns the total.
async fn settled_total(instances: &[&Instance], at_least: usize, settle: Duration) -> usize {
    let deadline = Instant::now() + Duration::from_secs(10);
    while total(instances) < at_least && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    tokio::time::sleep(settle).await;
    total(instances)
}

/// Two instances of one subscriber in one consumer group: an event is handled once, by either of
/// them, not once per instance.
#[tokio::test]
#[serial]
async fn an_event_is_handled_once_by_a_consumer_group_across_namespaces() {
    let Some(broker) = docker::start_rabbitmq().await else {
        return;
    };
    let url = broker.amqp_url();
    let suffix = Uuid::new_v4().simple().to_string();
    let topic = format!("orders-{suffix}");
    let function_id = format!("app::on_order_{suffix}");
    let Some(a) = instance(
        &format!("r1-web-a-{suffix}"),
        config(&url, None),
        Some(&function_id),
    )
    .await
    else {
        return;
    };
    let Some(b) = instance(
        &format!("r1-web-b-{suffix}"),
        config(&url, None),
        Some(&function_id),
    )
    .await
    else {
        return;
    };
    subscribe(&a, &function_id, &topic);
    subscribe(&b, &function_id, &topic);
    tokio::time::sleep(Duration::from_millis(500)).await;

    publish(&a, &topic, "one").await;
    let handled = settled_total(&[&a, &b], 1, Duration::from_secs(2)).await;
    a.stop().await;
    b.stop().await;
    assert_eq!(
        handled, 1,
        "one event, one logical subscriber: handled once"
    );
}

/// An instance that goes away for good (replaced by one in a namespace with a new name) leaves no
/// queue behind that keeps collecting events nobody will read.
#[tokio::test]
#[serial]
async fn a_replaced_instance_leaves_no_queue_collecting_events() {
    let Some(broker) = docker::start_rabbitmq().await else {
        return;
    };
    let url = broker.amqp_url();
    let suffix = Uuid::new_v4().simple().to_string();
    let topic = format!("orders-{suffix}");
    let function_id = format!("app::on_order_{suffix}");
    let old_ns = format!("r1-web-a-{suffix}");
    let Some(old) = instance(&old_ns, config(&url, None), Some(&function_id)).await else {
        return;
    };
    subscribe(&old, &function_id, &topic);
    tokio::time::sleep(Duration::from_millis(500)).await;
    old.stop().await;

    // Its replacement: the same function, in an instance with a new namespace.
    let Some(new) = instance(
        &format!("r2-web-a-{suffix}"),
        config(&url, None),
        Some(&function_id),
    )
    .await
    else {
        return;
    };
    subscribe(&new, &function_id, &topic);
    tokio::time::sleep(Duration::from_millis(500)).await;

    publish(&new, &topic, "after-deploy").await;
    let handled = settled_total(&[&new], 1, Duration::from_secs(1)).await;
    let orphan = queue_counts(&url, &format!("iii.{topic}.{function_id}@{old_ns}.queue")).await;
    new.stop().await;
    assert_eq!(handled, 1, "the new instance handles the event");
    assert!(
        !matches!(orphan, Some((messages, 0)) if messages > 0),
        "the old instance's queue must not keep collecting events: {orphan:?}"
    );
}

/// An instance that does not host a function enqueues a job for it, and an instance that hosts it
/// runs it.
#[tokio::test]
#[serial]
async fn a_job_enqueued_by_one_instance_runs_in_an_instance_that_hosts_the_function() {
    let Some(broker) = docker::start_rabbitmq().await else {
        return;
    };
    let url = broker.amqp_url();
    let suffix = Uuid::new_v4().simple().to_string();
    let queue = format!("jobs-{suffix}");
    let function_id = format!("app::send_email_{suffix}");
    let Some(web) = instance(
        &format!("r1-web-a-{suffix}"),
        config(&url, Some(&queue)),
        None,
    )
    .await
    else {
        return;
    };
    let Some(worker) = instance(
        &format!("r1-worker-a-{suffix}"),
        config(&url, Some(&queue)),
        Some(&function_id),
    )
    .await
    else {
        return;
    };

    enqueue(&web, &function_id, &queue).await;
    let handled = settled_total(&[&worker], 1, Duration::from_millis(500)).await;
    web.stop().await;
    worker.stop().await;
    assert_eq!(handled, 1, "the worker instance runs the job");
}

/// An RBAC listener (`III_ENGINE_RBAC_WS_URL`, auth function `test::auth`) whose grant keeps each
/// instance to its own namespace, a few functions of `default` and the trigger types it needs,
/// including what the queue worker uses to hold a job for its target: `engine::functions::info`
/// and the engine's change triggers.
async fn rbac_trusted() -> Option<Arc<IIIClient>> {
    let trusted = engine::connect_fresh().await?;
    trusted.register_function(
        "test::auth",
        RegisterFunction::new_async(|input: Value| async move {
            let namespace = input["query_params"]["namespace"][0]
                .as_str()
                .unwrap_or_default()
                .to_string();
            Ok::<Value, Error>(json!({
                "namespaces": { namespace.as_str(): ["*"] },
                "allowed_functions": [
                    "configuration::ensure",
                    "configuration::get",
                    "engine::workers::list",
                    "engine::functions::list",
                    "engine::functions::info"
                ],
                "allowed_trigger_types": [
                    "http", "cron", "state", "durable:subscriber", "subscribe", "database::row-changed",
                    "engine::functions-available", "engine::workers-available"
                ],
                "allow_trigger_type_registration": true,
                "context": { "namespace": namespace },
            }))
        }),
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    Some(trusted)
}

async fn rbac_instance(
    namespace: &str,
    config: QueueConfig,
    function_id: Option<&str>,
) -> Option<Instance> {
    let rbac_url = std::env::var("III_ENGINE_RBAC_WS_URL").ok()?;
    let iii = Arc::new(iii_sdk::register_worker(
        &format!("{rbac_url}/?namespace={namespace}"),
        options(namespace),
    ));
    tokio::time::sleep(Duration::from_millis(500)).await;
    start_instance(iii, namespace, config, function_id).await
}

async fn enqueue(from: &Instance, function_id: &str, queue: &str) {
    let receipt = from
        .iii
        .trigger(
            TriggerRequest {
                function_id: function_id.to_string(),
                payload: json!({ "to": "a@example.com" }),
                action: Some(TriggerAction::Enqueue {
                    queue: queue.to_string(),
                }),
                timeout_ms: Some(5_000),
            }
            .namespace(&from.namespace),
        )
        .await
        .expect("enqueue should be accepted");
    assert!(receipt["messageReceiptId"].is_string());
}

/// Under a namespace-scoped RBAC grant, a `web` instance enqueues a job for a function only the
/// `worker` instance hosts, and the worker runs it.
#[tokio::test]
#[serial]
async fn under_a_scoped_grant_a_job_crosses_from_web_to_worker() {
    if std::env::var("III_ENGINE_RBAC_WS_URL").is_err() {
        eprintln!("[skip] III_ENGINE_RBAC_WS_URL not set");
        return;
    }
    let Some(broker) = docker::start_rabbitmq().await else {
        return;
    };
    let url = broker.amqp_url();
    let suffix = Uuid::new_v4().simple().to_string();
    let queue = format!("jobs-{suffix}");
    let function_id = format!("app::send_email_{suffix}");
    let Some(trusted) = rbac_trusted().await else {
        return;
    };
    let web = rbac_instance(
        &format!("r1-web-a-{suffix}"),
        config(&url, Some(&queue)),
        None,
    )
    .await
    .expect("web instance");
    let worker = rbac_instance(
        &format!("r1-worker-a-{suffix}"),
        config(&url, Some(&queue)),
        Some(&function_id),
    )
    .await
    .expect("worker instance");

    enqueue(&web, &function_id, &queue).await;
    let handled = settled_total(&[&worker], 1, Duration::from_millis(500)).await;
    web.stop().await;
    worker.stop().await;
    trusted.shutdown_async().await;
    assert_eq!(handled, 1, "the worker instance runs the job");
}

/// Under a namespace-scoped RBAC grant, an instance that hosts a function runs the jobs it
/// enqueues for it.
#[tokio::test]
#[serial]
async fn under_a_scoped_grant_an_instance_runs_its_own_enqueued_job() {
    if std::env::var("III_ENGINE_RBAC_WS_URL").is_err() {
        eprintln!("[skip] III_ENGINE_RBAC_WS_URL not set");
        return;
    }
    let Some(broker) = docker::start_rabbitmq().await else {
        return;
    };
    let url = broker.amqp_url();
    let suffix = Uuid::new_v4().simple().to_string();
    let queue = format!("jobs-{suffix}");
    let function_id = format!("app::send_email_{suffix}");
    let Some(trusted) = rbac_trusted().await else {
        return;
    };
    let app = rbac_instance(
        &format!("r1-app-a-{suffix}"),
        config(&url, Some(&queue)),
        Some(&function_id),
    )
    .await
    .expect("app instance");

    enqueue(&app, &function_id, &queue).await;
    let handled = settled_total(&[&app], 1, Duration::from_millis(500)).await;
    app.stop().await;
    trusted.shutdown_async().await;
    assert_eq!(handled, 1, "the instance runs its own job");
}
