use std::sync::Arc;

use serde_json::Value;

use crate::fs::error::FsError;
use crate::fs::{FsBackend, MkdirRequest, MkdirResponse};
use crate::functions::fs_dispatch::{err_to_string, pick_backend};

pub async fn handle(
    host: Arc<dyn FsBackend>,
    iii: iii_sdk::III,
    sandbox_enabled: bool,
    payload: Value,
) -> Result<MkdirResponse, String> {
    let req: MkdirRequest = serde_json::from_value(payload)
        .map_err(|e| FsError::new("S210", format!("bad mkdir payload: {e}")).to_json())?;
    let (target, args) = req.split();
    let backend = pick_backend(target, host, iii, sandbox_enabled);
    backend.mkdir(args).await.map_err(err_to_string)
}
