//! `browser::screencast::start` / `stop` / `browser::frame` — the live-view
//! pipeline behind the console viewport. Chromium pushes encoded frames
//! continuously (`Page.startScreencast`); the worker fans each one out on the
//! `browser::frame-event` trigger (bind with a `session_id` filter) and keeps
//! the newest in memory, which `browser::frame` hands out for a viewer's
//! first paint. All three are internal console-UI plumbing, not agent
//! surface — agents read pages with `browser::snapshot` and
//! `browser::screenshot`.

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
    /// Frame cursor from the previous read; the response omits `frame` while
    /// the newest frame still has this seq.
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
