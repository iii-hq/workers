use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use crate::state;

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
    let worker_id = payload
        .get("worker_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: worker_id".to_string()))?;

    let worker_state = state::state_get(iii, "coding:workers", worker_id).await?;

    let language = worker_state
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let files = worker_state
        .get("files")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));

    let name = worker_state
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(worker_id);

    let deployment_id = format!(
        "deploy_{}_{}",
        worker_id,
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("0000")
    );

    let instructions = match language {
        "rust" => format!(
            "1. cd into the worker directory\n2. Run: cargo build --release\n3. Start the worker: ./target/release/{} --url ws://<engine-host>:49134\n4. Verify registration via introspect::functions",
            name
        ),
        "typescript" => "1. cd into the worker directory\n2. Run: npm install\n3. Run: npm run build\n4. Start the worker: III_URL=ws://<engine-host>:49134 npm start\n5. Verify registration via introspect::functions".to_string(),
        "python" => "1. cd into the worker directory\n2. Run: pip install -e .\n3. Start the worker: III_URL=ws://<engine-host>:49134 python3 src/worker.py\n4. Verify registration via introspect::functions".to_string(),
        _ => "Refer to the generated files for deployment instructions.".to_string(),
    };

    let deployment_record = serde_json::json!({
        "deployment_id": deployment_id,
        "worker_id": worker_id,
        "name": name,
        "language": language,
        "deployed_at": chrono::Utc::now().to_rfc3339(),
        "instructions": instructions,
    });

    state::state_set(iii, "coding:deployments", &deployment_id, deployment_record).await?;

    Ok(serde_json::json!({
        "deployed": true,
        "worker_id": worker_id,
        "deployment_id": deployment_id,
        "files": files,
        "instructions": instructions,
    }))
}
