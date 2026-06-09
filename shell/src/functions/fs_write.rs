use std::sync::Arc;

use serde_json::Value;

use crate::fs::error::FsError;
use crate::fs::{FsBackend, WriteFileResult, WriteRequest, WriteResponse};
use crate::functions::fs_dispatch::pick_backend;

pub async fn handle(
    host: Arc<dyn FsBackend>,
    iii: iii_sdk::III,
    sandbox_enabled: bool,
    payload: Value,
) -> Result<WriteResponse, iii_sdk::IIIError> {
    let req: WriteRequest = serde_json::from_value(payload)
        .map_err(|e| FsError::new("S210", format!("bad write payload: {e}")))?;
    let (target, mut specs, batch) = req.into_specs().map_err(iii_sdk::IIIError::from)?;
    let backend = pick_backend(target, host, iii, sandbox_enabled);

    // Single-file form (`path`+`content`): return the backend response verbatim
    // (files stays empty), preserving the { bytes_written, path } shape for
    // sandbox round-trips and prior single-file callers. The batch form always
    // returns the per-file `files` array below, even for one entry, so a caller
    // that sends `files` always gets `files` back.
    if !batch {
        return backend
            .write(specs.pop().unwrap())
            .await
            .map_err(iii_sdk::IIIError::from);
    }

    // Batch: write each file in order, aggregating per-file results. A failure
    // on any file aborts and surfaces that error (files written so far remain).
    let mut files = Vec::with_capacity(specs.len());
    let mut total: u64 = 0;
    for spec in specs {
        let r = backend.write(spec).await.map_err(iii_sdk::IIIError::from)?;
        total += r.bytes_written;
        files.push(WriteFileResult {
            path: r.path,
            bytes_written: r.bytes_written,
        });
    }
    Ok(WriteResponse {
        bytes_written: total,
        path: String::new(),
        files,
    })
}
