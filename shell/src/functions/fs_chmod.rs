use std::sync::Arc;

use serde_json::Value;

use crate::fs::error::FsError;
use crate::fs::{ChmodRequest, ChmodResponse, FsBackend};
use crate::functions::fs_dispatch::pick_backend;

pub async fn handle(
    host: Arc<dyn FsBackend>,
    iii: iii_sdk::III,
    sandbox_enabled: bool,
    payload: Value,
) -> Result<ChmodResponse, iii_sdk::IIIError> {
    let req: ChmodRequest = serde_json::from_value(payload)
        .map_err(|e| FsError::new("S210", format!("bad chmod payload: {e}")))?;
    let (target, args) = req.split();
    let backend = pick_backend(target, host, iii, sandbox_enabled);
    backend.chmod(args).await.map_err(iii_sdk::IIIError::from)
}
