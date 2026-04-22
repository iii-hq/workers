use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

pub fn build_handler(
    iii: Arc<III>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let iii = iii.clone();
        Box::pin(async move { handle(&iii).await })
    }
}

pub async fn handle(iii: &III) -> Result<Value, IIIError> {
    let functions = iii.list_functions().await?;

    let entries: Vec<Value> = functions
        .iter()
        .map(|f| {
            serde_json::json!({
                "id": f.function_id,
                "description": f.description,
                "request_format": f.request_format,
                "response_format": f.response_format,
                "metadata": f.metadata,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "functions": entries,
        "count": entries.len()
    }))
}
