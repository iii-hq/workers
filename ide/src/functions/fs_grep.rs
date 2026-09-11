use std::sync::Arc;

use serde_json::Value;

use crate::fs::error::FsError;
use crate::fs::{FsBackend, GrepRequest, GrepResponse};
use crate::functions::fs_dispatch::pick_backend;

pub async fn handle(
    host: Arc<dyn FsBackend>,
    iii: iii_sdk::IIIClient,
    sandbox_enabled: bool,
    payload: Value,
) -> Result<GrepResponse, iii_sdk::errors::Error> {
    let req: GrepRequest = serde_json::from_value(payload)
        .map_err(|e| FsError::new("S210", format!("bad grep payload: {e}")))?;
    let (target, args) = req.split();
    let backend = pick_backend(target, host, iii, sandbox_enabled);
    backend
        .grep(args)
        .await
        .map_err(iii_sdk::errors::Error::from)
}
