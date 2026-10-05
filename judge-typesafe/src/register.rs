//! Typed bus registration and console configuration-form assets.
use crate::secrets::{bus_fetch, SecretCache};
use crate::{client::JevClient, configuration::SharedConfig};
use iii_sdk::{errors::Error, IIIClient, RegisterFunction};
use judge_contract::{
    CancelRequest, CancelResponse, ErrorCode, EvaluateRequest, EvaluateResponse, ModelsRequest,
    ModelsResponse, ProviderError, Stats,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// Register evaluation, model listing and caller-scoped cancellation. Provider
/// handlers snapshot config once and share this client's transport/permits.
/// A missing key is an ordinary typed error,
/// so schema capture and worker readiness never require provider credentials.
/// Returns the cache `secret://` keys resolve through; bind it to
/// `secrets::changed` with [`crate::secrets::register_secret_trigger`].
pub fn register(iii: &IIIClient, config: SharedConfig, client: JevClient) -> Arc<SecretCache> {
    let secrets = Arc::new(SecretCache::new(bus_fetch(iii.clone())));
    let models_config = config.clone();
    let models_client = client.clone();
    let models_secrets = secrets.clone();
    let cancel_client = client.clone();
    let evaluate_secrets = secrets.clone();
    let registration = RegisterFunction::new_async(move |mut payload: Value| {
        let config = config.clone();
        let client = client.clone();
        let secrets = evaluate_secrets.clone();
        async move {
            let caller = take_caller_id(&mut payload);
            let request = match serde_json::from_value::<EvaluateRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return Ok(EvaluateResponse::Error {
                        code: ErrorCode::InvalidRequest,
                        http_status: None,
                        provider_error: None,
                        retry_after_ms: None,
                        stats: Stats::default(),
                    });
                }
            };
            let snapshot = config.read().await.clone();
            let api_key = match secrets.configured_key(snapshot.api_key.as_deref()).await {
                Ok(api_key) => api_key,
                Err(reason) => {
                    return Ok(EvaluateResponse::Error {
                        code: ErrorCode::MissingKey,
                        http_status: None,
                        provider_error: Some(credential_error(reason)),
                        retry_after_ms: None,
                        stats: Stats::default(),
                    })
                }
            };
            Ok::<EvaluateResponse, Error>(
                client
                    .with_caller_id(caller.as_deref())
                    .with_api_key(api_key.as_deref())
                    .with_limits(snapshot.execution_limits())
                    .evaluate(request, &snapshot.model)
                    .await,
            )
        }
    });
    // The raw transport wrapper keeps a typed response and explicitly restores
    // the shared public request schema, as the provider registration helpers do.
    let request_schema = serde_json::to_value(schemars::schema_for!(EvaluateRequest))
        .expect("JEV request schema serializes");
    iii.register_function(crate::EVALUATE_ID, registration.request_format(request_schema).description("Evaluate Noul, Choice and Score questions against arbitrary JSON state using JEV. Results are atomic; stats retain known accepted usage and mark unknown counters incomplete. No credentials are accepted in the request.").metadata(provider_metadata()));

    let registration = RegisterFunction::new_async(move |mut payload: Value| {
        let config = models_config.clone();
        let client = models_client.clone();
        let secrets = models_secrets.clone();
        async move {
            let caller = take_caller_id(&mut payload);
            let request = match serde_json::from_value::<ModelsRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return Ok(ModelsResponse::Error {
                        code: ErrorCode::InvalidRequest,
                        http_status: None,
                        provider_error: None,
                        retry_after_ms: None,
                        stats: Stats::default(),
                    })
                }
            };
            let snapshot = config.read().await.clone();
            let api_key = match secrets.configured_key(snapshot.api_key.as_deref()).await {
                Ok(api_key) => api_key,
                Err(reason) => {
                    return Ok(ModelsResponse::Error {
                        code: ErrorCode::MissingKey,
                        http_status: None,
                        provider_error: Some(credential_error(reason)),
                        retry_after_ms: None,
                        stats: Stats::default(),
                    })
                }
            };
            Ok::<ModelsResponse, Error>(
                client
                    .with_caller_id(caller.as_deref())
                    .with_api_key(api_key.as_deref())
                    .with_limits(snapshot.execution_limits())
                    .list_models(request)
                    .await,
            )
        }
    });
    let request_schema = serde_json::to_value(schemars::schema_for!(ModelsRequest))
        .expect("JEV models request schema serializes");
    iii.register_function(crate::MODELS_ID, registration.request_format(request_schema).description("List the provider's available model names, descriptions and release dates. Shares evaluation credentials, transport limits and permits; performs no inference.").metadata(provider_metadata()));

    let registration = RegisterFunction::new_async(move |mut payload: Value| {
        let client = cancel_client.clone();
        async move {
            let caller = take_caller_id(&mut payload);
            let request = match serde_json::from_value::<CancelRequest>(payload) {
                Ok(request) => request,
                Err(_) => {
                    return Ok(CancelResponse::Error {
                        code: ErrorCode::InvalidRequest,
                    });
                }
            };
            Ok::<CancelResponse, Error>(client.with_caller_id(caller.as_deref()).cancel(request))
        }
    });
    let request_schema = serde_json::to_value(schemars::schema_for!(CancelRequest))
        .expect("JEV cancel request schema serializes");
    iii.register_function(crate::CANCEL_ID, registration.request_format(request_schema).description("Signal cancellation of an active evaluation or model listing owned by the calling worker. Returns whether a signal was accepted; does not roll back provider work. Requires the same worker replica as the original call.").metadata(provider_metadata()));
    secrets
}

/// `missing_key` diagnostics for a `secret://` key that did not resolve: the reason
/// names the secret and the fix, never a value.
fn credential_error(reason: String) -> ProviderError {
    ProviderError {
        detail: None,
        message: Some(reason),
        truncated: false,
    }
}

/// Callers go through the `judge` hub, which selects the provider and checks
/// its replies, so the provider surface stays out of default discovery
/// (`engine::functions::list` without `include_internal`). It is still
/// callable by id.
fn provider_metadata() -> Value {
    json!({ "internal": true })
}

// The engine stamps this trusted transport field into top-level objects. Remove
// only that field before strict public parsing. Missing/malformed metadata must
// explicitly clear the direct client's local identity at the bus boundary.
fn take_caller_id(payload: &mut Value) -> Option<String> {
    match payload.as_object_mut()?.remove("_caller_worker_id")? {
        Value::String(caller) => Some(caller),
        _ => None,
    }
}

/// Register the configuration form supplied by the UI bundle.
#[cfg(feature = "console-ui")]
pub fn register_console_ui(iii: &Arc<IIIClient>) {
    crate::ui::register(iii);
}
