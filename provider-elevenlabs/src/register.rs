//! Boot wiring: function surface, the router::ready rebind, and the
//! declare-with-backoff loop (spec § Registration lifecycle).
use crate::discovery::{make_refresh_models, refresh_models, static_models};
use crate::speech::{Voices, DEFAULT_API_URL};
use crate::surface;
use crate::{router_client, state, PROVIDER_ID};
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::types::router::{
    ProviderDeclaration, ProviderDefaults, ProviderReadyAck, RouterReadyEvent,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

/// Env var the router (and, as a fallback, this provider) reads for the key.
pub const CREDENTIAL_ENV_VAR: &str = "ELEVENLABS_API_KEY";

pub fn declaration() -> ProviderDeclaration {
    ProviderDeclaration {
        id: PROVIDER_ID.into(),
        display_name: Some("ElevenLabs".into()),
        credential_env_var: Some(CREDENTIAL_ENV_VAR.into()),
        defaults: Some(ProviderDefaults {
            api_url: Some(DEFAULT_API_URL.into()),
            max_tokens: None,
            extra: BTreeMap::new(),
        }),
        config_schema: None, // the router's default {api_key, api_url, max_tokens}
        // Listing gates the router's refresh-on-config-change call, which
        // must fire so the live voices' models appear as soon as a key lands.
        supports_model_listing: Some(true),
        // A static slice so a fresh install has speech models to pick before
        // the first refresh; refresh_models replaces it with the live list.
        models: Some(static_models()),
        // Self-reported; availability mapping only, never authorization.
        worker_id: Some("provider-elevenlabs".into()),
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
                    "[provider-elevenlabs] store registration_token failed ({e}); retrying in {delay:?}"
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
                println!("[provider-elevenlabs] registered with llm-router");
                return;
            }
            Err(e) => {
                eprintln!("[provider-elevenlabs] register failed ({e}); retrying in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}

/// Register, then replace the static slice with the live model list.
/// Failures are logged and left to the next config-change refresh.
pub async fn declare_and_refresh(iii: IIIClient, http: reqwest::Client) {
    declare_with_backoff(iii.clone()).await;
    match refresh_models(&iii, &http).await {
        Ok(count) => println!("[provider-elevenlabs] catalog refreshed: {count} models"),
        Err(e) => eprintln!("[provider-elevenlabs] post-register refresh failed ({e})"),
    }
}

pub async fn register_provider(iii: IIIClient) -> Result<(), Error> {
    // Shared per-process cache for the registration token and the resolve
    // response (see llm_router::provider_scaffold::cache). Invalidated on
    // router::ready and on upstream auth errors.
    let cache = ScaffoldCache::new();
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client");
    let voices = Voices::new();

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
    }
    {
        let (iii_tts, http_tts, cache_tts, voices_tts) =
            (iii.clone(), http.clone(), cache.clone(), voices.clone());
        iii.register_function(
            surface::SPEAK_ID,
            RegisterFunction::new_async(move |req: crate::speech::SpeakRequest| {
                let (iii, http, cache, voices) = (
                    iii_tts.clone(),
                    http_tts.clone(),
                    cache_tts.clone(),
                    voices_tts.clone(),
                );
                async move { crate::speech::speak(&iii, &http, &cache, &voices, req).await }
            })
            .description(surface::SPEAK_DESC)
            .metadata(json!({ "internal": true })),
        );
    }
    iii.register_function(
        surface::REFRESH_MODELS_ID,
        RegisterFunction::new_async(make_refresh_models(iii.clone(), http.clone()))
            .description(surface::REFRESH_MODELS_DESC)
            .metadata(json!({ "internal": true })),
    );

    // Re-declare when the router restarts: bind to the router::ready trigger type.
    {
        let (iii_ready, http_ready, cache_ready) = (iii.clone(), http.clone(), cache.clone());
        iii.register_function(
            surface::ON_ROUTER_READY_ID,
            RegisterFunction::new_async(move |_event: RouterReadyEvent| {
                let (iii, http) = (iii_ready.clone(), http_ready.clone());
                cache_ready.invalidate();
                async move {
                    tokio::spawn(declare_and_refresh(iii, http));
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

    // Boot declare, off the boot path (a missing router must not block boot).
    tokio::spawn(declare_and_refresh(iii, http));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::declaration;

    #[test]
    fn declaration_names_the_key_and_ships_speech_models() {
        let d = declaration();
        assert_eq!(super::CREDENTIAL_ENV_VAR, "ELEVENLABS_API_KEY");
        assert_eq!(
            d.credential_env_var.as_deref(),
            Some(super::CREDENTIAL_ENV_VAR)
        );
        let models = d.models.expect("static slice");
        assert!(models.iter().all(|m| m.speech.is_some()));
        assert!(models.iter().any(|m| m.id == "scribe_v1"));
    }
}
