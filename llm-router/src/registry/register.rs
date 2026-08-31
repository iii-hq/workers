//! The `router::provider::register` iii function (spec § register,
//! § Registration lifecycle): validate → token-gated prepare → entry-schema
//! re-compose → durable static models → registry commit → publish + emits.
//! The entry write lock spans that transaction so concurrent provider boots
//! cannot invalidate one another's staged schemas or tokens.
//!
//! Engine-backed coverage: tests/integration.rs (declare, token gate,
//! takeover rejection, schema composition).
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::router::{ProviderRegisterRequest, ProviderRegisterResponse};
use futures::future::BoxFuture;
use iii_sdk::{errors::Error, IIIClient};
use serde_json::{json, Value};

use crate::catalog::store::CatalogStore;
use crate::config::entry::{register_entry, EntryWriteLock};
use crate::config::schema::{provider_entry_schema, validate_custom_schema};
use crate::registry::store::{ProviderRecord, RegistryStore};
use crate::triggers::{self, RouterEvents};

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn schemas_for(records: &[ProviderRecord]) -> BTreeMap<String, Value> {
    records
        .iter()
        .map(|rec| {
            let schema = provider_entry_schema(
                rec.declaration.config_schema.as_ref(),
                &serde_json::to_value(rec.declaration.defaults.clone()).unwrap_or(Value::Null),
            );
            (rec.declaration.id.clone(), schema)
        })
        .collect()
}

fn failure_with_rollbacks(original: Error, failures: Vec<String>) -> Error {
    if failures.is_empty() {
        original
    } else {
        Error::Handler(format!(
            "{original}; registration rollback incomplete: {}",
            failures.join("; ")
        ))
    }
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
            // This lock is the provider-registration transaction boundary. It
            // also serializes against configuration writes/reloads.
            let entry_guard = entry_lock.lock().await;
            let prepared = registry
                .prepare_upsert(declaration, worker_id, input.token)
                .await
                .map_err(Error::from)?;
            let current_records = registry.list().await;
            let previous_schemas = schemas_for(&current_records);
            let mut candidate_records = current_records;
            candidate_records.retain(|record| record.declaration.id != id);
            candidate_records.push(prepared.record().clone());
            let candidate_schemas = schemas_for(&candidate_records);

            // Configuration is the first fallible external step. If the call
            // reports failure after a partial remote apply, restore the prior
            // schema while the transaction lock is still held.
            if let Err(error) = register_entry(&iii, &candidate_schemas).await {
                let mut rollback_failures = Vec::new();
                if let Err(rollback) = register_entry(&iii, &previous_schemas).await {
                    rollback_failures.push(format!("configuration: {rollback}"));
                }
                drop(entry_guard);
                return Err(failure_with_rollbacks(error, rollback_failures));
            }

            // Persist static models without exposing them in memory. A failed
            // catalog write leaves the registry untouched, so a fresh token
            // can be minted again on retry.
            let mut prepared_catalog = if let Some(models) = static_models {
                if models.is_empty() {
                    None
                } else {
                    match catalog.prepare_slice(&id, models).await {
                        Ok(prepared) => Some(prepared),
                        Err(error) => {
                            let mut rollback_failures = Vec::new();
                            if let Err(rollback) = register_entry(&iii, &previous_schemas).await {
                                rollback_failures.push(format!("configuration: {rollback}"));
                            }
                            drop(entry_guard);
                            return Err(failure_with_rollbacks(error, rollback_failures));
                        }
                    }
                }
            } else {
                None
            };

            // The token hash becomes durable only after configuration and the
            // optional catalog slice succeeded. If registry persistence fails,
            // keep the staged catalog invisible and compensate both durable
            // side effects before returning the original failure.
            let upserted = match registry.commit_upsert(prepared).await {
                Ok(upserted) => upserted,
                Err(error) => {
                    let mut rollback_failures = Vec::new();
                    if let Some(prepared) = prepared_catalog.take() {
                        if let Err(rollback) = prepared.rollback().await {
                            // No registry record was published, so routing
                            // excludes this owner even if its durable catalog
                            // rollback also fails. A restart may temporarily
                            // reload that orphan until the next reconcile.
                            rollback_failures.push(format!("catalog: {rollback}"));
                        }
                    }
                    if let Err(rollback) = register_entry(&iii, &previous_schemas).await {
                        rollback_failures.push(format!("configuration: {rollback}"));
                    }
                    drop(entry_guard);
                    return Err(failure_with_rollbacks(error.into(), rollback_failures));
                }
            };
            if let Some(prepared) = prepared_catalog {
                prepared.commit();
            }
            drop(entry_guard);

            let token = upserted.token;
            let availability_recovered = upserted.availability_recovered;

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

            if let Some(count) = upserted
                .record
                .declaration
                .models
                .as_ref()
                .filter(|models| !models.is_empty())
                .map(Vec::len)
            {
                events
                    .emit(
                        triggers::MODELS_CHANGED,
                        json!({ "provider": id, "count": count }),
                    )
                    .await;
            }

            Ok(ProviderRegisterResponse {
                ok: true,
                id,
                registration_token: token,
            })
        })
    }
}
