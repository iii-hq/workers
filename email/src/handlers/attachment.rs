use iii_sdk::channels::ChannelWriter;
use iii_sdk::{errors::Error, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use crate::provider::StreamRef;

#[derive(Debug, Deserialize, JsonSchema)]
struct AttachReq {
    account: String,
    folder: String,
    uid: u32,
    part_id: String,
    response: StreamRef,
}

#[derive(Debug, Serialize, JsonSchema)]
struct AttachmentGetResp {
    ok: bool,
}

pub fn register(iii: &Arc<IIIClient>, pool: &Arc<crate::provider::imap::ImapPool>) {
    let pool = pool.clone();
    let iii_inner = iii.clone();
    iii.register_function(
        "email::attachment::get",
        RegisterFunction::new_async(move |req: AttachReq| {
            let pool = pool.clone();
            let iii = iii_inner.clone();
            async move {
                let writer = ChannelWriter::new(iii.address(), &req.response.into());

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
                    return Err(Error::Handler(
                        json!({"code":"E625","message":format!("attachment fetch failed: {e}")})
                            .to_string(),
                    ));
                }

                writer.close().await.map_err(|e| {
                    Error::Handler(
                        json!({"code":"E621","message":format!("channel close failed: {e}")})
                            .to_string(),
                    )
                })?;
                Ok::<_, Error>(AttachmentGetResp { ok: true })
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
