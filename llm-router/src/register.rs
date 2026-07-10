//! Wiring: boot order per the design doc — load stores → register trigger
//! types → register iii functions → bind triggers → register the configuration
//! entry → read settings → emit `router::ready`.
//!
//! Every registration below is a direct `iii_sdk` call:
//! `iii.register_function(id, RegisterFunction::new_async(...))` for the
//! function surface and `iii.register_trigger(RegisterTriggerInput { .. })`
//! for trigger bindings. Router events fan out via worker-owned trigger types
//! (`triggers::RouterEvents`).
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::catalog::handlers::{make_models_get, make_models_list, make_models_supports};
use crate::catalog::reconcile::make_models_reconcile;
use crate::catalog::store::CatalogStore;
use crate::channels::open_sink;
use crate::chat::abort::make_abort;
use crate::chat::chat::{ChatFnInput, ChatPipeline};
use crate::chat::complete::make_complete;
use crate::chat::inflight::InflightMap;
use crate::config::entry::{read_entry_value, register_entry, EntryWriteLock};
use crate::config::on_changed::make_on_config_changed;
use crate::config::schema::provider_entry_schema;
use crate::registry::availability::make_provider_list;
use crate::registry::register::make_provider_register;
use crate::registry::resolve::{make_provider_resolve, make_update_credential};
use crate::registry::store::RegistryStore;
use crate::settings::{parse_settings, RouterSettings};
use crate::surface;
use crate::triggers::RouterEvents;
use crate::types::errors::invalid_request_from_serde;

pub struct RouterRefs {
    pub registry: Arc<RegistryStore>,
    pub catalog: Arc<CatalogStore>,
    pub inflight: Arc<InflightMap>,
    pub settings: Arc<RwLock<RouterSettings>>,
}

pub async fn register_router(iii: IIIClient) -> Result<RouterRefs, Error> {
    // 1–2. restore durable stores
    let registry = Arc::new(RegistryStore::new(iii.clone()));
    let catalog = Arc::new(CatalogStore::new(iii.clone()));
    registry.load().await?;
    catalog.load().await?;

    // 3. shared settings + inflight + router event fan-out
    let settings = Arc::new(RwLock::new(RouterSettings::default()));
    let inflight = Arc::new(InflightMap::default());
    let entry_lock = EntryWriteLock::default();
    let events = RouterEvents::register(&iii);

    let pipeline = Arc::new(ChatPipeline {
        iii: iii.clone(),
        registry: registry.clone(),
        catalog: catalog.clone(),
        inflight: inflight.clone(),
        settings: settings.clone(),
        events: events.clone(),
    });

    // 4. function surface
    {
        let (iii_for_chat, pipeline) = (iii.clone(), pipeline.clone());
        iii.register_function(
            surface::CHAT_ID,
            RegisterFunction::new_async_with_bad_request(
                move |input: ChatFnInput| {
                    let (iii, pipeline) = (iii_for_chat.clone(), pipeline.clone());
                    async move {
                        let sink = open_sink(&iii, &input.writer_ref).await?;
                        let result = pipeline.run(input.call, sink.clone()).await;
                        sink.close(); // the handler owns closing the caller's channel
                        result
                    }
                },
                invalid_request_from_serde,
            )
            .description(surface::CHAT_DESC),
        );
    }
    iii.register_function(
        surface::COMPLETE_ID,
        RegisterFunction::new_async_with_bad_request(
            make_complete(iii.clone(), pipeline.clone()),
            invalid_request_from_serde,
        )
        .description(surface::COMPLETE_DESC),
    );
    iii.register_function(
        surface::ABORT_ID,
        RegisterFunction::new_async(make_abort(inflight.clone())).description(surface::ABORT_DESC),
    );
    iii.register_function(
        surface::MODELS_LIST_ID,
        RegisterFunction::new_async(make_models_list(catalog.clone()))
            .description(surface::MODELS_LIST_DESC),
    );
    iii.register_function(
        surface::MODELS_GET_ID,
        RegisterFunction::new_async(make_models_get(catalog.clone()))
            .description(surface::MODELS_GET_DESC),
    );
    iii.register_function(
        surface::MODELS_SUPPORTS_ID,
        RegisterFunction::new_async(make_models_supports(catalog.clone()))
            .description(surface::MODELS_SUPPORTS_DESC),
    );
    iii.register_function(
        surface::PROVIDER_LIST_ID,
        RegisterFunction::new_async(make_provider_list(iii.clone(), registry.clone()))
            .description(surface::PROVIDER_LIST_DESC),
    );
    iii.register_function(
        surface::SYSTEM_PROMPT_GET_ID,
        RegisterFunction::new_async(crate::system_prompt::make_system_prompt_get(
            iii.clone(),
            registry.clone(),
        ))
        .description(surface::SYSTEM_PROMPT_GET_DESC),
    );
    iii.register_function(
        surface::ROUTE_ID,
        RegisterFunction::new_async(crate::routing::make_route(
            registry.clone(),
            catalog.clone(),
            settings.clone(),
        ))
        .description(surface::ROUTE_DESC),
    );
    iii.register_function(
        surface::PROVIDER_REGISTER_ID,
        RegisterFunction::new_async_with_bad_request(
            make_provider_register(
                iii.clone(),
                registry.clone(),
                catalog.clone(),
                entry_lock.clone(),
                events.clone(),
            ),
            invalid_request_from_serde,
        )
        .description(surface::PROVIDER_REGISTER_DESC),
    );
    iii.register_function(
        surface::PROVIDER_RESOLVE_ID,
        RegisterFunction::new_async(make_provider_resolve(iii.clone(), registry.clone()))
            .description(surface::PROVIDER_RESOLVE_DESC),
    );
    iii.register_function(
        surface::UPDATE_CREDENTIAL_ID,
        RegisterFunction::new_async(make_update_credential(
            iii.clone(),
            registry.clone(),
            entry_lock,
        ))
        .description(surface::UPDATE_CREDENTIAL_DESC),
    );
    iii.register_function(
        surface::MODELS_RECONCILE_ID,
        RegisterFunction::new_async_with_bad_request(
            make_models_reconcile(registry.clone(), catalog.clone(), events.clone()),
            invalid_request_from_serde,
        )
        .description(surface::MODELS_RECONCILE_DESC),
    );

    // 5. bound trigger: configuration change (paste-a-key)
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
            surface::ON_CONFIG_CHANGED_ID,
            RegisterFunction::new_async(make_on_config_changed(
                iii.clone(),
                lookup,
                settings.clone(),
                2000,
            ))
            .description(surface::ON_CONFIG_CHANGED_DESC),
        );
    }
    let _ = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".into(),
        function_id: surface::ON_CONFIG_CHANGED_ID.into(),
        config: json!({ "configuration_id": "llm-router", "event_types": ["configuration:updated"] }),
        metadata: None,
    });

    // 6. (re)register the entry from the restored registry, then read settings
    let mut provider_schemas = BTreeMap::new();
    for rec in registry.list().await {
        let schema = provider_entry_schema(
            rec.declaration.config_schema.as_ref(),
            &serde_json::to_value(rec.declaration.defaults.clone()).unwrap_or(Value::Null),
            rec.declaration.system_prompt.as_deref(),
        );
        provider_schemas.insert(rec.declaration.id.clone(), schema);
    }
    register_entry(&iii, &provider_schemas).await?;
    *settings.write().unwrap() = parse_settings(&read_entry_value(&iii).await);

    // 7. ready — providers re-declare on this
    events.emit(crate::triggers::READY, json!({})).await;

    // The ready fan-out only reaches providers whose `router::ready` binding
    // still exists — and the engine drops those bindings when THIS worker
    // (the trigger type's owner) disconnects, so providers that outlived a
    // router restart never hear it. Nudge every restored provider directly:
    // `provider::<id>::on_router_ready` is the deterministic per-provider
    // handler (same one the fan-out targets), a live provider re-declares —
    // flipping the boot-reset availability back up — and a dead one is
    // function_not_found, which is exactly the right answer. Detached: boot
    // must not block on provider round-trips.
    {
        let iii = iii.clone();
        let ids = registry.ids().await;
        tokio::spawn(async move {
            for id in ids {
                let _ = iii
                    .trigger(TriggerRequest {
                        function_id: format!("provider::{id}::on_router_ready"),
                        payload: json!({}),
                        action: None,
                        timeout_ms: Some(10_000),
                    })
                    .await;
            }
        });
    }

    Ok(RouterRefs {
        registry,
        catalog,
        inflight,
        settings,
    })
}
