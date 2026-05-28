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

                let outcome: Result<MoveOutcome, async_imap::error::Error> = async {
                    let session = guard.session();
                    match session.uid_mv(req.uid.to_string(), &req.dst_folder).await {
                        Ok(()) => Ok(MoveOutcome::Move),
                        Err(e) => {
                            tracing::info!(error = %e, "UID MOVE failed; falling back to COPY+STORE");
                            session.uid_copy(req.uid.to_string(), &req.dst_folder).await?;
                            let mut stream = session
                                .uid_store(req.uid.to_string(), "+FLAGS.SILENT (\\Deleted)")
                                .await?;
                            while stream.next().await.is_some() {}
                            Ok(MoveOutcome::CopyStore)
                        }
                    }
                }
                .await;

                match outcome {
                    Ok(MoveOutcome::Move) => Ok::<_, IIIError>(json!({ "ok": true, "method": "MOVE" })),
                    Ok(MoveOutcome::CopyStore) => {
                        Ok(json!({ "ok": true, "method": "COPY+STORE" }))
                    }
                    Err(e) => {
                        guard.poison();
                        Err(IIIError::Handler(
                            json!({"code":"E624","message":format!("imap move failed: {e}")})
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
