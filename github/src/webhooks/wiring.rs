use iii_sdk::{
    Call, CallRequest, Deliver, Match, ObserveRejection, Observer, ObserverOptions,
    RegisterFunction, Source,
};

use super::*;

/// How long a subscriber's target may take to answer one notification.
const TARGET_TIMEOUT_MS: u64 = 8_000;

/// How long an observer of a source this worker consumes may stay pending
/// before activation logs it.
const ACTIVATION_WAIT_SECS: u64 = 60;

/// The consumer group of the durable job queue: the queue worker keeps one
/// durable queue for the group, keyed by the group, whose jobs outlive the
/// observers that consume them.
pub const WEBHOOK_CONSUMER_GROUP: &str = "github-webhooks";

/// The engine's longest observer id, in bytes.
const MAX_OBSERVER_ID_LEN: usize = 128;

/// The id of the observer that consumes the webhook job queue:
/// `<worker name>:webhooks:<queue>`, preceded by `<namespace>/` when the
/// worker has a namespace. It is the same for every process of the worker,
/// so the consumer keeps one identity across restarts: the new process's
/// `observe` replaces the observer the previous process left suspended at
/// once (the engine disposes it and announces the new one to the queue
/// worker), and the engine holds one observer for the consumer, under that
/// id, through the grace period. The jobs the previous process left unacked
/// stay in the durable queue of the consumer group
/// ([`WEBHOOK_CONSUMER_GROUP`]), which the queue worker keys by the group
/// whatever the observer id. An id longer than the engine's 128 bytes becomes
/// `<worker name>:webhooks:sha256:<digest of the whole id>`, cut to 128 bytes.
pub fn queue_observer_id(namespace: Option<&str>, worker_name: &str, queue: &str) -> String {
    let id = match namespace {
        Some(namespace) => format!("{namespace}/{worker_name}:webhooks:{queue}"),
        None => format!("{worker_name}:webhooks:{queue}"),
    };
    if id.len() <= MAX_OBSERVER_ID_LEN {
        return id;
    }
    use sha2::Digest;
    let digest = hex::encode(Sha256::digest(id.as_bytes()));
    let mut short = format!("{worker_name}:webhooks:sha256:{digest}");
    let mut end = short.len().min(MAX_OBSERVER_ID_LEN);
    while !short.is_char_boundary(end) {
        end -= 1;
    }
    short.truncate(end);
    short
}

/// Refusal codes that mean another observer still holds what this one asks
/// for, until it goes away: the webhook route (`HTTP_ROUTE_CONFLICT`), or the
/// queue observer's id (`OBSERVER_ID_CONFLICT`), held by a connection of the
/// previous process that the engine has not seen close yet, or by another
/// worker until the engine's grace period (`observers.observer_grace_ms`,
/// 30 s by default) ends.
const HELD_BY_ANOTHER_OBSERVER: &[&str] = &["HTTP_ROUTE_CONFLICT", ID_HELD_BY_ANOTHER_CONNECTION];

/// The refusal the engine answers itself when another connection holds the
/// observer's id. It is the observe's final answer: the engine keeps nothing
/// under the id for the refused connection, and the handle's `dispose` sends
/// nothing and resolves at once.
const ID_HELD_BY_ANOTHER_CONNECTION: &str = "OBSERVER_ID_CONFLICT";

/// First wait before observing a refused source again; it doubles up to
/// [`REOBSERVE_MAX_SECS`].
const REOBSERVE_FIRST_SECS: u64 = 1;
const REOBSERVE_MAX_SECS: u64 = 30;

/// The SDK dispatches callbacks on a current-thread runtime. Drive service
/// futures on the blocking pool while the original runtime keeps driving I/O.
/// Store::read/change stay synchronous for lifecycle compatibility; no disk I/O
/// or SQLite mutex wait runs on the SDK executor, including detached recovery.
pub(super) async fn storage_task<T: Send + 'static>(
    future: impl std::future::Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || runtime.block_on(future)).await?
}

// The engine adds this transport field before typed SDK deserialization.
// Strip only that field, keeping strict business requests and their public schemas.
#[derive(JsonSchema)]
#[schemars(transparent)]
struct EngineRequest<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for EngineRequest<T> {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let mut payload = Value::deserialize(deserializer)?;
        if let Some(fields) = payload.as_object_mut() {
            fields.remove("_caller_worker_id");
        }
        T::deserialize(payload)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

impl Service {
    /// `onObserverAdded` of `github::pr::on-event`: validate the observer's
    /// args and persist it as a subscriber under its id. The engine holds its
    /// target, namespace and metadata.
    async fn add_subscriber(
        self: Arc<Self>,
        obs: Observer,
    ) -> std::result::Result<(), ObserveRejection> {
        let filter: EventFilter = obs.args()?;
        if let Some(repo) = &filter.repo {
            validate_repo(repo).map_err(|e| ObserveRejection::new(INVALID_ARGS, e.to_string()))?;
        }
        if let Some(policy) = &filter.notifications {
            policy
                .validate()
                .map_err(|e| ObserveRejection::new(INVALID_ARGS, e.to_string()))?;
        }
        let id = obs.id().to_owned();
        storage_task(async move {
            if let Some(store) = &self.store {
                store.change(|d| {
                    d.subscribers.insert(id.clone(), Subscriber { id, filter });
                    Ok(())
                })?;
            }
            Ok(())
        })
        .await
        .map_err(|e| ObserveRejection::new(STORAGE_FAILURE, e.to_string()))
    }

    /// `onObserverRemoved` of `github::pr::on-event`: drop the subscriber and
    /// stop the watches it was the last listener of.
    async fn remove_subscriber(self: Arc<Self>, obs: Observer) {
        let id = obs.id().to_owned();
        let result = storage_task(async move {
            if self.store.is_some() {
                self.drop_subscribers(&[id]).await?;
            }
            Ok(())
        })
        .await;
        if let Err(e) = result {
            tracing::warn!(observer = obs.id(), error = %e, "dropping a github::pr::on-event subscriber failed");
        }
    }
}

/// Rejection code of an observer whose repo or notification policy is invalid.
pub const INVALID_ARGS: &str = "INVALID_ARGS";
/// Rejection code of an observer the webhook store could not persist.
pub const STORAGE_FAILURE: &str = "STORAGE_FAILURE";

/// `onObserverDeliver` of `github::pr::on-event`. Each call carries one
/// subscriber's notification (its id in `metadata.subscriber`, matched by
/// key), and that subscriber's target answers it: the target's result, or
/// error, is the call's, so the durable job retries a failed delivery. A
/// subscriber whose filters reject the notification acks it with `null`.
async fn deliver_notification(
    iii: &IIIClient,
    call: Call,
    observers: Vec<Observer>,
) -> std::result::Result<Value, iii_sdk::Error> {
    let Some(obs) = observers.first() else {
        let subscriber = call
            .metadata
            .as_ref()
            .and_then(|m| m["subscriber"].as_str())
            .unwrap_or_default();
        return Err(ObserveRejection::new(
            "OBSERVER_NOT_ACTIVE",
            format!("subscriber `{subscriber}` of {ON_EVENT_FN} is not active"),
        )
        .into());
    };
    if !obs.matches(&call) {
        return Ok(Value::Null);
    }
    iii.call(
        CallRequest::observer(obs, call.input)
            .timeout(std::time::Duration::from_millis(TARGET_TIMEOUT_MS)),
    )
    .await
}

/// Registration has no external prerequisite. Disabled defaults never open the
/// database, probe gh, acquire a tunnel, register HTTP or publish to a queue.
pub async fn register(iii: &Arc<IIIClient>, cell: &ConfigCell, engine_url: &str) {
    let config = cell.read().await.webhooks.clone();
    let (store, storage_error) = if config.enabled {
        let storage_config = config.clone();
        match storage_task(async move {
            validate_config(&storage_config)?;
            Store::open(Path::new(&storage_config.storage_path))
        })
        .await
        {
            Ok(s) => (Some(s), None),
            Err(e) => {
                tracing::error!(error = %e, "webhook storage unavailable; mutations disabled");
                (None, Some(e.to_string()))
            }
        }
    } else {
        (None, None)
    };
    let service = Arc::new(Service {
        iii: iii.clone(),
        cell: cell.clone(),
        config,
        engine_url: engine_url.into(),
        store,
        storage_error,
        operations: Mutex::new(()),
        #[cfg(test)]
        bus: None,
    });
    // github::pr::on-event: hooks by id first, so they see every observer.
    let hooks = service.clone();
    iii.on_observer_added_for(ON_EVENT_FN, move |obs: Observer| {
        hooks.clone().add_subscriber(obs)
    });
    let hooks = service.clone();
    iii.on_observer_removed_for(ON_EVENT_FN, move |obs: Observer| {
        hooks.clone().remove_subscriber(obs)
    });
    iii.on_observer_match_for(
        ON_EVENT_FN,
        Match::keys(
            |obs| obs.id().into(),
            |call| {
                call.metadata
                    .as_ref()
                    .and_then(|m| m["subscriber"].as_str())
                    .into()
            },
        ),
    );
    let engine = iii.as_ref().clone();
    iii.on_observer_deliver_for(ON_EVENT_FN, move |call: Call, observers: Vec<Observer>| {
        let engine = engine.clone();
        async move { deliver_notification(&engine, call, observers).await }
    });
    iii.register_function(
        ON_EVENT_FN,
        RegisterFunction::new(|event: notifications::NotificationEvent| {
            Ok::<_, iii_sdk::Error>(event)
        })
        .description(ON_EVENT_DESCRIPTION)
        .args_format::<EventFilter>()
        .private(),
    );
    macro_rules! function {
        ($id:expr, $desc:expr, $req:ty, $resp:ty, $method:ident $(, $field:ident)?) => {{
            let service = service.clone();
            iii.register_function($id, RegisterFunction::new_async(move |EngineRequest(req): EngineRequest<$req>| {
                let service = service.clone();
                async move {
                    let result: Result<$resp> = storage_task(async move { service.$method(req $(.$field)?).await }).await;
                    result.map_err(iii_sdk::Error::from)
                }
            }).description($desc));
        }};
    }
    let s = service.clone();
    iii.register_function(
        "github::pr::watch",
        RegisterFunction::new_async(move |EngineRequest(req): EngineRequest<WatchRequest>| {
            let s = s.clone();
            async move {
                storage_task(async move {
                    // Definite missing prerequisites refuse with the fix list.
                    if s.store.is_some() {
                        s.require_setup().await?;
                    }
                    s.watch(req).await
                })
                .await
                .map_err(iii_sdk::Error::from)
            }
        })
        .description("Watch a PR via authenticated webhooks until merged/closed; expires_at must be in the future within webhooks.max_watch_days (default 30, the quick-tunnel lease cap). Refuses with the fix list when a webhook prerequisite is missing (see github::setup::webhooks-status)."),
    );
    function!(
        "github::setup::webhooks-status",
        "Checklist of PR webhook prerequisites (quick-tunnel worker, cloudflared binary, http webhook listener), whether webhooks are enabled/active, and how to fix each missing item. Read-only; never installs anything.",
        setup::SetupStatusRequest,
        setup::SetupStatus,
        setup_status
    );
    function!(
        "github::setup::enable-http-listener",
        "Turn on the http worker's restricted webhook listener (127.0.0.1, default port 3112) that quick-tunnel forwards GitHub deliveries to. Returns the updated checklist.",
        setup::EnableListenerRequest,
        setup::SetupStatus,
        enable_http_listener
    );
    function!(
        "github::setup::enable-webhooks",
        "Set webhooks.enabled. Enabling is refused until every prerequisite is verified; the github worker must restart to apply (restart_required).",
        setup::EnableWebhooksRequest,
        setup::SetupStatus,
        enable_webhooks
    );
    let s = service.clone();
    iii.register_function("github::pr::unwatch", RegisterFunction::new_async(move |EngineRequest(req): EngineRequest<WatchId>| {
        let s = s.clone(); async move { storage_task(async move { s.unwatch(&req.watch_id).await }).await.map_err(iii_sdk::Error::from) }
    }).description("Stop a watch and clean up only installation-owned resources; failures remain cleanup_pending."));
    let s = service.clone();
    iii.register_function(
        "github::pr::watch-status",
        RegisterFunction::new_async(move |EngineRequest(req): EngineRequest<WatchId>| {
            let s = s.clone();
            async move {
                storage_task(async move { s.status(&req.watch_id) })
                    .await
                    .map_err(iii_sdk::Error::from)
            }
        })
        .description("Read persisted snapshot and health without secrets or polling GitHub."),
    );
    let s = service.clone();
    iii.register_function("github::pr::event-detail", RegisterFunction::new_async(move |EngineRequest(req): EngineRequest<notifications::EventDetailRequest>| {
        let s = s.clone(); async move { storage_task(async move { s.event_detail(req) }).await.map_err(iii_sdk::Error::from) }
    }).description("Read a retained full webhook event by id without polling GitHub; seven-day bounded retention."));
    let s = service.clone();
    iii.register_function("github::pr::recover", RegisterFunction::new_async(move |EngineRequest(req): EngineRequest<RecoverRequest>| {
        let s = s.clone(); async move { storage_task(async move { s.recover(req.repo.as_deref()).await }).await.map_err(iii_sdk::Error::from) }
    }).description("Manually reconcile watches and request bounded redelivery of failed owned-hook deliveries."));
    let s = service.clone();
    iii.register_function("github::webhooks::receive", RegisterFunction::new_async(move |req: ReceiveRequest| {
        let s = s.clone(); async move {
            let response = s.receive(req).await;
            if response.status_code == 202 {
                let drain = s.clone();
                tokio::spawn(async move { if let Err(e) = storage_task(async move { drain.publish_pending().await }).await { tracing::error!(error = %e, "outbox publication failed; durable jobs retained"); } });
            }
            Ok::<_, iii_sdk::Error>(response)
        }
    }).description("Internal raw-stream authenticated GitHub webhook ingress; public HTTP route only."));
    let s = service.clone();
    iii.register_function(
        "github::webhooks::process",
        RegisterFunction::new_async(move |req: JobRequest| {
            let s = s.clone();
            async move {
                storage_task(async move { s.process(&req.job_id).await })
                    .await
                    .map_err(iii_sdk::Error::from)
            }
        })
        .description("Internal durable job consumer; commit effects before queue ack."),
    );
    let s = service.clone();
    iii.register_function(
        "github::webhooks::tunnel-changed",
        RegisterFunction::new_async(move |req: TunnelSnapshot| {
            let s = s.clone();
            async move {
                let task_service = s.clone();
                let response =
                    storage_task(async move { task_service.tunnel_changed(req).await }).await?;
                // The durable acceptance precedes return. Queue failure/crash is
                // recovered by maintenance/startup, not a lossy detached task.
                tokio::spawn(async move {
                    if let Err(e) = storage_task(async move { s.publish_pending().await }).await {
                        tracing::error!(error = %e, "tunnel lifecycle job retained for retry");
                    }
                });
                Ok::<_, iii_sdk::Error>(response)
            }
        })
        .description(
            "Internal tunnel change acceptance; queues authoritative lifecycle reconciliation.",
        ),
    );
    let s = service.clone();
    iii.register_function(
        "github::webhooks::maintain",
        RegisterFunction::new_async(move |_req: MaintenanceRequest| {
            let s = s.clone();
            async move {
                storage_task(async move {
                    let response = s.maintain().await?;
                    if let Err(e) = s.reconcile_subscribers().await {
                        tracing::warn!(error = %e, "subscriber reconciliation skipped");
                    }
                    Ok(response)
                })
                .await
                .map_err(iii_sdk::Error::from)
            }
        })
        .description("Internal bounded cleanup, expiry and outbox recovery; never polls PRs."),
    );
    if service.store.is_some() {
        activate(&service);
        tracing::info!(
            "webhook observers submitted; activation failures are logged and recorded in health"
        );
    }
}
#[derive(Deserialize, JsonSchema)]
struct MaintenanceRequest {
    #[serde(flatten)]
    _fields: HashMap<String, Value>,
}
pub(super) fn validate_config(config: &WebhookConfig) -> Result<()> {
    config.notifications.validate()?;
    if config.storage_path.trim().is_empty()
        || config.max_body_bytes == 0
        || config.max_pending == 0
        || !super::types::valid_watch_days(config.max_watch_days)
        || config.orphan_grace_minutes > super::types::MAX_ORPHAN_GRACE_MINUTES
        || config.queue.trim().is_empty()
        || config.queue.chars().any(char::is_control)
        || config.tunnel_id.is_empty()
        || config.tunnel_id.len() > 128
        || !config
            .tunnel_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
    {
        return Err(Failure::Invalid(
            "invalid webhook storage_path, queue, tunnel_id or zero capacity".into(),
        ));
    }
    Ok(())
}
fn activate(s: &Arc<Service>) {
    // No private server. Only this route opts into the sibling HTTP worker's
    // separate public listener; all other functions remain on the private bus.
    let queue_observer = queue_observer_id(
        s.iii.namespace().as_deref(),
        &crate::worker_name(),
        &s.config.queue,
    );
    for (source, target, options) in observed_sources(&s.config, queue_observer) {
        tokio::spawn(keep_observed(s.clone(), source, target, options));
    }
    let service = s.clone();
    tokio::spawn(async move {
        let task_service = service.clone();
        if let Err(e) = storage_task(async move { task_service.recover(None).await }).await {
            let error = e.to_string();
            if let Err(storage_error) = storage_task(async move {
                service.store()?.change(|d| {
                    d.last_error = Some(error);
                    Ok(())
                })
            })
            .await
            {
                tracing::error!(error = %storage_error, "cannot persist startup recovery failure");
            }
            tracing::error!(error = %e, "webhook startup recovery failed; durable work retained");
        }
    });
}

/// The sources the webhook plumbing observes, with their targets and
/// observer options: the queue consumer observes under `queue_observer`
/// ([`queue_observer_id`]).
pub(super) fn observed_sources(
    config: &WebhookConfig,
    queue_observer: String,
) -> [(Source, &'static str, ObserverOptions); 4] {
    [
        (
            Source::function("http::on-request").args(json!({
                "method": "POST",
                "path": "/webhooks/github/:endpoint_id",
                "public_webhook": true,
            })),
            "github::webhooks::receive",
            ObserverOptions::default(),
        ),
        (
            Source::function("queue::on-message").args(json!({
                "topic": config.queue,
                "group": WEBHOOK_CONSUMER_GROUP,
                "max_retries": 5,
                "backoff_ms": 1000,
            })),
            "github::webhooks::process",
            ObserverOptions::default().id(queue_observer),
        ),
        (
            Source::function("quick-tunnel::on-changed")
                .filters(json!({ "input.tunnel_id": { "$eq": config.tunnel_id } }))
                .deliver(Deliver::Output),
            "github::webhooks::tunnel-changed",
            ObserverOptions::default(),
        ),
        (
            Source::function("cron::on-tick")
                .args(json!({ "expression": "*/10 * * * * *" }))
                .deliver(Deliver::Output),
            "github::webhooks::maintain",
            ObserverOptions::default(),
        ),
    ]
}

/// Observe `source` with `target` until the observer is active, or refused
/// for a reason another try cannot change.
///
/// An observer stays `pending` while its source's worker is not running and
/// becomes active once it registers; one still pending after
/// [`ACTIVATION_WAIT_SECS`] is logged. A refusal is recorded as the webhook
/// health's `last_error`. A refusal in [`HELD_BY_ANOTHER_OBSERVER`] (the
/// route or the observer id still held by another observer) observes again
/// with the same `options`, waiting [`REOBSERVE_FIRST_SECS`] and then twice
/// as long each time up to [`REOBSERVE_MAX_SECS`]; once the observer is
/// active, the error it recorded is cleared.
///
/// A refused observer's handle holds the observer's id until `dispose` is
/// called, so every refused observer is disposed before the wait. An
/// observer its function's host refused (`HTTP_ROUTE_CONFLICT`) stays with
/// the engine, which confirms its disposal at once. The engine's own refusal
/// ([`ID_HELD_BY_ANOTHER_CONNECTION`]) is the observe's final answer: the
/// engine keeps nothing for it, its `dispose` sends nothing and resolves at
/// once, and the next observe uses the same id.
async fn keep_observed(
    s: Arc<Service>,
    source: Source,
    target: &'static str,
    mut options: ObserverOptions,
) {
    let mut retry = std::time::Duration::from_secs(REOBSERVE_FIRST_SECS);
    let mut recorded: Option<String> = None;
    loop {
        let handle = s.iii.observe_with(source.clone(), target, options.clone());
        let wait = std::time::Duration::from_secs(ACTIVATION_WAIT_SECS);
        let outcome = match tokio::time::timeout(wait, handle.ready()).await {
            Ok(outcome) => outcome,
            Err(_) => {
                tracing::warn!(
                    observer = handle.id(),
                    function = source.function_id(),
                    state = %handle.state(),
                    "webhook observer not active yet; its source's worker may not be running"
                );
                handle.ready().await
            }
        };
        let error = match outcome {
            Ok(()) => {
                if let Some(error) = recorded {
                    tracing::info!(
                        observer = handle.id(),
                        function = source.function_id(),
                        "webhook observer active after a refusal"
                    );
                    record_activation_error(&s, Some(error), None).await;
                }
                return;
            }
            Err(e) => e,
        };
        let code = handle
            .reason()
            .map(|reason| reason.code)
            .unwrap_or_default();
        let held = HELD_BY_ANOTHER_OBSERVER.contains(&code.as_str());
        if recorded.is_none() {
            tracing::error!(
                observer = handle.id(),
                function = source.function_id(),
                error = %error,
                retry = held,
                "webhook observer refused"
            );
            let message = format!("webhook observer {} refused: {error}", handle.id());
            record_activation_error(&s, None, Some(message.clone())).await;
            recorded = Some(message);
        } else {
            tracing::debug!(observer = handle.id(), error = %error, "webhook observer refused again");
        }
        if !held {
            return;
        }
        if code == ID_HELD_BY_ANOTHER_CONNECTION {
            options = options.id(handle.id());
        }
        let _ = handle.dispose().await;
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(std::time::Duration::from_secs(REOBSERVE_MAX_SECS));
    }
}

/// Set the webhook health's `last_error` to `error`, when it still is
/// `expected` (`None`: whatever it is).
async fn record_activation_error(
    s: &Arc<Service>,
    expected: Option<String>,
    error: Option<String>,
) {
    let s = s.clone();
    if let Err(storage_error) = storage_task(async move {
        s.store()?.change(|d| {
            if expected.is_none() || d.last_error == expected {
                d.last_error = error;
            }
            Ok(())
        })
    })
    .await
    {
        tracing::error!(error = %storage_error, "cannot persist webhook observer activation");
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;

    #[test]
    fn the_queue_observer_id_is_the_worker_and_its_queue() {
        assert_eq!(
            queue_observer_id(None, "github", "github-webhooks"),
            "github:webhooks:github-webhooks"
        );
        assert_eq!(
            queue_observer_id(Some("ops"), "github", "github-webhooks"),
            "ops/github:webhooks:github-webhooks"
        );
        // The queue observer observes under it.
        let sources = observed_sources(&WebhookConfig::default(), "fixed".into());
        let queue = sources
            .iter()
            .find(|(source, _, _)| source.function_id() == "queue::on-message")
            .unwrap();
        assert_eq!(queue.2, ObserverOptions::default().id("fixed"));
        assert!(sources
            .iter()
            .filter(|(source, _, _)| source.function_id() != "queue::on-message")
            .all(|(_, _, options)| *options == ObserverOptions::default()));
    }

    #[test]
    fn a_long_queue_observer_id_keeps_a_digest_within_the_engine_limit() {
        let queue = "q".repeat(200);
        let id = queue_observer_id(Some("ops"), "github", &queue);
        assert!(id.len() <= MAX_OBSERVER_ID_LEN, "{id}");
        assert!(id.starts_with("github:webhooks:sha256:"), "{id}");
        // Stable, and apart for another queue or namespace.
        assert_eq!(id, queue_observer_id(Some("ops"), "github", &queue));
        assert_ne!(id, queue_observer_id(Some("dev"), "github", &queue));
        assert_ne!(
            id,
            queue_observer_id(Some("ops"), "github", &"r".repeat(200))
        );
    }

    #[test]
    fn engine_metadata_keeps_strict_requests_and_schemas() {
        fn check<T: serde::de::DeserializeOwned + JsonSchema>(payload: Value) {
            let expected = serde_json::to_value(schemars::schema_for!(T)).unwrap();
            let actual = serde_json::to_value(schemars::schema_for!(EngineRequest<T>)).unwrap();
            assert_eq!(
                actual, expected,
                "transport must not change the public schema"
            );
            serde_json::from_value::<EngineRequest<T>>(payload.clone()).unwrap();
            let mut routed = payload;
            routed["_caller_worker_id"] = json!("harness");
            serde_json::from_value::<EngineRequest<T>>(routed.clone()).unwrap();
            routed["unexpected"] = json!(true);
            assert!(serde_json::from_value::<EngineRequest<T>>(routed).is_err());
        }
        check::<WatchRequest>(json!({"watch_id":"test", "repo":"owner/repo", "number":1,
            "expires_at":"2030-01-01T00:00:00Z"}));
        check::<WatchId>(json!({"watch_id":"test"}));
        check::<RecoverRequest>(json!({}));
    }
}
