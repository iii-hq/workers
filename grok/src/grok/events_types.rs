//! Serde mirror of the streaming-json events `grok --single <prompt>
//! --output-format streaming-json` writes to stdout, one JSON object per line.
//!
//! Captured from Grok CLI 0.2.77. The stream is delta-based: assistant text
//! arrives as `text` chunks, the turn closes with a single `end` event carrying
//! the stop reason and the Grok session id, and failures arrive as `error`.
//! Parsing is lenient — unknown event types fall through to `Unknown` and are
//! skipped, and every line is forwarded verbatim onto `grok::events`, so a
//! newer CLI that adds event types never drops raw data.

use serde::Deserialize;

/// One line of the streaming-json stream, externally tagged on `type`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GrokEvent {
    /// Assistant text delta: `{ "type": "text", "data": "<chunk>" }`.
    Text { data: String },
    /// Reasoning delta (reasoning models only): `{ "type": "thought", "data": "<chunk>" }`.
    Thought { data: String },
    /// Terminal frame: `{ "type": "end", "stopReason", "sessionId" }`.
    End(EndEvent),
    /// Failure: `{ "type": "error", "message": "<msg>" }`.
    Error { message: String },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct EndEvent {
    #[serde(rename = "stopReason", default)]
    pub stop_reason: String,
    #[serde(rename = "sessionId", default)]
    pub session_id: String,
}
