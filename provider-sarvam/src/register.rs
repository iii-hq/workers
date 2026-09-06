//! Boot wiring: function surface, the router::ready rebind, and the
//! declare-with-backoff loop (spec § Registration lifecycle).
use crate::config::{DEFAULT_API_URL, DEFAULT_MAX_TOKENS};
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
pub const CREDENTIAL_ENV_VAR: &str = "SARVAM_API_KEY";

pub fn declaration() -> ProviderDeclaration {
    ProviderDeclaration {
        icon_svg: None,
        id: PROVIDER_ID.into(),
        display_name: Some("Sarvam AI".into()),
        credential_env_var: Some(CREDENTIAL_ENV_VAR.into()),
        defaults: Some(ProviderDefaults {
            api_url: Some(DEFAULT_API_URL.into()),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
            extra: BTreeMap::new(),
        }),
        config_schema: None,
        supports_model_listing: Some(true),
        models: None,
        worker_id: Some("provider-sarvam".into()),
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
                    "[provider-sarvam] store registration_token failed ({e}); retrying in {delay:?}"
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
                println!("[provider-sarvam] registered with llm-router");
                return;
            }
            Err(e) => {
                eprintln!("[provider-sarvam] register failed ({e}); retrying in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}

/// Register, then reconcile the curated catalog. The declaration carries no
/// models, so the slice is empty until this refresh lands; failures are
/// logged and left to the next config-change refresh.
pub async fn declare_and_refresh(iii: IIIClient) {
    declare_with_backoff(iii.clone()).await;
    match refresh_models(&iii).await {
        Ok(count) => println!("[provider-sarvam] catalog refreshed: {count} models"),
        Err(e) => eprintln!("[provider-sarvam] post-register refresh failed ({e})"),
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
    let cache = ScaffoldCache::new();
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(read_timeout())
        .build()
        .expect("reqwest client");

    let aborts = StreamAborts::new();

    iii.register_function(
        surface::STREAM_ID,
        typed_async_with_bad_request(
            make_stream(iii.clone(), http.clone(), cache.clone(), aborts.clone()),
            invalid_request_from_serde,
        )
        .description(surface::STREAM_DESC)
        .metadata(json!({ "internal": true })),
    );
    iii.register_function(
        surface::ABORT_ID,
        typed_async_with_bad_request(make_abort(aborts), invalid_request_from_serde)
            .description(surface::ABORT_DESC)
            .metadata(json!({ "internal": true })),
    );
    iii.register_function(
        surface::REFRESH_MODELS_ID,
        RegisterFunction::new_async(make_refresh_models(iii.clone()))
            .description(surface::REFRESH_MODELS_DESC)
            .metadata(json!({ "internal": true })),
    );

    {
        let (iii_stt, http_stt, cache_stt) = (iii.clone(), http.clone(), cache.clone());
        iii.register_function(
            surface::TRANSCRIBE_ID,
            RegisterFunction::new_async(move |req: crate::speech::TranscribeRequest| {
                let (iii, http, cache) = (iii_stt.clone(), http_stt.clone(), cache_stt.clone());
                async move { crate::speech::transcribe(&iii, &http, &cache, req).await }
            })
            .description(surface::TRANSCRIBE_DESC)
            .metadata(json!({ "internal": true })),
        );
        let (iii_tts, http_tts, cache_tts) = (iii.clone(), http.clone(), cache.clone());
        iii.register_function(
            surface::SPEAK_ID,
            RegisterFunction::new_async(move |req: crate::speech::SpeakRequest| {
                let (iii, http, cache) = (iii_tts.clone(), http_tts.clone(), cache_tts.clone());
                async move { crate::speech::speak(&iii, &http, &cache, req).await }
            })
            .description(surface::SPEAK_DESC)
            .metadata(json!({ "internal": true })),
        );
    }
    iii.register_function(
        surface::COUNT_TOKENS_ID,
        RegisterFunction::new_async(|req: crate::count_tokens::CountTokensRequest| async move {
            crate::count_tokens::handle(req)
        })
        .description(surface::COUNT_TOKENS_DESC)
        .metadata(json!({ "internal": true })),
    );

    {
        let iii_ready = iii.clone();
        let cache_ready = cache.clone();
        iii.register_function(
            surface::ON_ROUTER_READY_ID,
            RegisterFunction::new_async(move |_event: RouterReadyEvent| {
                let iii = iii_ready.clone();
                cache_ready.invalidate();
                async move {
                    tokio::spawn(declare_and_refresh(iii));
                    Ok::<_, Error>(ProviderReadyAck { ok: true })
                }
            })
            .description(surface::ON_ROUTER_READY_DESC)
            .metadata(json!({ "internal": true })),
        );
    }
    let _ = iii.register_trigger(RegisterTriggerInput::new(
        "router::ready",
        surface::ON_ROUTER_READY_ID,
        json!({}),
    ));

    tokio::spawn(declare_and_refresh(iii));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::declaration;

    #[test]
    fn declaration_uses_credential_env_var_const() {
        assert_eq!(super::CREDENTIAL_ENV_VAR, "SARVAM_API_KEY");
        assert_eq!(
            declaration().credential_env_var.as_deref(),
            Some(super::CREDENTIAL_ENV_VAR)
        );
    }
}
