//! Boot wiring: function surface, the router::ready rebind, and the
//! declare-with-backoff loop (spec § Registration lifecycle).
use crate::config::{DEFAULT_API_URL, DEFAULT_MAX_TOKENS};
use crate::discovery::{
    make_refresh_models, refresh_models, refresh_models_periodically, CatalogRefreshState,
};
use crate::errors::invalid_request_from_serde;
use crate::stream_fn::make_stream;
use crate::surface;
use crate::{auth, router_client, state, PROVIDER_ID};
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::types::router::{
    ProviderDeclaration, ProviderDefaults, ProviderReadyAck, RouterReadyEvent,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

pub fn declaration() -> ProviderDeclaration {
    ProviderDeclaration {
        id: PROVIDER_ID.into(),
        display_name: Some("OpenAI Codex".into()),
        // OAuth-only: credentials come from the auth-credentials vault, never
        // from a router-config api_key or an env var.
        credential_env_var: None,
        defaults: Some(ProviderDefaults {
            api_url: Some(DEFAULT_API_URL.into()),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
            extra: BTreeMap::new(),
        }),
        config_schema: None,
        // The authenticated Codex `/models` endpoint is reconciled after
        // registration and periodically while the worker is running.
        supports_model_listing: Some(true),
        models: None,
        // Identity prompt served to agents via router::system_prompt::get;
        // operators can override or disable it in the llm-router config slice.
        system_prompt: Some(include_str!("../prompts/identity.txt").to_string()),
        worker_id: Some("provider-openai-codex".into()),
    }
}

/// One registration attempt: declare (with the persisted token when present)
/// and persist the token the router returns.
pub async fn declare_once(iii: &IIIClient) -> Result<(), Error> {
    let token = state::load_token(iii).await;
    let mut payload = serde_json::to_value(declaration()).expect("serializable declaration");
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
                    "[provider-openai-codex] store registration_token failed ({e}); retrying in {delay:?}"
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
pub async fn declare_with_backoff(iii: IIIClient) {
    let mut delay = Duration::from_millis(500);
    loop {
        match declare_once(&iii).await {
            Ok(()) => {
                println!("[provider-openai-codex] registered with llm-router");
                return;
            }
            Err(e) => {
                eprintln!("[provider-openai-codex] register failed ({e}); retrying in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}

/// Register, seed the vault from `~/.codex/auth.json` if it has no credential
/// yet (best-effort, read-only), then reconcile the dynamic backend catalog.
pub async fn declare_and_refresh(
    iii: IIIClient,
    http: reqwest::Client,
    refresh_state: Arc<CatalogRefreshState>,
) {
    declare_with_backoff(iii.clone()).await;
    auth::import_codex_home_if_absent(&iii).await;
    match refresh_models(&iii, &http, &refresh_state, true).await {
        Ok(count) => println!("[provider-openai-codex] catalog reconciled: {count} models"),
        Err(e) => eprintln!(
            "[provider-openai-codex] post-register reconcile failed ({e}); keeping last known catalog"
        ),
    }
}

/// Upstream read-silence bound, overridable via `PROVIDER_READ_TIMEOUT_SECS`:
/// a fixed 120s cap must not undercut router idle/stream budgets deliberately
/// raised for slow endpoints (long prompt eval on self-hosted gateways).
fn read_timeout() -> Duration {
    std::env::var("PROVIDER_READ_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(120))
}

pub async fn register_provider(iii: IIIClient) -> Result<(), Error> {
    // Shared per-process cache for the registration token and the resolve
    // response (see llm_router::provider_scaffold::cache). Invalidated on
    // router::ready — a restarted router may carry new config and reissues
    // declare/refresh anyway — and on upstream auth errors (stream_fn).
    let cache = ScaffoldCache::new();
    // Reads are silence-bounded: a stalled upstream otherwise pings the router
    // past its idle guard until the engine kills the call at stream_timeout.
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(read_timeout())
        .build()
        .expect("reqwest client");
    let refresh_state = Arc::new(CatalogRefreshState::default());

    iii.register_function(
        surface::STREAM_ID,
        RegisterFunction::new_async_with_bad_request(
            make_stream(iii.clone(), http.clone(), cache.clone()),
            invalid_request_from_serde,
        )
        .description(surface::STREAM_DESC)
        .metadata(json!({ "internal": true })),
    );
    iii.register_function(
        surface::REFRESH_MODELS_ID,
        RegisterFunction::new_async(make_refresh_models(
            iii.clone(),
            http.clone(),
            refresh_state.clone(),
        ))
        .description(surface::REFRESH_MODELS_DESC)
        .metadata(json!({ "internal": true })),
    );

    {
        let iii_ready = iii.clone();
        let http_ready = http.clone();
        let refresh_state_ready = refresh_state.clone();
        let cache_ready = cache.clone();
        iii.register_function(
            surface::ON_ROUTER_READY_ID,
            RegisterFunction::new_async(move |_event: RouterReadyEvent| {
                let (iii, http, refresh_state) = (
                    iii_ready.clone(),
                    http_ready.clone(),
                    refresh_state_ready.clone(),
                );
                cache_ready.invalidate();
                async move {
                    tokio::spawn(declare_and_refresh(iii, http, refresh_state));
                    Ok::<_, Error>(ProviderReadyAck { ok: true })
                }
            })
            .description(surface::ON_ROUTER_READY_DESC)
            .metadata(json!({ "internal": true })),
        );
    }
    let _ = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "router::ready".into(),
        function_id: surface::ON_ROUTER_READY_ID.into(),
        config: json!({}),
        metadata: None,
    });

    tokio::spawn(declare_and_refresh(
        iii.clone(),
        http.clone(),
        refresh_state.clone(),
    ));
    tokio::spawn(refresh_models_periodically(iii, http, refresh_state));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::declaration;

    /// The declared identity prompt is the embedded prompts/identity.txt and
    /// keeps the invariants the harness pins on its default prompt.
    #[test]
    fn declaration_ships_the_identity_prompt() {
        let declaration = declaration();
        let prompt = declaration
            .system_prompt
            .as_deref()
            .expect("declared prompt");
        assert_eq!(prompt, include_str!("../prompts/identity.txt"));
        assert!(prompt.starts_with("You are an iii agent worker."));
        assert!(prompt.contains("agent_trigger"));
        assert!(prompt.contains("## Autonomy and persistence"));
        assert_eq!(declaration.supports_model_listing, Some(true));
        assert!(declaration.models.is_none());
    }
}
