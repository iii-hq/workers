use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use iii_sdk::{
    protocol::{TriggerRequest, TriggerRequestWithMetadata},
    trigger::{TriggerConfig, TriggerHandler},
    Error, IIIClient, RegisterFunction, RegisterTriggerType,
};
use tokio::{
    sync::{broadcast, RwLock},
    task::JoinHandle,
};

use crate::{api::*, manager::Manager};

pub const ACQUIRE: &str = "quick-tunnel::acquire";
pub const RELEASE: &str = "quick-tunnel::release";
pub const STATUS: &str = "quick-tunnel::status";
pub const CHANGED: &str = "quick-tunnel::changed";

#[derive(Clone)]
pub struct Changes {
    targets: Arc<Vec<String>>,
    bindings: Arc<RwLock<BTreeMap<String, (ChangedConfig, TriggerConfig)>>>,
}

impl Changes {
    pub fn new(targets: Vec<String>) -> Self {
        Self {
            targets: Arc::new(targets),
            bindings: Arc::default(),
        }
    }

    async fn dispatch(&self, iii: &IIIClient, event: &Snapshot) {
        let bindings: Vec<_> = self
            .bindings
            .read()
            .await
            .values()
            .filter(|(filter, _)| {
                filter
                    .tunnel_id
                    .as_ref()
                    .is_none_or(|id| id == &event.tunnel_id)
            })
            .map(|(_, binding)| binding.clone())
            .collect();
        for binding in bindings {
            match routed_request(&binding, event) {
                Ok(request) => {
                    if let Err(error) = iii.trigger(request).await {
                        tracing::warn!(trigger_id = %binding.id, %error, "changed delivery failed; consumer must reconcile status");
                    }
                }
                Err(error) => tracing::error!(%error, "changed serialization failed"),
            }
        }
    }
}

/// Preserve target metadata and resolved namespace, including the legacy default.
pub fn routed_request(
    binding: &TriggerConfig,
    event: &Snapshot,
) -> Result<TriggerRequestWithMetadata, Error> {
    let mut request: TriggerRequestWithMetadata = TriggerRequest {
        function_id: binding.function_id.clone(),
        payload: serde_json::to_value(event)?,
        action: None,
        timeout_ms: Some(5_000),
    }
    .namespace(binding.namespace.as_deref().unwrap_or("default"));
    if let Some(metadata) = &binding.metadata {
        request = request.metadata(metadata.clone());
    }
    Ok(request)
}

#[async_trait]
impl TriggerHandler for Changes {
    async fn register_trigger(&self, binding: TriggerConfig) -> Result<(), Error> {
        let filter: ChangedConfig = serde_json::from_value(binding.config.clone())?;
        if filter
            .tunnel_id
            .as_ref()
            .is_some_and(|id| !self.targets.contains(id))
        {
            return Err(Error::Handler("unknown tunnel_id".into()));
        }
        if binding
            .namespace
            .as_deref()
            .is_some_and(|ns| ns.trim().is_empty())
        {
            return Err(Error::Handler("empty namespace".into()));
        }
        let mut bindings = self.bindings.write().await;
        if bindings.len() >= 1024 && !bindings.contains_key(&binding.id) {
            return Err(Error::Handler("subscription limit reached".into()));
        }
        bindings.insert(binding.id.clone(), (filter, binding));
        Ok(())
    }
    async fn unregister_trigger(&self, binding: TriggerConfig) -> Result<(), Error> {
        self.bindings.write().await.remove(&binding.id);
        Ok(())
    }
}

/// Registers only local SDK handlers; never invokes cloudflared during registration.
pub fn register(iii: Arc<IIIClient>, manager: Manager, targets: Vec<String>) -> JoinHandle<()> {
    let changes = Changes::new(targets.clone());
    let mut events = manager.subscribe();
    iii.register_trigger_type(RegisterTriggerType::new(CHANGED, "Tunnel lifecycle snapshot changed; subscribe before acquire, then read status to close races", changes.clone())
        .trigger_request_format::<ChangedConfig>().call_request_format::<Snapshot>());
    let acquire = manager.clone();
    iii.register_function(ACQUIRE, RegisterFunction::new_async(move |request: AcquireRequest| {
        let manager = acquire.clone();
        async move { manager.acquire(request).await.map_err(|e| Error::Handler(e.to_string())) }
    }).description("Acquire or extend an expiring durable lease for an operator-authorized local target; returns immediately, possibly starting"));
    let release = manager.clone();
    iii.register_function(
        RELEASE,
        RegisterFunction::new_async(move |request: ReleaseRequest| {
            let manager = release.clone();
            async move {
                manager
                    .release(request)
                    .await
                    .map_err(|e| Error::Handler(e.to_string()))
            }
        })
        .description(
            "Release one lease idempotently; the last lease stops and reaps the tunnel child",
        ),
    );
    let status = manager.clone();
    iii.register_function(
        STATUS,
        RegisterFunction::new_async(move |request: StatusRequest| {
            let manager = status.clone();
            async move {
                manager
                    .status(request)
                    .await
                    .map_err(|e| Error::Handler(e.to_string()))
            }
        })
        .description(
            "Read the latest tunnel generation, connection state, public URL and non-secret leases",
        ),
    );
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => changes.dispatch(&iii, &event).await,
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!(count, "changed receiver lagged; reconciling snapshots");
                    for id in &targets {
                        if let Ok(status) = manager
                            .status(StatusRequest {
                                tunnel_id: id.clone(),
                            })
                            .await
                        {
                            changes.dispatch(&iii, &status.snapshot).await;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}
