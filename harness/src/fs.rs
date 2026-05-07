//! Harness-side filesystem adapters that wrap the consolidated `shell`
//! worker. The shell worker's `shell::fs::read` returns a streaming
//! `StreamChannelRef`; the harness web FilesystemPanel preview wants
//! inline bytes. `read_inline` bridges the two.

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
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => format!("<binary {} bytes>", bytes_read),
    };
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
