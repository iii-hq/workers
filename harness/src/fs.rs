//! Harness-side filesystem adapters wrapping the consolidated `shell` worker.
//!
//! The shell worker's `shell::fs::read` returns a streaming `StreamChannelRef`;
//! the harness web FilesystemPanel preview wants inline bytes. This module
//! provides `read_inline` to bridge the two.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Default cap for inline reads. Matches the historical
/// `shell-filesystem::read` cap so existing UI behaviour is preserved.
pub const DEFAULT_MAX_INLINE_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
pub struct ReadInlineArgs {
    pub path: String,
    #[serde(default)]
    pub max_bytes: Option<usize>,
}

/// Wire-compatible mirror of `shell::fs::ReadResponseWire` so we can
/// deserialize the shell worker's response without depending on the
/// `iii-shell` crate.
#[derive(Debug, Deserialize)]
pub struct ShellFsReadResponse {
    pub content: ContentRef,
    pub size: u64,
    #[allow(dead_code)]
    pub mode: String,
    #[allow(dead_code)]
    pub mtime: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ContentRef {
    pub channel_id: String,
    pub access_key: String,
    #[serde(default)]
    pub direction: String,
}

/// Pure helper: build the legacy `shell::filesystem::read` envelope from
/// drained bytes plus the upstream `size`. Truncation is detected when
/// `bytes.len() < total_size`.
pub fn build_inline_envelope(bytes: &[u8], total_size: u64) -> Value {
    let bytes_read = bytes.len();
    let truncated = (bytes_read as u64) < total_size;
    let text = std::str::from_utf8(bytes)
        .map_or_else(|_| format!("<binary {} bytes>", bytes_read), str::to_string);
    json!({
        "content": [{ "type": "text", "text": text }],
        "details": {
            "size": total_size,
            "truncated": truncated,
            "bytes_read": bytes_read,
        },
        "terminate": false,
    })
}

use iii_sdk::channels::{ChannelDirection, ChannelReader, StreamChannelRef};
use iii_sdk::{IIIError, TriggerRequest, III};

const READ_TRIGGER_TIMEOUT_MS: u64 = 30_000;

/// Drive `shell::fs::read` and inline up to `max_bytes` of the streamed
/// channel into a legacy `{content:[{text}], details:{...}}` envelope.
///
/// `engine_ws_base` is the same URL passed to `register_worker` in
/// `main.rs`; the harness already threads it through
/// `register_with_iii_with_engine_url`, so callers can pass it directly.
pub async fn read_inline(
    iii: &III,
    engine_ws_base: &str,
    args: ReadInlineArgs,
) -> Result<Value, IIIError> {
    let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_INLINE_BYTES);

    // 1. Ask the shell worker to set up the read channel.
    let raw = iii
        .trigger(TriggerRequest {
            function_id: "shell::fs::read".into(),
            payload: json!({ "path": args.path }),
            action: None,
            timeout_ms: Some(READ_TRIGGER_TIMEOUT_MS),
        })
        .await
        .map_err(|e| IIIError::Handler(format!("shell::fs::read trigger failed: {e}")))?;

    let parsed: ShellFsReadResponse = serde_json::from_value(raw.clone()).map_err(|e| {
        IIIError::Handler(format!(
            "shell::fs::read returned unexpected shape: {e} (raw: {raw})"
        ))
    })?;

    // 2. Drain the channel into a capped buffer.
    let chan_ref = StreamChannelRef {
        channel_id: parsed.content.channel_id.clone(),
        access_key: parsed.content.access_key.clone(),
        direction: ChannelDirection::Read,
    };
    let reader = ChannelReader::new(engine_ws_base, &chan_ref);
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < max_bytes {
        match reader.next_binary().await {
            Ok(Some(chunk)) => {
                let remaining = max_bytes.saturating_sub(buf.len());
                if chunk.len() <= remaining {
                    buf.extend_from_slice(&chunk);
                } else {
                    buf.extend_from_slice(&chunk[..remaining]);
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = reader.close().await;
                return Err(IIIError::Handler(format!("channel drain failed: {e}")));
            }
        }
    }
    let _ = reader.close().await;

    // 3. Build the inline envelope (pure, unit-tested in Task 8).
    Ok(build_inline_envelope(&buf, parsed.size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_inline_envelope_round_trips_small_utf8() {
        let env = build_inline_envelope(b"hello", 5);
        assert_eq!(env["content"][0]["type"], "text");
        assert_eq!(env["content"][0]["text"], "hello");
        assert_eq!(env["details"]["size"], 5);
        assert_eq!(env["details"]["truncated"], false);
        assert_eq!(env["details"]["bytes_read"], 5);
        assert_eq!(env["terminate"], false);
    }

    #[test]
    fn build_inline_envelope_marks_truncation_when_drained_is_short() {
        // total_size = 1000 but we only got 256 bytes back.
        let bytes = vec![b'x'; 256];
        let env = build_inline_envelope(&bytes, 1000);
        assert_eq!(env["details"]["size"], 1000);
        assert_eq!(env["details"]["truncated"], true);
        assert_eq!(env["details"]["bytes_read"], 256);
        assert_eq!(env["content"][0]["text"].as_str().unwrap().len(), 256);
    }

    #[test]
    fn build_inline_envelope_emits_binary_marker_for_invalid_utf8() {
        // 0xFF is invalid UTF-8 in any leading position.
        let bytes = vec![0xFF, 0xFE, 0xFD];
        let env = build_inline_envelope(&bytes, 3);
        assert_eq!(env["content"][0]["text"], "<binary 3 bytes>");
        assert_eq!(env["details"]["size"], 3);
        assert_eq!(env["details"]["truncated"], false);
        assert_eq!(env["details"]["bytes_read"], 3);
    }
}
