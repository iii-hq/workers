use async_trait::async_trait;
use iii_sdk::{
    protocol::RegisterTriggerInput,
    trigger::{TriggerConfig, TriggerHandler},
    RegisterFunction, RegisterTriggerType,
};

use super::*;

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
        if let Some(store) = &self.0.store {
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
    }
    async fn unregister_trigger(
        &self,
        config: TriggerConfig,
    ) -> std::result::Result<(), iii_sdk::Error> {
        if let Some(store) = &self.0.store {
            store.change(|d| {
                d.subscribers.remove(&config.id);
                // Explicit cancellation means no further callback to this binding.
                d.jobs.retain(
                    |_, j| !matches!(j, Job::Notify { target, .. } if target.id == config.id),
                );
                Ok(())
            })?;
        }
        Ok(())
    }
}

/// Registration has no external prerequisite. Disabled defaults never open the
/// database, probe gh, acquire a tunnel, register HTTP or publish to a queue.
pub async fn register(iii: &Arc<IIIClient>, cell: &ConfigCell, engine_url: &str) {
    let config = cell.read().await.webhooks.clone();
    let store = if config.enabled {
        match validate_config(&config).and_then(|()| Store::open(Path::new(&config.storage_path))) {
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
    iii.register_trigger_type(RegisterTriggerType::new("github::pr::event", "Durable PR lifecycle, comment, review and per-entity CI events. Arm before watch and read one snapshot after watch.", EventHandler(service.clone())).trigger_request_format::<EventFilter>().call_request_format::<PrEvent>());
    macro_rules! function {
        ($id:expr, $desc:expr, $req:ty, $resp:ty, $method:ident $(, $field:ident)?) => {{
            let service = service.clone();
            iii.register_function($id, RegisterFunction::new_async(move |req: $req| {
                let service = service.clone();
                async move {
                    let result: Result<$resp> = service.$method(req $(.$field)?).await;
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
    iii.register_function("github::pr::unwatch", RegisterFunction::new_async(move |req: WatchId| {
        let s = s.clone(); async move { s.unwatch(&req.watch_id).await.map_err(iii_sdk::Error::from) }
    }).description("Stop a watch and clean up only installation-owned resources; failures remain cleanup_pending."));
    let s = service.clone();
    iii.register_function(
        "github::pr::watch-status",
        RegisterFunction::new(move |req: WatchId| {
            s.status(&req.watch_id).map_err(iii_sdk::Error::from)
        })
        .description("Read persisted snapshot and health without secrets or polling GitHub."),
    );
    let s = service.clone();
    iii.register_function("github::pr::recover", RegisterFunction::new_async(move |req: RecoverRequest| {
        let s = s.clone(); async move { s.recover(req.repo.as_deref()).await.map_err(iii_sdk::Error::from) }
    }).description("Manually reconcile watches and request bounded redelivery of failed owned-hook deliveries."));
    let s = service.clone();
    iii.register_function("github::webhooks::receive", RegisterFunction::new_async(move |req: ReceiveRequest| {
        let s = s.clone(); async move {
            let response = s.receive(req).await;
            if response.status_code == 202 {
                let drain = s.clone();
                tokio::spawn(async move { if let Err(e) = drain.publish_pending().await { tracing::error!(error = %e, "outbox publication failed; durable jobs retained"); } });
            }
            Ok::<_, iii_sdk::Error>(response)
        }
    }).description("Internal raw-stream authenticated GitHub webhook ingress; public HTTP route only."));
    let s = service.clone();
    iii.register_function(
        "github::webhooks::process",
        RegisterFunction::new_async(move |req: JobRequest| {
            let s = s.clone();
            async move { s.process(&req.job_id).await.map_err(iii_sdk::Error::from) }
        })
        .description("Internal durable job consumer; commit effects before queue ack."),
    );
    let s = service.clone();
    iii.register_function(
        "github::webhooks::tunnel-changed",
        RegisterFunction::new_async(move |req: TunnelSnapshot| {
            let s = s.clone();
            async move {
                let response = s.tunnel_changed(req).await?;
                // The durable acceptance precedes return. Queue failure/crash is
                // recovered by maintenance/startup, not a lossy detached task.
                tokio::spawn(async move {
                    if let Err(e) = s.publish_pending().await {
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
            async move { s.maintain().await.map_err(iii_sdk::Error::from) }
        })
        .description("Internal bounded cleanup, expiry and outbox recovery; never polls PRs."),
    );
    if service.store.is_some() {
        match activate(&service).await {
            Ok(()) => tracing::info!("webhook bindings submitted; SDK reports asynchronous registration failures in logs"),
            Err(e) => {
                if let Err(storage_error) = service.record_failure(&e) {
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
            json!({"expression":"0 * * * * *"}),
        ),
    ] {
        s.iii
            .register_trigger(RegisterTriggerInput::new(kind, function, config))?;
    }
    let service = s.clone();
    tokio::spawn(async move {
        if let Err(e) = service.recover(None).await {
            if let Err(storage_error) = service.record_failure(&e) {
                tracing::error!(error = %storage_error, "cannot persist startup recovery failure");
            }
            tracing::error!(error = %e, "webhook startup recovery failed; durable work retained");
        }
    });
    Ok(())
}
