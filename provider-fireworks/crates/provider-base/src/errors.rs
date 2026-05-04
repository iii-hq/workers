//! Provider-side error helpers.
//!
//! Streams must never throw; errors land as the final `Error` event with a
//! classified `error_kind`. These helpers make that contract one-liner-easy
//! for every provider crate.

use harness_types::{
    AssistantMessage, AssistantMessageEvent, ContentBlock, ErrorKind, StopReason, TextContent,
};
use overflow_classify::classify_error;

/// Classify an error message and HTTP status into the canonical `ErrorKind`.
///
/// Thin re-export so provider crates can avoid taking a direct dep on
/// `overflow-classify`.
pub fn classify_provider_error(text: &str, http_status: Option<u16>) -> ErrorKind {
    classify_error(text, http_status)
}

/// Build a final `error` event for the stream protocol from an error message
/// and an optional HTTP status.
pub fn error_event(
    error_text: impl Into<String>,
    http_status: Option<u16>,
    model: impl Into<String>,
    provider: impl Into<String>,
) -> AssistantMessageEvent {
    let text: String = error_text.into();
    let kind = classify_provider_error(&text, http_status);
    let final_msg = AssistantMessage {
        content: vec![ContentBlock::Text(TextContent { text: text.clone() })],
        stop_reason: StopReason::Error,
        error_message: Some(text),
        error_kind: Some(kind),
        usage: None,
        model: model.into(),
        provider: provider.into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    };
    AssistantMessageEvent::Error { error: final_msg }
}
