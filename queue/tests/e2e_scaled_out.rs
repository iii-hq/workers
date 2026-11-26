#![cfg(feature = "rabbitmq")]
//! One app scaled out as several namespaces (a namespace per instance, each with its own queue
//! worker, all on one RabbitMQ vhost): its subscriptions form consumer groups (`group`) and its
//! function-queue jobs run in the worker that takes them (`run_jobs_in: worker`).
//! Needs an engine (`III_ENGINE_WS_URL`) and Docker; skips without them unless `III_E2E_REQUIRE`.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use iii_queue::boot::BootHandle;
use iii_queue::config::{AdapterEntry, QueueConfig, RunJobsIn};
use iii_queue::functions::{DLQ_MESSAGES_FN_ID, DLQ_TOPICS_FN_ID, PUBLISH_FN_ID};
use iii_queue::on_message::ON_MESSAGE_FN;
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    CallAction, CallRequest, IIIClient, InitOptions, ObserverHandle, ObserverOptions,
    RegisterFunction, Source, WorkerIdentityMode,
};
use lapin::options::QueueDeclareOptions;
use lapin::types::FieldTable;
use lapin::{Connection, ConnectionProperties};
use serde_json::{Value, json};
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

/// Subscribe `function_id` to `topic` in the instance's consumer group named for the function:
/// observe the instance's `queue::on-message` with the topic and the group as args.
fn subscribe(instance: &Instance, function_id: &str, topic: &str) -> ObserverHandle {
    instance.iii.observe(
        Source::function(ON_MESSAGE_FN).args(
            json!({ "topic": topic, "group": function_id, "max_retries": 1, "backoff_ms": 5 }),
        ),
        function_id,
    )
}

async fn publish(instance: &Instance, topic: &str, value: &str) {
    instance
        .iii
        .call(
            CallRequest {
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
    let _ = subscribe(&a, &function_id, &topic);
    let _ = subscribe(&b, &function_id, &topic);
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
    let old_observer = subscribe(&old, &function_id, &topic);
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
    let _ = subscribe(&new, &function_id, &topic);
    tokio::time::sleep(Duration::from_millis(500)).await;

    publish(&new, &topic, "after-deploy").await;
    let handled = settled_total(&[&new], 1, Duration::from_secs(1)).await;
    // The group is the subscription: the old observer never had a queue of its own.
    let orphan = queue_counts(
        &url,
        &format!("iii.{topic}.observer:{}.queue", old_observer.id()),
    )
    .await;
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
/// instance to its own namespace and a few functions of `default`, including what the queue
/// worker uses to hold a job for its target: `engine::functions::info` and the engine's change
/// events, which it observes.
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
                    "engine::functions::info",
                    "engine::on-functions-changed",
                    "engine::on-workers-changed"
                ],
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
        .call(
            CallRequest {
                function_id: function_id.to_string(),
                payload: json!({ "to": "a@example.com" }),
                action: Some(CallAction::Enqueue {
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

async fn retry_until_handled(
    app: &Instance,
    handled: &AtomicUsize,
    deadline: Duration,
    request: impl Fn() -> CallRequest,
) -> bool {
    let before = handled.load(Ordering::SeqCst);
    let end = Instant::now() + deadline;
    while Instant::now() < end {
        // Fails or is dropped while the adapter is being rebuilt.
        let _ = app.iii.call(request().namespace(&app.namespace)).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        if handled.load(Ordering::SeqCst) > before {
            return true;
        }
    }
    false
}

/// A broker that drops every connection (a RabbitMQ restart, say) is noticed:
/// the worker rebuilds its adapter and re-attaches the topic subscriber and
/// the function queue consumer, so delivery resumes without restarting the
/// worker. Each path counts its own deliveries, so one cannot pass for the
/// other.
#[tokio::test]
#[serial]
async fn delivery_resumes_after_the_broker_drops_every_connection() {
    let Some(broker) = docker::start_rabbitmq().await else {
        return;
    };
    let url = broker.amqp_url();
    let suffix = Uuid::new_v4().simple().to_string();
    let topic = format!("orders-{suffix}");
    let queue = format!("mail-{suffix}");
    let on_order = format!("app::on_order_{suffix}");
    let send_mail = format!("app::send_mail_{suffix}");
    let Some(app) = instance(
        &format!("reconnect-{suffix}"),
        config(&url, Some(&queue)),
        Some(&on_order),
    )
    .await
    else {
        return;
    };
    let mails = Arc::new(AtomicUsize::new(0));
    {
        let mails = mails.clone();
        app.iii.register_function(
            send_mail.as_str(),
            RegisterFunction::new_async(move |_payload: Value| {
                let mails = mails.clone();
                async move {
                    mails.fetch_add(1, Ordering::SeqCst);
                    Ok::<Value, Error>(json!({ "ok": true }))
                }
            }),
        );
    }
    let _ = subscribe(&app, &on_order, &topic);
    tokio::time::sleep(Duration::from_millis(500)).await;
    publish(&app, &topic, "before").await;
    enqueue(&app, &send_mail, &queue).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while (app.fires.load(Ordering::SeqCst) == 0 || mails.load(Ordering::SeqCst) == 0)
        && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        app.fires.load(Ordering::SeqCst),
        1,
        "the topic path delivers before the drop"
    );
    assert_eq!(
        mails.load(Ordering::SeqCst),
        1,
        "the queue path delivers before the drop"
    );

    broker.close_all_connections();

    let fires = app.fires.clone();
    let topic_resumed =
        retry_until_handled(&app, &fires, Duration::from_secs(60), || CallRequest {
            function_id: PUBLISH_FN_ID.to_string(),
            payload: json!({ "topic": topic, "data": { "value": "after" } }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    let queue_resumed =
        retry_until_handled(&app, &mails, Duration::from_secs(60), || CallRequest {
            function_id: send_mail.clone(),
            payload: json!({ "to": "a@example.com" }),
            action: Some(CallAction::Enqueue {
                queue: queue.clone(),
            }),
            timeout_ms: Some(5_000),
        })
        .await;
    app.stop().await;
    assert!(
        topic_resumed,
        "the topic subscriber is re-attached after the drop"
    );
    assert!(
        queue_resumed,
        "the function queue consumer is re-attached after the drop"
    );
}

/// A channel-level error does not rebuild the adapter. `dlq_topics` declares,
/// passively, `<topic>.dlq` for a subscribed topic, which is never declared,
/// and RabbitMQ answers NOT_FOUND by closing that channel. A rebuild would
/// requeue the unacked delivery of a job still running, and the job would run
/// a second time while its first run continues (#1919).
#[tokio::test]
#[serial]
async fn a_channel_error_does_not_deliver_a_running_job_again() {
    let Some(broker) = docker::start_rabbitmq().await else {
        return;
    };
    let url = broker.amqp_url();
    let suffix = Uuid::new_v4().simple().to_string();
    let topic = format!("orders-{suffix}");
    let queue = format!("slow-{suffix}");
    let on_order = format!("app::on_order_{suffix}");
    let slow = format!("app::slow_{suffix}");
    let Some(app) = instance(
        &format!("channel-error-{suffix}"),
        config(&url, Some(&queue)),
        Some(&on_order),
    )
    .await
    else {
        return;
    };
    let runs = Arc::new(AtomicUsize::new(0));
    {
        let runs = runs.clone();
        app.iii.register_function(
            slow.as_str(),
            RegisterFunction::new_async(move |_payload: Value| {
                let runs = runs.clone();
                async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    Ok::<Value, Error>(json!({ "ok": true }))
                }
            }),
        );
    }
    subscribe_and_deliver_once(&app, &on_order, &topic).await;
    enqueue(&app, &slow, &queue).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while runs.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the job starts");

    dlq_topics(&app).await;

    // Past the end of the first run, so a redelivery has had time to start.
    tokio::time::sleep(Duration::from_secs(6)).await;
    let runs = runs.load(Ordering::SeqCst);
    app.stop().await;
    assert_eq!(runs, 1, "the running job is not delivered again");
}

/// Subscribes and waits for one delivery, so the topic is known to the adapter
/// and `dlq_topics` probes its `<topic>.dlq`, which is never declared.
async fn subscribe_and_deliver_once(app: &Instance, function_id: &str, topic: &str) {
    let _ = subscribe(app, function_id, topic);
    tokio::time::sleep(Duration::from_millis(500)).await;
    publish(app, topic, "probe").await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.fires.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(app.fires.load(Ordering::SeqCst), 1, "the topic delivers");
}

async fn dlq_topics(app: &Instance) {
    app.iii
        .call(
            CallRequest {
                function_id: DLQ_TOPICS_FN_ID.to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(5_000),
            }
            .namespace(&app.namespace),
        )
        .await
        .expect("dlq_topics should succeed");
}

/// Channel errors do not wear out the connection's channel ids. lapin keeps a
/// server-closed channel's id poisoned for the connection's life, and a reused
/// poisoned id stays stuck, so errors that no longer rebuild the adapter would
/// exhaust its ids and every DLQ and inspection call would fail.
/// `channel_max=8` makes that a few calls instead of thousands.
#[tokio::test]
#[serial]
async fn channel_errors_do_not_exhaust_the_channel_ids() {
    let Some(broker) = docker::start_rabbitmq().await else {
        return;
    };
    let url = format!("{}?channel_max=8", broker.amqp_url());
    let suffix = Uuid::new_v4().simple().to_string();
    let topic = format!("orders-{suffix}");
    let queue = format!("mail-{suffix}");
    let on_order = format!("app::on_order_{suffix}");
    let Some(app) = instance(
        &format!("channel-ids-{suffix}"),
        config(&url, Some(&queue)),
        Some(&on_order),
    )
    .await
    else {
        return;
    };
    subscribe_and_deliver_once(&app, &on_order, &topic).await;

    for _ in 0..20 {
        dlq_topics(&app).await;
    }

    // The function queue's dead-letter queue exists, so this needs only a
    // channel that opens.
    let browsed = app
        .iii
        .call(
            CallRequest {
                function_id: DLQ_MESSAGES_FN_ID.to_string(),
                payload: json!({ "topic": queue }),
                action: None,
                timeout_ms: Some(5_000),
            }
            .namespace(&app.namespace),
        )
        .await;
    app.stop().await;
    browsed.expect("an operation channel still opens after channel errors");
}

/// A subscription whose queue was made earlier with another `max_priority`
/// fails that subscribe only. Its declare used to run on the shared channel,
/// whose close rebuilt the adapter, ran the running job of another
/// subscription again, and repeated on every resubscribe.
#[tokio::test]
#[serial]
async fn a_conflicting_subscription_does_not_run_a_running_job_again() {
    let Some(broker) = docker::start_rabbitmq().await else {
        return;
    };
    let url = broker.amqp_url();
    let suffix = Uuid::new_v4().simple().to_string();
    let busy_topic = format!("busy-{suffix}");
    let new_topic = format!("new-{suffix}");
    let busy = format!("app::busy_{suffix}");
    let newcomer = format!("app::newcomer_{suffix}");
    let newcomer_observer = format!("newcomer-{suffix}");
    {
        let connection = Connection::connect(&url, ConnectionProperties::default())
            .await
            .expect("connect to the broker");
        let channel = connection.create_channel().await.expect("open a channel");
        let mut args = FieldTable::default();
        args.insert(
            "x-max-priority".into(),
            lapin::types::AMQPValue::ShortShortUInt(5),
        );
        channel
            .queue_declare(
                &format!("iii.{new_topic}.observer:{newcomer_observer}.queue"),
                QueueDeclareOptions {
                    durable: true,
                    ..QueueDeclareOptions::default()
                },
                args,
            )
            .await
            .expect("declare the newcomer's queue with another priority");
        let _ = connection.close(0, "").await;
    }
    let Some(app) = instance(&format!("conflict-{suffix}"), config(&url, None), None).await else {
        return;
    };
    let runs = Arc::new(AtomicUsize::new(0));
    {
        let runs = runs.clone();
        app.iii.register_function(
            busy.as_str(),
            RegisterFunction::new_async(move |_payload: Value| {
                let runs = runs.clone();
                async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    Ok::<Value, Error>(json!({ "ok": true }))
                }
            }),
        );
    }
    app.iii.register_function(
        newcomer.as_str(),
        RegisterFunction::new_async(|_payload: Value| async move {
            Ok::<Value, Error>(json!({ "ok": true }))
        }),
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let _busy = subscribe(&app, &busy, &busy_topic);
    tokio::time::sleep(Duration::from_millis(500)).await;
    publish(&app, &busy_topic, "one").await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while runs.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the job starts");

    let _newcomer = app.iii.observe_with(
        Source::function(ON_MESSAGE_FN)
            .args(json!({ "topic": new_topic, "queue_config": { "max_priority": 10 } })),
        newcomer.as_str(),
        ObserverOptions::default().id(newcomer_observer),
    );
    tokio::time::sleep(Duration::from_secs(8)).await;
    let runs = runs.load(Ordering::SeqCst);
    app.stop().await;
    assert_eq!(runs, 1, "the running job is not run again");
}
