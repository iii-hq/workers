//! Wiring: boot order per the design doc — load stores → register iii
//! functions → bind triggers → register the configuration entry → read
//! settings → publish `router::ready`.
//!
//! Every registration below is a direct `iii_sdk` call:
//! `iii.register_function(id, RegisterFunction::new_async(...))` for the
//! function surface and `iii.register_trigger(RegisterTriggerInput { .. })`
//! for trigger bindings. Router events go out over the engine's `iii-pubsub`
//! worker (`triggers::publish`).
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, III};
use serde_json::{json, Value};

use crate::catalog::handlers::{make_models_get, make_models_list, make_models_supports};
use crate::catalog::reconcile::make_models_reconcile;
use crate::catalog::store::CatalogStore;
use crate::channels::open_sink;
use crate::chat::abort::make_abort;
use crate::chat::chat::{ChatCall, ChatPipeline};
use crate::chat::complete::make_complete;
use crate::chat::inflight::InflightMap;
use crate::config::entry::{read_entry_value, register_entry, EntryWriteLock};
use crate::config::on_changed::make_on_config_changed;
use crate::config::schema::default_provider_schema;
use crate::registry::availability::{make_on_worker_available, make_provider_list};
use crate::registry::register::make_provider_register;
use crate::registry::resolve::{make_provider_resolve, make_update_credential};
use crate::registry::store::RegistryStore;
use crate::settings::{parse_settings, RouterSettings};
use crate::triggers;
use crate::types::errors::{RouterCode, RouterError};

pub struct RouterRefs {
    pub registry: Arc<RegistryStore>,
    pub catalog: Arc<CatalogStore>,
    pub inflight: Arc<InflightMap>,
    pub settings: Arc<RwLock<RouterSettings>>,
}

pub async fn register_router(iii: III) -> Result<RouterRefs, IIIError> {
    // 1–2. restore durable stores
    let registry = Arc::new(RegistryStore::new(iii.clone()));
    let catalog = Arc::new(CatalogStore::new(iii.clone()));
    registry.load().await?;
    catalog.load().await?;

    // 3. shared settings + inflight
    let settings = Arc::new(RwLock::new(RouterSettings::default()));
    let inflight = Arc::new(InflightMap::default());
    let entry_lock = EntryWriteLock::default();

    let pipeline = Arc::new(ChatPipeline {
        iii: iii.clone(),
        registry: registry.clone(),
        catalog: catalog.clone(),
        inflight: inflight.clone(),
        settings: settings.clone(),
    });

    // 4. function surface
    {
        let (iii_for_chat, pipeline) = (iii.clone(), pipeline.clone());
        iii.register_function(
            "router::chat",
            RegisterFunction::new_async(move |raw: Value| {
                let (iii, pipeline) = (iii_for_chat.clone(), pipeline.clone());
                async move {
                    let writer_ref = serde_json::from_value(
                        raw.get("writer_ref").cloned().unwrap_or(Value::Null),
                    )
                    .map_err(|_| {
                        IIIError::from(RouterError::new(
                            RouterCode::InvalidRequest,
                            "writer_ref (direction write) is required",
                        ))
                    })?;
                    let call: ChatCall = serde_json::from_value(raw).map_err(|e| {
                        IIIError::from(RouterError::new(RouterCode::InvalidRequest, e.to_string()))
                    })?;
                    let sink = open_sink(&iii, &writer_ref).await?;
                    let result = pipeline.run(call, sink.clone()).await;
                    sink.close(); // the handler owns closing the caller's channel
                    result.map(|r| serde_json::to_value(r).expect("serializable response"))
                }
            }),
        );
    }
    iii.register_function(
        "router::complete",
        RegisterFunction::new_async(make_complete(iii.clone(), pipeline.clone())),
    );
    iii.register_function(
        "router::abort",
        RegisterFunction::new_async(make_abort(inflight.clone())),
    );
    iii.register_function(
        "router::models::list",
        RegisterFunction::new_async(make_models_list(catalog.clone())),
    );
    iii.register_function(
        "router::models::get",
        RegisterFunction::new_async(make_models_get(catalog.clone())),
    );
    iii.register_function(
        "router::models::supports",
        RegisterFunction::new_async(make_models_supports(catalog.clone())),
    );
    iii.register_function(
        "router::provider::list",
        RegisterFunction::new_async(make_provider_list(iii.clone(), registry.clone())),
    );
    iii.register_function(
        "router::provider::register",
        RegisterFunction::new_async(make_provider_register(
            iii.clone(),
            registry.clone(),
            catalog.clone(),
            entry_lock.clone(),
        )),
    );
    iii.register_function(
        "router::provider::resolve",
        RegisterFunction::new_async(make_provider_resolve(iii.clone(), registry.clone())),
    );
    iii.register_function(
        "router::provider::update_credential",
        RegisterFunction::new_async(make_update_credential(
            iii.clone(),
            registry.clone(),
            entry_lock,
        )),
    );
    iii.register_function(
        "router::models::reconcile",
        RegisterFunction::new_async(make_models_reconcile(
            iii.clone(),
            registry.clone(),
            catalog.clone(),
        )),
    );

    // 5. bound triggers: topology + configuration change (paste-a-key)
    iii.register_function(
        "router::on_worker_available",
        RegisterFunction::new_async(make_on_worker_available(iii.clone(), registry.clone())),
    );
    let _ = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "subscribe".into(),
        function_id: "router::on_worker_available".into(),
        config: json!({ "topic": "engine::workers-available" }),
        metadata: None,
    });
    {
        let registry_for_listing = registry.clone();
        let lookup: crate::config::on_changed::ListingLookup = Arc::new(move |id: &str| {
            let registry = registry_for_listing.clone();
            let id = id.to_string();
            Box::pin(async move {
                registry
                    .get(&id)
                    .await
                    .and_then(|r| r.declaration.supports_model_listing)
                    .unwrap_or(false)
            })
        });
        iii.register_function(
            "router::on_config_changed",
            RegisterFunction::new_async(make_on_config_changed(
                iii.clone(),
                lookup,
                settings.clone(),
                2000,
            )),
        );
    }
    let _ = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".into(),
        function_id: "router::on_config_changed".into(),
        config: json!({ "configuration_id": "llm-router", "event_types": ["configuration:updated"] }),
        metadata: None,
    });

    // 6. (re)register the entry from the restored registry, then read settings
    let mut provider_schemas = BTreeMap::new();
    for rec in registry.list().await {
        let schema = rec.declaration.config_schema.clone().unwrap_or_else(|| {
            default_provider_schema(
                &serde_json::to_value(rec.declaration.defaults.clone()).unwrap_or(Value::Null),
            )
        });
        provider_schemas.insert(rec.declaration.id.clone(), schema);
    }
    register_entry(&iii, &provider_schemas).await?;
    *settings.write().unwrap() = parse_settings(&read_entry_value(&iii).await);

    // 7. ready — providers re-declare on this
    triggers::publish(&iii, triggers::READY, json!({})).await;

    Ok(RouterRefs {
        registry,
        catalog,
        inflight,
        settings,
    })
}
