use iii_sdk::channels::{ChannelWriter, StreamChannelRef};
use iii_sdk::{IIIError, RegisterFunction, III};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
struct AttachReq {
    account: String,
    folder: String,
    uid: u32,
    part_id: String,
    response: StreamChannelRef,
}

pub fn register(iii: &Arc<III>, pool: &Arc<crate::provider::imap::ImapPool>) {
    let pool = pool.clone();
    let iii_inner = iii.clone();
    iii.register_function(
        "email::attachment::get",
        RegisterFunction::new_async(move |raw: Value| {
            let pool = pool.clone();
            let iii = iii_inner.clone();
            async move {
                let req: AttachReq = serde_json::from_value(raw).map_err(|e| {
                    IIIError::Handler(
                        json!({"code":"E611","message":format!("bad payload: {e}")}).to_string(),
                    )
                })?;
                let writer = ChannelWriter::new(iii.address(), &req.response);

                let mut guard = pool.acquire(&req.account, &req.folder).await?;
                let fetch_result = {
                    let session = guard.session();
                    crate::provider::imap::fetch::stream_part_bytes(
                        session,
                        req.uid,
                        &req.part_id,
                        &writer,
                    )
                    .await
                };
                if let Err(e) = fetch_result {
                    guard.poison();
                    let _ = writer.close().await;
                    return Err(IIIError::Handler(
                        json!({"code":"E625","message":format!("attachment fetch failed: {e}")})
                            .to_string(),
                    ));
                }

                writer.close().await.map_err(|e| {
                    IIIError::Handler(
                        json!({"code":"E621","message":format!("channel close failed: {e}")})
                            .to_string(),
                    )
                })?;
                Ok::<_, IIIError>(Value::Null)
            }
        })
        .description(
            "Stream an attachment's raw bytes from a message. Payload: \
             { account, folder, uid, part_id }. `part_id` comes from \
             email::get's attachments[].part_id. Bytes flow over the response \
             channel in chunks as IMAP delivers them; no in-memory buffering. \
             Channel closes on EOF.",
        ),
    );
}
