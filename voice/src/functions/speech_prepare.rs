//! Stateless preparation for browser-owned streaming read aloud. No synthesis.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AppState;

pub const ID: &str = "voice::speech::prepare";
pub const DESC: &str = "Prepare a growing Markdown message as plain speech text without synthesizing audio. \
    Incomplete inline syntax is withheld until complete=true or more text arrives. \
    Returns the configured per-request speech limit for client-side chunking; no state, downloads or playback.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    pub text: String,
    /// Flush incomplete syntax only when the message is no longer growing.
    #[serde(default)]
    pub complete: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    pub text: String,
    pub max_chunk_chars: usize,
}

pub async fn handle(state: &AppState, req: Request) -> Result<Response, String> {
    if req.text.len() > 262_144 {
        return Err("Streaming read aloud supports messages up to 256 KiB".into());
    }
    Ok(Response {
        text: crate::speech_text::streaming_text(&req.text, req.complete),
        max_chunk_chars: state.cfg.read().await.tts.max_speak_chars.min(600),
    })
}
