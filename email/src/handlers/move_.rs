use futures_util::StreamExt;
use iii_sdk::{IIIError, RegisterFunction, III};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
struct MoveReq {
    account: String,
    folder: String,
    uid: u32,
    dst_folder: String,
}

enum MoveOutcome {
    Move,
    CopyStore,
}

enum MoveError {
    /// uid_mv failed AND the copy fallback failed before any side effect.
    BothFailed(async_imap::error::Error),
    /// uid_copy succeeded but the subsequent uid_store +FLAGS \Deleted failed.
    /// The message now exists in BOTH source and destination folders — caller
    /// must reconcile (delete the duplicate, or retry the STORE).
    PartialCopyStore(async_imap::error::Error),
}

pub fn register(iii: &Arc<III>, pool: &Arc<crate::provider::imap::ImapPool>) {
    let pool = pool.clone();
    iii.register_function(
        "email::move",
        RegisterFunction::new_async(move |raw: Value| {
            let pool = pool.clone();
            async move {
                let req: MoveReq = serde_json::from_value(raw).map_err(|e| {
                    IIIError::Handler(
                        json!({"code":"E611","message":format!("bad payload: {e}")}).to_string(),
                    )
                })?;
                let mut guard = pool.acquire(&req.account, &req.folder).await?;
                let uid_str = req.uid.to_string();

                let outcome: Result<MoveOutcome, MoveError> = async {
                    let session = guard.session();
                    match session.uid_mv(&uid_str, &req.dst_folder).await {
                        Ok(()) => Ok(MoveOutcome::Move),
                        Err(mv_err) => {
                            tracing::info!(error = %mv_err, "UID MOVE failed; falling back to COPY+STORE");
                            session
                                .uid_copy(&uid_str, &req.dst_folder)
                                .await
                                .map_err(MoveError::BothFailed)?;
                            // uid_copy succeeded — if the STORE fails from here,
                            // the message exists in BOTH source and dst.
                            match session
                                .uid_store(&uid_str, "+FLAGS.SILENT (\\Deleted)")
                                .await
                            {
                                Ok(mut stream) => {
                                    while stream.next().await.is_some() {}
                                    Ok(MoveOutcome::CopyStore)
                                }
                                Err(store_err) => Err(MoveError::PartialCopyStore(store_err)),
                            }
                        }
                    }
                }
                .await;

                match outcome {
                    Ok(MoveOutcome::Move) => Ok::<_, IIIError>(json!({ "ok": true, "method": "MOVE" })),
                    Ok(MoveOutcome::CopyStore) => {
                        Ok(json!({ "ok": true, "method": "COPY+STORE" }))
                    }
                    Err(MoveError::BothFailed(e)) => {
                        guard.poison();
                        Err(IIIError::Handler(
                            json!({"code":"E624","message":format!("imap move failed: {e}")})
                                .to_string(),
                        ))
                    }
                    Err(MoveError::PartialCopyStore(e)) => {
                        guard.poison();
                        Err(IIIError::Handler(
                            json!({
                                "code": "E627",
                                "message": format!(
                                    "copy succeeded but STORE \\Deleted failed; \
                                     message now exists in BOTH `{}` and `{}` — \
                                     reconcile manually: {e}",
                                    req.folder, req.dst_folder,
                                ),
                                "partial": true,
                                "source_folder": req.folder,
                                "dst_folder": req.dst_folder,
                            })
                            .to_string(),
                        ))
                    }
                }
            }
        })
        .description(
            "Move a message to another folder. Payload: \
             { account, folder, uid, dst_folder }. Uses RFC 6851 UID MOVE when \
             the server supports it; falls back to COPY + STORE \\Deleted otherwise. \
             Destination folder must exist on the server.",
        ),
    );
}
