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
    ModelsRequest, ModelsResponse, Stats, CANCEL_FUNCTION_ID, FUNCTION_ID,
    MAX_PROVIDER_REQUEST_ID_BYTES, MODELS_FUNCTION_ID,
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
    judge_contract::is_valid_provider(provider)
        .then_some(())
        .ok_or(ErrorCode::InvalidRequest)
}

/// The provider a call goes to: the request's own `provider`, else the
/// calling session's (`PROVIDER_BAGGAGE_KEY`, stamped per turn by the
/// harness), else the hub's configured default. An unusable baggage value is
/// ignored rather than failing the call: it is an unauthenticated preference.
fn resolve_provider(
    explicit: Option<Value>,
    session: Option<&str>,
    default_provider: &str,
) -> Result<String, ErrorCode> {
    match explicit {
        Some(Value::String(provider)) => Ok(provider),
        None | Some(Value::Null) => Ok(session
            .filter(|p| judge_contract::is_valid_provider(p))
            .unwrap_or(default_provider)
            .to_owned()),
        Some(_) => Err(ErrorCode::InvalidRequest),
    }
}

/// The calling session's provider from the handler's OTel baggage.
fn session_provider() -> Option<String> {
    use opentelemetry::baggage::BaggageExt;
    opentelemetry::Context::current()
        .baggage()
        .get(judge_contract::PROVIDER_BAGGAGE_KEY)
        .map(|value| value.to_string())
}

/// Register `judge::evaluate`, `judge::models::list` and `judge::cancel`.
/// Each call snapshots `config` for the `judge-<provider>` worker used when a
/// request omits `provider`; that worker need not be running at registration.
pub fn register(iii: &Arc<IIIClient>, config: SharedConfig) {
    let (engine, cell) = (iii.clone(), config.clone());
    let registration = RegisterFunction::new_async(move |payload: Value| {
        let (engine, cell) = (engine.clone(), cell.clone());
        async move {
            let timeout = timeout_ms(&payload)
                .unwrap_or(0)
                .saturating_add(FORWARD_SLACK_MS);
            let reply = call(&engine, &cell, FUNCTION_ID, payload, timeout).await?;
            Ok::<EvaluateResponse, Error>(reply.unwrap_or_else(|code| EvaluateResponse::Error {
                code,
                http_status: None,
                provider_error: None,
                retry_after_ms: None,
                stats: Stats::default(),
            }))
        }
    });
    iii.register_function(FUNCTION_ID, registration.request_format(request_schema::<EvaluateRequest>()).description("Evaluate Noul, Choice and Score questions against arbitrary JSON state through the selected judge-<provider> worker. Results are atomic; stats retain known accepted usage. No credentials are accepted in the request."));

    let (engine, cell) = (iii.clone(), config.clone());
    let registration = RegisterFunction::new_async(move |payload: Value| {
        let (engine, cell) = (engine.clone(), cell.clone());
        async move {
            let timeout = timeout_ms(&payload)
                .unwrap_or(MODELS_DEFAULT_TIMEOUT_MS)
                .saturating_add(FORWARD_SLACK_MS);
            let reply = call(&engine, &cell, MODELS_FUNCTION_ID, payload, timeout).await?;
            Ok::<ModelsResponse, Error>(reply.unwrap_or_else(|code| ModelsResponse::Error {
                code,
                http_status: None,
                provider_error: None,
                retry_after_ms: None,
                stats: Stats::default(),
            }))
        }
    });
    // Internal: agents have no use for the catalog; iii-directory and the
    // Console read it by id (`engine::functions::list` shows it only with
    // `include_internal`).
    iii.register_function(MODELS_FUNCTION_ID, registration.request_format(request_schema::<ModelsRequest>()).description("List the selected provider's available model names, descriptions and release dates; performs no inference.").metadata(json!({ "internal": true })));

    let (engine, cell) = (iii.clone(), config);
    let registration = RegisterFunction::new_async(move |payload: Value| {
        let (engine, cell) = (engine.clone(), cell.clone());
        async move {
            let reply = call(
                &engine,
                &cell,
                CANCEL_FUNCTION_ID,
                payload,
                CANCEL_TIMEOUT_MS,
            )
            .await?;
            Ok::<CancelResponse, Error>(reply.unwrap_or_else(|code| CancelResponse::Error { code }))
        }
    });
    iii.register_function(CANCEL_FUNCTION_ID, registration.request_format(request_schema::<judge_contract::CancelRequest>()).description("Signal cancellation of an active evaluation or model listing started by the calling worker through the same provider. Returns whether a signal was accepted; does not roll back provider work."));
}

/// Snapshot the default provider, forward, and enforce the shared response
/// contract on the reply. `Ok(Err(code))` is a typed judge error.
async fn call<T: DeserializeOwned>(
    iii: &IIIClient,
    config: &SharedConfig,
    public_id: &str,
    payload: Value,
    timeout_ms: u64,
) -> Result<Result<T, ErrorCode>, Error> {
    let provider = config.read().await.provider.clone();
    Ok(forward(iii, &provider, public_id, payload, timeout_ms)
        .await?
        .and_then(|value| serde_json::from_value(value).map_err(|_| ErrorCode::InvalidResponse)))
}

fn timeout_ms(payload: &Value) -> Option<u64> {
    payload.get("timeout_ms").and_then(Value::as_u64)
}

/// Shared contract schema plus the hub-only `provider` selector.
fn request_schema<T: schemars::JsonSchema>() -> Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(T)).expect("judge request schema serializes");
    schema["properties"]["provider"] = json!({
        "type": "string",
        "pattern": "^[a-z0-9-]{1,64}$",
        "description": "Provider worker suffix (judge-<provider>). Defaults to the calling session's provider (OTel baggage iii.judge.provider), then the hub's JUDGE_PROVIDER."
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
    let provider = match resolve_provider(
        fields.remove("provider"),
        session_provider().as_deref(),
        default_provider,
    ) {
        Ok(provider) => provider,
        Err(code) => return Ok(Err(code)),
    };
    if let Err(code) = validate_provider(&provider) {
        return Ok(Err(code));
    }
    // Providers scope cancellation by (engine caller, request_id). Through the
    // hub the engine caller is always the hub, so the original caller becomes
    // part of the id, length-prefixed so `("a/b", "c")` and `("a", "b/c")` never
    // collide. A direct provider call cannot forge that prefix: its own
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
            let scoped = format!("{}:{caller}/{id}", caller.len());
            if scoped.len() > MAX_PROVIDER_REQUEST_ID_BYTES {
                return Ok(Err(ErrorCode::InvalidRequest));
            }
            Some(scoped)
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

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::baggage::BaggageExt;
    use opentelemetry::KeyValue;

    #[test]
    fn request_then_session_then_hub_default() {
        let explicit = Some(Value::String("semif".into()));
        assert_eq!(
            resolve_provider(explicit, Some("laya"), "typesafe").unwrap(),
            "semif"
        );
        assert_eq!(
            resolve_provider(None, Some("laya"), "typesafe").unwrap(),
            "laya"
        );
        assert_eq!(
            resolve_provider(Some(Value::Null), Some("laya"), "typesafe").unwrap(),
            "laya"
        );
        assert_eq!(
            resolve_provider(None, None, "typesafe").unwrap(),
            "typesafe"
        );
        // an unusable session value never fails the call
        assert_eq!(
            resolve_provider(None, Some("Bad Provider"), "typesafe").unwrap(),
            "typesafe"
        );
        assert!(resolve_provider(Some(json!(1)), None, "typesafe").is_err());
    }

    #[test]
    fn session_provider_reads_the_callers_baggage() {
        assert_eq!(session_provider(), None);
        let cx = opentelemetry::Context::current_with_baggage(vec![KeyValue::new(
            judge_contract::PROVIDER_BAGGAGE_KEY,
            "semif",
        )]);
        let _guard = cx.attach();
        assert_eq!(session_provider().as_deref(), Some("semif"));
    }
}
