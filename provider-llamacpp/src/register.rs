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
use llm_router::types::router::{
    ProviderDeclaration, ProviderDefaults, ProviderReadyAck, RouterReadyEvent,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

pub fn declaration() -> ProviderDeclaration {
    ProviderDeclaration {
        id: PROVIDER_ID.into(),
        display_name: Some("llama.cpp".into()),
        credential_env_var: Some("LLAMACPP_API_KEY".into()),
        defaults: Some(ProviderDefaults {
            api_url: Some(DEFAULT_API_URL.into()),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
            extra: BTreeMap::new(),
        }),
        config_schema: None, // the router's default {api_key, api_url, max_tokens}
        // llama.cpp's `GET /v1/models` is a real listing endpoint (unlike
        // Z.AI/OpenAI-cloud); this also gates the router's
        // refresh-on-config-change call, which must fire so the catalog
        // appears the moment an operator points api_url at a running server.
        supports_model_listing: Some(true),
        // No static slice: refresh_models discovers the catalog live from
        // the resolved server's `/v1/models` + `/props` right after
        // registration (see declare_and_refresh) — no credential required.
        models: None,
        // Identity prompt served to agents via router::system_prompt::get;
        // operators can override or disable it in the llm-router config slice.
        system_prompt: Some(include_str!("../prompts/identity.txt").to_string()),
        // Self-reported; availability mapping only, never authorization.
        worker_id: Some("provider-llamacpp".into()),
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
    // Streaming uses no total timeout (the router owns stream budgets);
    // connect failures surface fast.
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client");

    iii.register_function(
        surface::STREAM_ID,
        RegisterFunction::new_async_with_bad_request(
            make_stream(iii.clone(), http.clone()),
            invalid_request_from_serde,
        )
        .description(surface::STREAM_DESC),
    );
    iii.register_function(
        surface::REFRESH_MODELS_ID,
        RegisterFunction::new_async(make_refresh_models(iii.clone(), http.clone()))
            .description(surface::REFRESH_MODELS_DESC),
    );

    // Re-declare when the router restarts: bind to the router::ready trigger type.
    {
        let iii_ready = iii.clone();
        let http_ready = http.clone();
        iii.register_function(
            surface::ON_ROUTER_READY_ID,
            RegisterFunction::new_async(move |_event: RouterReadyEvent| {
                let iii = iii_ready.clone();
                let http = http_ready.clone();
                async move {
                    tokio::spawn(declare_and_refresh(iii, http));
                    Ok::<_, Error>(ProviderReadyAck { ok: true })
                }
            })
            .description(surface::ON_ROUTER_READY_DESC),
        );
    }

    {
        let iii_embed = iii.clone();
        let http_embed = http.clone();
        iii.register_function(
            surface::EMBED_ID,
            RegisterFunction::new_async(move |req: crate::embed::EmbedRequest| {
                let (iii, http) = (iii_embed.clone(), http_embed.clone());
                async move { crate::embed::handle(&iii, &http, req).await }
            })
            .description(surface::EMBED_DESC)
            .metadata(json!({ "internal": true })),
        );
    }
    let _ = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "router::ready".into(),
        function_id: surface::ON_ROUTER_READY_ID.into(),
        config: json!({}),
        metadata: None,
    });

    // Boot declare, off the boot path (a missing router must not block boot).
    tokio::spawn(declare_and_refresh(iii, http));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::declaration;

    /// The declared identity prompt is the embedded prompts/identity.txt and
    /// keeps the invariants the harness pins on its default prompt.
    #[test]
    fn declaration_ships_the_identity_prompt() {
        let prompt = declaration().system_prompt.expect("declared prompt");
        assert_eq!(prompt, include_str!("../prompts/identity.txt"));
        assert!(prompt.starts_with("You are an iii agent worker."));
        assert!(prompt.contains("agent_trigger"));
        assert!(prompt.contains("IMPORTANT: NEVER invent function ids"));
    }
}
