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
use llm_router::types::router::{
    ProviderDeclaration, ProviderDefaults, ProviderReadyAck, RouterReadyEvent,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

/// Env var the router (and, as a fallback, this provider) reads for the key.
pub const CREDENTIAL_ENV_VAR: &str = "OPENCODE_GO_API_KEY";

pub fn declaration() -> ProviderDeclaration {
    ProviderDeclaration {
        id: PROVIDER_ID.into(),
        display_name: Some("OpenCode Go".into()),
        credential_env_var: Some(CREDENTIAL_ENV_VAR.into()),
        defaults: Some(ProviderDefaults {
            api_url: Some(DEFAULT_API_URL.into()),
            max_tokens: Some(DEFAULT_MAX_TOKENS),
            extra: BTreeMap::new(),
        }),
        config_schema: None,
        supports_model_listing: Some(true),
        models: None,
        system_prompt: Some(include_str!("../prompts/identity.txt").to_string()),
        worker_id: Some("provider-opencode-go".into()),
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
                    "[provider-opencode-go] store registration_token failed ({e}); retrying in {delay:?}"
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
                println!("[provider-opencode-go] registered with llm-router");
                return;
            }
            Err(e) => {
                eprintln!("[provider-opencode-go] register failed ({e}); retrying in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}

/// Register, then populate the catalog from the live API.
pub async fn declare_and_refresh(iii: IIIClient, http: reqwest::Client) {
    declare_with_backoff(iii.clone()).await;
    match refresh_models(&iii, &http).await {
        Ok(count) => println!("[provider-opencode-go] catalog refreshed: {count} models"),
        Err(e) => eprintln!("[provider-opencode-go] post-register refresh failed ({e})"),
    }
}

/// Upstream read-silence bound, overridable via `PROVIDER_READ_TIMEOUT_SECS`.
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
        RegisterFunction::new_async_with_bad_request(
            make_stream(iii.clone(), http.clone(), cache.clone(), aborts.clone()),
            invalid_request_from_serde,
        )
        .description(surface::STREAM_DESC)
        .metadata(json!({ "internal": true })),
    );
    iii.register_function(
        surface::ABORT_ID,
        RegisterFunction::new_async_with_bad_request(
            make_abort(aborts),
            invalid_request_from_serde,
        )
        .description(surface::ABORT_DESC)
        .metadata(json!({ "internal": true })),
    );
    iii.register_function(
        surface::REFRESH_MODELS_ID,
        RegisterFunction::new_async(make_refresh_models(iii.clone(), http.clone()))
            .description(surface::REFRESH_MODELS_DESC)
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
    let _ = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "router::ready".into(),
        function_id: surface::ON_ROUTER_READY_ID.into(),
        config: json!({}),
        metadata: None,
    });

    // Boot declare, off the boot path.
    tokio::spawn(declare_and_refresh(iii, http));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::declaration;

    #[test]
    fn declaration_ships_the_identity_prompt() {
        let prompt = declaration().system_prompt.expect("declared prompt");
        assert!(prompt.starts_with("You are an OpenCode Go iii agent worker."));
        assert!(prompt.contains("agent_trigger"));
        assert!(prompt.contains("## Autonomy and persistence"));
    }

    #[test]
    fn declaration_uses_credential_env_var_const() {
        assert_eq!(super::CREDENTIAL_ENV_VAR, "OPENCODE_GO_API_KEY");
        assert_eq!(
            declaration().credential_env_var.as_deref(),
            Some(super::CREDENTIAL_ENV_VAR)
        );
    }
}
