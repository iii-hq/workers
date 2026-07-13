//! `browser::pick::hint` — hover preview for pick mode. The console UI
//! throttle-calls this on viewport mousemove and draws the highlight box
//! client-side over the screenshot, so the human sees what a click would
//! pick without waiting for the next screenshot frame.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::events::Bounds;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PickHintInput {
    pub session_id: String,
    /// Viewport x of the cursor.
    pub x: f64,
    /// Viewport y of the cursor.
    pub y: f64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PickHintOutput {
    /// False when no element sits at the point.
    pub hit: bool,
    /// Lowercase tag name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classes: Option<String>,
    /// Viewport-space box to draw the highlight over.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,
}
