//! `browser::styles::read` / `write` — the design-sidebar backing: computed
//! styles for a picked element, and live inline-style edits that render on
//! the next screencast/screenshot without touching source files.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The curated property set returned when the caller does not name
/// properties: what a design panel shows (layout, box, type, color,
/// appearance), not all ~340 computed properties.
pub const DEFAULT_PROPERTIES: &[&str] = &[
    "display",
    "position",
    "flex-direction",
    "justify-content",
    "align-items",
    "gap",
    "grid-template-columns",
    "width",
    "height",
    "min-width",
    "max-width",
    "padding",
    "margin",
    "font-family",
    "font-size",
    "font-weight",
    "line-height",
    "letter-spacing",
    "text-align",
    "color",
    "background-color",
    "background-image",
    "border",
    "border-radius",
    "box-shadow",
    "opacity",
    "overflow",
    "z-index",
    "transform",
    "transition",
];

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StylesReadInput {
    pub session_id: String,
    /// Element ref from `browser::snapshot`, `browser::dom::read`, or a
    /// pick.
    pub r#ref: String,
    /// Computed property names to return. Omit for a curated design-panel
    /// set; pass `["*"]` for every computed property.
    #[serde(default)]
    pub properties: Option<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StyleProperty {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StylesReadOutput {
    pub r#ref: String,
    pub properties: Vec<StyleProperty>,
    /// The element's inline `style` attribute, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_style: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StylesWriteInput {
    pub session_id: String,
    /// Element ref to edit.
    pub r#ref: String,
    /// CSS property name (`background-color`).
    pub property: String,
    /// CSS value (`#101418`). Empty string removes the inline property.
    pub value: String,
    /// Apply with `!important`.
    #[serde(default)]
    pub important: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StylesWriteOutput {
    pub ok: bool,
    /// The element's inline `style` attribute after the edit.
    pub inline_style: String,
}
