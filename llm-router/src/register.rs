//! Wiring: restore stores → register and fetch configuration → build the live
//! in-memory snapshot and the secret-reference cache → register functions →
//! observe and reconcile the configuration entry → send `router::on-ready` →
//! observe the (optional) secrets worker's `secrets::on-changed`.
//!
//! Function registrations use `iii_sdk` directly, with the shared typed
//! registration adapter where malformed payloads have a stable public error
//! code. Each observer is `iii.observe(...)` with the event delivered as
//! output; dropping its handle keeps it for the life of the connection. Router
//! events are the worker's own private functions (`events::RouterEvents`).
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::engine::EngineSources;
use iii_sdk::errors::Error;
use iii_sdk::{Deliver, IIIClient, RegisterFunction, Source};
use serde_json::{json, Value};

use crate::catalog::handlers::{
    make_models_budget, make_models_get, make_models_list, make_models_supports,
};
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
use crate::config::state::{new_config_cell, snapshot, ConfigCell};
use crate::events::RouterEvents;
use crate::provider_scaffold::registration::typed_async_with_bad_request;
use crate::registry::availability::make_provider_list;
use crate::registry::refresh::{bus_fire, RefreshQueue};
use crate::registry::register::make_provider_register;
use crate::registry::resolve::{make_provider_resolve, make_update_credential, referenced_secrets};
use crate::registry::store::RegistryStore;
use crate::secrets::on_changed::{make_on_secret_changed, refresh_referencing_providers};
use crate::secrets::{bus_fetch, spawn_unavailable_retry, SecretCache};
use crate::surface;
use crate::types::errors::invalid_request_from_serde;
use crate::types::router::{ConfigChangedEvent, FunctionsChangedEvent, RouterAck};

/// Quiet period before queued provider follow-ups (discovery after a
/// configuration change, a credential refresh after a secret change) fire.
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(2000);

/// `metadata.internal = true` keeps a registration out of the default
/// `engine::functions::list`: orchestrator/provider plumbing, invoked by id.
/// Agent-discoverable reads (models::list/get/supports, provider::list) stay
/// visible.
fn internal_meta() -> Value {
    json!({ "internal": true })
}

pub struct RouterRefs {
    pub registry: Arc<RegistryStore>,
    pub catalog: Arc<CatalogStore>,
    pub inflight: Arc<InflightMap>,
    pub config: ConfigCell,
}

/// Register routing functions and subscribe to updates of this instance's configuration entry.
pub async fn register_router(iii: IIIClient) -> Result<RouterRefs, Error> {
    // 1–2. restore durable stores
    let registry = Arc::new(RegistryStore::new(iii.clone()));
    let catalog = Arc::new(CatalogStore::new(iii.clone()));
    registry.load().await?;
    catalog.load().await?;

    // 3. Register and fetch the authoritative configuration before exposing
    // any request handlers. Re-registration preserves the stored value.
    let entry_lock = EntryWriteLock::default();
    let mut provider_schemas = BTreeMap::new();
    for rec in registry.list().await {
        let schema = provider_entry_schema(
            rec.declaration.config_schema.as_ref(),
            &serde_json::to_value(rec.declaration.defaults.clone()).unwrap_or(Value::Null),
        );
        provider_schemas.insert(rec.declaration.id.clone(), schema);
    }
    register_entry(&iii, &provider_schemas).await?;
    let config = new_config_cell(read_entry_value(&iii).await?);

    // Provider follow-ups share one debounce, and `secret://` / `env://` references
    // resolve through the `secrets` worker into an in-memory cache whose
    // changes refresh the providers that use them.
    let refresh = RefreshQueue::new(bus_fire(iii.clone()), REFRESH_DEBOUNCE);
    let secrets = Arc::new(SecretCache::new(bus_fetch(iii.clone())).with_listener(
        refresh_referencing_providers(config.clone(), refresh.clone()),
    ));

    // 4. Shared runtime state + the router's event functions.
    let inflight = Arc::new(InflightMap::default());
    let events = RouterEvents::register(&iii);

    let pipeline = Arc::new(ChatPipeline {
        iii: iii.clone(),
        registry: registry.clone(),
        catalog: catalog.clone(),
        inflight: inflight.clone(),
        config: config.clone(),
        secrets: secrets.clone(),
        events: events.clone(),
    });

    // 5. Function surface.
    {
        let (iii_for_chat, pipeline) = (iii.clone(), pipeline.clone());
        iii.register_function(
            surface::CHAT_ID,
            typed_async_with_bad_request(
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
            .description(surface::CHAT_DESC)
            .metadata(internal_meta()),
        );
    }
    iii.register_function(
        surface::COMPLETE_ID,
        typed_async_with_bad_request(
            make_complete(iii.clone(), pipeline.clone()),
            invalid_request_from_serde,
        )
        .description(surface::COMPLETE_DESC)
        .metadata(internal_meta()),
    );
    iii.register_function(
        surface::ABORT_ID,
        RegisterFunction::new_async(make_abort(inflight.clone()))
            .description(surface::ABORT_DESC)
            .metadata(internal_meta()),
    );
    iii.register_function(
        surface::EMBED_ID,
        RegisterFunction::new_async(crate::embed::make_embed(
            iii.clone(),
            registry.clone(),
            config.clone(),
            secrets.clone(),
        ))
        .description(surface::EMBED_DESC)
        .metadata(internal_meta()),
    );
    iii.register_function(
        surface::TRANSCRIBE_ID,
        RegisterFunction::new_async(crate::speech::make_transcribe(
            iii.clone(),
            registry.clone(),
            catalog.clone(),
        ))
        .description(surface::TRANSCRIBE_DESC)
        .metadata(internal_meta()),
    );
    iii.register_function(
        surface::SPEAK_ID,
        RegisterFunction::new_async(crate::speech::make_speak(
            iii.clone(),
            registry.clone(),
            catalog.clone(),
        ))
        .description(surface::SPEAK_DESC)
        .metadata(internal_meta()),
    );
    iii.register_function(
        surface::COUNT_TOKENS_ID,
        RegisterFunction::new_async(crate::count_tokens::make_count_tokens(
            iii.clone(),
            registry.clone(),
            catalog.clone(),
            config.clone(),
        ))
        .description(surface::COUNT_TOKENS_DESC)
        .metadata(internal_meta()),
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
        surface::MODELS_BUDGET_ID,
        RegisterFunction::new_async(make_models_budget(
            catalog.clone(),
            registry.clone(),
            config.clone(),
        ))
        .description(surface::MODELS_BUDGET_DESC)
        .metadata(internal_meta()),
    );
    iii.register_function(
        surface::MODELS_SUPPORTS_ID,
        RegisterFunction::new_async(make_models_supports(catalog.clone()))
            .description(surface::MODELS_SUPPORTS_DESC),
    );
    iii.register_function(
        surface::PROVIDER_LIST_ID,
        RegisterFunction::new_async(make_provider_list(
            config.clone(),
            registry.clone(),
            secrets.clone(),
            catalog.clone(),
        ))
        .description(surface::PROVIDER_LIST_DESC),
    );
    iii.register_function(
        surface::ROUTE_ID,
        RegisterFunction::new_async(crate::routing::make_route(
            registry.clone(),
            catalog.clone(),
            config.clone(),
        ))
        .description(surface::ROUTE_DESC)
        .metadata(internal_meta()),
    );
    iii.register_function(
        surface::PROVIDER_REGISTER_ID,
        typed_async_with_bad_request(
            make_provider_register(
                iii.clone(),
                registry.clone(),
                catalog.clone(),
                entry_lock.clone(),
                events.clone(),
            ),
            invalid_request_from_serde,
        )
        .description(surface::PROVIDER_REGISTER_DESC)
        .metadata(internal_meta())
        .observable(false),
    );
    iii.register_function(
        surface::PROVIDER_RESOLVE_ID,
        RegisterFunction::new_async(make_provider_resolve(
            config.clone(),
            entry_lock.clone(),
            registry.clone(),
            secrets.clone(),
        ))
        .description(surface::PROVIDER_RESOLVE_DESC)
        .metadata(internal_meta())
        .observable(false),
    );
    iii.register_function(
        surface::UPDATE_CREDENTIAL_ID,
        RegisterFunction::new_async(make_update_credential(
            iii.clone(),
            registry.clone(),
            config.clone(),
            entry_lock.clone(),
        ))
        .description(surface::UPDATE_CREDENTIAL_DESC)
        .metadata(internal_meta())
        .observable(false),
    );
    iii.register_function(
        surface::MODELS_RECONCILE_ID,
        typed_async_with_bad_request(
            make_models_reconcile(
                registry.clone(),
                catalog.clone(),
                events.clone(),
                config.clone(),
                secrets.clone(),
                entry_lock.clone(),
            ),
            invalid_request_from_serde,
        )
        .description(surface::MODELS_RECONCILE_DESC)
        .metadata(internal_meta())
        .observable(false),
    );

    // 6. Observe configuration changes only after every consumer of the live
    // snapshot has been built.
    let on_config_changed = {
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
        make_on_config_changed(
            iii.clone(),
            lookup,
            config.clone(),
            entry_lock.clone(),
            secrets.clone(),
            refresh.clone(),
        )
    };
    iii.register_function(
        surface::ON_CONFIG_CHANGED_ID,
        RegisterFunction::new_async(on_config_changed.clone())
            .description(surface::ON_CONFIG_CHANGED_DESC)
            .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
    iii.observe(
        Source::function(EngineSources::CONFIGURATION_UPDATED)
            .filters(json!({
                "input.id": { "$eq": crate::config::entry::config_id() },
                "input.event_type": { "$eq": "configuration:updated" },
            }))
            .deliver(Deliver::Output),
        surface::ON_CONFIG_CHANGED_ID,
    );

    // Close the boot race between the initial fetch and the observer becoming
    // active by running the SAME operation each delivery runs: one reconcile
    // keeps the snapshot, the fingerprint baseline, and any due
    // `refresh_models` in step (a bare `apply_config` would install a newer
    // value the handler then diffs against a stale baseline).
    let ack = on_config_changed(ConfigChangedEvent { id: None }).await?;
    if !ack.ok {
        tracing::warn!(
            "boot config reconcile kept the step-3 snapshot; authoritative re-read failed"
        );
    }

    // 7. ready — providers re-declare on this. Their observers of
    // `router::on-ready` outlive a router restart (pending while it is down):
    // each one, and each one added later, receives the ready event once
    // (`events::RouterEvents`).
    events.ready().await;

    // A provider that reconnects to the engine within the observer grace
    // period has its observer of `router::on-ready` added again, receives the
    // ready event and re-declares. One that reconnects after the grace period
    // gets its FUNCTIONS replayed by the SDK, but its observer is disposed, so
    // nothing re-pushes its catalog (application state this worker holds)
    // until the provider's own periodic timer (three minutes for
    // openai-codex). Observe the engine's registration stream and nudge each
    // provider that is newly live, so a returning provider is resolvable in
    // seconds. See `registry::rediscover`.
    //
    // The same stream is how a router that booted before the secrets worker
    // learns it arrived: references that failed as unreachable are re-read,
    // and any that now resolve refresh their providers.
    //
    // The event is advisory: the engine calls it once per ~100ms burst of
    // registry changes (any register, including a schema-only re-register,
    // and any removal). Both followers keep their own quiet period; see
    // `SweepHandle`.
    {
        let iii_handler = iii.clone();
        let sweep = crate::registry::rediscover::spawn_debounced_sweep(iii_handler);
        let secrets_retry = spawn_unavailable_retry(secrets.clone());
        iii.register_function(
            surface::ON_FUNCTIONS_CHANGED_ID,
            RegisterFunction::new_async(move |_event: FunctionsChangedEvent| {
                let (sweep, secrets_retry) = (sweep.clone(), secrets_retry.clone());
                async move {
                    // The handler only marks work pending; each follower
                    // runs once changes go quiet, so a spread-out burst
                    // (several events) costs one sweep and one re-read.
                    sweep.request();
                    secrets_retry.request();
                    Ok::<RouterAck, Error>(RouterAck { ok: true })
                }
            })
            .description(surface::ON_FUNCTIONS_CHANGED_DESC)
            .metadata(json!({ "internal": true, "trace_hidden": true })),
        );
        iii.observe(
            Source::function(EngineSources::FUNCTIONS_CHANGED).deliver(Deliver::Output),
            surface::ON_FUNCTIONS_CHANGED_ID,
        );
    }

    // 8. Secret changes. `secrets::on-changed` belongs to the optional
    // `secrets` worker: while it is absent this observer waits `pending`, and
    // it becomes active whenever the function registers (again after a
    // secrets worker restart), so nothing here retries. Cache TTLs bound
    // staleness if an event is ever missed.
    iii.register_function(
        surface::ON_SECRET_CHANGED_ID,
        RegisterFunction::new_async(make_on_secret_changed(config.clone(), secrets.clone()))
            .description(surface::ON_SECRET_CHANGED_DESC)
            .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
    iii.observe(
        Source::function(crate::secrets::ON_CHANGED_FN).deliver(Deliver::Output),
        surface::ON_SECRET_CHANGED_ID,
    );
    // Warm the cache off the boot path; a secrets worker that is not up yet
    // is retried when it registers (see the registration stream above).
    {
        let names: Vec<String> = referenced_secrets(&snapshot(&config)).into_keys().collect();
        let secrets = secrets.clone();
        tokio::spawn(async move { secrets.ensure(names).await });
    }

    Ok(RouterRefs {
        registry,
        catalog,
        inflight,
        config,
    })
}
