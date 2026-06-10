//! The `router::provider::register` iii function (spec § register,
//! § Registration lifecycle): validate → token-gated upsert → entry-schema
//! re-compose (under the entry write lock) → static models reconcile → emits.
//!
//! Engine-backed coverage: tests/integration.rs (declare, token gate,
//! takeover rejection, schema composition).
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{ProviderDeclaration, ProviderRegisterResponse};
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::catalog::store::CatalogStore;
use crate::config::entry::{register_entry, EntryWriteLock};
use crate::config::schema::{default_provider_schema, validate_custom_schema};
use crate::registry::store::RegistryStore;
use crate::triggers::TriggerEmitter;

#[derive(Deserialize)]
struct RegisterInput {
    #[serde(flatten)]
    declaration: ProviderDeclaration,
    token: Option<String>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn make_provider_register(
    iii: III,
    registry: Arc<RegistryStore>,
    catalog: Arc<CatalogStore>,
    emitter: TriggerEmitter,
    entry_lock: EntryWriteLock,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let (iii, registry, catalog, emitter, entry_lock) = (
            iii.clone(),
            registry.clone(),
            catalog.clone(),
            emitter.clone(),
            entry_lock.clone(),
        );
        Box::pin(async move {
            let input: RegisterInput = serde_json::from_value(raw).map_err(|e| {
                IIIError::from(RouterError::new(RouterCode::InvalidRequest, e.to_string()))
            })?;
            let declaration = input.declaration;

            if !valid_id(&declaration.id) {
                return Err(RouterError::new(
                    RouterCode::InvalidRequest,
                    format!("invalid provider id: {}", declaration.id),
                )
                .into());
            }
            if let Some(schema) = &declaration.config_schema {
                validate_custom_schema(schema).map_err(IIIError::from)?;
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
            let (_, token) = registry
                .upsert(declaration, worker_id, input.token)
                .await
                .map_err(IIIError::from)?;

            // Re-compose the entry schema from every registered declaration —
            // under the entry write lock so concurrent boots compose.
            {
                let _guard = entry_lock.lock().await;
                let mut provider_schemas = BTreeMap::new();
                for rec in registry.list().await {
                    let schema = rec.declaration.config_schema.clone().unwrap_or_else(|| {
                        default_provider_schema(
                            &serde_json::to_value(rec.declaration.defaults.clone())
                                .unwrap_or(Value::Null),
                        )
                    });
                    provider_schemas.insert(rec.declaration.id.clone(), schema);
                }
                register_entry(&iii, &provider_schemas).await?;
            }

            emitter
                .emit(
                    "router::provider::changed",
                    json!({ "provider": id, "op": "register" }),
                )
                .await;

            // Static catalog slice: reconciled at registration (spec § register).
            if let Some(models) = static_models {
                if !models.is_empty() {
                    let count = models.len();
                    catalog.set_slice(&id, models).await?;
                    emitter
                        .emit(
                            "router::models::changed",
                            json!({ "provider": id, "count": count }),
                        )
                        .await;
                }
            }

            Ok(serde_json::to_value(ProviderRegisterResponse {
                ok: true,
                id,
                registration_token: token,
            })
            .expect("serializable response"))
        })
    }
}
