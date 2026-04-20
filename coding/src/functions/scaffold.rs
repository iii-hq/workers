use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::config::CodingConfig;
use crate::state;
use crate::templates::{
    python_worker_template, rust_worker_template, typescript_worker_template, FunctionDef,
    TriggerDef,
};

pub fn build_handler(
    iii: Arc<III>,
    config: Arc<CodingConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let iii = iii.clone();
        let config = config.clone();
        Box::pin(async move { handle(&iii, &config, payload).await })
    }
}

pub async fn handle(
    iii: &III,
    config: &CodingConfig,
    payload: Value,
) -> Result<Value, IIIError> {
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: name".to_string()))?;

    let language = payload
        .get("language")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: language".to_string()))?;

    if !config.supported_languages.contains(&language.to_string()) {
        return Err(IIIError::Handler(format!(
            "unsupported language: {}. supported: {:?}",
            language, config.supported_languages
        )));
    }

    let functions: Vec<FunctionDef> = payload
        .get("functions")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let triggers: Vec<TriggerDef> = payload
        .get("triggers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let worker_id = format!("{}_{}", name, uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("0000"));

    let worker_files = match language {
        "rust" => rust_worker_template(name, &functions, &triggers),
        "typescript" => typescript_worker_template(name, &functions, &triggers),
        "python" => python_worker_template(name, &functions, &triggers),
        _ => {
            return Err(IIIError::Handler(format!(
                "unsupported language: {}",
                language
            )));
        }
    };

    let workspace_path = format!("{}/{}", config.workspace_dir, worker_id);
    write_files_to_disk(&workspace_path, &worker_files.files, config.max_file_size_kb)?;

    let files_json: Vec<Value> = worker_files
        .files
        .iter()
        .map(|f| {
            serde_json::json!({
                "path": f.path,
                "content": f.content,
                "language": f.language,
            })
        })
        .collect();

    let worker_state = serde_json::json!({
        "worker_id": worker_id,
        "name": name,
        "language": language,
        "functions": functions,
        "triggers": triggers,
        "files": files_json,
        "workspace_path": workspace_path,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });

    state::state_set(iii, "coding:workers", &worker_id, worker_state).await?;

    Ok(serde_json::json!({
        "worker_id": worker_id,
        "files": files_json,
        "function_count": functions.len(),
        "trigger_count": triggers.len(),
    }))
}

fn write_files_to_disk(
    base_path: &str,
    files: &[crate::templates::GeneratedFile],
    max_file_size_kb: u64,
) -> Result<(), IIIError> {
    let max_bytes = max_file_size_kb as usize * 1024;
    for file in files {
        let full_path = format!("{}/{}", base_path, file.path);
        if let Some(parent) = std::path::Path::new(&full_path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| IIIError::Handler(format!("failed to create directory: {}", e)))?;
        }
        if file.content.len() > max_bytes {
            return Err(IIIError::Handler(format!(
                "file {} exceeds max size of {}KB",
                file.path, max_file_size_kb
            )));
        }
        std::fs::write(&full_path, &file.content)
            .map_err(|e| IIIError::Handler(format!("failed to write file {}: {}", file.path, e)))?;
    }
    Ok(())
}
