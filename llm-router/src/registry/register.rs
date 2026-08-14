//! The `router::provider::register` iii function (spec § register,
//! § Registration lifecycle): validate → token-gated upsert → entry-schema
//! re-compose (under the entry write lock) → static models reconcile → emits.
//!
//! Engine-backed coverage: tests/integration.rs (declare, token gate,
//! takeover rejection, schema composition).
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{
    ProviderRegisterRequest, ProviderRegisterResponse, ProviderUnregisterRequest,
    ProviderUnregisterResponse,
};
use futures::future::BoxFuture;
use iii_sdk::{errors::Error, IIIClient};
use serde_json::{json, Value};

use crate::catalog::store::CatalogStore;
use crate::config::entry::{register_entry, EntryWriteLock};
use crate::config::schema::{provider_entry_schema, validate_custom_schema};
use crate::registry::store::RegistryStore;
use crate::triggers::{self, RouterEvents};

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn make_provider_register(
    iii: IIIClient,
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
    entry_lock: EntryWriteLock,
    events: Arc<RouterEvents>,
) -> impl Fn(ProviderRegisterRequest) -> BoxFuture<'static, Result<ProviderRegisterResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |input: ProviderRegisterRequest| {
        let (iii, registry, catalog, entry_lock, events) = (
            iii.clone(),
            registry.clone(),
            catalog.clone(),
            entry_lock.clone(),
            events.clone(),
        );
        Box::pin(async move {
            let declaration = input.declaration;

            if !valid_id(&declaration.id) {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("invalid provider id: {}", declaration.id),
                )
                .into());
            }
            if let Some(schema) = &declaration.config_schema {
                validate_custom_schema(schema).map_err(Error::from)?;
            }
            if let Some(models) = &declaration.models {
                for m in models {
                    if m.provider != declaration.id {
                        return Err(RouterError::new(
                            RouterCode::InvalidRequest,
                            format!(
                                "static model {} declares provider {}, expected {}",
                                m.id, m.provider, declaration.id
                            ),
                        )
                        .into());
                    }
                }
            }

            let worker_id = declaration.worker_id.clone();
            let static_models = declaration.models.clone();
            let id = declaration.id.clone();
            let upserted = registry
                .upsert(declaration, worker_id, input.token)
                .await
                .map_err(Error::from)?;
            let token = upserted.token;
            let availability_recovered = upserted.availability_recovered;

            // Re-compose the entry schema from every registered declaration —
            // under the entry write lock so concurrent boots compose.
            {
                let _guard = entry_lock.lock().await;
                let mut provider_schemas = BTreeMap::new();
                for rec in registry.list().await {
                    let schema = provider_entry_schema(
                        rec.declaration.config_schema.as_ref(),
                        &serde_json::to_value(rec.declaration.defaults.clone())
                            .unwrap_or(Value::Null),
                        rec.declaration.system_prompt.as_deref(),
                    );
                    provider_schemas.insert(rec.declaration.id.clone(), schema);
                }
                register_entry(&iii, &provider_schemas).await?;
            }

            events
                .emit(
                    triggers::PROVIDER_CHANGED,
                    json!({ "provider": id, "op": "register" }),
                )
                .await;

            // A down provider coming back up via re-registration is an
            // availability transition; emit it explicitly so subscribers tracking
            // op:"available"/"unavailable" don't stay stuck on the prior down state.
            if availability_recovered {
                events
                    .emit(
                        triggers::PROVIDER_CHANGED,
                        json!({ "provider": id, "op": "available" }),
                    )
                    .await;
            }

            // Static catalog slice: reconciled at registration (spec § register).
            if let Some(models) = static_models {
                if !models.is_empty() {
                    let count = models.len();
                    catalog.set_slice(&id, models).await?;
                    events
                        .emit(
                            triggers::MODELS_CHANGED,
                            json!({ "provider": id, "count": count }),
                        )
                        .await;
                }
            }

            Ok(ProviderRegisterResponse {
                ok: true,
                id,
                registration_token: token,
            })
        })
    }
}

/// The `router::provider::unregister` iii function — operator escape hatch
/// for a token lock-out: without it, a provider whose state diverged from the
/// router's persisted registry ("bound to another worker") could never come
/// back. Drops the record and its catalog slice, re-composes the entry schema,
/// and emits provider/model change events so open consoles converge.
pub fn make_provider_unregister(
    iii: IIIClient,
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
    entry_lock: EntryWriteLock,
    events: Arc<RouterEvents>,
) -> impl Fn(
    ProviderUnregisterRequest,
) -> BoxFuture<'static, Result<ProviderUnregisterResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |input: ProviderUnregisterRequest| {
        let (iii, registry, catalog, entry_lock, events) = (
            iii.clone(),
            registry.clone(),
            catalog.clone(),
            entry_lock.clone(),
            events.clone(),
        );
        Box::pin(async move {
            let id = input.id;
            if !valid_id(&id) {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("invalid provider id: {id}"),
                )
                .into());
            }
            let removed = registry.remove(&id).await.map_err(|e| {
                Error::from(RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("registry persist failed: {e}"),
                ))
            })?;
            if !removed {
                return Ok(ProviderUnregisterResponse {
                    ok: true,
                    removed: false,
                });
            }
            catalog.remove_slice(&id).await?;

            {
                let _guard = entry_lock.lock().await;
                let mut provider_schemas = BTreeMap::new();
                for rec in registry.list().await {
                    let schema = provider_entry_schema(
                        rec.declaration.config_schema.as_ref(),
                        &serde_json::to_value(rec.declaration.defaults.clone())
                            .unwrap_or(Value::Null),
                        rec.declaration.system_prompt.as_deref(),
                    );
                    provider_schemas.insert(rec.declaration.id.clone(), schema);
                }
                register_entry(&iii, &provider_schemas).await?;
            }

            events
                .emit(
                    triggers::PROVIDER_CHANGED,
                    json!({ "provider": id, "op": "unregister" }),
                )
                .await;
            events
                .emit(
                    triggers::MODELS_CHANGED,
                    json!({ "provider": id, "count": 0 }),
                )
                .await;

            Ok(ProviderUnregisterResponse {
                ok: true,
                removed: true,
            })
        })
    }
}
