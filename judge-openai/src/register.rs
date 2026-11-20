//! Typed bus registration and console configuration-form assets.
use crate::{
    client::{DecisionsClient, credential_error},
    codex::{self, CodexSource},
    config::{Auth, OpenAiConfig},
    configuration::SharedConfig,
};
use iii_sdk::{IIIClient, RegisterFunction, errors::Error};
use judge_contract::{
    CancelRequest, CancelResponse, ErrorCode, EvaluateRequest, EvaluateResponse, ModelsRequest,
    ModelsResponse, Stats, StatusResponse,
};
use judge_provider::secrets::{SecretCache, bus_fetch};
use serde_json::{Value, json};
use std::sync::Arc;

/// Register evaluation, model listing and caller-scoped cancellation. Provider
/// handlers snapshot config once and share this client's transport/permits.
/// A missing key is an ordinary typed error,
/// so schema capture and worker readiness never require provider credentials.
/// Returns the cache `secret://` keys resolve through; evict from it on
/// `secrets::on-changed` with [`judge_provider::secrets::observe_secret_changes`].
pub fn register(
    iii: &IIIClient,
    config: SharedConfig,
    client: DecisionsClient,
) -> Arc<SecretCache> {
    let secrets = Arc::new(SecretCache::new(bus_fetch(iii.clone()), "judge-openai"));
    let codex = Arc::new(CodexSource::new(codex::bus_fetch(iii.clone())));
    let models_config = config.clone();
    let models_client = client.clone();
    let (models_secrets, models_codex) = (secrets.clone(), codex.clone());
    let cancel_client = client.clone();
    let (evaluate_secrets, evaluate_codex) = (secrets.clone(), codex.clone());
    let (status_config, status_client, status_secrets, status_codex) =
        (config.clone(), client.clone(), secrets.clone(), codex);
    let registration = RegisterFunction::new_async(move |mut payload: Value| {
        let config = config.clone();
        let client = client.clone();
        let secrets = evaluate_secrets.clone();
        let codex = evaluate_codex.clone();
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
            let client = match bind(&client, &snapshot, &secrets, &codex).await {
                Ok(client) => client,
                Err(reason) => {
                    return Ok(EvaluateResponse::Error {
                        code: ErrorCode::MissingKey,
                        http_status: None,
                        provider_error: Some(credential_error(reason)),
                        retry_after_ms: None,
                        stats: Stats::default(),
                    });
                }
            };
            Ok::<EvaluateResponse, Error>(
                client
                    .with_caller_id(caller.as_deref())
                    .with_limits(snapshot.execution_limits())
                    .evaluate(request, &snapshot.model)
                    .await,
            )
        }
    });
    // The raw transport wrapper keeps a typed response and explicitly restores
    // the shared public request schema, as the provider registration helpers do.
    let request_schema = serde_json::to_value(schemars::schema_for!(EvaluateRequest))
        .expect("OpenAI request schema serializes");
    iii.register_function(crate::EVALUATE_ID, registration.request_format(request_schema).description("Evaluate Noul, Choice and Score questions against arbitrary JSON state using the OpenAI Decisions API. Results are atomic; stats retain known accepted usage and mark unknown counters incomplete. No credentials are accepted in the request.").metadata(provider_metadata()));

    let registration = RegisterFunction::new_async(move |mut payload: Value| {
        let config = models_config.clone();
        let client = models_client.clone();
        let secrets = models_secrets.clone();
        let codex = models_codex.clone();
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
                    });
                }
            };
            let snapshot = config.read().await.clone();
            let client = match bind(&client, &snapshot, &secrets, &codex).await {
                Ok(client) => client,
                Err(reason) => {
                    return Ok(ModelsResponse::Error {
                        code: ErrorCode::MissingKey,
                        http_status: None,
                        provider_error: Some(credential_error(reason)),
                        retry_after_ms: None,
                        stats: Stats::default(),
                    });
                }
            };
            Ok::<ModelsResponse, Error>(
                client
                    .with_caller_id(caller.as_deref())
                    .with_limits(snapshot.execution_limits())
                    .list_models(request)
                    .await,
            )
        }
    });
    let request_schema = serde_json::to_value(schemars::schema_for!(ModelsRequest))
        .expect("OpenAI models request schema serializes");
    iii.register_function(crate::MODELS_ID, registration.request_format(request_schema).description("List the Decisions models this key can use (only supported ones), with descriptions and release dates; empty when none is visible. Shares evaluation credentials, transport limits and permits; performs no inference. With auth: codex it returns the built-in catalog without HTTP: success means a Codex session reaches the worker, not that OpenAI accepts it.").metadata(provider_metadata()));

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
        .expect("OpenAI cancel request schema serializes");
    iii.register_function(crate::CANCEL_ID, registration.request_format(request_schema).description("Signal cancellation of an active evaluation or model listing owned by the calling worker. Returns whether a signal was accepted; does not roll back provider work. Requires the same worker replica as the original call.").metadata(provider_metadata()));
    // What the Console's judge picker reads to say a chosen judge is not set up.
    let registration = RegisterFunction::new_async(move |_payload: Value| {
        let config = status_config.clone();
        let client = status_client.clone();
        let secrets = status_secrets.clone();
        let codex = status_codex.clone();
        async move {
            let snapshot = config.read().await.clone();
            let status = match bind(&client, &snapshot, &secrets, &codex).await {
                Err(reason) => StatusResponse {
                    configured: false,
                    reason: Some(reason),
                },
                Ok(_) if snapshot.auth == Auth::Codex => match codex.current().await {
                    Ok(()) => StatusResponse {
                        configured: true,
                        reason: None,
                    },
                    Err(reason) => StatusResponse {
                        configured: false,
                        reason: Some(reason),
                    },
                },
                Ok(client) if client.has_key() => StatusResponse {
                    configured: true,
                    reason: None,
                },
                Ok(_) => StatusResponse {
                    configured: false,
                    reason: Some(MISSING_KEY.into()),
                },
            };
            Ok::<StatusResponse, Error>(status)
        }
    });
    iii.register_function(crate::STATUS_ID, registration.request_format(json!({"type": "object"})).description("Whether evaluations hold a usable key (api_key, a secret:// reference or OPENAI_API_KEY at boot; with auth: codex, a Codex session from provider-openai-codex), with the fix when they do not. No call to OpenAI; never returns the key.").metadata(provider_metadata()));
    secrets
}

/// `status` when neither the configuration nor the boot environment holds a key.
const MISSING_KEY: &str = "no API key: save one in the judge-openai settings, or export OPENAI_API_KEY in its process and restart it";

/// Bind this call's credential: the configured key (or the boot key), or with
/// `auth: codex` the Codex source, asked inside the call. `Err` is the
/// actionable reason a `secret://` reference cannot be used.
async fn bind(
    client: &DecisionsClient,
    snapshot: &OpenAiConfig,
    secrets: &SecretCache,
    codex: &Arc<CodexSource>,
) -> Result<DecisionsClient, String> {
    match snapshot.auth {
        Auth::ApiKey => {
            let api_key = secrets.configured_key(snapshot.api_key.as_deref()).await?;
            Ok(client.with_api_key(api_key.as_deref()))
        }
        Auth::Codex => Ok(client.with_codex(codex.clone())),
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
