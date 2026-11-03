use std::sync::Arc;

use iii_sdk::{CallAction, CallRequest, Error, IIIClient, RegisterFunction};
use tokio::{sync::broadcast, task::JoinHandle};

use crate::{api::*, manager::Manager};

// Accept engine transport metadata without weakening the strict public API.
#[derive(schemars::JsonSchema)]
#[schemars(transparent)]
struct EngineRequest<T>(T);

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for EngineRequest<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut payload = serde_json::Value::deserialize(deserializer)?;
        if let Some(fields) = payload.as_object_mut() {
            fields.remove("_caller_worker_id");
        }
        T::deserialize(payload)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

pub const ACQUIRE: &str = "quick-tunnel::acquire";
pub const RELEASE: &str = "quick-tunnel::release";
pub const STATUS: &str = "quick-tunnel::status";
/// Observable: each call is one tunnel lifecycle snapshot.
pub const ON_CHANGED_FN: &str = "quick-tunnel::on-changed";

/// Calls [`ON_CHANGED_FN`] with `snapshot` when something observes it.
/// Fire-and-forget: iii delivers the call to the observers whose filters
/// match, and a slow observer never holds the lease task.
pub async fn emit(iii: &IIIClient, snapshot: &Snapshot) {
    if !iii.is_observed(ON_CHANGED_FN) {
        return;
    }
    let payload = match serde_json::to_value(snapshot) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(%error, "changed serialization failed");
            return;
        }
    };
    let request = CallRequest {
        function_id: ON_CHANGED_FN.into(),
        payload,
        action: Some(CallAction::Void),
        timeout_ms: None,
    };
    if let Err(error) = iii.call(request).await {
        tracing::warn!(tunnel_id = %snapshot.tunnel_id, %error, "changed call failed; observers reconcile with quick-tunnel::status");
    }
}

/// Registers only local SDK handlers; never invokes cloudflared during registration.
pub fn register(iii: Arc<IIIClient>, manager: Manager, targets: Vec<String>) -> JoinHandle<()> {
    let mut events = manager.subscribe();
    iii.register_function(
        ON_CHANGED_FN,
        RegisterFunction::new(|snapshot: Snapshot| Ok::<_, Error>(snapshot))
            .description("Tunnel lifecycle snapshot changed. Observe it before acquire, filtered on input.tunnel_id, then read status to close races")
            .private(),
    );
    let acquire = manager.clone();
    iii.register_function(ACQUIRE, RegisterFunction::new_async(move |EngineRequest(request): EngineRequest<AcquireRequest>| {
        let manager = acquire.clone();
        async move { manager.acquire(request).await.map_err(|e| Error::Handler(e.to_string())) }
    }).description("Acquire or extend an expiring durable lease for an operator-authorized local target; returns immediately, possibly starting"));
    let release = manager.clone();
    iii.register_function(
        RELEASE,
        RegisterFunction::new_async(
            move |EngineRequest(request): EngineRequest<ReleaseRequest>| {
                let manager = release.clone();
                async move {
                    manager
                        .release(request)
                        .await
                        .map_err(|e| Error::Handler(e.to_string()))
                }
            },
        )
        .description(
            "Release one lease idempotently; the last lease stops and reaps the tunnel child",
        ),
    );
    let status = manager.clone();
    iii.register_function(
        STATUS,
        RegisterFunction::new_async(
            move |EngineRequest(request): EngineRequest<StatusRequest>| {
                let manager = status.clone();
                async move {
                    manager
                        .status(request)
                        .await
                        .map_err(|e| Error::Handler(e.to_string()))
                }
            },
        )
        .description(
            "Read the latest tunnel generation, connection state, public URL and non-secret leases",
        ),
    );
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => emit(&iii, &event).await,
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!(count, "changed receiver lagged; reconciling snapshots");
                    for id in &targets {
                        if let Ok(status) = manager
                            .status(StatusRequest {
                                tunnel_id: id.clone(),
                            })
                            .await
                        {
                            emit(&iii, &status.snapshot).await;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}
