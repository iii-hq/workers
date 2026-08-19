//! Arena-based XML document tree.
//!
//! This module implements the core tree representation using arena allocation
//! with typed indices. All nodes live in a contiguous `Vec<NodeData>` owned by
//! the `Document`, and are referenced by `NodeId` — a newtype over `NonZeroU32`.
//!
//! This design provides O(1) node access, cache-friendly layout, no reference
//! counting overhead, and safe bulk deallocation (drop the `Document` and
//! everything is freed).
//!
//! # Architecture
//!
//! Unlike libxml2's web of raw C pointers, we use arena indices for all
//! navigation links (parent, first\_child, last\_child, next\_sibling,
//! prev\_sibling). This avoids borrow checker issues, reference cycles,
//! and per-node heap allocation.

mod node;

pub use node::NodeKind;

use crate::error::{ParseDiagnostic, ParseError};
use std::collections::HashMap;
use std::num::NonZeroU32;

/// A typed index into the document's node arena.
///
/// `NodeId` is a newtype over `NonZeroU32`, meaning it can never be zero
/// and `Option<NodeId>` has the same size as `NodeId` (niche optimization).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct NodeId(NonZeroU32);

impl NodeId {
    /// Creates a `NodeId` from a raw index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is 0.
    #[allow(clippy::expect_used, clippy::cast_possible_truncation)]
    #[inline]
    fn from_index(index: usize) -> Self {
        Self(NonZeroU32::new(index as u32).expect("NodeId index must be non-zero"))
    }

    /// Returns the raw index as a `usize` for indexing into the arena.
    #[inline]
    fn as_index(self) -> usize {
        self.0.get() as usize
    }

    /// Converts this `NodeId` to a raw `u32` for FFI interop.
    ///
    /// The returned value is always non-zero (valid `NodeId`s start at 1).
    /// Use 0 to represent "no node" in FFI code.
    #[must_use]
    pub fn into_raw(self) -> u32 {
        self.0.get()
    }

    /// Creates a `NodeId` from a raw `u32`, if non-zero.
    ///
    /// Returns `None` if `raw` is 0 (which represents "no node" in FFI code).
    #[must_use]
    pub fn from_raw(raw: u32) -> Option<Self> {
        NonZeroU32::new(raw).map(Self)
    }
}

/// Storage for a single node in the document arena.
///
/// Each node stores its kind (element, text, comment, etc.) and links to
/// parent, children, and siblings for tree navigation. Access individual
/// nodes via [`Document::node`].
#[derive(Debug, Clone)]
pub struct NodeData {
    /// What kind of node this is (element, text, comment, etc.) and its payload.
    pub kind: NodeKind,
    /// Parent node, if any. The document root node has no parent.
    pub parent: Option<NodeId>,
    /// First child node.
    pub first_child: Option<NodeId>,
    /// Last child node (for O(1) append).
    pub last_child: Option<NodeId>,
    /// Next sibling.
    pub next_sibling: Option<NodeId>,
    /// Previous sibling.
    pub prev_sibling: Option<NodeId>,
}

impl NodeData {
    fn new(kind: NodeKind) -> Self {
        Self {
            kind,
            parent: None,
            first_child: None,
            last_child: None,
            next_sibling: None,
            prev_sibling: None,
        }
    }
}

/// An XML attribute on an element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// The attribute name (the local part, e.g., `"lang"` for `xml:lang`).
    pub name: String,
    /// The attribute value (fully expanded — entity references resolved).
    pub value: String,
    /// Namespace prefix, if any (e.g., `"xml"` for `xml:lang`).
    pub prefix: Option<String>,
    /// Namespace URI after resolution, if any.
    pub namespace: Option<String>,
    /// The original attribute value text before entity expansion, if it
    /// contained entity references. Used for serialization to preserve
    /// entity references in the output (matching libxml2 behavior).
    pub raw_value: Option<String>,
}

/// An XML document.
///
/// The `Document` owns all nodes in an arena and provides methods for
/// tree navigation and mutation. All tree operations go through
/// `&Document` (navigation) or `&mut Document` (mutation).
///
/// # Examples
///
/// ```
/// use xmloxide::Document;
///
/// let doc = Document::parse_str("<root/>").unwrap();
/// let root = doc.root_element().unwrap();
/// assert_eq!(doc.node_name(root), Some("root"));
/// ```
#[derive(Debug, Clone)]
pub struct Document {
    /// The node arena. Index 0 is unused (placeholder for `NonZeroU32`).
    nodes: Vec<NodeData>,
    /// The document root node id (the Document node, not the root element).
    root: NodeId,
    /// XML version from the XML declaration (e.g., "1.0").
    pub version: Option<String>,
    /// Encoding from the XML declaration (e.g., "UTF-8").
    pub encoding: Option<String>,
    /// Standalone flag from the XML declaration.
    pub standalone: Option<bool>,
    /// Diagnostics collected during parsing (warnings and recovered errors).
    pub diagnostics: Vec<ParseDiagnostic>,
    /// Mapping from ID attribute values to element nodes.
    ///
    /// Populated during DTD validation when attributes of type ID are
    /// validated. Used by [`element_by_id`](Document::element_by_id) and
    /// the `XPath` `id()` function.
    id_map: HashMap<String, NodeId>,
}

impl Document {
    /// Creates a new empty document.
    ///
    /// The document contains a single root Document node.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(64)
    }

    /// Creates a new empty document with pre-allocated capacity for `n` nodes.
    #[must_use]
    pub fn with_capacity(n: usize) -> Self {
        let mut nodes = Vec::with_capacity(n);
        // Index 0: placeholder (NodeId uses NonZeroU32)
        nodes.push(NodeData::new(NodeKind::Document));
        // Index 1: the document root node
        nodes.push(NodeData::new(NodeKind::Document));
        let root = NodeId::from_index(1);
        Self {
            nodes,
            root,
            version: None,
            encoding: None,
            standalone: None,
            diagnostics: Vec::new(),
            id_map: HashMap::new(),
        }
    }

    /// Parses an XML string into a `Document`.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the input is not well-formed XML.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::Document;
    ///
    /// let doc = Document::parse_str("<root><child/></root>").unwrap();
    /// ```
    pub fn parse_str(input: &str) -> Result<Self, ParseError> {
        // Strip leading UTF-8 BOM (U+FEFF) if present — per XML 1.0 §4.3.3,
        // the BOM is used for encoding detection and should be ignored.
        let had_bom = input.starts_with('\u{FEFF}');
        let input = input.strip_prefix('\u{FEFF}').unwrap_or(input);

        // Check encoding declaration compatibility.
        if let Some(enc) = Self::extract_encoding_from_decl(input) {
            let enc_lower = enc.to_ascii_lowercase();
            if had_bom && enc_lower != "utf-8" {
                // UTF-8 BOM present but encoding is not UTF-8.
                return Err(crate::error::ParseError {
                    message: format!("UTF-8 BOM present but encoding declared as '{enc}'"),
                    location: crate::error::SourceLocation::default(),
                    diagnostics: Vec::new(),
                });
            }
            // Reject clearly incompatible encodings (multi-byte or
            // non-ASCII-compatible). UTF-8, US-ASCII, and single-byte
            // ISO-8859 variants are compatible with UTF-8 text.
            let incompatible = enc_lower.starts_with("utf-16")
                || enc_lower.starts_with("utf-32")
                || enc_lower.starts_with("ucs")
                || enc_lower == "ebcdic";
            if incompatible {
                return Err(crate::error::ParseError {
                    message: format!(
                        "encoding declaration '{enc}' is incompatible with actual encoding"
                    ),
                    location: crate::error::SourceLocation::default(),
                    diagnostics: Vec::new(),
                });
            }
            // Reject encoding names that are not recognized by encoding_rs
            // (excluding UTF-8 and ASCII which are always valid for Rust strings).
            if enc_lower != "utf-8"
                && enc_lower != "us-ascii"
                && enc_lower != "ascii"
                && encoding_rs::Encoding::for_label(enc.as_bytes()).is_none()
            {
                return Err(crate::error::ParseError {
                    message: format!("unsupported encoding '{enc}'"),
                    location: crate::error::SourceLocation::default(),
                    diagnostics: Vec::new(),
                });
            }
        }

        crate::parser::parse_str(input)
    }

    /// Extracts the encoding value from an XML declaration, if present.
    fn extract_encoding_from_decl(input: &str) -> Option<String> {
        let trimmed = input.trim_start();
        if !trimmed.starts_with("<?xml") {
            return None;
        }
        // Find the end of the XML declaration
        let decl_end = trimmed.find("?>")?;
        let decl = &trimmed[..decl_end];

        // Look for encoding="..." or encoding='...'
        let enc_pos = decl.find("encoding")?;
        let after_enc = &decl[enc_pos + 8..].trim_start();
        let after_eq = after_enc.strip_prefix('=')?.trim_start();
        let quote = after_eq.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let value_start = 1;
        let value_end = after_eq[value_start..].find(quote)?;
        Some(after_eq[value_start..value_start + value_end].to_string())
    }

    /// Parses XML from raw bytes, detecting encoding automatically.
    ///
    /// Uses BOM sniffing and XML declaration inspection to determine the
    /// encoding, then transcodes to UTF-8 before parsing. See
    /// [`crate::encoding::decode_to_utf8`] for the full detection pipeline.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the encoding cannot be determined, the bytes
    /// cannot be transcoded, or the resulting XML is not well-formed.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::Document;
    ///
    /// let doc = Document::parse_bytes(b"<root/>").unwrap();
    /// let root = doc.root_element().unwrap();
    /// assert_eq!(doc.node_name(root), Some("root"));
    /// ```
    pub fn parse_bytes(input: &[u8]) -> Result<Self, ParseError> {
        use crate::encoding::decode_to_utf8;
        use crate::error::SourceLocation;

        let utf8 = decode_to_utf8(input).map_err(|e| ParseError {
            message: e.message,
            location: SourceLocation::default(),
            diagnostics: Vec::new(),
        })?;

        // Skip BOM and encoding checks — decode_to_utf8 already handled
        // encoding detection and transcoding. Go directly to the parser.
        let text = utf8.strip_prefix('\u{FEFF}').unwrap_or(&utf8);
        crate::parser::parse_str(text)
    }

    /// Parses an XML file from the filesystem.
    ///
    /// Reads the file as raw bytes and uses automatic encoding detection
    /// (BOM sniffing and XML declaration inspection) before parsing.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the file cannot be read, the encoding
    /// cannot be determined, or the XML is not well-formed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use xmloxide::Document;
    ///
    /// let doc = Document::parse_file("document.xml").unwrap();
    /// ```
    pub fn parse_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, ParseError> {
        use crate::error::SourceLocation;

        let bytes = std::fs::read(path.as_ref()).map_err(|e| ParseError {
            message: format!("failed to read file: {e}"),
            location: SourceLocation::default(),
            diagnostics: Vec::new(),
        })?;
        Self::parse_bytes(&bytes)
    }

    /// Returns the document root `NodeId`.
    ///
    /// This is the synthetic `Document` node that sits above the root element,
    /// processing instructions, comments, and DOCTYPE in the prolog. To get the
    /// root *element*, use [`root_element`](Self::root_element).
    #[must_use]
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Returns the root element of the document (the single top-level element).
    ///
    /// Returns `None` if the document has no element children.
    #[must_use]
    pub fn root_element(&self) -> Option<NodeId> {
        self.children(self.root)
            .find(|&id| matches!(self.node(id).kind, NodeKind::Element { .. }))
    }

    /// Returns a reference to the [`NodeData`] for the given node.
    ///
    /// Use this to inspect a node's [`kind`](NodeData::kind) and navigation
    /// links. For common queries, prefer the typed accessors like
    /// [`node_name`](Self::node_name) and [`node_text`](Self::node_text).
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to a valid node in this document.
    #[must_use]
    #[inline]
    pub fn node(&self, id: NodeId) -> &NodeData {
        &self.nodes[id.as_index()]
    }

    /// Returns a mutable reference to the `NodeData` for the given node.
    #[inline]
    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut NodeData {
        &mut self.nodes[id.as_index()]
    }

    /// Returns `true` if the node is an element.
    #[must_use]
    #[inline]
    pub fn is_element(&self, id: NodeId) -> bool {
        matches!(self.node(id).kind, NodeKind::Element { .. })
    }

    /// Returns the local name of a node, if applicable.
    ///
    /// For elements, this is the tag name (e.g., `"div"`). For processing
    /// instructions, this is the target (e.g., `"xml-stylesheet"`). Text,
    /// comment, CDATA, and document nodes return `None`.
    #[must_use]
    #[inline]
    pub fn node_name(&self, id: NodeId) -> Option<&str> {
        match &self.node(id).kind {
            NodeKind::Element { name, .. }
            | NodeKind::ProcessingInstruction { target: name, .. } => Some(name),
            _ => None,
        }
    }

    /// Returns the namespace URI of an element node, if any.
    ///
    /// Non-element nodes always return `None`. Elements that have no namespace
    /// declaration in scope also return `None`.
    #[must_use]
    pub fn node_namespace(&self, id: NodeId) -> Option<&str> {
        match &self.node(id).kind {
            NodeKind::Element { namespace, .. } => namespace.as_deref(),
            _ => None,
        }
    }

    /// Returns the namespace prefix of an element node, if any.
    ///
    /// For example, returns `Some("svg")` for `<svg:rect>`.
    /// Non-element nodes always return `None`.
    #[must_use]
    pub fn node_prefix(&self, id: NodeId) -> Option<&str> {
        match &self.node(id).kind {
            NodeKind::Element { prefix, .. } => prefix.as_deref(),
            _ => None,
        }
    }

    /// Returns the direct text content of a text, comment, CDATA, or PI node.
    ///
    /// For text, comment, and CDATA nodes, returns their string content. For
    /// processing instructions, returns the data portion (after the target).
    /// For element nodes, returns `None` — use
    /// [`text_content`](Self::text_content) to get the concatenated text of
    /// all descendant text nodes.
    #[must_use]
    pub fn node_text(&self, id: NodeId) -> Option<&str> {
        match &self.node(id).kind {
            NodeKind::Text { content }
            | NodeKind::Comment { content }
            | NodeKind::CData { content } => Some(content),
            NodeKind::ProcessingInstruction { data, .. } => data.as_deref(),
            _ => None,
        }
    }

    /// Returns the concatenated text content of a node and all its descendants.
    ///
    /// Recursively collects text from all descendant text and CDATA nodes.
    /// For a leaf text node, this is equivalent to [`node_text`](Self::node_text).
    /// For an element, this concatenates all nested text content (matching
    /// the DOM `textContent` property).
    #[must_use]
    pub fn text_content(&self, id: NodeId) -> String {
        let mut result = String::new();
        self.collect_text(id, &mut result);
        result
    }

    fn collect_text(&self, id: NodeId, buf: &mut String) {
        match &self.node(id).kind {
            NodeKind::Text { content } | NodeKind::CData { content } => {
                buf.push_str(content);
            }
            NodeKind::EntityRef { value, .. } => {
                // When the entity's replacement text was parsed as content
                // (XML 1.0 §4.4), the expansion lives in the children.
                // Fall back to the stored replacement value otherwise.
                if self.node(id).first_child.is_some() {
                    for child in self.children(id) {
                        self.collect_text(child, buf);
                    }
                } else if let Some(val) = value {
                    buf.push_str(val);
                }
            }
            _ => {
                for child in self.children(id) {
                    self.collect_text(child, buf);
                }
            }
        }
    }

    /// Returns the attributes of an element node as a slice.
    ///
    /// Each [`Attribute`] contains the name, value, optional namespace prefix,
    /// and namespace URI. Returns an empty slice for non-element nodes.
    #[must_use]
    #[inline]
    pub fn attributes(&self, id: NodeId) -> &[Attribute] {
        match &self.node(id).kind {
            NodeKind::Element { attributes, .. } => attributes,
            _ => &[],
        }
    }

    /// Returns the value of an attribute by local name on an element node.
    ///
    /// Performs a linear scan of the element's attributes. Returns `None` if
    /// the attribute is not present or the node is not an element.
    #[must_use]
    #[inline]
    pub fn attribute(&self, id: NodeId, name: &str) -> Option<&str> {
        self.attributes(id)
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }

    // --- ID lookup ---

    /// Associates an ID value with an element node.
    ///
    /// Called during DTD validation when an attribute of type ID is found.
    /// Subsequent calls to [`element_by_id`](Document::element_by_id) will
    /// return the associated node.
    pub fn set_id(&mut self, id: &str, node: NodeId) {
        self.id_map.insert(id.to_string(), node);
    }

    /// Looks up an element by its ID attribute value.
    ///
    /// Returns the `NodeId` of the element that was registered with
    /// [`set_id`](Document::set_id) for the given ID string, or `None`
    /// if no such ID exists.
    #[must_use]
    pub fn element_by_id(&self, id: &str) -> Option<NodeId> {
        self.id_map.get(id).copied()
    }

    // --- Navigation ---

    /// Returns the parent of a node, or `None` for the document root.
    #[must_use]
    #[inline]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    /// Returns the first child of a node, or `None` if it has no children.
    #[inline]
    #[must_use]
    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).first_child
    }

    /// Returns the last child of a node, or `None` if it has no children.
    #[inline]
    #[must_use]
    pub fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).last_child
    }

    /// Returns the next sibling of a node, or `None` if it is the last child.
    #[inline]
    #[must_use]
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).next_sibling
    }

    /// Returns the previous sibling of a node, or `None` if it is the first child.
    #[inline]
    #[must_use]
    pub fn prev_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).prev_sibling
    }

    /// Returns an iterator over the direct children of a node.
    ///
    /// Yields each child `NodeId` in document order (first child to last).
    /// For a depth-first traversal that includes nested descendants, use
    /// [`descendants`](Self::descendants).
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            doc: self,
            next: self.node(id).first_child,
        }
    }

    /// Returns an iterator over a node and its ancestors, walking up to the
    /// document root.
    ///
    /// The first item yielded is `id` itself, followed by its parent, then
    /// grandparent, and so on up to the document root node.
    pub fn ancestors(&self, id: NodeId) -> Ancestors<'_> {
        Ancestors {
            doc: self,
            next: Some(id),
        }
    }

    /// Returns an iterator over all descendants of a node in depth-first
    /// (pre-order) traversal.
    ///
    /// Does *not* yield `id` itself — only its children, grandchildren, etc.
    /// For iterating only the direct children, use
    /// [`children`](Self::children).
    pub fn descendants(&self, id: NodeId) -> Descendants<'_> {
        Descendants {
            doc: self,
            root: id,
            next: self.first_child(id),
        }
    }

    // --- Mutation ---

    /// Allocates a new node in the arena and returns its `NodeId`.
    ///
    /// The new node is detached (has no parent). Use
    /// [`append_child`](Self::append_child),
    /// [`prepend_child`](Self::prepend_child), or
    /// [`insert_before`](Self::insert_before) to attach it to the tree.
    pub fn create_node(&mut self, kind: NodeKind) -> NodeId {
        let index = self.nodes.len();
        self.nodes.push(NodeData::new(kind));
        NodeId::from_index(index)
    }

    /// Appends a child node to the end of a parent's child list.
    ///
    /// The child becomes the new [`last_child`](Self::last_child) of `parent`.
    /// If the parent had no children, the child also becomes the
    /// [`first_child`](Self::first_child).
    ///
    /// # Panics
    ///
    /// Panics (debug-only) if `child` already has a parent. Call
    /// [`detach`](Self::detach) first to re-parent an existing node.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) {
        debug_assert!(
            self.node(child).parent.is_none(),
            "child already has a parent; detach it first"
        );

        self.node_mut(child).parent = Some(parent);

        if let Some(last) = self.node(parent).last_child {
            self.node_mut(last).next_sibling = Some(child);
            self.node_mut(child).prev_sibling = Some(last);
            self.node_mut(parent).last_child = Some(child);
        } else {
            self.node_mut(parent).first_child = Some(child);
            self.node_mut(parent).last_child = Some(child);
        }
    }

    /// Inserts `new_child` immediately before `reference` in the sibling list.
    ///
    /// The new child is given the same parent as `reference` and is linked as
    /// its previous sibling.
    ///
    /// # Panics
    ///
    /// Panics if `reference` has no parent or if `new_child` already has a
    /// parent (detach it first).
    #[allow(clippy::expect_used)]
    pub fn insert_before(&mut self, reference: NodeId, new_child: NodeId) {
        debug_assert!(
            self.node(new_child).parent.is_none(),
            "new_child already has a parent; detach it first"
        );

        let parent = self
            .node(reference)
            .parent
            .expect("reference has no parent");
        self.node_mut(new_child).parent = Some(parent);

        if let Some(prev) = self.node(reference).prev_sibling {
            self.node_mut(prev).next_sibling = Some(new_child);
            self.node_mut(new_child).prev_sibling = Some(prev);
        } else {
            self.node_mut(parent).first_child = Some(new_child);
        }

        self.node_mut(new_child).next_sibling = Some(reference);
        self.node_mut(reference).prev_sibling = Some(new_child);
    }

    /// Prepends a child node as the first child of a parent.
    ///
    /// If the parent already has children, the new child is inserted before
    /// the current first child. Otherwise, it becomes the only child.
    pub fn prepend_child(&mut self, parent: NodeId, child: NodeId) {
        if let Some(first) = self.first_child(parent) {
            self.insert_before(first, child);
        } else {
            self.append_child(parent, child);
        }
    }

    /// Removes a node from the tree by detaching it from its parent.
    ///
    /// The node and its subtree remain allocated in the arena but become
    /// unreachable through tree navigation. This is an alias for
    /// [`detach`](Self::detach).
    pub fn remove_node(&mut self, id: NodeId) {
        self.detach(id);
    }

    /// Detaches a node from its parent without freeing it from the arena.
    ///
    /// Updates sibling and parent links so the node is no longer reachable
    /// through tree traversal. The node's own children are left intact, so the
    /// detached subtree remains internally connected. If the node has no
    /// parent, this is a no-op.
    pub fn detach(&mut self, id: NodeId) {
        let Some(parent) = self.node(id).parent else {
            return;
        };

        let prev = self.node(id).prev_sibling;
        let next = self.node(id).next_sibling;

        match prev {
            Some(p) => self.node_mut(p).next_sibling = next,
            None => self.node_mut(parent).first_child = next,
        }

        match next {
            Some(n) => self.node_mut(n).prev_sibling = prev,
            None => self.node_mut(parent).last_child = prev,
        }

        self.node_mut(id).parent = None;
        self.node_mut(id).prev_sibling = None;
        self.node_mut(id).next_sibling = None;
    }

    /// Deep-copies a node and all its descendants within this document.
    ///
    /// The cloned subtree is detached (has no parent). If `deep` is `false`,
    /// only the node itself is cloned without its children.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::Document;
    ///
    /// let mut doc = Document::parse_str("<root><child>Hello</child></root>").unwrap();
    /// let root = doc.root_element().unwrap();
    /// let child = doc.first_child(root).unwrap();
    /// let cloned = doc.clone_node(child, true);
    /// doc.append_child(root, cloned);
    /// ```
    pub fn clone_node(&mut self, id: NodeId, deep: bool) -> NodeId {
        let kind = self.node(id).kind.clone();
        let new_id = self.create_node(kind);
        if deep {
            // Clone children recursively
            let children: Vec<NodeId> = self.children(id).collect();
            for child_id in children {
                let cloned_child = self.clone_node(child_id, true);
                self.append_child(new_id, cloned_child);
            }
        }
        new_id
    }

    /// Sets the text content of a text, CDATA, or comment node.
    ///
    /// For element nodes, this removes all children and replaces them with
    /// a single text node containing the given content.
    ///
    /// Returns `true` if the content was set, `false` if the node type does
    /// not support text content (e.g., the document root).
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::Document;
    ///
    /// let mut doc = Document::parse_str("<root>old</root>").unwrap();
    /// let root = doc.root_element().unwrap();
    /// let text_node = doc.first_child(root).unwrap();
    /// assert!(doc.set_text_content(text_node, "new"));
    /// assert_eq!(doc.text_content(root), "new");
    /// ```
    pub fn set_text_content(&mut self, id: NodeId, content: &str) -> bool {
        match &self.node(id).kind {
            NodeKind::Text { .. } | NodeKind::CData { .. } | NodeKind::Comment { .. } => {
                match &mut self.node_mut(id).kind {
                    NodeKind::Text {
                        content: ref mut c, ..
                    }
                    | NodeKind::CData {
                        content: ref mut c, ..
                    }
                    | NodeKind::Comment {
                        content: ref mut c, ..
                    } => {
                        *c = content.to_string();
                    }
                    _ => unreachable!(),
                }
                true
            }
            NodeKind::Element { .. } => {
                // Remove all children
                let children: Vec<NodeId> = self.children(id).collect();
                for child in children {
                    self.remove_node(child);
                }
                // Add a single text node
                let text = self.create_node(NodeKind::Text {
                    content: content.to_string(),
                });
                self.append_child(id, text);
                true
            }
            NodeKind::ProcessingInstruction { .. } => {
                if let NodeKind::ProcessingInstruction { data, .. } = &mut self.node_mut(id).kind {
                    *data = Some(content.to_string());
                }
                true
            }
            NodeKind::Document | NodeKind::DocumentType { .. } | NodeKind::EntityRef { .. } => {
                false
            }
        }
    }

    /// Creates a new element node (detached) and returns its `NodeId`.
    ///
    /// Use [`append_child`](Self::append_child), [`prepend_child`](Self::prepend_child),
    /// or [`insert_before`](Self::insert_before) to attach it.
    pub fn create_element(&mut self, name: &str) -> NodeId {
        self.create_node(NodeKind::Element {
            name: name.to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        })
    }

    /// Creates a new text node (detached) and returns its `NodeId`.
    pub fn create_text(&mut self, content: &str) -> NodeId {
        self.create_node(NodeKind::Text {
            content: content.to_string(),
        })
    }

    /// Creates a new comment node (detached) and returns its `NodeId`.
    pub fn create_comment(&mut self, content: &str) -> NodeId {
        self.create_node(NodeKind::Comment {
            content: content.to_string(),
        })
    }

    /// Creates a new processing instruction node (detached) and returns its `NodeId`.
    pub fn create_processing_instruction(&mut self, target: &str, data: Option<&str>) -> NodeId {
        self.create_node(NodeKind::ProcessingInstruction {
            target: target.to_string(),
            data: data.map(ToString::to_string),
        })
    }

    /// Inserts `new_child` immediately after `reference` in the sibling list.
    ///
    /// If `reference` is the last child, this is equivalent to appending to the parent.
    ///
    /// # Panics
    ///
    /// Panics if `reference` has no parent or if `new_child` already has a parent.
    #[allow(clippy::expect_used)]
    pub fn insert_after(&mut self, reference: NodeId, new_child: NodeId) {
        debug_assert!(
            self.node(new_child).parent.is_none(),
            "new_child already has a parent; detach it first"
        );

        let parent = self
            .node(reference)
            .parent
            .expect("reference has no parent");
        self.node_mut(new_child).parent = Some(parent);

        if let Some(next) = self.node(reference).next_sibling {
            self.node_mut(next).prev_sibling = Some(new_child);
            self.node_mut(new_child).next_sibling = Some(next);
        } else {
            self.node_mut(parent).last_child = Some(new_child);
        }

        self.node_mut(new_child).prev_sibling = Some(reference);
        self.node_mut(reference).next_sibling = Some(new_child);
    }

    /// Replaces `old_node` with `new_node` in the tree.
    ///
    /// The old node is detached and the new node takes its place in the
    /// sibling list. Returns the id of the old (now detached) node.
    ///
    /// # Panics
    ///
    /// Panics if `old_node` has no parent or if `new_node` already has a parent.
    pub fn replace_node(&mut self, old_node: NodeId, new_node: NodeId) -> NodeId {
        self.insert_before(old_node, new_node);
        self.detach(old_node);
        old_node
    }

    /// Sets an attribute on an element node. If the attribute already exists,
    /// its value is updated. Returns `true` on success, `false` if the node
    /// is not an element.
    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) -> bool {
        if let NodeKind::Element { attributes, .. } = &mut self.node_mut(id).kind {
            if let Some(attr) = attributes.iter_mut().find(|a| a.name == name) {
                attr.value = value.to_string();
                attr.raw_value = None;
            } else {
                attributes.push(Attribute {
                    name: name.to_string(),
                    value: value.to_string(),
                    prefix: None,
                    namespace: None,
                    raw_value: None,
                });
            }
            true
        } else {
            false
        }
    }

    /// Removes an attribute by name from an element node.
    ///
    /// Returns `true` if the attribute was found and removed, `false` if the
    /// attribute was not present or the node is not an element.
    pub fn remove_attribute(&mut self, id: NodeId, name: &str) -> bool {
        if let NodeKind::Element { attributes, .. } = &mut self.node_mut(id).kind {
            let len = attributes.len();
            attributes.retain(|a| a.name != name);
            attributes.len() < len
        } else {
            false
        }
    }

    /// Renames an element node. Returns `true` on success, `false` if the
    /// node is not an element.
    pub fn rename_element(&mut self, id: NodeId, new_name: &str) -> bool {
        if let NodeKind::Element { name, .. } = &mut self.node_mut(id).kind {
            *name = new_name.to_string();
            true
        } else {
            false
        }
    }

    /// Returns the total number of nodes in the document.
    ///
    /// This includes all node types (elements, text, comments, etc.) but
    /// excludes the internal arena placeholder. Detached nodes that have not
    /// been garbage-collected are still counted.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len() - 1 // subtract placeholder at index 0
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

// --- Iterators ---

/// Iterator over the direct children of a node.
///
/// Created by [`Document::children`]. Yields each child's `NodeId` in
/// document order by following `next_sibling` links.
pub struct Children<'a> {
    doc: &'a Document,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = self.doc.nodes[current.as_index()].next_sibling;
        Some(current)
    }
}

/// Iterator over a node and its ancestors, walking up toward the document root.
///
/// Created by [`Document::ancestors`]. The first item yielded is the starting
/// node itself, followed by its parent, grandparent, etc.
pub struct Ancestors<'a> {
    doc: &'a Document,
    next: Option<NodeId>,
}

impl Iterator for Ancestors<'_> {
    type Item = NodeId;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = self.doc.node(current).parent;
        Some(current)
    }
}

/// Depth-first (pre-order) iterator over all descendants of a node.
///
/// Created by [`Document::descendants`]. Yields every node in the subtree
/// below the starting node, but does *not* yield the starting node itself.
pub struct Descendants<'a> {
    doc: &'a Document,
    root: NodeId,
    next: Option<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        let nodes = &self.doc.nodes;

        // Try to go deeper first
        let node = &nodes[current.as_index()];
        if let Some(child) = node.first_child {
            self.next = Some(child);
            return Some(current);
        }

        // Try next sibling
        if let Some(sibling) = node.next_sibling {
            self.next = Some(sibling);
            return Some(current);
        }

        // Walk up to find an ancestor with a next sibling
        let mut ancestor = node.parent;
        while let Some(anc) = ancestor {
            if anc == self.root {
                self.next = None;
                return Some(current);
            }
            let anc_node = &nodes[anc.as_index()];
            if let Some(sibling) = anc_node.next_sibling {
                self.next = Some(sibling);
                return Some(current);
            }
            ancestor = anc_node.parent;
        }

        self.next = None;
        Some(current)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_new_document_has_root() {
        let doc = Document::new();
        assert!(matches!(doc.node(doc.root()).kind, NodeKind::Document));
        assert_eq!(doc.node_count(), 1); // just the root
    }

    #[test]
    fn test_create_and_append_element() {
        let mut doc = Document::new();
        let root = doc.root();
        let elem = doc.create_node(NodeKind::Element {
            name: "div".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        doc.append_child(root, elem);

        assert_eq!(doc.first_child(root), Some(elem));
        assert_eq!(doc.last_child(root), Some(elem));
        assert_eq!(doc.parent(elem), Some(root));
        assert_eq!(doc.node_name(elem), Some("div"));
    }

    #[test]
    fn test_append_multiple_children() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        let c = doc.create_node(NodeKind::Text {
            content: "C".to_string(),
        });

        doc.append_child(root, a);
        doc.append_child(root, b);
        doc.append_child(root, c);

        assert_eq!(doc.first_child(root), Some(a));
        assert_eq!(doc.last_child(root), Some(c));
        assert_eq!(doc.next_sibling(a), Some(b));
        assert_eq!(doc.next_sibling(b), Some(c));
        assert_eq!(doc.next_sibling(c), None);
        assert_eq!(doc.prev_sibling(c), Some(b));
        assert_eq!(doc.prev_sibling(b), Some(a));
        assert_eq!(doc.prev_sibling(a), None);
    }

    #[test]
    fn test_children_iterator() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        let c = doc.create_node(NodeKind::Text {
            content: "C".to_string(),
        });

        doc.append_child(root, a);
        doc.append_child(root, b);
        doc.append_child(root, c);

        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children, vec![a, b, c]);
    }

    #[test]
    fn test_children_iterator_empty() {
        let doc = Document::new();
        let children: Vec<NodeId> = doc.children(doc.root()).collect();
        assert!(children.is_empty());
    }

    #[test]
    fn test_insert_before() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let c = doc.create_node(NodeKind::Text {
            content: "C".to_string(),
        });
        doc.append_child(root, a);
        doc.append_child(root, c);

        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        doc.insert_before(c, b);

        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children, vec![a, b, c]);
        assert_eq!(doc.parent(b), Some(root));
    }

    #[test]
    fn test_insert_before_first_child() {
        let mut doc = Document::new();
        let root = doc.root();

        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        doc.append_child(root, b);

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        doc.insert_before(b, a);

        assert_eq!(doc.first_child(root), Some(a));
        assert_eq!(doc.next_sibling(a), Some(b));
    }

    #[test]
    fn test_detach() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        let c = doc.create_node(NodeKind::Text {
            content: "C".to_string(),
        });

        doc.append_child(root, a);
        doc.append_child(root, b);
        doc.append_child(root, c);

        doc.detach(b);

        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children, vec![a, c]);
        assert_eq!(doc.parent(b), None);
        assert_eq!(doc.next_sibling(a), Some(c));
        assert_eq!(doc.prev_sibling(c), Some(a));
    }

    #[test]
    fn test_detach_first_child() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        doc.append_child(root, a);
        doc.append_child(root, b);

        doc.detach(a);
        assert_eq!(doc.first_child(root), Some(b));
        assert_eq!(doc.prev_sibling(b), None);
    }

    #[test]
    fn test_detach_last_child() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        doc.append_child(root, a);
        doc.append_child(root, b);

        doc.detach(b);
        assert_eq!(doc.last_child(root), Some(a));
        assert_eq!(doc.next_sibling(a), None);
    }

    #[test]
    fn test_detach_only_child() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        doc.append_child(root, a);
        doc.detach(a);

        assert_eq!(doc.first_child(root), None);
        assert_eq!(doc.last_child(root), None);
    }

    #[test]
    fn test_ancestors_iterator() {
        let mut doc = Document::new();
        let root = doc.root();

        let parent = doc.create_node(NodeKind::Element {
            name: "parent".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        let child = doc.create_node(NodeKind::Element {
            name: "child".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });

        doc.append_child(root, parent);
        doc.append_child(parent, child);

        let ancestors: Vec<NodeId> = doc.ancestors(child).collect();
        assert_eq!(ancestors, vec![child, parent, root]);
    }

    #[test]
    fn test_descendants_iterator() {
        let mut doc = Document::new();
        let root = doc.root();

        let p = doc.create_node(NodeKind::Element {
            name: "p".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        let a = doc.create_node(NodeKind::Text {
            content: "hello ".to_string(),
        });
        let b = doc.create_node(NodeKind::Element {
            name: "b".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        let b_text = doc.create_node(NodeKind::Text {
            content: "world".to_string(),
        });

        doc.append_child(root, p);
        doc.append_child(p, a);
        doc.append_child(p, b);
        doc.append_child(b, b_text);

        let desc: Vec<NodeId> = doc.descendants(root).collect();
        assert_eq!(desc, vec![p, a, b, b_text]);
    }

    #[test]
    fn test_text_content() {
        let mut doc = Document::new();
        let root = doc.root();

        let p = doc.create_node(NodeKind::Element {
            name: "p".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        let text1 = doc.create_node(NodeKind::Text {
            content: "hello ".to_string(),
        });
        let bold = doc.create_node(NodeKind::Element {
            name: "b".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        let text2 = doc.create_node(NodeKind::Text {
            content: "world".to_string(),
        });

        doc.append_child(root, p);
        doc.append_child(p, text1);
        doc.append_child(p, bold);
        doc.append_child(bold, text2);

        assert_eq!(doc.text_content(p), "hello world");
    }

    #[test]
    fn test_text_content_entity_ref_children_take_priority() {
        // When an EntityRef node carries parsed replacement children
        // (XML 1.0 §4.4), text_content reads them, not the stored value.
        let mut doc = Document::new();
        let root = doc.root();
        let entity = doc.create_node(NodeKind::EntityRef {
            name: "e".to_string(),
            value: Some("raw value".to_string()),
        });
        let text = doc.create_node(NodeKind::Text {
            content: "expanded".to_string(),
        });
        doc.append_child(root, entity);
        doc.append_child(entity, text);
        assert_eq!(doc.text_content(root), "expanded");
    }

    #[test]
    fn test_text_content_entity_ref_value_fallback() {
        // A childless EntityRef (e.g. built programmatically) falls back to
        // its stored replacement value.
        let mut doc = Document::new();
        let root = doc.root();
        let entity = doc.create_node(NodeKind::EntityRef {
            name: "e".to_string(),
            value: Some("fallback".to_string()),
        });
        doc.append_child(root, entity);
        assert_eq!(doc.text_content(root), "fallback");
    }

    #[test]
    fn test_attributes() {
        let mut doc = Document::new();
        let root = doc.root();

        let elem = doc.create_node(NodeKind::Element {
            name: "div".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![
                Attribute {
                    name: "id".to_string(),
                    value: "main".to_string(),
                    prefix: None,
                    namespace: None,
                    raw_value: None,
                },
                Attribute {
                    name: "class".to_string(),
                    value: "container".to_string(),
                    prefix: None,
                    namespace: None,
                    raw_value: None,
                },
            ],
        });
        doc.append_child(root, elem);

        assert_eq!(doc.attribute(elem, "id"), Some("main"));
        assert_eq!(doc.attribute(elem, "class"), Some("container"));
        assert_eq!(doc.attribute(elem, "style"), None);
        assert_eq!(doc.attributes(elem).len(), 2);
    }

    #[test]
    fn test_root_element() {
        let mut doc = Document::new();
        let root = doc.root();

        // No element children yet
        assert_eq!(doc.root_element(), None);

        let elem = doc.create_node(NodeKind::Element {
            name: "root".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        doc.append_child(root, elem);

        assert_eq!(doc.root_element(), Some(elem));
    }

    #[test]
    fn test_node_text() {
        let mut doc = Document::new();

        let text = doc.create_node(NodeKind::Text {
            content: "hello".to_string(),
        });
        assert_eq!(doc.node_text(text), Some("hello"));

        let comment = doc.create_node(NodeKind::Comment {
            content: "a comment".to_string(),
        });
        assert_eq!(doc.node_text(comment), Some("a comment"));

        let cdata = doc.create_node(NodeKind::CData {
            content: "cdata content".to_string(),
        });
        assert_eq!(doc.node_text(cdata), Some("cdata content"));

        let elem = doc.create_node(NodeKind::Element {
            name: "div".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        assert_eq!(doc.node_text(elem), None);
    }

    #[test]
    fn test_element_by_id_none() {
        let doc = Document::new();
        assert_eq!(doc.element_by_id("nonexistent"), None);
    }

    #[test]
    fn test_set_id_and_lookup() {
        let mut doc = Document::new();
        let root = doc.root();
        let elem = doc.create_node(NodeKind::Element {
            name: "item".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        doc.append_child(root, elem);
        doc.set_id("a", elem);
        assert_eq!(doc.element_by_id("a"), Some(elem));
        assert_eq!(doc.element_by_id("b"), None);
    }

    #[test]
    fn test_remove_node_middle_child() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        let c = doc.create_node(NodeKind::Text {
            content: "C".to_string(),
        });

        doc.append_child(root, a);
        doc.append_child(root, b);
        doc.append_child(root, c);

        doc.remove_node(b);

        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children, vec![a, c]);
        assert_eq!(doc.parent(b), None);
        assert_eq!(doc.next_sibling(b), None);
        assert_eq!(doc.prev_sibling(b), None);
        assert_eq!(doc.next_sibling(a), Some(c));
        assert_eq!(doc.prev_sibling(c), Some(a));
    }

    #[test]
    fn test_remove_node_only_child() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Element {
            name: "only".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        doc.append_child(root, a);

        doc.remove_node(a);

        assert_eq!(doc.first_child(root), None);
        assert_eq!(doc.last_child(root), None);
        assert_eq!(doc.parent(a), None);
    }

    #[test]
    fn test_remove_node_first_child() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        doc.append_child(root, a);
        doc.append_child(root, b);

        doc.remove_node(a);

        assert_eq!(doc.first_child(root), Some(b));
        assert_eq!(doc.prev_sibling(b), None);
        assert_eq!(doc.parent(a), None);
    }

    #[test]
    fn test_remove_node_last_child() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        doc.append_child(root, a);
        doc.append_child(root, b);

        doc.remove_node(b);

        assert_eq!(doc.last_child(root), Some(a));
        assert_eq!(doc.next_sibling(a), None);
        assert_eq!(doc.parent(b), None);
    }

    #[test]
    fn test_remove_node_no_parent_is_noop() {
        let mut doc = Document::new();
        let orphan = doc.create_node(NodeKind::Text {
            content: "orphan".to_string(),
        });

        // Removing a node with no parent should not panic
        doc.remove_node(orphan);

        assert_eq!(doc.parent(orphan), None);
    }

    #[test]
    fn test_prepend_child_to_empty_parent() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Element {
            name: "first".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });

        doc.prepend_child(root, a);

        assert_eq!(doc.first_child(root), Some(a));
        assert_eq!(doc.last_child(root), Some(a));
        assert_eq!(doc.parent(a), Some(root));
    }

    #[test]
    fn test_prepend_child_before_existing() {
        let mut doc = Document::new();
        let root = doc.root();

        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        let c = doc.create_node(NodeKind::Text {
            content: "C".to_string(),
        });
        doc.append_child(root, b);
        doc.append_child(root, c);

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        doc.prepend_child(root, a);

        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children, vec![a, b, c]);
        assert_eq!(doc.first_child(root), Some(a));
        assert_eq!(doc.last_child(root), Some(c));
        assert_eq!(doc.parent(a), Some(root));
        assert_eq!(doc.next_sibling(a), Some(b));
        assert_eq!(doc.prev_sibling(b), Some(a));
    }

    #[test]
    fn test_prepend_child_multiple_times() {
        let mut doc = Document::new();
        let root = doc.root();

        let c = doc.create_node(NodeKind::Text {
            content: "C".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });

        doc.prepend_child(root, c);
        doc.prepend_child(root, b);
        doc.prepend_child(root, a);

        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children, vec![a, b, c]);
    }

    #[test]
    fn test_node_namespace_with_namespace() {
        let mut doc = Document::new();
        let elem = doc.create_node(NodeKind::Element {
            name: "rect".to_string(),
            prefix: Some("svg".to_string()),
            namespace: Some("http://www.w3.org/2000/svg".to_string()),
            attributes: vec![],
        });

        assert_eq!(doc.node_namespace(elem), Some("http://www.w3.org/2000/svg"));
    }

    #[test]
    fn test_node_namespace_without_namespace() {
        let mut doc = Document::new();
        let elem = doc.create_node(NodeKind::Element {
            name: "div".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });

        assert_eq!(doc.node_namespace(elem), None);
    }

    #[test]
    fn test_node_namespace_non_element() {
        let mut doc = Document::new();
        let text = doc.create_node(NodeKind::Text {
            content: "hello".to_string(),
        });
        let comment = doc.create_node(NodeKind::Comment {
            content: "a comment".to_string(),
        });

        assert_eq!(doc.node_namespace(text), None);
        assert_eq!(doc.node_namespace(comment), None);
    }

    #[test]
    fn test_first_child_of_leaf_node() {
        let mut doc = Document::new();
        let root = doc.root();

        let leaf = doc.create_node(NodeKind::Text {
            content: "leaf".to_string(),
        });
        doc.append_child(root, leaf);

        assert_eq!(doc.first_child(leaf), None);
    }

    #[test]
    fn test_last_child_of_leaf_node() {
        let mut doc = Document::new();
        let root = doc.root();

        let leaf = doc.create_node(NodeKind::Text {
            content: "leaf".to_string(),
        });
        doc.append_child(root, leaf);

        assert_eq!(doc.last_child(leaf), None);
    }

    #[test]
    fn test_first_child_last_child_single_child() {
        let mut doc = Document::new();
        let root = doc.root();

        let only = doc.create_node(NodeKind::Element {
            name: "only".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        doc.append_child(root, only);

        assert_eq!(doc.first_child(root), Some(only));
        assert_eq!(doc.last_child(root), Some(only));
    }

    #[test]
    fn test_first_child_last_child_multiple_children() {
        let mut doc = Document::new();
        let root = doc.root();

        let first = doc.create_node(NodeKind::Text {
            content: "first".to_string(),
        });
        let middle = doc.create_node(NodeKind::Text {
            content: "middle".to_string(),
        });
        let last = doc.create_node(NodeKind::Text {
            content: "last".to_string(),
        });

        doc.append_child(root, first);
        doc.append_child(root, middle);
        doc.append_child(root, last);

        assert_eq!(doc.first_child(root), Some(first));
        assert_eq!(doc.last_child(root), Some(last));
        assert_ne!(doc.first_child(root), doc.last_child(root));
    }

    #[test]
    fn test_next_sibling_last_has_none() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });

        doc.append_child(root, a);
        doc.append_child(root, b);

        assert_eq!(doc.next_sibling(a), Some(b));
        assert_eq!(doc.next_sibling(b), None);
    }

    #[test]
    fn test_prev_sibling_first_has_none() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });

        doc.append_child(root, a);
        doc.append_child(root, b);

        assert_eq!(doc.prev_sibling(a), None);
        assert_eq!(doc.prev_sibling(b), Some(a));
    }

    #[test]
    fn test_next_prev_sibling_chain() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        let b = doc.create_node(NodeKind::Text {
            content: "B".to_string(),
        });
        let c = doc.create_node(NodeKind::Text {
            content: "C".to_string(),
        });

        doc.append_child(root, a);
        doc.append_child(root, b);
        doc.append_child(root, c);

        // Forward traversal
        assert_eq!(doc.next_sibling(a), Some(b));
        assert_eq!(doc.next_sibling(b), Some(c));
        assert_eq!(doc.next_sibling(c), None);

        // Backward traversal
        assert_eq!(doc.prev_sibling(c), Some(b));
        assert_eq!(doc.prev_sibling(b), Some(a));
        assert_eq!(doc.prev_sibling(a), None);
    }

    #[test]
    fn test_parse_str_simple_element() {
        let Ok(doc) = Document::parse_str("<root/>") else {
            panic!("failed to parse simple element");
        };
        let Some(root) = doc.root_element() else {
            panic!("parsed document has no root element");
        };
        assert_eq!(doc.node_name(root), Some("root"));
    }

    #[test]
    fn test_parse_str_nested_elements() {
        let Ok(doc) = Document::parse_str("<parent><child/></parent>") else {
            panic!("failed to parse nested elements");
        };
        let Some(root) = doc.root_element() else {
            panic!("parsed document has no root element");
        };
        assert_eq!(doc.node_name(root), Some("parent"));

        let Some(child) = doc.first_child(root) else {
            panic!("root element has no children");
        };
        assert_eq!(doc.node_name(child), Some("child"));
    }

    #[test]
    fn test_parse_str_with_text_content() {
        let Ok(doc) = Document::parse_str("<msg>hello</msg>") else {
            panic!("failed to parse element with text");
        };
        let Some(root) = doc.root_element() else {
            panic!("parsed document has no root element");
        };
        assert_eq!(doc.text_content(root), "hello");
    }

    #[test]
    fn test_parse_str_with_attributes() {
        let Ok(doc) = Document::parse_str(r#"<div id="main" class="x"/>"#) else {
            panic!("failed to parse element with attributes");
        };
        let Some(root) = doc.root_element() else {
            panic!("parsed document has no root element");
        };
        assert_eq!(doc.attribute(root, "id"), Some("main"));
        assert_eq!(doc.attribute(root, "class"), Some("x"));
    }

    #[test]
    fn test_parse_bytes_utf8() {
        let input = b"<root>hello</root>";
        let Ok(doc) = Document::parse_bytes(input) else {
            panic!("failed to parse bytes");
        };
        let Some(root) = doc.root_element() else {
            panic!("parsed document has no root element");
        };
        assert_eq!(doc.node_name(root), Some("root"));
        assert_eq!(doc.text_content(root), "hello");
    }

    #[test]
    fn test_parse_bytes_with_xml_declaration() {
        let input = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><data/>";
        let Ok(doc) = Document::parse_bytes(input) else {
            panic!("failed to parse bytes with XML declaration");
        };
        let Some(root) = doc.root_element() else {
            panic!("parsed document has no root element");
        };
        assert_eq!(doc.node_name(root), Some("data"));
    }

    #[test]
    fn test_parse_bytes_with_bom() {
        // UTF-8 BOM followed by XML
        let mut input = vec![0xEF, 0xBB, 0xBF];
        input.extend_from_slice(b"<root/>");
        let Ok(doc) = Document::parse_bytes(&input) else {
            panic!("failed to parse bytes with BOM");
        };
        let Some(root) = doc.root_element() else {
            panic!("parsed document has no root element");
        };
        assert_eq!(doc.node_name(root), Some("root"));
    }

    #[test]
    fn test_node_count_empty_document() {
        let doc = Document::new();
        // A new document has exactly 1 node: the document root node
        assert_eq!(doc.node_count(), 1);
    }

    #[test]
    fn test_node_count_after_creating_nodes() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Element {
            name: "a".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        assert_eq!(doc.node_count(), 2);

        let b = doc.create_node(NodeKind::Text {
            content: "text".to_string(),
        });
        assert_eq!(doc.node_count(), 3);

        doc.append_child(root, a);
        doc.append_child(a, b);

        // Appending does not change the count — nodes already exist in arena
        assert_eq!(doc.node_count(), 3);
    }

    #[test]
    fn test_node_count_after_remove() {
        let mut doc = Document::new();
        let root = doc.root();

        let a = doc.create_node(NodeKind::Text {
            content: "A".to_string(),
        });
        doc.append_child(root, a);
        assert_eq!(doc.node_count(), 2);

        // Removing a node does not free it from the arena
        doc.remove_node(a);
        assert_eq!(doc.node_count(), 2);
    }

    #[test]
    fn test_clone_node_shallow() {
        let mut doc = Document::parse_str("<root><child>Hello</child></root>").unwrap();
        let root = doc.root_element().unwrap();
        let child = doc.first_child(root).unwrap();

        let cloned = doc.clone_node(child, false);
        assert_eq!(doc.node_name(cloned), Some("child"));
        // Shallow clone has no children
        assert!(doc.first_child(cloned).is_none());
        // Clone is detached
        assert!(doc.parent(cloned).is_none());
    }

    #[test]
    fn test_clone_node_deep() {
        let mut doc =
            Document::parse_str("<root><parent><child>Hello</child></parent></root>").unwrap();
        let root = doc.root_element().unwrap();
        let parent_elem = doc.first_child(root).unwrap();

        let cloned = doc.clone_node(parent_elem, true);
        assert_eq!(doc.node_name(cloned), Some("parent"));
        // Deep clone has children
        let cloned_child = doc.first_child(cloned).unwrap();
        assert_eq!(doc.node_name(cloned_child), Some("child"));
        let cloned_text = doc.first_child(cloned_child).unwrap();
        assert_eq!(doc.node_text(cloned_text), Some("Hello"));
        // Clone is detached
        assert!(doc.parent(cloned).is_none());
        // Original is unchanged
        assert!(doc.first_child(parent_elem).is_some());
    }

    #[test]
    fn test_clone_node_and_append() {
        let mut doc = Document::parse_str("<root><item>A</item></root>").unwrap();
        let root = doc.root_element().unwrap();
        let item = doc.first_child(root).unwrap();

        let cloned = doc.clone_node(item, true);
        doc.append_child(root, cloned);

        let children: Vec<_> = doc.children(root).collect();
        assert_eq!(children.len(), 2);
        assert_eq!(doc.text_content(children[0]), "A");
        assert_eq!(doc.text_content(children[1]), "A");
    }

    #[test]
    fn test_parse_file_nonexistent() {
        let result = Document::parse_file("/nonexistent/path.xml");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("failed to read file"));
    }

    #[test]
    fn test_parse_file_valid() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("xmloxide_test_parse_file");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.xml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"<root>Hello</root>").unwrap();
        drop(f);

        let doc = Document::parse_file(&path).unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
        assert_eq!(doc.text_content(root), "Hello");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_create_element_convenience() {
        let mut doc = Document::new();
        let elem = doc.create_element("div");
        assert_eq!(doc.node_name(elem), Some("div"));
        assert!(matches!(doc.node(elem).kind, NodeKind::Element { .. }));
    }

    #[test]
    fn test_create_text_convenience() {
        let mut doc = Document::new();
        let t = doc.create_text("hello");
        assert_eq!(doc.node_text(t), Some("hello"));
    }

    #[test]
    fn test_create_comment_convenience() {
        let mut doc = Document::new();
        let c = doc.create_comment("a comment");
        assert_eq!(doc.node_text(c), Some("a comment"));
        assert!(matches!(doc.node(c).kind, NodeKind::Comment { .. }));
    }

    #[test]
    fn test_create_processing_instruction() {
        let mut doc = Document::new();
        let pi = doc.create_processing_instruction("target", Some("data"));
        assert_eq!(doc.node_name(pi), Some("target"));
        assert_eq!(doc.node_text(pi), Some("data"));
    }

    #[test]
    fn test_insert_after() {
        let mut doc = Document::new();
        let root = doc.root();
        let a = doc.create_text("A");
        let b = doc.create_text("B");
        let c = doc.create_text("C");

        doc.append_child(root, a);
        doc.append_child(root, c);
        doc.insert_after(a, b);

        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children, vec![a, b, c]);
        assert_eq!(doc.next_sibling(a), Some(b));
        assert_eq!(doc.next_sibling(b), Some(c));
        assert_eq!(doc.prev_sibling(c), Some(b));
    }

    #[test]
    fn test_insert_after_last() {
        let mut doc = Document::new();
        let root = doc.root();
        let a = doc.create_text("A");
        let b = doc.create_text("B");

        doc.append_child(root, a);
        doc.insert_after(a, b);

        assert_eq!(doc.last_child(root), Some(b));
        assert_eq!(doc.next_sibling(a), Some(b));
        assert_eq!(doc.prev_sibling(b), Some(a));
    }

    #[test]
    fn test_replace_node() {
        let mut doc = Document::new();
        let root = doc.root();
        let a = doc.create_text("A");
        let b = doc.create_text("B");
        let c = doc.create_text("C");
        let new_b = doc.create_text("NEW_B");

        doc.append_child(root, a);
        doc.append_child(root, b);
        doc.append_child(root, c);

        doc.replace_node(b, new_b);

        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children, vec![a, new_b, c]);
        assert!(doc.parent(b).is_none()); // old node detached
    }

    #[test]
    fn test_set_attribute() {
        let mut doc = Document::new();
        let elem = doc.create_element("div");
        assert!(doc.set_attribute(elem, "class", "foo"));
        assert_eq!(doc.attribute(elem, "class"), Some("foo"));

        // Update existing
        assert!(doc.set_attribute(elem, "class", "bar"));
        assert_eq!(doc.attribute(elem, "class"), Some("bar"));
    }

    #[test]
    fn test_set_attribute_on_non_element() {
        let mut doc = Document::new();
        let text = doc.create_text("hello");
        assert!(!doc.set_attribute(text, "class", "foo"));
    }

    #[test]
    fn test_remove_attribute() {
        let mut doc = Document::new();
        let elem = doc.create_element("div");
        doc.set_attribute(elem, "class", "foo");
        doc.set_attribute(elem, "id", "bar");

        assert!(doc.remove_attribute(elem, "class"));
        assert_eq!(doc.attribute(elem, "class"), None);
        assert_eq!(doc.attribute(elem, "id"), Some("bar"));

        // Removing non-existent returns false
        assert!(!doc.remove_attribute(elem, "class"));
    }

    #[test]
    fn test_remove_attribute_on_non_element() {
        let mut doc = Document::new();
        let text = doc.create_text("hello");
        assert!(!doc.remove_attribute(text, "class"));
    }

    #[test]
    fn test_rename_element() {
        let mut doc = Document::new();
        let elem = doc.create_element("div");
        assert!(doc.rename_element(elem, "span"));
        assert_eq!(doc.node_name(elem), Some("span"));
    }

    #[test]
    fn test_rename_non_element() {
        let mut doc = Document::new();
        let text = doc.create_text("hello");
        assert!(!doc.rename_element(text, "span"));
    }
}
