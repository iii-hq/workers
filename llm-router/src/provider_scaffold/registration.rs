//! Typed registration helpers shared by router provider workers.
//!
//! iii-sdk 0.23 maps typed payload deserialization failures to its generic
//! `invocation_failed` wire error. Providers have an older, stable
//! `provider/invalid_request` contract. This adapter keeps the SDK's generated
//! request and response schemas while letting each provider map malformed
//! payloads to its public error code.

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::RegisterFunction;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

/// Register a typed async handler with a caller-owned deserialization error.
pub fn typed_async_with_bad_request<F, T, Fut, R, B>(
    handler: F,
    on_bad_request: B,
) -> RegisterFunction
where
    F: Fn(T) -> Fut + Send + Sync + 'static,
    T: DeserializeOwned + JsonSchema + Send + 'static,
    Fut: Future<Output = Result<R, Error>> + Send + 'static,
    R: Serialize + JsonSchema + Send + 'static,
    B: Fn(serde_json::Error) -> Error + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    let on_bad_request = Arc::new(on_bad_request);
    let registration = RegisterFunction::new_async(move |payload: Value| {
        let handler = Arc::clone(&handler);
        let on_bad_request = Arc::clone(&on_bad_request);
        async move {
            let input = serde_json::from_value(payload).map_err(|error| on_bad_request(error))?;
            handler(input).await
        }
    });

    let request_format = serde_json::to_value(
        schemars::r#gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<T>(),
    )
    .expect("typed request schema serializes");

    registration.request_format(request_format)
}
