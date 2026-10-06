//! Boot wiring: function surface, the router::ready rebind, and the
//! declare-with-backoff loop (spec § Registration lifecycle).
use crate::config::DEFAULT_MAX_TOKENS;
use crate::discovery::{make_refresh_models, refresh_models};
use crate::errors::invalid_request_from_serde;
use crate::stream_fn::make_stream;
use crate::surface;
use crate::{router_client, state, PROVIDER_ID};
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use llm_router::provider_scaffold::aborts::{make_abort, StreamAborts};
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::provider_scaffold::registration::typed_async_with_bad_request;
use llm_router::types::router::{
    ProviderDeclaration, ProviderDefaults, ProviderReadyAck, RouterReadyEvent,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

/// Env var the router (and, as a fallback, this provider) reads for the key.
pub const CREDENTIAL_ENV_VAR: &str = "LLAMACPP_API_KEY";

/// Shown by consoles under a context-overflow failure on a llama.cpp model.
pub const CONTEXT_OVERFLOW_HINT: &str = "llama.cpp serves each model with the context size it was started with (--ctx-size) and rejects anything larger, so compacting cannot fix this. Raise it on the server or in the per-model configuration options in the desktop app; see the llama.cpp server docs: https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md";

pub fn declaration() -> ProviderDeclaration {
    declaration_with(None)
}

/// The boot declaration plus the default-model list discovery ranked
/// (loaded first, then largest; see `discovery::rank_default_models`).
pub fn declaration_with_defaults(default_models: Vec<String>) -> ProviderDeclaration {
    declaration_with(Some(default_models))
}

fn declaration_with(default_models: Option<Vec<String>>) -> ProviderDeclaration {
    ProviderDeclaration {
        id: PROVIDER_ID.into(),
        display_name: Some("llama.cpp".into()),
        credential_env_var: Some(CREDENTIAL_ENV_VAR.into()),
        defaults: Some(ProviderDefaults {
            // Unset on purpose: the router returns `defaults.api_url` as the
            // resolved url, which would stop discovery from probing both
            // local ports (config::DEFAULT_API_URL_CANDIDATES).
            api_url: None,
            max_tokens: Some(DEFAULT_MAX_TOKENS),
            extra: BTreeMap::new(),
        }),
        config_schema: None, // the router's default {api_key, api_url, max_tokens}
        // llama.cpp's `GET /v1/models` is a real listing endpoint (unlike
        // Z.AI/OpenAI-cloud); this also gates the router's
        // refresh-on-config-change call, which must fire so the catalog
        // appears the moment an operator points api_url at a running server.
        supports_model_listing: Some(true),
        // Local, user-supplied models: nothing to recommend at boot. After
        // each discovery the provider re-declares with the served models
        // ranked, loaded first then largest (redeclare_with_defaults).
        default_models,
        default_thinking_level: None,
        // The server rejects any prompt over its --ctx-size, so compacting
        // in a console cannot fix an overflow against a small window.
        context_overflow_hint: Some(CONTEXT_OVERFLOW_HINT.into()),
        // No static slice: refresh_models discovers the catalog live from
        // the resolved server's `/v1/models` + `/props` right after
        // registration (see declare_and_refresh) — no credential required.
        models: None,
        // Self-reported; availability mapping only, never authorization.
        worker_id: Some("provider-llamacpp".into()),
        // The mark the console paints beside this provider's models; the
        // router carries it verbatim in `router::provider::list`.
        icon_svg: Some(
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icon.svg")).into(),
        ),
    }
}

/// One registration attempt: declare (with the persisted token when present)
/// and persist the token the router returns.
pub async fn declare_once(iii: &IIIClient) -> Result<(), Error> {
    declare_payload(iii, current_declaration()).await
}

/// The last default list sent, so a refresh that finds the same listing
/// does not re-register for nothing.
static DECLARED_DEFAULTS: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

fn remember_defaults(preferred: Vec<String>) {
    if let Ok(mut last) = DECLARED_DEFAULTS.lock() {
        *last = Some(preferred);
    }
}

/// The boot declaration plus whatever discovery ranked so far: a
/// router::ready rebind re-declares through here, so it never erases the
/// default list from the router's record (which the unchanged-check in
/// `redeclare_with_defaults` would then not restore).
fn current_declaration() -> ProviderDeclaration {
    declaration_with(DECLARED_DEFAULTS.lock().ok().and_then(|last| last.clone()))
}

/// Re-register with discovery's ranked default list when it changed.
pub async fn redeclare_with_defaults(iii: &IIIClient, preferred: Vec<String>) -> Result<(), Error> {
    if preferred.is_empty() {
        return Ok(());
    }
    let unchanged = DECLARED_DEFAULTS
        .lock()
        .map(|last| last.as_ref() == Some(&preferred))
        .unwrap_or(false);
    if unchanged {
        return Ok(());
    }
    declare_payload(iii, declaration_with_defaults(preferred.clone())).await?;
    remember_defaults(preferred);
    Ok(())
}

async fn declare_payload(iii: &IIIClient, declaration: ProviderDeclaration) -> Result<(), Error> {
    let token = state::load_token(iii).await;
    let mut payload = serde_json::to_value(declaration).expect("serializable declaration");
    if let Some(t) = &token {
        payload["token"] = json!(t);
    }
    let resp = router_client::register(iii, payload).await?;
    if let Some(t) = resp.get("registration_token").and_then(Value::as_str) {
        if token.as_deref() != Some(t) {
            persist_registration_token(iii, t).await?;
        }
    }
    Ok(())
}

async fn persist_registration_token(iii: &IIIClient, token: &str) -> Result<(), Error> {
    let mut delay = Duration::from_millis(200);
    for attempt in 0..5 {
        match state::store_token(iii, token).await {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 4 => {
                eprintln!(
                    "[provider-llamacpp] store registration_token failed ({e}); retrying in {delay:?}"
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(2));
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("persist_registration_token loop always returns");
}

/// Retry until acknowledged: covers provider-before-router boot order.
/// A token mismatch also lands here — it never resolves on its own and
/// needs the operator to clear the binding (logged every attempt).
pub async fn declare_with_backoff(iii: IIIClient) {
    let mut delay = Duration::from_millis(500);
    loop {
        match declare_once(&iii).await {
            Ok(()) => {
                println!("[provider-llamacpp] registered with llm-router");
                return;
            }
            Err(e) => {
                eprintln!("[provider-llamacpp] register failed ({e}); retrying in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}

/// Register, then run live model discovery against the resolved server. The
/// declaration carries no models, so the slice is empty until this refresh
/// lands; failures are logged and left to the next config-change refresh.
pub async fn declare_and_refresh(iii: IIIClient, http: reqwest::Client) {
    declare_with_backoff(iii.clone()).await;
    match refresh_models(&iii, &http).await {
        Ok(count) => println!("[provider-llamacpp] catalog refreshed: {count} models"),
        Err(e) => eprintln!("[provider-llamacpp] post-register refresh failed ({e})"),
    }
}

pub async fn register_provider(iii: IIIClient) -> Result<(), Error> {
    // Shared per-process cache for the registration token and the resolve
    // response (see llm_router::provider_scaffold::cache). Invalidated on
    // router::ready — a restarted router may carry new config and reissues
    // declare/refresh anyway — and on upstream auth errors (stream_fn).
    let cache = ScaffoldCache::new();
    // Streaming uses no total timeout (the router owns stream budgets);
    // connect failures surface fast.
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client");

    // request_id → live upstream cancel, shared by stream (registers) and
    // abort (signals) — see llm_router::provider_scaffold::aborts.
    let aborts = StreamAborts::new();

    iii.register_function(
        surface::STREAM_ID,
        typed_async_with_bad_request(
            make_stream(iii.clone(), http.clone(), cache.clone(), aborts.clone()),
            invalid_request_from_serde,
        )
        .description(surface::STREAM_DESC),
    );
    iii.register_function(
        surface::ABORT_ID,
        typed_async_with_bad_request(make_abort(aborts), invalid_request_from_serde)
            .description(surface::ABORT_DESC)
            .metadata(json!({ "internal": true })),
    );
    iii.register_function(
        surface::REFRESH_MODELS_ID,
        RegisterFunction::new_async(make_refresh_models(iii.clone(), http.clone()))
            .description(surface::REFRESH_MODELS_DESC),
    );

    let iii_count = iii.clone();
    let http_count = http.clone();
    let cache_count = cache.clone();
    iii.register_function(
        surface::COUNT_TOKENS_ID,
        RegisterFunction::new_async(move |req: crate::count_tokens::CountTokensRequest| {
            let (iii, http, cache) = (iii_count.clone(), http_count.clone(), cache_count.clone());
            async move { crate::count_tokens::handle(&iii, &http, &cache, req).await }
        })
        .description(surface::COUNT_TOKENS_DESC)
        .metadata(json!({ "internal": true })),
    );

    // Re-declare when the router restarts: bind to the router::ready trigger type.
    {
        let iii_ready = iii.clone();
        let http_ready = http.clone();
        let cache_ready = cache.clone();
        iii.register_function(
            surface::ON_ROUTER_READY_ID,
            RegisterFunction::new_async(move |_event: RouterReadyEvent| {
                let iii = iii_ready.clone();
                let http = http_ready.clone();
                cache_ready.invalidate();
                async move {
                    tokio::spawn(declare_and_refresh(iii, http));
                    Ok::<_, Error>(ProviderReadyAck { ok: true })
                }
            })
            .description(surface::ON_ROUTER_READY_DESC)
            // Invoked by id (the router's ready fan-out and its re-discovery
            // nudge), never discovered — same as every other provider's ready
            // handler. Untagged, this was the ONE provider handler visible in
            // the default `engine::functions::list`, which made a router-side
            // provider sweep look like it worked while finding 1 of 4.
            .metadata(json!({ "internal": true })),
        );
    }

    {
        let iii_embed = iii.clone();
        let http_embed = http.clone();
        let cache_embed = cache.clone();
        iii.register_function(
            surface::EMBED_ID,
            RegisterFunction::new_async(move |req: crate::embed::EmbedRequest| {
                let (iii, http, cache) =
                    (iii_embed.clone(), http_embed.clone(), cache_embed.clone());
                async move { crate::embed::handle(&iii, &http, &cache, req).await }
            })
            .description(surface::EMBED_DESC)
            .metadata(json!({ "internal": true })),
        );
    }
    let _ = iii.register_trigger(RegisterTriggerInput::new(
        "router::ready",
        surface::ON_ROUTER_READY_ID,
        json!({}),
    ));

    // Boot declare, off the boot path (a missing router must not block boot).
    tokio::spawn(declare_and_refresh(iii, http));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{current_declaration, declaration, declaration_with_defaults, remember_defaults};

    #[test]
    fn declaration_leaves_api_url_unset_so_discovery_probes() {
        // The router hands `defaults.api_url` back as the resolved url, and
        // discovery only probes both local ports when it is unset.
        assert_eq!(declaration().defaults.and_then(|d| d.api_url), None);
    }

    #[test]
    fn discovered_defaults_ride_the_redeclaration() {
        assert_eq!(declaration().default_models, None);
        let decl = declaration_with_defaults(vec!["a-27B".into(), "b-7B".into()]);
        assert_eq!(
            decl.default_models,
            Some(vec!["a-27B".to_string(), "b-7B".to_string()])
        );
        assert_eq!(decl.id, declaration().id);
    }

    #[test]
    fn rebind_redeclares_with_the_last_discovered_defaults() {
        // router::ready re-runs declare_once after discovery already sent a
        // ranked list; the plain boot declaration would wipe it from the
        // router's record and the unchanged-check would then skip the fix.
        remember_defaults(vec!["served-27B".into(), "served-7B".into()]);
        assert_eq!(
            current_declaration().default_models,
            Some(vec!["served-27B".to_string(), "served-7B".to_string()])
        );
    }

    #[test]
    fn declaration_uses_credential_env_var_const() {
        assert_eq!(super::CREDENTIAL_ENV_VAR, "LLAMACPP_API_KEY");
        assert_eq!(
            declaration().credential_env_var.as_deref(),
            Some(super::CREDENTIAL_ENV_VAR)
        );
    }
}
