use std::sync::Arc;

use serde_json::Value;

use crate::fs::error::FsError;
use crate::fs::{FsBackend, RmRequest, RmResponse};
use crate::functions::fs_dispatch::pick_backend;

pub async fn handle(
    host: Arc<dyn FsBackend>,
    iii: iii_sdk::IIIClient,
    sandbox_enabled: bool,
    payload: Value,
) -> Result<RmResponse, iii_sdk::errors::Error> {
    let req: RmRequest = serde_json::from_value(payload)
        .map_err(|e| FsError::new("S210", format!("bad rm payload: {e}")))?;
    let (target, args) = req.split();
    let backend = pick_backend(target, host, iii, sandbox_enabled);
    backend.rm(args).await.map_err(iii_sdk::errors::Error::from)
}
