//! Typed bus registration: `judge-decider::evaluate`, `::models::list`, `::cancel`.
use crate::{client::DeciderClient, configuration::SharedConfig};
use iii_sdk::{errors::Error, runtime::FunctionRef, IIIClient, RegisterFunction};
use judge_contract::{
    CancelRequest, CancelResponse, ErrorCode, EvaluateRequest, EvaluateResponse, ModelsRequest,
    ModelsResponse, Stats,
};
use serde_json::Value;
#[cfg(feature = "console-ui")]
use std::sync::Arc;

/// Handlers snapshot config once per call and share the loaded model. The
/// returned handles unregister them, which releases the model once in-flight
/// calls end.
pub fn register(iii: &IIIClient, config: SharedConfig, client: DeciderClient) -> Vec<FunctionRef> {
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
                    .evaluate(request)
                    .await,
            )
        }
    });
    let request_schema = serde_json::to_value(schemars::schema_for!(EvaluateRequest))
        .expect("decider request schema serializes");
    let evaluate = iii.register_function(crate::EVALUATE_ID, registration.request_format(request_schema).description("Evaluate Noul, Choice and Score questions against arbitrary JSON state with decider-4b (a GGUF LLM fine-tuned for typed decisions, read at the option labels) running inside this worker. Results are atomic; usage counts decoded prompt tokens. No credentials or endpoints are accepted in the request."));

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
        .expect("decider models request schema serializes");
    let models = iii.register_function(crate::MODELS_ID, registration.request_format(request_schema).description("Describe the loaded decider model (name, pinned revision, device, context window); performs no inference."));

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
        .expect("decider cancel request schema serializes");
    let cancel = iii.register_function(crate::CANCEL_ID, registration.request_format(request_schema).description("Signal cancellation of an active evaluation owned by the calling worker; the current batch finishes, later batches are skipped. Requires the same worker replica as the original call."));
    vec![evaluate, models, cancel]
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
