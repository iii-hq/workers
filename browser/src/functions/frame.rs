//! `browser::screencast::start` / `stop` / `browser::frame` — the live-view
//! pipeline behind the console viewport. Chromium pushes encoded frames
//! continuously (`Page.startScreencast`); the worker keeps only the newest
//! in memory; `browser::frame` hands it out without any capture round-trip,
//! so the UI can poll fast and stay smooth. All three are internal
//! console-UI plumbing, not agent surface — agents read pages with
//! `browser::snapshot` and `browser::screenshot`.

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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FrameInput {
    pub session_id: String,
    /// Frame cursor from the previous read; when the newest frame still has
    /// this seq the response omits `frame` (nothing changed, nothing to
    /// redraw).
    #[serde(default)]
    pub since_frame: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FrameOutput {
    /// Base64 JPEG of the newest frame; absent when `since_frame` is still
    /// current or no frame has arrived yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    /// Page-viewport width the frame maps to (input coordinate space).
    pub width: u32,
    /// Page-viewport height the frame maps to.
    pub height: u32,
    pub frame_seq: u64,
    pub timestamp: i64,
    /// False when no screencast is running (call screencast::start first).
    pub active: bool,
}
