//! `computer::screenshot` — desktop capture returned as viewable content
//! blocks (an image block plus a text line), the same envelope the browser
//! worker and `web::fetch` use, so the harness renders it inline for the
//! model.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotInput {
    pub session_id: String,
}

/// One block of a viewable response: an image block or a text line.
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
    pub width: u32,
    pub height: u32,
    /// Detected image mime (`image/png` or `image/jpeg`).
    pub mime: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScreenshotOutput {
    pub content: Vec<ContentBlock>,
    pub details: ScreenshotDetails,
}
