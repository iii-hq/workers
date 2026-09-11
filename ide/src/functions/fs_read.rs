use std::sync::Arc;

use serde_json::Value;

use crate::fs::error::FsError;
use crate::fs::{FsBackend, ReadRequest, ReadResponseWire};
use crate::functions::fs_dispatch::pick_backend;

pub async fn handle(
    host: Arc<dyn FsBackend>,
    iii: iii_sdk::IIIClient,
    sandbox_enabled: bool,
    payload: Value,
) -> Result<ReadResponseWire, iii_sdk::errors::Error> {
    let req: ReadRequest = serde_json::from_value(payload)
        .map_err(|e| FsError::new("S210", format!("bad read payload: {e}")))?;
    let (target, args) = req.split();
    let backend = pick_backend(target, host, iii, sandbox_enabled);
    let resp = backend
        .read(args)
        .await
        .map_err(iii_sdk::errors::Error::from)?;
    Ok(resp.into())
}
