//! IMAP FETCH helpers. Header summaries for list/idle, full body for get,
//! body-part streaming for attachment::get. All functions take `&mut Session`
//! so the caller owns serialization across the half-duplex IMAP socket.

use anyhow::{anyhow, Result};
use bytes::BytesMut;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use iii_sdk::channels::ChannelWriter;
use mail_parser::MessageParser;
use serde::Serialize;

use super::Session;

#[derive(Debug, Clone, Serialize)]
pub struct HeaderSummary {
    pub uid: u32,
    pub message_id: Option<String>,
    pub from: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub ts: Option<DateTime<Utc>>,
    pub seen: bool,
    pub flagged: bool,
}

pub async fn uids_above(session: &mut Session, high_water: u32) -> Result<Vec<u32>> {
    let query = format!("UID {}:*", high_water.saturating_add(1));
    let result = session
        .uid_search(&query)
        .await
        .map_err(|e| anyhow!("uid_search {query} failed: {e}"))?;
    let mut uids: Vec<u32> = result.into_iter().filter(|u| *u > high_water).collect();
    uids.sort_unstable();
    Ok(uids)
}

pub async fn header_summary(session: &mut Session, uid: u32) -> Result<HeaderSummary> {
    let query = "(UID FLAGS INTERNALDATE RFC822.HEADER BODY.PEEK[TEXT]<0.512>)";
    let mut stream = session
        .uid_fetch(uid.to_string(), query)
        .await
        .map_err(|e| anyhow!("uid_fetch headers for {uid} failed: {e}"))?;

    let mut summary = HeaderSummary {
        uid,
        message_id: None,
        from: None,
        subject: None,
        snippet: None,
        ts: None,
        seen: false,
        flagged: false,
    };

    while let Some(msg) = stream.next().await {
        let msg = msg.map_err(|e| anyhow!("fetch stream: {e}"))?;
        if let Some(flags) = msg.flags().next() {
            // Iterate via flags() lazily — async-imap exposes Flag enum per item.
            // We re-iterate below to scan all flags.
            let _ = flags;
        }
        for flag in msg.flags() {
            match flag {
                async_imap::types::Flag::Seen => summary.seen = true,
                async_imap::types::Flag::Flagged => summary.flagged = true,
                _ => {}
            }
        }
        if let Some(date) = msg.internal_date() {
            summary.ts = Some(date.with_timezone(&Utc));
        }
        if let Some(header_bytes) = msg.header() {
            if let Some(parsed) = MessageParser::default().parse(header_bytes) {
                summary.message_id = parsed.message_id().map(|s| s.to_string());
                summary.from = parsed
                    .from()
                    .and_then(|addrs| addrs.first())
                    .and_then(|a| a.address.as_ref().map(|s| s.to_string()));
                summary.subject = parsed.subject().map(|s| s.to_string());
            }
        }
        // BODY.PEEK[TEXT]<0.512> — short snippet.
        if let Some(text_section) = msg.text() {
            let s = String::from_utf8_lossy(text_section).to_string();
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                summary.snippet = Some(trimmed.chars().take(200).collect());
            }
        }
    }

    Ok(summary)
}

pub async fn full_body(session: &mut Session, uid: u32) -> Result<Vec<u8>> {
    let mut stream = session
        .uid_fetch(uid.to_string(), "BODY[]")
        .await
        .map_err(|e| anyhow!("uid_fetch body for {uid} failed: {e}"))?;
    let mut out = BytesMut::new();
    while let Some(msg) = stream.next().await {
        let msg = msg.map_err(|e| anyhow!("fetch stream: {e}"))?;
        if let Some(body) = msg.body() {
            out.extend_from_slice(body);
        }
    }
    Ok(out.to_vec())
}

/// Stream a specific MIME part directly onto `writer`. No buffering: bytes are
/// written as they arrive from IMAP.
pub async fn stream_part_bytes(
    session: &mut Session,
    uid: u32,
    part_id: &str,
    writer: &ChannelWriter,
) -> Result<()> {
    // Reject empty / whitespace-only part_id: BODY[] would fetch the
    // whole message, which silently bypasses the attachment contract.
    if part_id.trim().is_empty() {
        return Err(anyhow!("part_id must not be empty"));
    }
    let query = format!("BODY[{part_id}]");
    let mut stream = session
        .uid_fetch(uid.to_string(), query.as_str())
        .await
        .map_err(|e| anyhow!("uid_fetch part {part_id} for {uid} failed: {e}"))?;
    let mut written: usize = 0;
    while let Some(msg) = stream.next().await {
        let msg = msg.map_err(|e| anyhow!("fetch stream: {e}"))?;
        if let Some(body) = msg.body() {
            writer
                .write(body)
                .await
                .map_err(|e| anyhow!("channel write: {e}"))?;
            written += body.len();
        }
    }
    // A successful FETCH for a non-existent part returns no body bytes.
    // Treat zero-byte responses as not-found so the caller sees a real
    // error instead of an empty success.
    if written == 0 {
        return Err(anyhow!(
            "imap returned no bytes for BODY[{part_id}] on uid {uid} (part not found?)"
        ));
    }
    Ok(())
}
