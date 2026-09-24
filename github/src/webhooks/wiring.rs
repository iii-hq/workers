use async_trait::async_trait;
use iii_sdk::{
    protocol::RegisterTriggerInput,
    trigger::{TriggerConfig, TriggerHandler},
    RegisterFunction, RegisterTriggerType,
};

use super::*;
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

struct EventHandler(Arc<Service>);
#[async_trait]
impl TriggerHandler for EventHandler {
    async fn register_trigger(
        &self,
        config: TriggerConfig,
    ) -> std::result::Result<(), iii_sdk::Error> {
        let filter: EventFilter = serde_json::from_value(config.config)
            .map_err(|e| iii_sdk::Error::Handler(e.to_string()))?;
        if let Some(repo) = &filter.repo {
            validate_repo(repo)?;
        }
        if let Some(policy) = &filter.notifications {
            policy.validate()?;
        }
        let service = self.0.clone();
        storage_task(async move {
            if let Some(store) = &service.store {
                store.change(|d| {
                    d.subscribers.insert(
                        config.id.clone(),
                        Subscriber {
                            id: config.id,
                            function_id: config.function_id,
                            filter,
                            metadata: config.metadata,
                            namespace: config.namespace,
                        },
                    );
                    Ok(())
                })?;
            }
            Ok(())
        })
        .await
        .map_err(iii_sdk::Error::from)
    }
    async fn unregister_trigger(
        &self,
        config: TriggerConfig,
    ) -> std::result::Result<(), iii_sdk::Error> {
        let service = self.0.clone();
        storage_task(async move {
            if let Some(store) = &service.store {
                store.change(|d| {
                    d.subscribers.remove(&config.id);
                    // Explicit cancellation means no further callback to this binding.
                    d.jobs.retain(
                        |_, j| !matches!(j, Job::Notify { target, .. } | Job::ReviewNotify { target, .. } if target.id == config.id),
                    );
                    Ok(())
                })?;
            }
            Ok(())
        })
        .await
        .map_err(iii_sdk::Error::from)
    }
}

/// Registration has no external prerequisite. Disabled defaults never open the
/// database, probe gh, acquire a tunnel, register HTTP or publish to a queue.
pub async fn register(iii: &Arc<IIIClient>, cell: &ConfigCell, engine_url: &str) {
    let config = cell.read().await.webhooks.clone();
    let store = if config.enabled {
        let storage_config = config.clone();
        match storage_task(async move {
            validate_config(&storage_config)?;
            Store::open(Path::new(&storage_config.storage_path))
        })
        .await
        {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::error!(error = %e, "webhook storage unavailable; mutations disabled");
                None
            }
        }
    } else {
        None
    };
    let service = Arc::new(Service {
        iii: iii.clone(),
        cell: cell.clone(),
        config,
        engine_url: engine_url.into(),
        store,
        operations: Mutex::new(()),
        #[cfg(test)]
        bus: None,
    });
    iii.register_trigger_type(RegisterTriggerType::new("github::pr::event", "Durable PR lifecycle, comment, review and per-entity CI events. Arm before watch and read one snapshot after watch.", EventHandler(service.clone())).trigger_request_format::<EventFilter>().call_request_format::<notifications::NotificationEvent>());
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
    function!(
        "github::pr::watch",
        "Watch a PR via authenticated webhooks until merged/closed; expires_at must be in the future within 30 days.",
        WatchRequest,
        WatchResponse,
        watch
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
                storage_task(async move { s.maintain().await })
                    .await
                    .map_err(iii_sdk::Error::from)
            }
        })
        .description("Internal bounded cleanup, expiry and outbox recovery; never polls PRs."),
    );
    if service.store.is_some() {
        match activate(&service).await {
            Ok(()) => tracing::info!("webhook bindings submitted; SDK reports asynchronous registration failures in logs"),
            Err(e) => {
                let task_service = service.clone();
                let error = e.to_string();
                if let Err(storage_error) = storage_task(async move { task_service.store()?.change(|d| { d.last_error = Some(error); Ok(()) }) }).await {
                    tracing::error!(error = %storage_error, "cannot persist activation failure");
                }
                tracing::error!(error = %e, "webhook activation/recovery failed; inspect watch health and recover")
            }
        }
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
async fn activate(s: &Arc<Service>) -> Result<()> {
    // No private server. Only this route opts into the sibling HTTP worker's
    // separate public listener; all other functions remain on the private bus.
    for (kind, function, config) in [
        (
            "http",
            "github::webhooks::receive",
            json!({"api_path":"/webhooks/github/:endpoint_id","http_method":"POST","public_webhook":true}),
        ),
        (
            "durable:subscriber",
            "github::webhooks::process",
            json!({"queue":s.config.queue,"max_retries":5,"backoff_ms":1000}),
        ),
        (
            "quick-tunnel::changed",
            "github::webhooks::tunnel-changed",
            json!({"tunnel_id":s.config.tunnel_id}),
        ),
        (
            "cron",
            "github::webhooks::maintain",
            json!({"expression":"*/10 * * * * *"}),
        ),
    ] {
        s.iii
            .register_trigger(RegisterTriggerInput::new(kind, function, config))?;
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
    Ok(())
}

#[cfg(test)]
mod metadata_tests {
    use super::*;

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
