//! `browser::screenshot` — JPEG capture returned as viewable content blocks
//! (same envelope shape as `web::fetch` image responses, so the harness
//! renders it inline).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotInput {
    pub session_id: String,
    /// Capture the full scrollable page instead of the viewport.
    #[serde(default)]
    pub full_page: Option<bool>,
}

/// One block of a viewable response: an image block plus a text line.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ContentBlock {
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScreenshotDetails {
    pub session_id: String,
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScreenshotOutput {
    pub content: Vec<ContentBlock>,
    pub details: ScreenshotDetails,
}
