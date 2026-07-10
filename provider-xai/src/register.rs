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
        display_name: Some("xAI".into()),
        credential_env_var: Some("XAI_API_KEY".into()),
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
        // Identity prompt served to agents via router::system_prompt::get;
        // operators can override or disable it in the llm-router config slice.
        system_prompt: Some(include_str!("../prompts/identity.txt").to_string()),
        // Self-reported; availability mapping only, never authorization.
        worker_id: Some("provider-xai".into()),
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
                    "[provider-xai] store registration_token failed ({e}); retrying in {delay:?}"
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
                println!("[provider-xai] registered with llm-router");
                return;
            }
            Err(e) => {
                eprintln!("[provider-xai] register failed ({e}); retrying in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(10));
    }
}

/// Register, then populate the catalog from the live API. The declaration
/// carries no models, so the slice is empty until this refresh lands;
/// failures are logged and left to the next config-change refresh.
pub async fn declare_and_refresh(iii: IIIClient, http: reqwest::Client) {
    declare_with_backoff(iii.clone()).await;
    match refresh_models(&iii, &http).await {
        Ok(count) => println!("[provider-xai] catalog refreshed: {count} models"),
        Err(e) => eprintln!("[provider-xai] post-register refresh failed ({e})"),
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
    // Streaming uses no total timeout (the router owns stream budgets), but
    // reads are silence-bounded: a stalled upstream otherwise pings the router
    // past its idle guard until the engine kills the call at stream_timeout.
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(read_timeout())
        .build()
        .expect("reqwest client");

    // Worker-config cell (Agent Tools toggle): register its schema + hot-reload
    // trigger so operators flip live X/web tools in the console sidebar. The
    // schema register + first fetch run off the boot path (the configuration
    // worker may not be up yet).
    let cell = crate::configuration::new_cell();
    if let Err(e) = crate::configuration::register_config_trigger(&iii, cell.clone()) {
        eprintln!("[provider-xai] config-change trigger registration failed ({e})");
    }
    {
        let (iii_cfg, cell_cfg) = (iii.clone(), cell.clone());
        tokio::spawn(async move {
            // Retry until the configuration worker is reachable, so the console
            // config is eventually loaded and `make_stream` never serves the
            // default snapshot forever when the config bus was down at boot.
            let mut delay = Duration::from_millis(500);
            loop {
                match crate::configuration::register_config(&iii_cfg).await {
                    Ok(()) => {
                        crate::configuration::reconcile(&iii_cfg, &cell_cfg).await;
                        return;
                    }
                    Err(e) => {
                        eprintln!(
                            "[provider-xai] config bootstrap failed ({e}); retrying in {delay:?}"
                        );
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(Duration::from_secs(10));
                    }
                }
            }
        });
    }

    iii.register_function(
        surface::STREAM_ID,
        RegisterFunction::new_async_with_bad_request(
            make_stream(iii.clone(), http.clone(), cell.clone()),
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
                let (iii, http) = (iii_ready.clone(), http_ready.clone());
                async move {
                    tokio::spawn(declare_and_refresh(iii, http));
                    Ok::<_, Error>(ProviderReadyAck { ok: true })
                }
            })
            .description(surface::ON_ROUTER_READY_DESC),
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
        assert!(prompt.contains("## Autonomy and persistence"));
    }
}
