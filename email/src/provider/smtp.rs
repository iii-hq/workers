use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use iii_sdk::errors::Error;
use lettre::{
    message::{
        header::ContentType, Attachment as LAttach, Mailbox, Message, MultiPart, SinglePart,
    },
    transport::smtp::{authentication::Credentials, AsyncSmtpTransport},
    AsyncTransport, Tokio1Executor,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

use crate::config::SmtpConfig;
use crate::handlers::send::SendReq;

#[derive(Debug, Deserialize, JsonSchema, Clone)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    pub source: AttachmentSource,
}

#[derive(Debug, Deserialize, JsonSchema, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AttachmentSource {
    /// Inline base64 bytes. Best for LLM tool-use; bounces the full payload
    /// through the engine. For large files prefer a sidecar upload via the
    /// `storage` worker followed by a future stream-source variant.
    Base64(String),
}

pub async fn send(
    from: &str,
    cfg: &SmtpConfig,
    cred: &Value,
    req: SendReq,
    timeout_ms: u64,
) -> Result<String, Error> {
    let user = cred
        .get("username")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            Error::Handler(
                json!({"code":"E608","message":"credential missing `username`"}).to_string(),
            )
        })?;
    let pass = cred
        .get("password")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            Error::Handler(
                json!({"code":"E608","message":"credential missing `password`"}).to_string(),
            )
        })?;

    let from_mb: Mailbox = from.parse().map_err(|e| {
        Error::Handler(
            json!({"code":"E609","message":format!("invalid from address `{from}`: {e}")})
                .to_string(),
        )
    })?;

    let mut builder = Message::builder()
        .from(from_mb)
        .subject(req.subject.clone());

    for addr in &req.to {
        builder = builder.to(parse_addr("to", addr)?);
    }
    for addr in &req.cc {
        builder = builder.cc(parse_addr("cc", addr)?);
    }
    for addr in &req.bcc {
        builder = builder.bcc(parse_addr("bcc", addr)?);
    }
    if let Some(ref reply) = req.reply_to {
        builder = builder.reply_to(parse_addr("reply_to", reply)?);
    }
    if let Some(ref irt) = req.in_reply_to {
        builder = builder.in_reply_to(irt.clone());
    }
    if !req.references.is_empty() {
        builder = builder.references(req.references.join(" "));
    }

    let body_part = build_body(req.html.as_deref(), req.text.as_deref())?;

    let multipart = if req.attachments.is_empty() {
        body_part
    } else {
        let mut mp = MultiPart::mixed().multipart(into_multipart(body_part));
        for a in req.attachments {
            mp = mp.singlepart(build_attachment_part(a)?);
        }
        mp
    };

    let message = builder.multipart(multipart).map_err(|e| {
        Error::Handler(json!({"code":"E609","message":format!("build message: {e}")}).to_string())
    })?;

    let creds = Credentials::new(user.to_string(), pass.to_string());
    let transport: AsyncSmtpTransport<Tokio1Executor> = if cfg.starttls {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)
            .map_err(|e| {
                Error::Handler(json!({"code":"E609","message":e.to_string()}).to_string())
            })?
            .port(cfg.port)
            .credentials(creds)
            .timeout(Some(Duration::from_millis(timeout_ms)))
            .build()
    } else {
        // starttls: false → plain SMTP. `builder_dangerous` skips implicit
        // TLS. Operators opting out of starttls are expected to be on a
        // trusted network (loopback / VPN / private VLAN).
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host)
            .port(cfg.port)
            .credentials(creds)
            .timeout(Some(Duration::from_millis(timeout_ms)))
            .build()
    };

    let response = transport.send(message).await.map_err(|e| {
        Error::Handler(
            json!({"code":"E620","message":format!("smtp send failed: {e}")}).to_string(),
        )
    })?;

    // lettre returns the SMTP response; extract the queued Message-ID if the
    // server included one, otherwise synthesize from response code+message.
    let message_id = response
        .message()
        .next()
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}", response.code()));

    Ok(message_id)
}

fn parse_addr(field: &str, addr: &str) -> Result<Mailbox, Error> {
    addr.parse::<Mailbox>().map_err(|e| {
        Error::Handler(
            json!({"code":"E609","message":format!("invalid {field} address `{addr}`: {e}")})
                .to_string(),
        )
    })
}

fn build_body(html: Option<&str>, text: Option<&str>) -> Result<MultiPart, Error> {
    let mp = match (html, text) {
        (Some(h), Some(t)) => MultiPart::alternative()
            .singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_PLAIN)
                    .body(t.to_string()),
            )
            .singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_HTML)
                    .body(h.to_string()),
            ),
        (Some(h), None) => MultiPart::alternative().singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_HTML)
                .body(h.to_string()),
        ),
        (None, Some(t)) => MultiPart::alternative().singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(t.to_string()),
        ),
        (None, None) => unreachable!("handler validates body presence"),
    };
    Ok(mp)
}

fn into_multipart(mp: MultiPart) -> MultiPart {
    mp
}

fn build_attachment_part(a: Attachment) -> Result<SinglePart, Error> {
    let ct: ContentType = a.content_type.parse().map_err(|e| {
        Error::Handler(
            json!({
                "code":"E609",
                "message":format!("invalid attachment content_type `{}`: {e}", a.content_type)
            })
            .to_string(),
        )
    })?;
    // E626 distinguishes "malformed base64 payload" from E605 which is
    // reserved for size-limit violations.
    let bytes = match a.source {
        AttachmentSource::Base64(s) => B64.decode(s.as_bytes()).map_err(|e| {
            Error::Handler(
                json!({"code":"E626","message":format!("attachment `{}` base64 decode failed: {e}", a.filename)}).to_string(),
            )
        })?,
    };

    Ok(LAttach::new(a.filename).body(bytes, ct))
}
