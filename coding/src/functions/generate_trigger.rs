use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::templates::{
    generate_trigger_code_python, generate_trigger_code_rust, generate_trigger_code_typescript,
    TriggerDef,
};

pub fn build_handler(
    _iii: Arc<III>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| Box::pin(async move { handle(payload).await })
}

pub async fn handle(payload: Value) -> Result<Value, IIIError> {
    let function_id = payload
        .get("function_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: function_id".to_string()))?;

    let trigger_type = payload
        .get("trigger_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: trigger_type".to_string()))?;

    let config = payload
        .get("config")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    let language = payload
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("rust");

    match trigger_type {
        "http" | "cron" | "durable::subscriber" => {}
        _ => {
            return Err(IIIError::Handler(format!(
                "unsupported trigger type: {}. supported: http, cron, durable::subscriber",
                trigger_type
            )));
        }
    }

    let trigger_def = TriggerDef {
        trigger_type: trigger_type.to_string(),
        function_id: function_id.to_string(),
        config: config.clone(),
    };

    let registration_code = match language {
        "rust" => generate_trigger_code_rust(&trigger_def),
        "typescript" => generate_trigger_code_typescript(&trigger_def),
        "python" => generate_trigger_code_python(&trigger_def),
        _ => {
            return Err(IIIError::Handler(format!(
                "unsupported language: {}",
                language
            )));
        }
    };

    Ok(serde_json::json!({
        "trigger_type": trigger_type,
        "function_id": function_id,
        "registration_code": registration_code,
        "config": config,
    }))
}
