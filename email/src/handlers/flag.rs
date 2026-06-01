use futures_util::StreamExt;
use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Deserialize, JsonSchema)]
struct FlagReq {
    account: String,
    folder: String,
    uid: u32,
    flag: String,
    #[serde(default = "default_add")]
    add: bool,
}
fn default_add() -> bool {
    true
}

pub fn register(iii: &Arc<III>, pool: &Arc<crate::provider::imap::ImapPool>) {
    let pool = pool.clone();
    iii.register_function(
        "email::flag",
        RegisterFunction::new_async(move |req: FlagReq| {
            let pool = pool.clone();
            async move {
                let allowed = ["seen", "flagged", "answered", "deleted", "draft"];
                if !allowed.iter().any(|a| a.eq_ignore_ascii_case(&req.flag)) {
                    return Err(IIIError::Handler(
                        json!({
                            "code":"E622",
                            "message":format!("flag `{}` not in {{seen, flagged, answered, deleted, draft}}", req.flag)
                        }).to_string(),
                    ));
                }

                let op = if req.add { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
                let store_query = format!("{op} (\\{})", capitalize(&req.flag));

                let mut guard = pool.acquire(&req.account, &req.folder).await?;

                // Drive the entire IMAP exchange inside an async block so the
                // &mut session borrow expires before we touch `guard` again.
                let outcome: Result<(), async_imap::error::Error> = async {
                    let session = guard.session();
                    let mut stream = session.uid_store(req.uid.to_string(), &store_query).await?;
                    while let Some(item) = stream.next().await {
                        if let Err(e) = item {
                            tracing::warn!(error = %e, "store stream item error");
                        }
                    }
                    Ok(())
                }
                .await;

                match outcome {
                    Ok(()) => Ok::<_, IIIError>(json!({ "ok": true })),
                    Err(e) => {
                        guard.poison();
                        Err(IIIError::Handler(
                            json!({"code":"E623","message":format!("uid_store failed: {e}")})
                                .to_string(),
                        ))
                    }
                }
            }
        })
        .description(
            "Add or remove a system flag on a message. Payload: \
             { account, folder, uid, flag, add?=true }. `flag` is one of \
             'seen', 'flagged', 'answered', 'deleted', 'draft' (without \
             the leading backslash). Marking a message 'deleted' does NOT \
             expunge — use email::move to trash, or future email::expunge.",
        ),
    );
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => {
            c.to_ascii_uppercase().to_string() + chars.as_str().to_ascii_lowercase().as_str()
        }
        None => String::new(),
    }
}
