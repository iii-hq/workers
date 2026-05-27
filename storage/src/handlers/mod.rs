//! RPC handlers for `storage::*` functions.

use crate::backend::Backend;
use crate::error::StorageError;
use iii_sdk::{IIIError, RegisterFunction, III};
use std::collections::HashMap;
use std::sync::Arc;

pub mod delete_object;
pub mod get_object;
pub mod presign_url;
pub mod put_object;

/// Maximum inline body size accepted by `putObject` and returned by `getObject`.
/// 10 MiB matches the spec.
pub const INLINE_BODY_CAP: u64 = 10 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    /// Keyed by the **worker-facing bucket name** (config map key).
    pub backends: Arc<HashMap<String, Arc<dyn Backend>>>,
}

impl AppState {
    pub fn backend(&self, bucket: &str) -> Result<&Arc<dyn Backend>, StorageError> {
        self.backends
            .get(bucket)
            .ok_or_else(|| StorageError::UnknownBucket {
                bucket: bucket.to_string(),
            })
    }
}

/// JSON-string error body, mirroring database's err_to_str.
pub fn err_to_str(e: StorageError) -> String {
    e.to_wire_string()
}

/// Register every `storage::*` RPC function with the engine.
///
/// Spec contract (`workers/binary-worker.md` §7): a worker exposes a single
/// `register_all` per-folder so `main.rs` stays a thin entry point.
pub fn register_all(iii: &III, state: &AppState) {
    register_put_object(iii, state);
    register_get_object(iii, state);
    register_delete_object(iii, state);
    register_presign_url(iii, state);
    tracing::info!("storage handlers registered");
}

fn register_put_object(iii: &III, state: &AppState) {
    let st = state.clone();
    iii.register_function(
        "storage::putObject",
        RegisterFunction::new_async(move |req: put_object::PutReq| {
            let st = st.clone();
            async move { put_object::handle(&st, req).await.map_err(IIIError::from) }
        })
        .description("Write an object to a configured bucket. Body is base64; max 10MB inline."),
    );
}

fn register_get_object(iii: &III, state: &AppState) {
    let st = state.clone();
    iii.register_function(
        "storage::getObject",
        RegisterFunction::new_async(move |req: get_object::GetReq| {
            let st = st.clone();
            async move { get_object::handle(&st, req).await.map_err(IIIError::from) }
        })
        .description("Read an object. Body is base64; for large objects use presignUrl."),
    );
}

fn register_delete_object(iii: &III, state: &AppState) {
    let st = state.clone();
    iii.register_function(
        "storage::deleteObject",
        RegisterFunction::new_async(move |req: delete_object::DeleteReq| {
            let st = st.clone();
            async move {
                delete_object::handle(&st, req)
                    .await
                    .map_err(IIIError::from)
            }
        })
        .description("Delete an object. No-op when the object does not exist."),
    );
}

fn register_presign_url(iii: &III, state: &AppState) {
    let st = state.clone();
    iii.register_function(
        "storage::presignUrl",
        RegisterFunction::new_async(move |req: presign_url::PresignReq| {
            let st = st.clone();
            async move { presign_url::handle(&st, req).await.map_err(IIIError::from) }
        })
        .description(
            "Issue a short-lived URL the browser can hit directly to PUT or GET an object.",
        ),
    );
}
