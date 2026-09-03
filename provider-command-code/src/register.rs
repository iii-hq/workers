use crate::config::{DEFAULT_API_URL, DEFAULT_MAX_TOKENS};
use crate::discovery::{make_refresh_models, refresh_models};
use crate::errors::invalid_request_from_serde;
use crate::stream_fn::make_stream;
use crate::surface;
use crate::{PROVIDER_ID, STATE_SCOPE};
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use llm_router::provider_scaffold::aborts::{make_abort, StreamAborts};
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::provider_scaffold::registration::typed_async_with_bad_request;
use llm_router::provider_scaffold::{router_client, state};
use llm_router::types::router::{
    ProviderDeclaration, ProviderDefaults, ProviderReadyAck, RouterReadyEvent,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

pub const CREDENTIAL_ENV_VAR: &str = "COMMAND_CODE_API_KEY";

pub fn declaration() -> ProviderDeclaration {
    ProviderDeclaration {
        id: PROVIDER_ID.into(),
        display_name: Some("Command Code".into()),
        credential_env_var: Some(CREDENTIAL_ENV_VAR.into()),
        defaults: Some(ProviderDefaults {
            api_url: Some(DEFAULT_API_URL.into()),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
            extra: BTreeMap::new(),
        }),
        config_schema: None,
        supports_model_listing: Some(true),
        models: None,
        worker_id: Some("provider-command-code".into()),
        // The mark the console paints beside this provider's models; the
        // router carries it verbatim in `router::provider::list`.
        icon_svg: Some(
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icon.svg")).into(),
        ),
    }
}

async fn persist_token(iii: &IIIClient, token: &str) -> Result<(), Error> {
    let mut delay = Duration::from_millis(200);
    for attempt in 0..5 {
        match state::store_token(iii, STATE_SCOPE, token).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt < 4 => {
                tracing::warn!(%error, ?delay, "failed to persist registration token; retrying");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(2));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

pub async fn declare_once(iii: &IIIClient) -> Result<(), Error> {
    let token = state::load_token(iii, STATE_SCOPE).await;
    let mut payload = serde_json::to_value(declaration()).expect("serializable declaration");
    if let Some(token) = &token {
        payload["token"] = json!(token);
    }
    let response = router_client::register(iii, payload).await?;
    if let Some(issued) = response.get("registration_token").and_then(Value::as_str) {
        if token.as_deref() != Some(issued) {
            persist_token(iii, issued).await?;
        }
    }
    Ok(())
}

pub async fn declare_with_backoff(iii: IIIClient) {
    let mut delay = Duration::from_millis(500);
    loop {
        match declare_once(&iii).await {
            Ok(()) => {
                tracing::info!("registered with llm-router");
                return;
            }
            Err(error) => {
                tracing::warn!(%error, ?delay, "provider registration failed; retrying");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}

async fn declare_and_refresh(iii: IIIClient, http: reqwest::Client) {
    declare_with_backoff(iii.clone()).await;
    match refresh_models(&iii, &http).await {
        Ok(count) => tracing::info!(count, "Command Code catalog refreshed"),
        Err(error) => tracing::warn!(%error, "post-registration model refresh failed"),
    }
}

fn read_timeout() -> Duration {
    std::env::var("PROVIDER_READ_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
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
        RegisterFunction::new_async(make_refresh_models(iii.clone(), http.clone()))
            .description(surface::REFRESH_MODELS_DESC)
            .metadata(json!({ "internal": true })),
    );
    {
        let iii_ready = iii.clone();
        let http_ready = http.clone();
        let cache_ready = cache.clone();
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
    tokio::spawn(declare_and_refresh(iii, http));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaration_uses_the_namespaced_provider_contract() {
        let declaration = declaration();
        assert_eq!(declaration.id, "command-code");
        assert_eq!(
            declaration.credential_env_var.as_deref(),
            Some("COMMAND_CODE_API_KEY")
        );
        assert_eq!(
            declaration.worker_id.as_deref(),
            Some("provider-command-code")
        );
        assert!(declaration.supports_model_listing.unwrap());
    }
}
