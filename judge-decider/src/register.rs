//! Typed bus registration: `judge-decider::evaluate`, `::models::list`, `::cancel`.
use crate::{client::DeciderClient, configuration::SharedConfig};
use iii_llama_runtime::ModelSlot;
use iii_sdk::{errors::Error, IIIClient, RegisterFunction};
use judge_contract::{
    CancelRequest, CancelResponse, ErrorCode, EvaluateRequest, EvaluateResponse, ModelsRequest,
    ModelsResponse, Stats,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// Handlers snapshot config once per call and take the model from `slot`,
/// which loads it on first use and releases it when idle
/// (`iii_llama_runtime::lifecycle`); a call waits for a load at most its own
/// budget.
///
/// Callers go through the `judge` hub, which selects the provider and checks
/// its replies, so the provider surface stays out of default discovery
/// (`engine::functions::list` without `include_internal`), as judge-typesafe's.
pub fn register(iii: &IIIClient, config: SharedConfig, slot: Arc<ModelSlot<DeciderClient>>) {
    let models_config = config.clone();
    let models_slot = slot.clone();
    let cancel_slot = slot.clone();
    let registration = RegisterFunction::new_async(move |mut payload: Value| {
        let config = config.clone();
        let slot = slot.clone();
        async move {
            let caller = take_caller_id(&mut payload);
            let mut request = match serde_json::from_value::<EvaluateRequest>(payload) {
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
            let client = match slot
                .for_request(&mut request.timeout_ms, request.expires_at_unix_ms)
                .await
            {
                Ok(client) => client,
                Err(unloaded) => return Ok(unloaded.evaluate_error()),
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
    iii.register_function(crate::EVALUATE_ID, registration.request_format(request_schema).description("Evaluate Noul, Choice and Score questions against arbitrary JSON state with decider-4b (a GGUF LLM fine-tuned for typed decisions, read at the option labels) running inside this worker. Results are atomic; usage counts decoded prompt tokens. No credentials or endpoints are accepted in the request.").metadata(json!({ "internal": true })));

    let registration = RegisterFunction::new_async(move |mut payload: Value| {
        let config = models_config.clone();
        let slot = models_slot.clone();
        async move {
            let caller = take_caller_id(&mut payload);
            let mut request = match serde_json::from_value::<ModelsRequest>(payload) {
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
            let client = match slot
                .for_request(&mut request.timeout_ms, request.expires_at_unix_ms)
                .await
            {
                Ok(client) => client,
                Err(unloaded) => return Ok(unloaded.models_error()),
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
    iii.register_function(crate::MODELS_ID, registration.request_format(request_schema).description("Describe the decider model, loading it if needed (name, pinned revision, device, context window); performs no inference.").metadata(json!({ "internal": true })));

    let registration = RegisterFunction::new_async(move |mut payload: Value| {
        let slot = cancel_slot.clone();
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
            // Nothing runs without a loaded model. ponytail: a cancel sent while
            // its call still waits for the load is not seen; that call runs to
            // its own deadline.
            let Some(client) = slot.loaded() else {
                return Ok(CancelResponse::Ok { cancelled: false });
            };
            Ok::<CancelResponse, Error>(client.with_caller_id(caller.as_deref()).cancel(request))
        }
    });
    let request_schema = serde_json::to_value(schemars::schema_for!(CancelRequest))
        .expect("decider cancel request schema serializes");
    iii.register_function(crate::CANCEL_ID, registration.request_format(request_schema).description("Signal cancellation of an active evaluation owned by the calling worker; the current batch finishes, later batches are skipped. Requires the same worker replica as the original call.").metadata(json!({ "internal": true })));
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
