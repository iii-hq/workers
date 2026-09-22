//! Typed bus registration: `judge-laya::evaluate`, `::models::list`, `::cancel`.
use crate::{client::LayaClient, configuration::SharedConfig};
use iii_sdk::{errors::Error, IIIClient, RegisterFunction};
use judge_contract::{
    CancelRequest, CancelResponse, ErrorCode, EvaluateRequest, EvaluateResponse, ModelsRequest,
    ModelsResponse, Stats,
};
use serde_json::Value;
#[cfg(feature = "console-ui")]
use std::sync::Arc;

/// Handlers snapshot config once per call and share the loaded model.
pub fn register(iii: &IIIClient, config: SharedConfig, client: LayaClient) {
    let models_config = config.clone();
    let models_client = client.clone();
    let cancel_client = client.clone();
    let registration = RegisterFunction::new_async(move |mut payload: Value| {
        let config = config.clone();
        let client = client.clone();
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
            Ok::<EvaluateResponse, Error>(
                client
                    .with_caller_id(caller.as_deref())
                    .with_limits(snapshot.limits())
                    .with_routing(snapshot.routing())
                    .evaluate(request)
                    .await,
            )
        }
    });
    let request_schema = serde_json::to_value(schemars::schema_for!(EvaluateRequest))
        .expect("laya request schema serializes");
    iii.register_function(crate::EVALUATE_ID, registration.request_format(request_schema).description("Evaluate Noul, Choice and Score questions against arbitrary JSON state with the laya model running inside this worker. Results are atomic; usage counts encoder tokens. No credentials or endpoints are accepted in the request."));

    let registration = RegisterFunction::new_async(move |mut payload: Value| {
        let config = models_config.clone();
        let client = models_client.clone();
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
            Ok::<ModelsResponse, Error>(
                client
                    .with_caller_id(caller.as_deref())
                    .with_limits(snapshot.limits())
                    .list_models(request)
                    .await,
            )
        }
    });
    let request_schema = serde_json::to_value(schemars::schema_for!(ModelsRequest))
        .expect("laya models request schema serializes");
    iii.register_function(crate::MODELS_ID, registration.request_format(request_schema).description("Describe the loaded laya checkpoints (name, encoder, revision, context window); the first card is the default. Performs no inference."));

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
        .expect("laya cancel request schema serializes");
    iii.register_function(crate::CANCEL_ID, registration.request_format(request_schema).description("Signal cancellation of an active evaluation owned by the calling worker; the current batch finishes, later batches are skipped. Requires the same worker replica as the original call."));
}

// The engine stamps this trusted transport field into top-level objects.
fn take_caller_id(payload: &mut Value) -> Option<String> {
    match payload.as_object_mut()?.remove("_caller_worker_id")? {
        Value::String(caller) => Some(caller),
        _ => None,
    }
}

#[cfg(feature = "console-ui")]
pub fn register_console_ui(iii: &Arc<IIIClient>) {
    crate::ui::register(iii);
}
