use iii_sdk::channels::ChannelWriter;
use iii_sdk::{errors::Error, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use crate::provider::StreamRef;

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchReq {
    account: String,
    #[serde(default = "default_folder")]
    folder: String,
    query: String,
    response: StreamRef,
}
fn default_folder() -> String {
    "INBOX".into()
}

#[derive(Debug, Serialize, JsonSchema)]
struct SearchResp {
    ok: bool,
}

pub fn register(iii: &Arc<IIIClient>, pool: &Arc<crate::provider::imap::ImapPool>) {
    let pool = pool.clone();
    let iii_inner = iii.clone();
    iii.register_function(
        "email::search",
        RegisterFunction::new_async(move |req: SearchReq| {
            let pool = pool.clone();
            let iii = iii_inner.clone();
            async move {
                let writer = ChannelWriter::new(iii.address(), &req.response.into());

                let mut guard = pool.acquire(&req.account, &req.folder).await?;

                let search_result = {
                    let session = guard.session();
                    session.uid_search(&req.query).await
                };
                let uids = match search_result {
                    Ok(u) => u,
                    Err(e) => {
                        guard.poison();
                        return Err(Error::Handler(
                            json!({"code":"E612","message":format!("uid_search failed: {e}")})
                                .to_string(),
                        ));
                    }
                };

                let mut sorted: Vec<u32> = uids.into_iter().collect();
                sorted.sort_unstable_by(|a, b| b.cmp(a));

                let session = guard.session();
                for uid in sorted {
                    let summary = match crate::provider::imap::fetch::header_summary(session, uid).await {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(uid, error = %e, "header_summary skipped during search");
                            continue;
                        }
                    };
                    let frame = match serde_json::to_vec(&summary) {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::warn!(uid, error = %e, "summary serialize failed");
                            continue;
                        }
                    };
                    if let Err(e) = writer.write(&frame).await {
                        tracing::warn!(uid, error = %e, "channel write failed; aborting search stream");
                        break;
                    }
                    let _ = writer.write(b"\n").await;
                }

                writer.close().await.map_err(|e| {
                    Error::Handler(
                        json!({"code":"E621","message":format!("channel close failed: {e}")})
                            .to_string(),
                    )
                })?;
                Ok::<_, Error>(SearchResp { ok: true })
            }
        })
        .description(
            "Stream search results from an account's IMAP. Payload: \
             { account, folder?='INBOX', query }. `query` is IMAP SEARCH syntax \
             (RFC 9051), e.g. 'FROM alice@x SINCE 1-Jan-2026' or 'UNSEEN \
             SUBJECT \"invoice\"'. Returns NDJSON frames on the response channel; \
             each frame is { uid, message_id, from, subject, snippet, ts, seen, \
             flagged }. Channel closes when search completes. No polling — frames \
             stream as IMAP returns matches.",
        ),
    );
}
