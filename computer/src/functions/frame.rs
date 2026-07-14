//! `computer::screencast::start` / `stop` / `computer::frame` — the live-view
//! pipeline behind the console viewport. The worker polls the backend
//! screenshot at the configured fps and pushes each frame onto the
//! `computer:frames` stream; `computer::frame` hands out the newest frame
//! without a capture round-trip so the UI can poll fast. All three are
//! internal console-UI plumbing, not agent surface — agents read the desktop
//! with `computer::screenshot` and `computer::observe`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreencastStartInput {
    pub session_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreencastStopInput {
    /// Stopping the screencast on an unknown session succeeds.
    pub session_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AckOutput {
    pub ok: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FrameInput {
    pub session_id: String,
    /// Frame cursor from the previous read; when the newest frame still has
    /// this seq the response omits `frame` (nothing changed).
    #[serde(default)]
    pub since_frame: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FrameOutput {
    /// Base64 image of the newest frame; absent when `since_frame` is still
    /// current or no frame has arrived yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    /// Image mime of the frame bytes.
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub frame_seq: u64,
    pub timestamp: i64,
    /// False when no screencast is running (call screencast::start first).
    pub active: bool,
}
