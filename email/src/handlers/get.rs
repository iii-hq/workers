use iii_sdk::{IIIError, RegisterFunction, III};
use mail_parser::{MessageParser, MimeHeaders};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Deserialize)]
struct GetReq {
    account: String,
    folder: String,
    uid: u32,
}

pub fn register(iii: &Arc<III>, pool: &Arc<crate::provider::imap::ImapPool>) {
    let pool = pool.clone();
    iii.register_function(
        "email::get",
        RegisterFunction::new_async(move |raw: Value| {
            let pool = pool.clone();
            async move {
                let req: GetReq = serde_json::from_value(raw).map_err(|e| {
                    IIIError::Handler(
                        json!({"code":"E611","message":format!("bad payload: {e}")}).to_string(),
                    )
                })?;
                let mut guard = pool.acquire(&req.account, &req.folder).await?;
                let body_result = {
                    let session = guard.session();
                    crate::provider::imap::fetch::full_body(session, req.uid).await
                };
                let body = match body_result {
                    Ok(b) => b,
                    Err(e) => {
                        guard.poison();
                        return Err(IIIError::Handler(
                            json!({"code":"E619","message":format!("fetch body failed: {e}")})
                                .to_string(),
                        ));
                    }
                };

                let parsed = MessageParser::default().parse(&body[..]).ok_or_else(|| {
                    IIIError::Handler(
                        json!({"code":"E619","message":"mime parse failed"}).to_string(),
                    )
                })?;

                let mut attachments = Vec::new();
                for (idx, part) in parsed.attachments().enumerate() {
                    attachments.push(json!({
                        "part_id": format!("{}", idx + 1),
                        "filename": part.attachment_name().unwrap_or(""),
                        "content_type": part.content_type().map(|c| c.c_type.to_string()).unwrap_or_default(),
                        "size": part.contents().len(),
                    }));
                }

                Ok::<_, IIIError>(json!({
                    "uid": req.uid,
                    "message_id": parsed.message_id().unwrap_or(""),
                    "from": parsed.from()
                        .and_then(|a| a.first())
                        .and_then(|a| a.address.as_ref().map(|s| s.to_string())),
                    "to": parsed.to()
                        .map(|tos| tos.iter()
                            .filter_map(|a| a.address.as_ref().map(|s| s.to_string()))
                            .collect::<Vec<_>>())
                        .unwrap_or_default(),
                    "subject": parsed.subject().unwrap_or(""),
                    "date": parsed.date().map(|d| d.to_rfc3339()),
                    "html": parsed.body_html(0).map(|s| s.to_string()),
                    "text": parsed.body_text(0).map(|s| s.to_string()),
                    "attachments": attachments,
                }))
            }
        })
        .description(
            "Fetch a single message by UID. Payload: { account, folder, uid }. \
             Returns { uid, message_id, from, to[], subject, date, html?, text?, \
             attachments: [{ part_id, filename, content_type, size }] }. Use \
             email::attachment::get with the returned part_id to stream attachment bytes.",
        ),
    );
}
