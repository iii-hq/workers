//! Node type definitions.
//!
//! The `NodeKind` enum represents all node types in an XML document tree,
//! corresponding to libxml2's `xmlElementType`. Each variant carries the
//! node-type-specific payload (e.g., element name and attributes, text content).

use super::Attribute;

/// The kind of an XML node and its associated data.
///
/// This enum carries the payload for each node type. Navigation links
/// (parent, children, siblings) are stored in `NodeData`, not here.
#[derive(Debug, Clone)]
pub enum NodeKind {
    /// The document node — there is exactly one per `Document`.
    Document,

    /// An element node, e.g., `<div class="x">`.
    Element {
        /// The element's local name (or full `QName` before namespace resolution).
        name: String,
        /// Namespace prefix (e.g., `"svg"` in `svg:rect`), if any.
        prefix: Option<String>,
        /// Namespace URI after resolution, if any.
        namespace: Option<String>,
        /// Attributes on this element.
        attributes: Vec<Attribute>,
    },

    /// A text node containing character data.
    Text {
        /// The text content (already decoded — character references resolved).
        content: String,
    },

    /// A CDATA section, e.g., `<![CDATA[...]]>`.
    CData {
        /// The CDATA content (no escaping applied).
        content: String,
    },

    /// A comment node, e.g., `<!-- ... -->`.
    Comment {
        /// The comment text (without the `<!--` and `-->` delimiters).
        content: String,
    },

    /// A processing instruction, e.g., `<?target data?>`.
    ProcessingInstruction {
        /// The PI target (e.g., `"xml-stylesheet"`).
        target: String,
        /// The PI data, if any.
        data: Option<String>,
    },

    /// An entity reference node (e.g., `&amp;` when not expanded).
    EntityRef {
        /// The entity name (without `&` and `;`).
        name: String,
        /// The expanded value of the entity (used for `text_content()`).
        value: Option<String>,
    },

    /// A document type declaration node, e.g., `<!DOCTYPE html>`.
    ///
    /// See XML 1.0 §2.8: `[28]` doctypedecl
    DocumentType {
        /// The root element name declared in the DOCTYPE.
        name: String,
        /// The SYSTEM identifier (URI), if any.
        system_id: Option<String>,
        /// The PUBLIC identifier, if any.
        public_id: Option<String>,
        /// The serialized internal subset content (between `[` and `]`), if any.
        internal_subset: Option<String>,
    },
}
