//! Forward `judge::*` calls to the selected `judge-<provider>` worker.
//!
//! The hub owns no credentials, limits or cancellation state. It validates the
//! routing fields (`provider`, `request_id`, caller metadata), scopes the
//! request id to the original caller so provider-side cancellation keeps its
//! ownership semantics, and enforces the shared response contract on whatever
//! the provider returns.
use crate::configuration::SharedConfig;
use iii_sdk::{errors::Error, protocol::TriggerRequest, IIIClient, RegisterFunction};
use judge_contract::{
    provider_function_id, CancelResponse, ErrorCode, EvaluateRequest, EvaluateResponse,
    ModelsRequest, ModelsResponse, Stats, CANCEL_FUNCTION_ID, FUNCTION_ID, MODELS_FUNCTION_ID,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::sync::Arc;

pub const DEFAULT_PROVIDER: &str = "typesafe";
/// The shared contract's `request_id` schema bound; providers accept longer
/// ids only because the hub prefixes them.
const MAX_REQUEST_ID_BYTES: usize = 128;
/// Bus slack over the caller's whole-call deadline, which the provider enforces.
const FORWARD_SLACK_MS: u64 = 5_000;
const MODELS_DEFAULT_TIMEOUT_MS: u64 = 30_000;
const CANCEL_TIMEOUT_MS: u64 = 10_000;

/// `judge-<provider>` suffixes are worker names: lowercase, digits and hyphens.
pub fn validate_provider(provider: &str) -> Result<(), ErrorCode> {
    let valid = !provider.is_empty()
        && provider.len() <= 64
        && provider
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    valid.then_some(()).ok_or(ErrorCode::InvalidRequest)
}

/// Register `judge::evaluate`, `judge::models::list` and `judge::cancel`.
/// Each call snapshots `config` for the `judge-<provider>` worker used when a
/// request omits `provider`; that worker need not be running at registration.
pub fn register(iii: &Arc<IIIClient>, config: SharedConfig) {
    let (engine, cell) = (iii.clone(), config.clone());
    let registration = RegisterFunction::new_async(move |payload: Value| {
        let (engine, cell) = (engine.clone(), cell.clone());
        async move {
            let provider = cell.read().await.provider.clone();
            let timeout = timeout_ms(&payload)
                .unwrap_or(0)
                .saturating_add(FORWARD_SLACK_MS);
            let reply = forward(&engine, &provider, FUNCTION_ID, payload, timeout).await?;
            Ok::<EvaluateResponse, Error>(reply.and_then(typed).unwrap_or_else(|code| {
                EvaluateResponse::Error {
                    code,
                    http_status: None,
                    provider_error: None,
                    retry_after_ms: None,
                    stats: Stats::default(),
                }
            }))
        }
    });
    iii.register_function(FUNCTION_ID, registration.request_format(request_schema::<EvaluateRequest>()).description("Evaluate Noul, Choice and Score questions against arbitrary JSON state through the selected judge-<provider> worker. Results are atomic; stats retain known accepted usage. No credentials are accepted in the request."));

    let (engine, cell) = (iii.clone(), config.clone());
    let registration = RegisterFunction::new_async(move |payload: Value| {
        let (engine, cell) = (engine.clone(), cell.clone());
        async move {
            let provider = cell.read().await.provider.clone();
            let timeout = timeout_ms(&payload)
                .unwrap_or(MODELS_DEFAULT_TIMEOUT_MS)
                .saturating_add(FORWARD_SLACK_MS);
            let reply = forward(&engine, &provider, MODELS_FUNCTION_ID, payload, timeout).await?;
            Ok::<ModelsResponse, Error>(reply.and_then(typed).unwrap_or_else(|code| {
                ModelsResponse::Error {
                    code,
                    http_status: None,
                    provider_error: None,
                    retry_after_ms: None,
                    stats: Stats::default(),
                }
            }))
        }
    });
    iii.register_function(MODELS_FUNCTION_ID, registration.request_format(request_schema::<ModelsRequest>()).description("List the selected provider's available model names, descriptions and release dates; performs no inference."));

    let (engine, cell) = (iii.clone(), config);
    let registration = RegisterFunction::new_async(move |payload: Value| {
        let (engine, cell) = (engine.clone(), cell.clone());
        async move {
            let provider = cell.read().await.provider.clone();
            let reply = forward(
                &engine,
                &provider,
                CANCEL_FUNCTION_ID,
                payload,
                CANCEL_TIMEOUT_MS,
            )
            .await?;
            Ok::<CancelResponse, Error>(
                reply
                    .and_then(typed)
                    .unwrap_or_else(|code| CancelResponse::Error { code }),
            )
        }
    });
    iii.register_function(CANCEL_FUNCTION_ID, registration.request_format(request_schema::<judge_contract::CancelRequest>()).description("Signal cancellation of an active evaluation or model listing started by the calling worker through the same provider. Returns whether a signal was accepted; does not roll back provider work."));
}

fn timeout_ms(payload: &Value) -> Option<u64> {
    payload.get("timeout_ms").and_then(Value::as_u64)
}

fn typed<T: DeserializeOwned>(value: Value) -> Result<T, ErrorCode> {
    serde_json::from_value(value).map_err(|_| ErrorCode::InvalidResponse)
}

/// Shared contract schema plus the hub-only `provider` selector.
fn request_schema<T: schemars::JsonSchema>() -> Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(T)).expect("judge request schema serializes");
    schema["properties"]["provider"] = json!({
        "type": "string",
        "pattern": "^[a-z0-9-]{1,64}$",
        "description": "Provider worker suffix (judge-<provider>). Defaults to the hub's JUDGE_PROVIDER."
    });
    schema
}

/// `Ok(Err(code))` is a typed judge error; `Err` is a bus failure the caller
/// handles separately, exactly as with a direct provider call.
async fn forward(
    iii: &IIIClient,
    default_provider: &str,
    public_id: &str,
    mut payload: Value,
    timeout_ms: u64,
) -> Result<Result<Value, ErrorCode>, Error> {
    let Some(fields) = payload.as_object_mut() else {
        return Ok(Err(ErrorCode::InvalidRequest));
    };
    // Engine-stamped and trusted; the provider sees the hub under this name.
    let caller = match fields.remove("_caller_worker_id") {
        Some(Value::String(caller)) if !caller.trim().is_empty() => Some(caller),
        _ => None,
    };
    let provider = match fields.remove("provider") {
        None | Some(Value::Null) => default_provider.to_owned(),
        Some(Value::String(provider)) => provider,
        Some(_) => return Ok(Err(ErrorCode::InvalidRequest)),
    };
    if let Err(code) = validate_provider(&provider) {
        return Ok(Err(code));
    }
    // Providers scope cancellation by (engine caller, request_id). Through the
    // hub the engine caller is always the hub, so the original caller becomes
    // part of the id. A direct provider call cannot forge that prefix: its own
    // engine-stamped caller differs. Non-string ids reach the provider's strict
    // parser untouched.
    let scoped = match fields.get("request_id") {
        Some(Value::String(id)) => {
            let Some(caller) = caller.as_deref() else {
                return Ok(Err(ErrorCode::InvalidRequest));
            };
            if id.len() > MAX_REQUEST_ID_BYTES {
                return Ok(Err(ErrorCode::InvalidRequest));
            }
            Some(format!("{caller}/{id}"))
        }
        _ => None,
    };
    if let Some(scoped) = scoped {
        fields.insert("request_id".into(), Value::String(scoped));
    }
    let request = TriggerRequest {
        function_id: provider_function_id(&provider, public_id),
        payload,
        action: None,
        timeout_ms: Some(timeout_ms),
    };
    match iii.trigger(request).await {
        Ok(reply) => Ok(Ok(reply)),
        Err(Error::Remote { code, .. }) if code.eq_ignore_ascii_case("function_not_found") => {
            Ok(Err(ErrorCode::ProviderUnavailable))
        }
        Err(error) => Err(error),
    }
}
