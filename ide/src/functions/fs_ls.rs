use std::sync::Arc;

use serde_json::Value;

use crate::fs::error::FsError;
use crate::fs::{FsBackend, LsRequest, LsResponse};
use crate::functions::fs_dispatch::pick_backend;

pub async fn handle(
    host: Arc<dyn FsBackend>,
    iii: iii_sdk::IIIClient,
    sandbox_enabled: bool,
    payload: Value,
) -> Result<LsResponse, iii_sdk::errors::Error> {
    // Both the payload-deser error (S210) and the backend error carry their
    // S-code to the wire `code` via `From<FsError> for Error` (Remote), so
    // an agent can branch on `error.code` instead of parsing the message.
    let req: LsRequest = serde_json::from_value(payload)
        .map_err(|e| FsError::new("S210", format!("bad ls payload: {e}")))?;
    let (page, page_size) = (req.page, req.page_size);
    let (target, args) = req.split();
    let backend = pick_backend(target, host, iii, sandbox_enabled);
    // Both backends return the whole directory; the page is cut here so the
    // wire contract is the same for host and sandbox targets.
    backend
        .ls(args)
        .await
        .map(|resp| resp.paginate(page, page_size))
        .map_err(iii_sdk::errors::Error::from)
}
