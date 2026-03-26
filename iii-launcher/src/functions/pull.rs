use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::IIIError;
use serde_json::Value;

use crate::adapter::RuntimeAdapter;

pub fn build_pull_handler(
    adapter: Arc<dyn RuntimeAdapter>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |payload: Value| {
        let adapter = adapter.clone();

        Box::pin(async move {
            let image = payload
                .get("image")
                .and_then(|v| v.as_str())
                .ok_or_else(|| IIIError::Handler("missing 'image' field".to_string()))?
                .to_string();

            let info = adapter
                .pull(&image)
                .await
                .map_err(|e| IIIError::Handler(format!("pull failed: {e}")))?;

            // Extract the worker manifest from /iii/worker.yaml
            let manifest_bytes = adapter
                .extract_file(&image, "/iii/worker.yaml")
                .await
                .map_err(|e| {
                    IIIError::Handler(format!("failed to extract /iii/worker.yaml: {e}"))
                })?;

            let manifest_str = String::from_utf8(manifest_bytes)
                .map_err(|e| IIIError::Handler(format!("manifest is not valid UTF-8: {e}")))?;

            let manifest: Value = serde_yaml::from_str(&manifest_str)
                .map_err(|e| IIIError::Handler(format!("failed to parse worker.yaml: {e}")))?;

            let result = serde_json::json!({
                "image": info.image,
                "manifest": manifest,
                "size_bytes": info.size_bytes,
            });

            Ok(result)
        })
    }
}
