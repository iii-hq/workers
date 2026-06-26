use iii_sdk::{errors::Error, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Deserialize, JsonSchema)]
struct ListReq {
    account: String,
    #[serde(default = "default_folder")]
    folder: String,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    since_uid: Option<u32>,
}
fn default_folder() -> String {
    "INBOX".into()
}
fn default_limit() -> u32 {
    50
}
const MAX_LIMIT: u32 = 1000;

#[derive(Debug, Serialize, JsonSchema)]
struct ListResp {
    items: Vec<crate::provider::imap::fetch::HeaderSummary>,
    next_since_uid: Option<u32>,
}

pub fn register(iii: &Arc<IIIClient>, pool: &Arc<crate::provider::imap::ImapPool>) {
    let pool = pool.clone();
    iii.register_function(
        "email::list",
        RegisterFunction::new_async(move |req: ListReq| {
            let pool = pool.clone();
            async move {
                let mut guard = pool.acquire(&req.account, &req.folder).await?;

                // RFC 9051 UID range — "1:*" if no since cursor, else "since+1:*".
                let lo = req.since_uid.map(|u| u.saturating_add(1)).unwrap_or(1);
                let query = format!("UID {lo}:*");
                let search = {
                    let session = guard.session();
                    session.uid_search(&query).await
                };
                let uids = match search {
                    Ok(u) => u,
                    Err(e) => {
                        guard.poison();
                        return Err(Error::Handler(
                            json!({"code":"E612","message":format!("uid_search failed: {e}")})
                                .to_string(),
                        ));
                    }
                };
                let session = guard.session();
                let mut uids: Vec<u32> = uids.into_iter().collect();
                uids.sort_unstable_by(|a, b| b.cmp(a)); // newest first
                let effective_limit = req.limit.min(MAX_LIMIT);
                uids.truncate(effective_limit as usize);

                let mut items = Vec::with_capacity(uids.len());
                for uid in &uids {
                    match crate::provider::imap::fetch::header_summary(session, *uid).await {
                        Ok(summary) => items.push(summary),
                        Err(e) => {
                            tracing::warn!(uid, error = %e, "header_summary skipped");
                        }
                    }
                }

                let next_cursor = uids.first().copied();
                Ok::<_, Error>(ListResp {
                    items,
                    next_since_uid: next_cursor,
                })
            }
        })
        .description(
            "List recent messages in a folder. Payload: { account, folder?='INBOX', \
             limit?=50, since_uid?=<int> }. Returns { items: [{ uid, message_id, from, \
             subject, snippet, ts, seen, flagged }], next_since_uid }. Pass next_since_uid \
             back as since_uid to page forward without missing or duplicating messages.",
        ),
    );
}
