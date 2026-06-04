use iii_sdk::{protocol::TriggerRequest, IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::provider::smtp::{Attachment, AttachmentSource};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendReq {
    pub account: String,
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    pub subject: String,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

pub fn register(iii: &Arc<III>, cfg: &Arc<crate::config::WorkerConfig>) {
    let cfg = cfg.clone();
    let iii_inner = iii.clone();
    iii.register_function(
        "email::send",
        RegisterFunction::new_async(move |req: SendReq| {
            let cfg = cfg.clone();
            let iii = iii_inner.clone();
            async move {
                if req.to.is_empty() {
                    return Err(IIIError::Handler(
                        json!({"code":"E601","message":"at least one recipient required in `to`"})
                            .to_string(),
                    ));
                }
                if req.html.is_none() && req.text.is_none() {
                    return Err(IIIError::Handler(
                        json!({"code":"E604","message":"provide at least one of `html` or `text`"})
                            .to_string(),
                    ));
                }
                let total = req.to.len() + req.cc.len() + req.bcc.len();
                if total > cfg.limits.max_recipients {
                    return Err(IIIError::Handler(
                        json!({
                            "code":"E602",
                            "message":format!("recipient count {} exceeds max {}", total, cfg.limits.max_recipients)
                        }).to_string(),
                    ));
                }

                let acct = cfg.accounts.get(&req.account).ok_or_else(|| {
                    IIIError::Handler(
                        json!({"code":"E600","message":format!("unknown account `{}`", req.account)})
                            .to_string(),
                    )
                })?;
                let smtp_cfg = acct.smtp.as_ref().ok_or_else(|| {
                    IIIError::Handler(
                        json!({"code":"E603","message":format!("account `{}` has no smtp transport configured", req.account)})
                            .to_string(),
                    )
                })?;

                // Validate attachment sizes up-front before any network call.
                let max_bytes = cfg.limits.max_attachment_bytes;
                for a in &req.attachments {
                    let AttachmentSource::Base64(ref b) = a.source;
                    // Base64 inflates by ~4/3 → conservative check on decoded size.
                    let approx = b.len() * 3 / 4;
                    if approx > max_bytes {
                        return Err(IIIError::Handler(
                            json!({
                                "code":"E605",
                                "message":format!("attachment `{}` exceeds max {} bytes", a.filename, max_bytes)
                            }).to_string(),
                        ));
                    }
                }

                let cred = iii
                    .trigger(TriggerRequest {
                        function_id: "auth::get_token".to_string(),
                        payload: json!({ "provider": format!("email::{}", req.account) }),
                        action: None,
                        timeout_ms: Some(5_000),
                    })
                    .await
                    .map_err(|e| {
                        IIIError::Handler(
                            json!({
                                "code":"E606",
                                "message":format!("auth::get_token failed for `email::{}`: {e}", req.account)
                            }).to_string(),
                        )
                    })?;
                if cred.is_null() {
                    return Err(IIIError::Handler(
                        json!({
                            "code":"E607",
                            "message":format!("no credential stored for `email::{}` — call auth::set_token first", req.account)
                        }).to_string(),
                    ));
                }

                let message_id = crate::provider::smtp::send(
                    &acct.from, smtp_cfg, &cred, req, cfg.limits.send_timeout_ms,
                ).await?;

                Ok::<_, IIIError>(json!({ "message_id": message_id }))
            }
        })
        .description(
            "Send an email via the account's SMTP transport. Payload: \
             { account: string (key from email worker config), to: string[], \
             cc?: string[], bcc?: string[], subject: string, html?: string, \
             text?: string, reply_to?: string, in_reply_to?: string (Message-ID), \
             references?: string[], attachments?: [{ filename, content_type, \
             source: { kind: 'base64', data } }] }. Returns { message_id }. \
             Credentials are fetched from auth-credentials under provider key \
             `email::<account>` ({ username, password }). Provide html OR text \
             (or both); at least one body is required.",
        ),
    );
}
