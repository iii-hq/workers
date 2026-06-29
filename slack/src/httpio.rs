//! Raw HTTP request/response over the engine's stream channels.
//!
//! HTTP-trigger handlers that need the **raw** request body (Slack signature
//! verification) read it from the engine-issued `request_body` read channel and
//! write the response onto the paired write channel.

use std::collections::HashMap;

use iii_sdk::channels::{ChannelDirection, ChannelReader, ChannelWriter};
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

pub struct Incoming {
    pub raw: Vec<u8>,
    /// Lowercased header map.
    pub headers: HashMap<String, String>,
}

impl Incoming {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// Read the raw request body + headers and return a writer for the response.
pub async fn read_incoming(
    iii: &IIIClient,
    input: &Value,
) -> Result<(Incoming, ChannelWriter), Error> {
    let refs = iii_sdk::channels::extract_channel_refs(input);

    let writer_ref = refs
        .iter()
        .find(|(_, r)| matches!(r.direction, ChannelDirection::Write))
        .map(|(_, r)| r.clone())
        .ok_or_else(|| Error::Handler("http: missing response channel".into()))?;
    let writer = ChannelWriter::new(iii.address(), &writer_ref);

    let reader_ref = refs
        .iter()
        .find(|(k, r)| k.contains("request_body") && matches!(r.direction, ChannelDirection::Read))
        .map(|(_, r)| r.clone());

    let raw = match reader_ref {
        Some(rr) => ChannelReader::new(iii.address(), &rr).read_all().await?,
        None => {
            // No raw channel — fall back to the parsed body (signature verify
            // will then fail, which is the safe default).
            serde_json::to_vec(input.get("body").unwrap_or(&Value::Null)).unwrap_or_default()
        }
    };

    let mut headers = HashMap::new();
    if let Some(obj) = input.get("headers").and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                headers.insert(k.to_ascii_lowercase(), s.to_string());
            }
        }
    }

    Ok((Incoming { raw, headers }, writer))
}

/// Write a complete HTTP response and close the channel.
pub async fn respond(writer: &ChannelWriter, status: u16, content_type: &str, body: &[u8]) {
    if let Err(e) = writer
        .send_message(&json!({ "type": "set_status", "status_code": status }).to_string())
        .await
    {
        tracing::warn!(error = %e, "http: set_status failed");
        return;
    }
    let _ = writer
        .send_message(
            &json!({ "type": "set_headers", "headers": { "content-type": content_type } })
                .to_string(),
        )
        .await;
    if !body.is_empty() {
        let _ = writer.write(body).await;
    }
    let _ = writer.close().await;
}

/// Outcome of reading and signature-verifying a Slack request.
pub enum Verified {
    Ok(Incoming, ChannelWriter),
    Rejected,
}

/// Read the raw request and verify its Slack signature. On a missing secret or a
/// bad signature this writes the rejection response (503/401) and returns
/// [`Verified::Rejected`]; callers just return on `Rejected`.
pub async fn read_verified(
    iii: &IIIClient,
    input: &Value,
    signing_secret: Option<&str>,
) -> Verified {
    let (incoming, writer) = match read_incoming(iii, input).await {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!(error = %e, "slack: could not read request");
            return Verified::Rejected;
        }
    };
    let Some(secret) = signing_secret.filter(|s| !s.trim().is_empty()) else {
        respond(&writer, 503, "text/plain", b"bridge not configured").await;
        return Verified::Rejected;
    };
    if let Err(e) = crate::signing::verify(
        secret,
        incoming.header("x-slack-signature"),
        incoming.header("x-slack-request-timestamp"),
        &incoming.raw,
        now_unix(),
    ) {
        tracing::warn!(reason = %e, "slack: signature verification failed");
        respond(&writer, 401, "text/plain", b"invalid signature").await;
        return Verified::Rejected;
    }
    Verified::Ok(incoming, writer)
}

/// Current unix time in seconds.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
