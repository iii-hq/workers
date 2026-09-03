//! `browser::resize` — set the session's live viewport size. The console
//! calls this as its browser pane resizes so the streamed frame fills the
//! pane exactly (no letterboxing) and click coordinates map 1:1; the device
//! toolbar calls it with a preset. Applied as a CDP device-metrics override.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bounds a viewport dimension can take, so a stray pane measurement can't
/// ask Chromium for something absurd.
pub const MIN_DIM: u32 = 200;
pub const MAX_DIM: u32 = 4000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResizeInput {
    pub session_id: String,
    /// Viewport width in CSS pixels (clamped 200..4000).
    pub width: u32,
    /// Viewport height in CSS pixels (clamped 200..4000).
    pub height: u32,
    /// Device pixel ratio. Default 1.
    #[serde(default)]
    pub device_scale_factor: Option<f64>,
    /// Emulate a mobile device (viewport meta, overlay scrollbars, touch).
    /// Default false. The device toolbar sets this for phone presets.
    #[serde(default)]
    pub mobile: Option<bool>,
    /// Mark this resize as a pane auto-fit; a fit is refused (current size
    /// returned) while more than one viewer watches the session.
    // Keeps two open panes from fighting over the shared viewport; explicit
    // resizes (device toolbar, agents) always apply.
    #[serde(default)]
    pub fit: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ResizeOutput {
    pub ok: bool,
    /// The clamped size actually applied.
    pub width: u32,
    pub height: u32,
}

pub fn clamp(value: u32) -> u32 {
    value.clamp(MIN_DIM, MAX_DIM)
}
