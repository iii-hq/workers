use std::sync::Arc;

use serde_json::Value;

use crate::fs::error::FsError;
use crate::fs::{FsBackend, LsRequest, LsResponse};
use crate::functions::fs_dispatch::pick_backend;

pub async fn handle(
    host: Arc<dyn FsBackend>,
    iii: iii_sdk::III,
    sandbox_enabled: bool,
    payload: Value,
) -> Result<LsResponse, iii_sdk::IIIError> {
    // Both the payload-deser error (S210) and the backend error carry their
    // S-code to the wire `code` via `From<FsError> for IIIError` (Remote), so
    // an agent can branch on `error.code` instead of parsing the message.
    let req: LsRequest = serde_json::from_value(payload)
        .map_err(|e| FsError::new("S210", format!("bad ls payload: {e}")))?;
    let (target, args) = req.split();
    let backend = pick_backend(target, host, iii, sandbox_enabled);
    backend.ls(args).await.map_err(iii_sdk::IIIError::from)
}
