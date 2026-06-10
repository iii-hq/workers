//! Wiring: boot order per the design doc — load stores → install trigger
//! types → register iii functions → bind triggers → register the
//! configuration entry → read settings → emit `router::ready`.
//!
//! iii functions registered here: router::chat, router::complete,
//! router::abort, router::models::{list,get,supports,reconcile},
//! router::provider::{list,register,resolve,update_credential}, plus the
//! trigger-bound router::on_worker_available (subscribe topic
//! engine::workers-available) and router::on_config_changed (configuration
//! trigger on the llm-router entry).
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::types::errors::{RouterCode, RouterError};
use serde_json::{json, Value};

use crate::bus::{handler, Bus, BusError};
use crate::catalog::queries::{models_get, models_list, models_supports};
use crate::catalog::reconcile::make_models_reconcile;
use crate::catalog::store::CatalogStore;
use crate::chat::abort::make_abort;
use crate::chat::chat::{ChatCall, ChatPipeline};
use crate::chat::complete::{make_complete, RunChat};
use crate::chat::inflight::InflightMap;
use crate::chat::relay::ChannelFactory;
use crate::config::entry::{read_entry_value, register_entry, EntryWriteLock};
use crate::config::on_changed::make_on_config_changed;
use crate::config::schema::default_provider_schema;
use crate::registry::availability::{make_on_worker_available, make_provider_list};
use crate::registry::register::make_provider_register;
use crate::registry::resolve::{make_provider_resolve, make_update_credential};
use crate::registry::store::RegistryStore;
use crate::settings::{parse_settings, RouterSettings};
use crate::triggers::TriggerEmitter;

pub struct RouterRefs {
    pub registry: Arc<RegistryStore>,
    pub catalog: Arc<CatalogStore>,
    pub emitter: TriggerEmitter,
    pub inflight: Arc<InflightMap>,
    pub settings: Arc<RwLock<RouterSettings>>,
}

pub async fn register_router(
    bus: Arc<dyn Bus>,
    channels: Arc<dyn ChannelFactory>,
) -> Result<RouterRefs, BusError> {
    // 1–2. restore durable stores; own trigger types (replays rebuild subscribers)
    let registry = Arc::new(RegistryStore::new(bus.clone()));
    let catalog = Arc::new(CatalogStore::new(bus.clone()));
    registry.load().await?;
    catalog.load().await?;
    let emitter = TriggerEmitter::install(bus.clone(), &*bus);

    // 3. shared settings + inflight
    let settings = Arc::new(RwLock::new(RouterSettings::default()));
    let inflight = Arc::new(InflightMap::default());
    let entry_lock = EntryWriteLock::default();

    let pipeline = Arc::new(ChatPipeline {
        bus: bus.clone(),
        registry: registry.clone(),
        catalog: catalog.clone(),
        emitter: emitter.clone(),
        inflight: inflight.clone(),
        settings: settings.clone(),
        channels: channels.clone(),
    });

    // 4. function surface
    {
        let (pipeline, channels) = (pipeline.clone(), channels.clone());
        bus.register_function(
            "router::chat",
            handler(move |raw: Value| {
                let (pipeline, channels) = (pipeline.clone(), channels.clone());
                async move {
                    let writer_ref = serde_json::from_value(
                        raw.get("writer_ref").cloned().unwrap_or(Value::Null),
                    )
                    .map_err(|_| {
                        BusError::from(RouterError::new(
                            RouterCode::InvalidRequest,
                            "writer_ref (direction write) is required",
                        ))
                    })?;
                    let call: ChatCall = serde_json::from_value(raw).map_err(|e| {
                        BusError::from(RouterError::new(RouterCode::InvalidRequest, e.to_string()))
                    })?;
                    let sink = channels.open_sink(&writer_ref).await?;
                    let result = pipeline.run(call, sink.clone()).await;
                    sink.close(); // the handler owns closing the caller's channel
                    result.map(|r| serde_json::to_value(r).expect("serializable response"))
                }
            }),
        );
    }
    {
        let pipeline = pipeline.clone();
        let run_chat: RunChat = Arc::new(move |call, sink| {
            let pipeline = pipeline.clone();
            Box::pin(async move {
                let result = pipeline.run(call, sink.clone()).await;
                sink.close(); // complete's internal channel must EOF for the drain
                result
            })
        });
        bus.register_function(
            "router::complete",
            make_complete(run_chat, channels.clone()),
        );
    }
    bus.register_function("router::abort", make_abort(inflight.clone()));
    {
        let catalog = catalog.clone();
        bus.register_function(
            "router::models::list",
            handler(move |raw: Value| {
                let catalog = catalog.clone();
                async move {
                    let models = models_list(
                        &catalog,
                        raw.get("provider").and_then(Value::as_str),
                        raw.get("capability").and_then(Value::as_str),
                    )
                    .await;
                    Ok(json!({ "models": models }))
                }
            }),
        );
    }
    {
        let catalog = catalog.clone();
        bus.register_function(
            "router::models::get",
            handler(move |raw: Value| {
                let catalog = catalog.clone();
                async move {
                    let model = models_get(
                        &catalog,
                        raw.get("provider").and_then(Value::as_str).unwrap_or(""),
                        raw.get("id").and_then(Value::as_str).unwrap_or(""),
                    )
                    .await;
                    Ok(match model {
                        Some(m) => json!({ "model": m }),
                        None => Value::Null, // null when unregistered (the cold-window signal)
                    })
                }
            }),
        );
    }
    {
        let catalog = catalog.clone();
        bus.register_function(
            "router::models::supports",
            handler(move |raw: Value| {
                let catalog = catalog.clone();
                async move {
                    let supported = models_supports(
                        &catalog,
                        raw.get("provider").and_then(Value::as_str).unwrap_or(""),
                        raw.get("id").and_then(Value::as_str).unwrap_or(""),
                        raw.get("capability").and_then(Value::as_str).unwrap_or(""),
                    )
                    .await;
                    Ok(json!({ "supported": supported }))
                }
            }),
        );
    }
    bus.register_function(
        "router::provider::list",
        make_provider_list(bus.clone(), registry.clone()),
    );
    bus.register_function(
        "router::provider::register",
        make_provider_register(
            bus.clone(),
            registry.clone(),
            catalog.clone(),
            emitter.clone(),
            entry_lock.clone(),
        ),
    );
    bus.register_function(
        "router::provider::resolve",
        make_provider_resolve(bus.clone(), registry.clone()),
    );
    bus.register_function(
        "router::provider::update_credential",
        make_update_credential(bus.clone(), registry.clone(), entry_lock),
    );
    bus.register_function(
        "router::models::reconcile",
        make_models_reconcile(registry.clone(), catalog.clone(), emitter.clone()),
    );

    // 5. bound triggers: topology + configuration change (paste-a-key)
    bus.register_function(
        "router::on_worker_available",
        make_on_worker_available(registry.clone(), emitter.clone()),
    );
    bus.register_trigger(
        "subscribe",
        "router::on_worker_available",
        json!({ "topic": "engine::workers-available" }),
    );
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
        bus.register_function(
            "router::on_config_changed",
            make_on_config_changed(bus.clone(), lookup, settings.clone(), 2000),
        );
    }
    bus.register_trigger(
        "configuration",
        "router::on_config_changed",
        json!({ "configuration_id": "llm-router", "event_types": ["configuration:updated"] }),
    );

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
    register_entry(&bus, &provider_schemas).await?;
    *settings.write().unwrap() = parse_settings(&read_entry_value(&bus).await);

    // 7. ready — providers re-declare on this
    emitter.emit("router::ready", json!({})).await;

    Ok(RouterRefs {
        registry,
        catalog,
        emitter,
        inflight,
        settings,
    })
}
