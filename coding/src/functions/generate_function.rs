use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::state;
use crate::templates::{
    generate_single_function_python, generate_single_function_rust,
    generate_single_function_typescript, FunctionDef,
};

pub fn build_handler(
    iii: Arc<III>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        Box::pin(async move { handle(&iii, payload).await })
    }
}

pub async fn handle(iii: &III, payload: Value) -> Result<Value, IIIError> {
    let language = payload
        .get("language")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: language".to_string()))?;

    let id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: id".to_string()))?;

    let description = payload
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let request_format = payload.get("request_format").cloned();
    let response_format = payload.get("response_format").cloned();

    let func_def = FunctionDef {
        id: id.to_string(),
        description: description.to_string(),
        request_format,
        response_format,
    };

    let generated = match language {
        "rust" => generate_single_function_rust(&func_def),
        "typescript" => generate_single_function_typescript(&func_def),
        "python" => generate_single_function_python(&func_def),
        _ => {
            return Err(IIIError::Handler(format!(
                "unsupported language: {}",
                language
            )));
        }
    };

    let function_id = format!(
        "fn_{}_{}",
        id.replace("::", "_").replace('-', "_"),
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("0000")
    );

    let function_state = serde_json::json!({
        "function_id": function_id,
        "original_id": id,
        "language": language,
        "file_path": generated.path,
        "content": generated.content,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });

    state::state_set(iii, "coding:functions", &function_id, function_state).await?;

    if let Some(worker_id) = payload.get("worker_id").and_then(|v| v.as_str()) {
        let worker_state = state::state_get(iii, "coding:workers", worker_id).await;
        if let Ok(mut ws) = worker_state {
            if let Some(files) = ws.get_mut("files").and_then(|v| v.as_array_mut()) {
                files.push(serde_json::json!({
                    "path": generated.path,
                    "content": generated.content,
                    "language": generated.language,
                }));
            }
            state::state_set(iii, "coding:workers", worker_id, ws).await?;
        }
    }

    Ok(serde_json::json!({
        "function_id": function_id,
        "file_path": generated.path,
        "content": generated.content,
        "language": language,
    }))
}
