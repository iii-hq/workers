//! `browser::dom::read` — DOM tree outline for the console's components
//! panel and for agents that need structure the a11y snapshot hides.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DomReadInput {
    pub session_id: String,
    /// Subtree root from an earlier ref (`e3`/`p1`) or dom node. Omit for
    /// the document root.
    #[serde(default)]
    pub r#ref: Option<String>,
    /// Levels of children to include (default 3).
    #[serde(default)]
    pub depth: Option<u32>,
}

/// One DOM node in the outline. `ref` resolves in `browser::act`,
/// `browser::styles::read`, and `browser::styles::write`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DomNode {
    pub r#ref: String,
    /// Lowercase tag (`div`, `button`) or node name (`#text`).
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classes: Option<String>,
    /// Trimmed text content for text nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Total children in the document, which may exceed `children` returned
    /// at this depth.
    pub child_count: u32,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<DomNode>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DomReadOutput {
    pub root: DomNode,
    /// True when the node cap cut the tree short; read a subtree via `ref`.
    pub truncated: bool,
}

pub const MAX_DOM_NODES: usize = 500;
pub const DEFAULT_DEPTH: u32 = 3;
