//! Token-gated catalog writes and optional latest-wins discovery completion.
use std::sync::Arc;

use crate::catalog::store::CatalogStore;
use crate::config::entry::EntryWriteLock;
use crate::config::state::{snapshot, ConfigCell};
use crate::registry::resolve::{
    discovery_fingerprint, ensure_provider_secret, resolve_provider_config,
};
use crate::registry::store::RegistryStore;
use crate::secrets::SecretCache;
use crate::triggers::{self, RouterEvents};
use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{
    DiscoveryOutcome, DiscoveryStatus, ModelsReconcileRequest, ModelsReconcileResponse,
};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use serde_json::json;

pub fn make_models_reconcile(
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
    events: Arc<RouterEvents>,
    config: ConfigCell,
    secrets: Arc<SecretCache>,
    entry_lock: EntryWriteLock,
) -> impl Fn(ModelsReconcileRequest) -> BoxFuture<'static, Result<ModelsReconcileResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |req: ModelsReconcileRequest| {
        let (registry, catalog, events, config, secrets, entry_lock) = (
            registry.clone(),
            catalog.clone(),
            events.clone(),
            config.clone(),
            secrets.clone(),
            entry_lock.clone(),
        );
        Box::pin(async move {
            let _guard = entry_lock.lock().await;
            let provider = req.provider;
            let record = registry
                .verify_token(&provider, req.token.as_deref())
                .await
                .map_err(Error::from)?;
            for model in &req.models {
                if model.provider != provider {
                    return Err(RouterError::new(
                        RouterCode::InvalidRequest,
                        "model provider mismatch",
                    )
                    .into());
                }
            }
            if let Some(report) = req.discovery {
                let config = snapshot(&config);
                ensure_provider_secret(&config, &provider, &secrets).await;
                let resolved = resolve_provider_config(&config, &record.declaration, &secrets);
                let fingerprint = discovery_fingerprint(&config, &resolved);
                if record.discovery_lease.as_ref() != Some(&(report.attempt, fingerprint)) {
                    return Err(RouterError::new(
                        RouterCode::InvalidRequest,
                        "discovery attempt superseded; retry",
                    )
                    .into());
                }
                if report
                    .http_status
                    .is_some_and(|status| !(100..=599).contains(&status))
                {
                    return Err(RouterError::new(
                        RouterCode::InvalidRequest,
                        "invalid discovery HTTP status",
                    )
                    .into());
                }
                let previous = catalog.slice(&provider).await;
                let preserve = report.outcome.preserves_catalog();
                // The router derives empty/success and stale; callers cannot mislabel them.
                let outcome = if report.outcome.is_success() {
                    if req.models.is_empty() {
                        DiscoveryOutcome::Empty
                    } else {
                        DiscoveryOutcome::Success
                    }
                } else {
                    report.outcome
                };
                let count = if preserve {
                    previous.len()
                } else {
                    req.models.len()
                };
                let status = DiscoveryStatus {
                    outcome,
                    http_status: report.http_status,
                    code: if outcome.is_success() {
                        None
                    } else {
                        report.code
                    },
                    stale: preserve && !outcome.blocks_catalog() && !previous.is_empty(),
                    checked_at_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0),
                };
                let prepared = if preserve {
                    None
                } else {
                    Some(catalog.prepare_slice(&provider, req.models).await?)
                };
                if let Err(error) = registry.complete_discovery(&provider, status).await {
                    if let Some(prepared) = prepared {
                        prepared.rollback().await?;
                    }
                    return Err(error);
                }
                if let Some(prepared) = prepared {
                    prepared.commit();
                }
                // Emitted even when the catalog/count is unchanged, including empty recovery.
                events
                    .emit(
                        triggers::PROVIDER_CHANGED,
                        json!({"provider": provider, "op": "discovery"}),
                    )
                    .await;
                events
                    .emit(
                        triggers::MODELS_CHANGED,
                        json!({"provider": provider, "count": count}),
                    )
                    .await;
                return Ok(ModelsReconcileResponse { provider, count });
            }
            // Legacy providers keep their exact replacement semantics.
            let count = req.models.len();
            catalog.set_slice(&provider, req.models).await?;
            events
                .emit(
                    triggers::MODELS_CHANGED,
                    json!({ "provider": provider, "count": count }),
                )
                .await;
            Ok(ModelsReconcileResponse { provider, count })
        })
    }
}
