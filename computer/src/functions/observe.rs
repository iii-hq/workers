//! `computer::observe` — a screenshot plus, optionally, the accessibility
//! tree. The image is the reliable signal on every guest; the a11y tree is
//! real on macOS and a stub or absent elsewhere, so it is opt-in and its
//! absence never fails the call.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::screenshot::ContentBlock;
use crate::backend::Screen;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ObserveInput {
    pub session_id: String,
    /// Also fetch the accessibility tree. macOS returns a real tree; other
    /// guests may return a stub or nothing (the field is then omitted).
    #[serde(default)]
    pub include_a11y: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ObserveDetails {
    pub session_id: String,
    pub screen: Screen,
    pub mime: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ObserveOutput {
    pub content: Vec<ContentBlock>,
    pub details: ObserveDetails,
    /// Accessibility tree, present only when requested and the guest exposes
    /// one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accessibility: Option<serde_json::Value>,
}
