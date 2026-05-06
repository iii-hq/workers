use std::sync::Arc;

use serde_json::Value;

use crate::fs::error::FsError;
use crate::fs::{FsBackend, WriteRequest, WriteResponse};
use crate::functions::fs_dispatch::{err_to_string, pick_backend};

pub async fn handle(
    host: Arc<dyn FsBackend>,
    iii: iii_sdk::III,
    sandbox_enabled: bool,
    payload: Value,
) -> Result<WriteResponse, String> {
    let req: WriteRequest = serde_json::from_value(payload)
        .map_err(|e| FsError::new("S210", format!("bad write payload: {e}")).to_json())?;
    let (target, args) = req.split();
    let backend = pick_backend(target, host, iii, sandbox_enabled);
    backend.write(args).await.map_err(err_to_string)
}
