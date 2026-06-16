//! Boot wiring: function surface, the router::ready rebind, and the
//! declare-with-backoff loop (spec § Registration lifecycle).
use crate::config::{DEFAULT_API_URL, DEFAULT_MAX_TOKENS};
use crate::discovery::{make_refresh_models, refresh_models};
use crate::stream_fn::make_stream;
use crate::{router_client, state, PROVIDER_ID};
use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, III};
use llm_router::types::router::{
    NoParams, ProviderAck, ProviderDeclaration, ProviderDefaults, ProviderStreamInput,
    RefreshModelsAck,
};
use llm_router::wire_schema::with_schemas;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

pub fn declaration() -> ProviderDeclaration {
    ProviderDeclaration {
        id: PROVIDER_ID.into(),
        display_name: Some("OpenAI".into()),
        credential_env_var: Some("OPENAI_API_KEY".into()),
        defaults: Some(ProviderDefaults {
            api_url: Some(DEFAULT_API_URL.into()),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
            extra: BTreeMap::new(),
        }),
        config_schema: None, // the router's default {api_key, api_url, max_tokens}
        supports_model_listing: Some(true),
        // No static slice: GET /v1/models is the source of truth, and a
        // refresh fires right after registration (see declare_and_refresh),
        // so the catalog fills from the API within seconds of boot.
        models: None,
        // Self-reported; availability mapping only, never authorization.
        worker_id: Some("provider-openai".into()),
    }
}

/// One registration attempt: declare (with the persisted token when present)
/// and persist the token the router returns.
pub async fn declare_once(iii: &III) -> Result<(), IIIError> {
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

async fn persist_registration_token(iii: &III, token: &str) -> Result<(), IIIError> {
    let mut delay = Duration::from_millis(200);
    for attempt in 0..5 {
        match state::store_token(iii, token).await {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 4 => {
                eprintln!(
                    "[provider-openai] store registration_token failed ({e}); retrying in {delay:?}"
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
pub async fn declare_with_backoff(iii: III) {
    let mut delay = Duration::from_millis(500);
    loop {
        match declare_once(&iii).await {
            Ok(()) => {
                println!("[provider-openai] registered with llm-router");
                return;
            }
            Err(e) => {
                eprintln!("[provider-openai] register failed ({e}); retrying in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}

/// Register, then populate the catalog from the live API. The declaration
/// carries no models, so the slice is empty until this refresh lands;
/// failures are logged and left to the next config-change refresh.
pub async fn declare_and_refresh(iii: III, http: reqwest::Client) {
    declare_with_backoff(iii.clone()).await;
    match refresh_models(&iii, &http).await {
        Ok(count) => println!("[provider-openai] catalog refreshed: {count} models"),
        Err(e) => eprintln!("[provider-openai] post-register refresh failed ({e})"),
    }
}

pub async fn register_provider(iii: III) -> Result<(), IIIError> {
    // Streaming uses no total timeout (the router owns stream budgets);
    // connect failures surface fast.
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client");

    iii.register_function(
        "provider::openai::stream",
        with_schemas::<ProviderStreamInput, ProviderAck>(RegisterFunction::new_async(make_stream(
            iii.clone(),
            http.clone(),
        ))),
    );
    iii.register_function(
        "provider::openai::refresh_models",
        with_schemas::<NoParams, RefreshModelsAck>(RegisterFunction::new_async(
            make_refresh_models(iii.clone(), http.clone()),
        )),
    );

    // Re-declare when the router restarts: router::ready rides iii-pubsub.
    {
        let iii_ready = iii.clone();
        let http_ready = http.clone();
        iii.register_function(
            "provider::openai::on_router_ready",
            with_schemas::<NoParams, ProviderAck>(RegisterFunction::new_async(
                move |_raw: Value| {
                    let (iii, http) = (iii_ready.clone(), http_ready.clone());
                    async move {
                        tokio::spawn(declare_and_refresh(iii, http));
                        Ok(json!({ "ok": true }))
                    }
                },
            )),
        );
    }
    let _ = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "subscribe".into(),
        function_id: "provider::openai::on_router_ready".into(),
        config: json!({ "topic": "router::ready" }),
        metadata: None,
    });

    // Boot declare, off the boot path (a missing router must not block boot).
    tokio::spawn(declare_and_refresh(iii, http));
    Ok(())
}
