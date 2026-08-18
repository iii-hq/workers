//! Pull-based streaming XML reader API.
//!
//! The `XmlReader` provides a cursor-style, pull-based interface for reading
//! XML documents. Instead of building a full tree in memory or requiring
//! callback implementations (SAX), the reader advances one node at a time
//! through the document, exposing the current node's properties via accessor
//! methods.
//!
//! This API is similar to libxml2's `xmlTextReader` and .NET's `XmlReader`.
//!
//! # Usage Pattern
//!
//! Call [`XmlReader::read`] repeatedly to advance through the document. Each
//! call moves the cursor to the next node. Use accessor methods like
//! [`XmlReader::node_type`], [`XmlReader::name`], and [`XmlReader::value`]
//! to inspect the current node. When `read()` returns `Ok(false)`, the end
//! of the document has been reached.
//!
//! # Examples
//!
//! ```
//! use xmloxide::reader::{XmlReader, XmlNodeType};
//!
//! let mut reader = XmlReader::new("<root><child>Hello</child></root>");
//! let mut elements = Vec::new();
//!
//! while reader.read().unwrap() {
//!     if reader.node_type() == XmlNodeType::Element {
//!         elements.push(reader.name().unwrap_or_default().to_string());
//!     }
//! }
//!
//! assert_eq!(elements, vec!["root", "child"]);
//! ```

use crate::error::{ErrorSeverity, ParseDiagnostic, ParseError};
use crate::parser::input::{
    parse_cdata_content, parse_comment_content, parse_pi_content, parse_xml_decl, split_name,
    NamespaceResolver, ParserInput,
};
use crate::parser::ParseOptions;

/// The type of the current node in the reader.
///
/// These correspond to the different kinds of nodes that the reader can
/// be positioned on while traversing an XML document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum XmlNodeType {
    /// No node — the reader has not been advanced yet or is in an
    /// indeterminate state.
    None,

    /// An element start tag, e.g. `<div>` or `<br/>`.
    ///
    /// For self-closing elements (`<br/>`), [`XmlReader::is_empty_element`]
    /// returns `true`.
    Element,

    /// An element end tag, e.g. `</div>`.
    ///
    /// Self-closing elements do not produce a separate `EndElement` node.
    EndElement,

    /// A text node containing character data.
    Text,

    /// A CDATA section, e.g. `<![CDATA[...]]>`.
    CData,

    /// An XML comment, e.g. `<!-- comment -->`.
    Comment,

    /// A processing instruction, e.g. `<?target data?>`.
    ProcessingInstruction,

    /// The XML declaration, e.g. `<?xml version="1.0"?>`.
    XmlDeclaration,

    /// A document type declaration, e.g. `<!DOCTYPE html>`.
    DocumentType,

    /// A whitespace-only text node in element content.
    Whitespace,

    /// An attribute node — the reader is positioned on an attribute after
    /// calling [`XmlReader::move_to_first_attribute`] or
    /// [`XmlReader::move_to_next_attribute`].
    Attribute,

    /// The end of the document has been reached.
    EndDocument,
}

impl std::fmt::Display for XmlNodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Element => write!(f, "Element"),
            Self::EndElement => write!(f, "EndElement"),
            Self::Text => write!(f, "Text"),
            Self::CData => write!(f, "CData"),
            Self::Comment => write!(f, "Comment"),
            Self::ProcessingInstruction => write!(f, "ProcessingInstruction"),
            Self::XmlDeclaration => write!(f, "XmlDeclaration"),
            Self::DocumentType => write!(f, "DocumentType"),
            Self::Whitespace => write!(f, "Whitespace"),
            Self::Attribute => write!(f, "Attribute"),
            Self::EndDocument => write!(f, "EndDocument"),
        }
    }
}

/// An attribute on the current element in the reader.
#[derive(Debug, Clone)]
struct ReaderAttribute {
    /// The local name of the attribute.
    local_name: String,
    /// The attribute value.
    value: String,
    /// The namespace prefix, if any.
    prefix: Option<String>,
    /// The namespace URI, if any.
    namespace_uri: Option<String>,
}

/// Internal representation of a node the reader is positioned on.
#[derive(Debug, Clone)]
struct ReaderNode {
    /// The type of this node.
    node_type: XmlNodeType,
    /// The local name (for elements, PIs) or the full name/target.
    local_name: String,
    /// The namespace prefix, if any.
    prefix: Option<String>,
    /// The namespace URI, if any.
    namespace_uri: Option<String>,
    /// The value/content (for text, comment, CDATA, PI data, attribute value).
    value: Option<String>,
    /// The depth of this node in the document tree.
    depth: u32,
    /// Whether this is an empty (self-closing) element.
    is_empty_element: bool,
    /// Attributes of the current element (empty for non-elements).
    attributes: Vec<ReaderAttribute>,
}

impl ReaderNode {
    fn new(node_type: XmlNodeType) -> Self {
        Self {
            node_type,
            local_name: String::new(),
            prefix: None,
            namespace_uri: None,
            value: None,
            depth: 0,
            is_empty_element: false,
            attributes: Vec::new(),
        }
    }
}

/// A pull-based streaming XML reader.
///
/// The reader parses an XML document incrementally, advancing one node at
/// a time. This is memory-efficient for large documents because it does not
/// build a full tree.
///
/// # Examples
///
/// ```
/// use xmloxide::reader::{XmlReader, XmlNodeType};
///
/// let mut reader = XmlReader::new("<doc attr=\"val\">text</doc>");
///
/// // Advance to <doc>
/// assert!(reader.read().unwrap());
/// assert_eq!(reader.node_type(), XmlNodeType::Element);
/// assert_eq!(reader.name(), Some("doc"));
/// assert_eq!(reader.depth(), 0);
/// assert_eq!(reader.attribute_count(), 1);
/// assert_eq!(reader.get_attribute("attr"), Some("val"));
///
/// // Advance to text content
/// assert!(reader.read().unwrap());
/// assert_eq!(reader.node_type(), XmlNodeType::Text);
/// assert_eq!(reader.value(), Some("text"));
///
/// // Advance to </doc>
/// assert!(reader.read().unwrap());
/// assert_eq!(reader.node_type(), XmlNodeType::EndElement);
///
/// // End of document
/// assert!(!reader.read().unwrap());
/// ```
#[allow(clippy::struct_excessive_bools)]
pub struct XmlReader<'a> {
    /// Shared low-level input state (position, peek, advance, name parsing, etc.).
    parser_input: ParserInput<'a>,
    /// Parser options.
    options: ParseOptions,
    /// Namespace resolver managing the scope stack.
    ns: NamespaceResolver,
    /// The current node the reader is positioned on.
    current: ReaderNode,
    /// Queued nodes to emit before parsing more input.
    /// For example, an element produces `Element` + `EndElement` for self-closing tags.
    queue: Vec<ReaderNode>,
    /// The current depth in the element tree.
    depth: u32,
    /// Whether parsing has started (first `read()` has been called).
    started: bool,
    /// Whether the document has ended.
    finished: bool,
    /// Whether we have parsed the prolog (xml decl, doctype, misc).
    prolog_parsed: bool,
    /// Whether the root element has been parsed.
    root_parsed: bool,
    /// Whether we are inside the root element content.
    in_element_content: bool,
    /// Stack of open element names (for matching end tags).
    element_stack: Vec<String>,
    /// Current attribute index when iterating over attributes.
    attribute_index: Option<usize>,
    /// The element node saved when navigating attributes.
    saved_element: Option<ReaderNode>,
}

impl<'a> XmlReader<'a> {
    /// Creates a new `XmlReader` from a string slice with default options.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::reader::XmlReader;
    ///
    /// let mut reader = XmlReader::new("<root/>");
    /// assert!(reader.read().unwrap());
    /// ```
    #[must_use]
    pub fn new(input: &'a str) -> Self {
        Self::with_options(input, ParseOptions::default())
    }

    /// Creates a new `XmlReader` from a string slice with custom parse options.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::reader::XmlReader;
    /// use xmloxide::parser::ParseOptions;
    ///
    /// let opts = ParseOptions::default().recover(true);
    /// let mut reader = XmlReader::with_options("<root/>", opts);
    /// assert!(reader.read().unwrap());
    /// ```
    #[must_use]
    pub fn with_options(input: &'a str, options: ParseOptions) -> Self {
        let mut pi = ParserInput::new(input);
        pi.set_recover(options.recover);
        pi.set_max_depth(options.max_depth);
        pi.set_max_name_length(options.max_name_length);
        pi.set_max_entity_expansions(options.max_entity_expansions);
        pi.set_entity_resolver(options.entity_resolver.clone());

        Self {
            parser_input: pi,
            options,
            ns: NamespaceResolver::new(),
            current: ReaderNode::new(XmlNodeType::None),
            queue: Vec::new(),
            depth: 0,
            started: false,
            finished: false,
            prolog_parsed: false,
            root_parsed: false,
            in_element_content: false,
            element_stack: Vec::new(),
            attribute_index: None,
            saved_element: None,
        }
    }

    // === Public API: reading ===

    /// Advances the reader to the next node in the document.
    ///
    /// Returns `Ok(true)` if the reader successfully advanced to a node,
    /// or `Ok(false)` if the end of the document has been reached.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the XML is malformed and recovery mode is
    /// not enabled.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::reader::XmlReader;
    ///
    /// let mut reader = XmlReader::new("<root/>");
    /// while reader.read().unwrap() {
    ///     // process each node
    /// }
    /// ```
    pub fn read(&mut self) -> Result<bool, ParseError> {
        // Reset attribute navigation state when advancing.
        self.attribute_index = None;
        self.saved_element = None;

        if self.finished {
            return Ok(false);
        }

        // Drain queued nodes first.
        if let Some(node) = self.queue.pop() {
            self.current = node;
            return Ok(true);
        }

        if !self.started {
            self.started = true;
        }

        self.read_next_node()
    }

    // === Public API: node type and properties ===

    /// Returns the type of the current node.
    ///
    /// Before the first call to [`read`](Self::read), returns
    /// [`XmlNodeType::None`].
    #[must_use]
    pub fn node_type(&self) -> XmlNodeType {
        self.current.node_type
    }

    /// Returns the qualified name of the current node.
    ///
    /// For elements, this returns `prefix:localname` if a prefix is present,
    /// or just `localname` otherwise. For processing instructions, this is
    /// the target. For `DocumentType` nodes, this is the root element name.
    /// For other node types, returns `None`.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match self.current.node_type {
            XmlNodeType::Element
            | XmlNodeType::EndElement
            | XmlNodeType::ProcessingInstruction
            | XmlNodeType::Attribute
            | XmlNodeType::DocumentType => {
                if self.current.local_name.is_empty() {
                    None
                } else {
                    Some(&self.current.local_name)
                }
            }
            _ => None,
        }
    }

    /// Returns the local name of the current node (without namespace prefix).
    ///
    /// For elements and attributes, this is the local part of the qualified
    /// name. For processing instructions, this is the target.
    #[must_use]
    pub fn local_name(&self) -> Option<&str> {
        match self.current.node_type {
            XmlNodeType::Element
            | XmlNodeType::EndElement
            | XmlNodeType::ProcessingInstruction
            | XmlNodeType::Attribute
            | XmlNodeType::DocumentType => {
                if self.current.local_name.is_empty() {
                    None
                } else {
                    Some(&self.current.local_name)
                }
            }
            _ => None,
        }
    }

    /// Returns the namespace prefix of the current node, if any.
    ///
    /// For elements and attributes with a prefix (e.g., `svg` in `<svg:rect>`),
    /// returns the prefix string. For unprefixed elements, processing
    /// instructions, text, comments, and other node types, returns `None`.
    #[must_use]
    pub fn prefix(&self) -> Option<&str> {
        self.current.prefix.as_deref()
    }

    /// Returns the namespace URI of the current node, if any.
    ///
    /// Namespace URIs are resolved for elements and attributes that are in a
    /// namespace (either via a prefix or a default namespace declaration).
    /// Returns `None` for nodes that have no namespace or for node types that
    /// do not carry namespace information (text, comments, etc.).
    #[must_use]
    pub fn namespace_uri(&self) -> Option<&str> {
        self.current.namespace_uri.as_deref()
    }

    /// Returns the value of the current node, if applicable.
    ///
    /// For text, CDATA, comment, whitespace, and attribute nodes, this is
    /// the text content. For processing instructions, this is the data
    /// portion. For elements and end elements, returns `None`.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.current.value.as_deref()
    }

    /// Returns whether the current node has a value.
    ///
    /// Returns `true` for node types that carry text content: text, CDATA,
    /// comment, whitespace, attribute, and processing instruction nodes.
    /// Returns `false` for elements, end elements, and the document type.
    #[must_use]
    pub fn has_value(&self) -> bool {
        self.current.value.is_some()
    }

    /// Returns whether the current element is a self-closing (empty) element.
    ///
    /// Returns `true` for elements like `<br/>`, `false` for elements
    /// like `<div>...</div>`. Always returns `false` for non-element nodes.
    #[must_use]
    pub fn is_empty_element(&self) -> bool {
        self.current.is_empty_element
    }

    /// Returns the depth of the current node in the document tree.
    ///
    /// The root element is at depth 0, its children at depth 1, and so on.
    /// Nodes in the prolog (XML declaration, DOCTYPE) are at depth 0.
    #[must_use]
    pub fn depth(&self) -> u32 {
        self.current.depth
    }

    /// Returns the number of attributes on the current element.
    ///
    /// Returns 0 for non-element nodes.
    #[must_use]
    pub fn attribute_count(&self) -> usize {
        self.current.attributes.len()
    }

    /// Returns the value of an attribute by name on the current element.
    ///
    /// Searches by the full attribute name (qualified name). Returns `None`
    /// if the attribute is not present or the current node is not an element.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::reader::{XmlReader, XmlNodeType};
    ///
    /// let mut reader = XmlReader::new("<root id=\"42\"/>");
    /// reader.read().unwrap();
    /// assert_eq!(reader.get_attribute("id"), Some("42"));
    /// assert_eq!(reader.get_attribute("missing"), None);
    /// ```
    #[must_use]
    pub fn get_attribute(&self, name: &str) -> Option<&str> {
        let attrs = &self.current.attributes;
        for attr in attrs {
            let full_name = match &attr.prefix {
                Some(pfx) => {
                    // Compare against "prefix:local_name"
                    if name.starts_with(pfx.as_str())
                        && name.as_bytes().get(pfx.len()) == Some(&b':')
                        && name[pfx.len() + 1..] == *attr.local_name
                    {
                        return Some(&attr.value);
                    }
                    continue;
                }
                None => &attr.local_name,
            };
            if full_name == name {
                return Some(&attr.value);
            }
        }
        None
    }

    /// Returns the value of an attribute by local name and namespace URI.
    ///
    /// Returns `None` if the attribute is not present, the namespace does
    /// not match, or the current node is not an element.
    #[must_use]
    pub fn get_attribute_ns(&self, local_name: &str, namespace_uri: &str) -> Option<&str> {
        self.current.attributes.iter().find_map(|attr| {
            if attr.local_name == local_name && attr.namespace_uri.as_deref() == Some(namespace_uri)
            {
                Some(attr.value.as_str())
            } else {
                None
            }
        })
    }

    // === Public API: attribute navigation ===

    /// Moves the reader to the first attribute of the current element.
    ///
    /// Returns `true` if the element has attributes and the reader was
    /// moved to the first one. Returns `false` if there are no attributes
    /// or the current node is not an element.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::reader::{XmlReader, XmlNodeType};
    ///
    /// let mut reader = XmlReader::new("<root a=\"1\" b=\"2\"/>");
    /// reader.read().unwrap();
    ///
    /// assert!(reader.move_to_first_attribute());
    /// assert_eq!(reader.node_type(), XmlNodeType::Attribute);
    /// assert_eq!(reader.name(), Some("a"));
    /// assert_eq!(reader.value(), Some("1"));
    /// ```
    pub fn move_to_first_attribute(&mut self) -> bool {
        if self.current.node_type != XmlNodeType::Element
            && self.current.node_type != XmlNodeType::Attribute
        {
            return false;
        }

        // Save the element node if we haven't already.
        if self.saved_element.is_none() {
            if self.current.node_type == XmlNodeType::Attribute {
                // Already navigating attributes; don't overwrite saved.
            } else {
                self.saved_element = Some(self.current.clone());
            }
        }

        let elem = self.saved_element.as_ref().unwrap_or(&self.current);

        if elem.attributes.is_empty() {
            return false;
        }

        let attr = &elem.attributes[0];
        self.current = ReaderNode {
            node_type: XmlNodeType::Attribute,
            local_name: attr.local_name.clone(),
            prefix: attr.prefix.clone(),
            namespace_uri: attr.namespace_uri.clone(),
            value: Some(attr.value.clone()),
            depth: elem.depth + 1,
            is_empty_element: false,
            attributes: elem.attributes.clone(),
        };
        self.attribute_index = Some(0);
        true
    }

    /// Moves the reader to the next attribute of the current element.
    ///
    /// Returns `true` if there is a next attribute. Returns `false` if
    /// there are no more attributes or the reader is not on an attribute.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::reader::{XmlReader, XmlNodeType};
    ///
    /// let mut reader = XmlReader::new("<root a=\"1\" b=\"2\"/>");
    /// reader.read().unwrap();
    ///
    /// assert!(reader.move_to_first_attribute());
    /// assert_eq!(reader.name(), Some("a"));
    ///
    /// assert!(reader.move_to_next_attribute());
    /// assert_eq!(reader.name(), Some("b"));
    ///
    /// assert!(!reader.move_to_next_attribute());
    /// ```
    pub fn move_to_next_attribute(&mut self) -> bool {
        let Some(idx) = self.attribute_index else {
            // If not currently on an attribute, try to start from the first.
            return self.move_to_first_attribute();
        };

        let elem = self.saved_element.as_ref().unwrap_or(&self.current);

        let next_idx = idx + 1;
        if next_idx >= elem.attributes.len() {
            return false;
        }

        let attr = &elem.attributes[next_idx];
        self.current = ReaderNode {
            node_type: XmlNodeType::Attribute,
            local_name: attr.local_name.clone(),
            prefix: attr.prefix.clone(),
            namespace_uri: attr.namespace_uri.clone(),
            value: Some(attr.value.clone()),
            depth: elem.depth + 1,
            is_empty_element: false,
            attributes: elem.attributes.clone(),
        };
        self.attribute_index = Some(next_idx);
        true
    }

    /// Moves the reader back to the element that owns the current attribute.
    ///
    /// Returns `true` if the reader was on an attribute and was moved back
    /// to the element. Returns `false` if the reader was not on an attribute.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::reader::{XmlReader, XmlNodeType};
    ///
    /// let mut reader = XmlReader::new("<root a=\"1\"/>");
    /// reader.read().unwrap();
    /// reader.move_to_first_attribute();
    /// assert_eq!(reader.node_type(), XmlNodeType::Attribute);
    ///
    /// assert!(reader.move_to_element());
    /// assert_eq!(reader.node_type(), XmlNodeType::Element);
    /// assert_eq!(reader.name(), Some("root"));
    /// ```
    pub fn move_to_element(&mut self) -> bool {
        if let Some(elem) = self.saved_element.take() {
            self.current = elem;
            self.attribute_index = None;
            true
        } else {
            false
        }
    }

    /// Returns the diagnostics collected during parsing.
    ///
    /// In recovery mode, this includes warnings and errors that were
    /// encountered but did not halt parsing.
    #[must_use]
    pub fn diagnostics(&self) -> &[ParseDiagnostic] {
        &self.parser_input.diagnostics
    }

    // === Internal: top-level node dispatch ===

    fn read_next_node(&mut self) -> Result<bool, ParseError> {
        // Parse prolog if not done yet.
        if !self.prolog_parsed {
            return self.read_prolog();
        }

        // Parse root element and content.
        if !self.root_parsed {
            return self.read_root_or_prolog_misc();
        }

        // If inside element content, parse child nodes.
        if self.in_element_content {
            return self.read_element_content();
        }

        // After root element, parse trailing misc.
        self.read_trailing_misc()
    }

    fn read_prolog(&mut self) -> Result<bool, ParseError> {
        self.parser_input.skip_whitespace();

        // Parse XML declaration if present.
        if self.parser_input.looking_at(b"<?xml ")
            || self.parser_input.looking_at(b"<?xml\t")
            || self.parser_input.looking_at(b"<?xml\r")
            || self.parser_input.looking_at(b"<?xml?>")
        {
            let node = self.parse_xml_declaration()?;
            self.current = node;
            // Don't set prolog_parsed yet; there may be misc nodes.
            return Ok(true);
        }

        self.prolog_parsed = true;
        self.read_root_or_prolog_misc()
    }

    fn read_root_or_prolog_misc(&mut self) -> Result<bool, ParseError> {
        self.parser_input.skip_whitespace();

        if self.parser_input.at_end() {
            self.finished = true;
            self.current = ReaderNode::new(XmlNodeType::EndDocument);
            return Ok(false);
        }

        // DOCTYPE
        if self.parser_input.looking_at(b"<!DOCTYPE") || self.parser_input.looking_at(b"<!doctype")
        {
            let node = self.parse_doctype()?;
            self.current = node;
            return Ok(true);
        }

        // Comment
        if self.parser_input.looking_at(b"<!--") {
            let node = self.parse_comment()?;
            self.current = node;
            return Ok(true);
        }

        // Processing instruction
        if self.parser_input.looking_at(b"<?") {
            let node = self.parse_processing_instruction()?;
            self.current = node;
            return Ok(true);
        }

        // Root element start.
        if self.parser_input.peek() == Some(b'<')
            && self
                .parser_input
                .peek_at(1)
                .is_some_and(|b| b != b'!' && b != b'?')
        {
            self.root_parsed = true;
            let node = self.parse_element_start()?;
            self.current = node;
            return Ok(true);
        }

        if !self.parser_input.at_end() && !self.options.recover {
            return Err(self.parser_input.fatal("expected root element"));
        }

        self.finished = true;
        self.current = ReaderNode::new(XmlNodeType::EndDocument);
        Ok(false)
    }

    fn read_element_content(&mut self) -> Result<bool, ParseError> {
        if self.parser_input.at_end() {
            if self.options.recover {
                // Force-close all open elements.
                if let Some(name) = self.element_stack.pop() {
                    self.depth -= 1;
                    self.parser_input.decrement_depth();
                    self.ns.pop_scope();
                    let mut node = ReaderNode::new(XmlNodeType::EndElement);
                    let (prefix, local_name) = split_name(&name);
                    node.local_name = local_name.to_string();
                    node.prefix = prefix.map(String::from);
                    node.depth = self.depth;
                    self.in_element_content = !self.element_stack.is_empty();
                    self.current = node;
                    return Ok(true);
                }
                self.finished = true;
                self.current = ReaderNode::new(XmlNodeType::EndDocument);
                return Ok(false);
            }
            return Err(self
                .parser_input
                .fatal("unexpected end of input in element content"));
        }

        // End tag.
        if self.parser_input.looking_at(b"</") {
            let node = self.parse_end_tag()?;
            self.current = node;
            return Ok(true);
        }

        // CDATA section.
        if self.parser_input.looking_at(b"<![CDATA[") {
            let node = self.parse_cdata()?;
            self.current = node;
            return Ok(true);
        }

        // Comment.
        if self.parser_input.looking_at(b"<!--") {
            let node = self.parse_comment()?;
            self.current = node;
            return Ok(true);
        }

        // Processing instruction.
        if self.parser_input.looking_at(b"<?") {
            let node = self.parse_processing_instruction()?;
            self.current = node;
            return Ok(true);
        }

        // Child element.
        if self.parser_input.peek() == Some(b'<')
            && self
                .parser_input
                .peek_at(1)
                .is_some_and(|b| b != b'!' && b != b'?')
        {
            let node = self.parse_element_start()?;
            self.current = node;
            return Ok(true);
        }

        // Character data (text).
        let node = self.parse_char_data()?;

        // Skip whitespace-only text nodes if no_blanks is enabled.
        if self.options.no_blanks && node.node_type == XmlNodeType::Whitespace {
            return self.read_element_content();
        }

        self.current = node;
        Ok(true)
    }

    fn read_trailing_misc(&mut self) -> Result<bool, ParseError> {
        self.parser_input.skip_whitespace();

        if self.parser_input.at_end() {
            self.finished = true;
            self.current = ReaderNode::new(XmlNodeType::EndDocument);
            return Ok(false);
        }

        // Comment.
        if self.parser_input.looking_at(b"<!--") {
            let node = self.parse_comment()?;
            self.current = node;
            return Ok(true);
        }

        // Processing instruction.
        if self.parser_input.looking_at(b"<?") {
            let node = self.parse_processing_instruction()?;
            self.current = node;
            return Ok(true);
        }

        if !self.options.recover {
            return Err(self.parser_input.fatal("content after document element"));
        }

        self.finished = true;
        self.current = ReaderNode::new(XmlNodeType::EndDocument);
        Ok(false)
    }

    // === Internal: parse individual constructs ===

    fn parse_xml_declaration(&mut self) -> Result<ReaderNode, ParseError> {
        let decl = parse_xml_decl(&mut self.parser_input)?;
        self.prolog_parsed = true;

        // Build the value string as "version=X encoding=Y standalone=Z".
        let mut value_parts = vec![format!("version={}", decl.version)];
        if let Some(ref enc) = decl.encoding {
            value_parts.push(format!("encoding={enc}"));
        }
        if let Some(sa) = decl.standalone {
            let sa_str = if sa { "yes" } else { "no" };
            value_parts.push(format!("standalone={sa_str}"));
        }

        let mut node = ReaderNode::new(XmlNodeType::XmlDeclaration);
        node.local_name = "xml".to_string();
        node.value = Some(value_parts.join(" "));
        node.depth = 0;
        Ok(node)
    }

    fn parse_doctype(&mut self) -> Result<ReaderNode, ParseError> {
        // Parse: <!DOCTYPE name (SYSTEM|PUBLIC ...) [internal subset]? >
        self.parser_input.expect_str(b"<!DOCTYPE")?;
        self.parser_input.skip_whitespace_required()?;
        let name = self.parser_input.parse_name()?;
        self.parser_input.skip_whitespace();

        if self.parser_input.looking_at(b"SYSTEM") {
            self.parser_input.expect_str(b"SYSTEM")?;
            self.parser_input.skip_whitespace_required()?;
            self.parser_input.parse_quoted_value()?;
            self.parser_input.skip_whitespace();
        } else if self.parser_input.looking_at(b"PUBLIC") {
            self.parser_input.expect_str(b"PUBLIC")?;
            self.parser_input.skip_whitespace_required()?;
            self.parser_input.parse_quoted_value()?;
            self.parser_input.skip_whitespace_required()?;
            self.parser_input.parse_quoted_value()?;
            self.parser_input.skip_whitespace();
        }

        if self.parser_input.peek() == Some(b'[') {
            self.parser_input.advance(1);
            let start = self.parser_input.pos();
            let mut bracket_depth: u32 = 1;
            while !self.parser_input.at_end() && bracket_depth > 0 {
                if self.parser_input.looking_at(b"<!--") {
                    self.parser_input.advance(4);
                    while !self.parser_input.at_end() && !self.parser_input.looking_at(b"-->") {
                        self.parser_input.advance(1);
                    }
                    if !self.parser_input.at_end() {
                        self.parser_input.advance(3);
                    }
                } else if let Some(b'"' | b'\'') = self.parser_input.peek() {
                    let quote = self.parser_input.peek().unwrap_or(b'"');
                    self.parser_input.advance(1);
                    while !self.parser_input.at_end() && self.parser_input.peek() != Some(quote) {
                        self.parser_input.advance(1);
                    }
                    if !self.parser_input.at_end() {
                        self.parser_input.advance(1);
                    }
                } else if self.parser_input.peek() == Some(b'[') {
                    bracket_depth += 1;
                    self.parser_input.advance(1);
                } else if self.parser_input.peek() == Some(b']') {
                    bracket_depth -= 1;
                    self.parser_input.advance(1);
                } else {
                    self.parser_input.advance(1);
                }
            }

            // Parse DTD internal subset for entity declarations
            let end = self.parser_input.pos() - 1;
            let subset_text = std::str::from_utf8(self.parser_input.slice(start, end))
                .ok()
                .map(str::to_string);
            if let Some(subset_text) = subset_text {
                if subset_text.contains('%') {
                    self.parser_input.has_pe_references = true;
                }
                if let Ok(dtd) = crate::validation::dtd::parse_dtd(&subset_text) {
                    for (ent_name, ent_decl) in &dtd.entities {
                        match &ent_decl.kind {
                            crate::validation::dtd::EntityKind::Internal(value) => {
                                self.parser_input
                                    .entity_map
                                    .insert(ent_name.clone(), value.clone());
                            }
                            crate::validation::dtd::EntityKind::External {
                                system_id,
                                public_id,
                            } => {
                                self.parser_input.entity_external.insert(
                                    ent_name.clone(),
                                    crate::parser::input::ExternalEntityInfo {
                                        system_id: system_id.clone(),
                                        public_id: public_id.clone(),
                                    },
                                );
                            }
                        }
                    }
                }
            }

            self.parser_input.skip_whitespace();
        }

        self.parser_input.expect_byte(b'>')?;

        let mut node = ReaderNode::new(XmlNodeType::DocumentType);
        node.local_name = name;
        node.depth = 0;
        Ok(node)
    }

    fn parse_element_start(&mut self) -> Result<ReaderNode, ParseError> {
        self.parser_input.increment_depth()?;
        self.parser_input.expect_byte(b'<')?;
        let name = self.parser_input.parse_name()?;

        // Parse attributes.
        let mut raw_attrs: Vec<(String, String)> = Vec::new();
        loop {
            let had_ws = self.parser_input.skip_whitespace();
            if self.parser_input.peek() == Some(b'>') || self.parser_input.looking_at(b"/>") {
                break;
            }
            if !had_ws {
                return Err(self
                    .parser_input
                    .fatal("whitespace required between attributes"));
            }
            let attr_name = self.parser_input.parse_name()?;
            self.parser_input.skip_whitespace();
            self.parser_input.expect_byte(b'=')?;
            self.parser_input.skip_whitespace();
            let attr_value = self.parser_input.parse_attribute_value()?;
            raw_attrs.push((attr_name, attr_value));
        }

        // Namespace processing.
        self.ns.push_scope();
        for (attr_name, attr_value) in &raw_attrs {
            if attr_name == "xmlns" {
                self.ns.bind(None, attr_value.clone());
            } else if let Some(prefix) = attr_name.strip_prefix("xmlns:") {
                self.ns.bind(Some(prefix.to_string()), attr_value.clone());
            }
        }

        // Resolve element namespace.
        let (prefix, local_name) = split_name(&name);
        let elem_ns = self.ns.resolve(prefix).map(String::from);

        // Build attribute list.
        let attributes: Vec<ReaderAttribute> = raw_attrs
            .iter()
            .map(|(attr_name, attr_value)| {
                let (attr_prefix, attr_local) = split_name(attr_name);
                let attr_ns = if attr_prefix == Some("xmlns")
                    || (attr_prefix.is_none() && attr_local == "xmlns")
                {
                    None
                } else {
                    attr_prefix
                        .and_then(|p| self.ns.resolve(Some(p)))
                        .map(String::from)
                };
                ReaderAttribute {
                    local_name: attr_local.to_string(),
                    value: attr_value.clone(),
                    prefix: attr_prefix.map(String::from),
                    namespace_uri: attr_ns,
                }
            })
            .collect();

        let is_empty = self.parser_input.looking_at(b"/>");
        if is_empty {
            self.parser_input.advance(2);
        } else {
            self.parser_input.expect_byte(b'>')?;
        }

        let current_depth = self.depth;

        let mut node = ReaderNode::new(XmlNodeType::Element);
        node.local_name = local_name.to_string();
        node.prefix = prefix.map(String::from);
        node.namespace_uri = elem_ns;
        node.depth = current_depth;
        node.is_empty_element = is_empty;
        node.attributes = attributes;

        if is_empty {
            // For empty elements, by convention in .NET-style readers, we
            // do NOT emit a separate EndElement. The is_empty_element flag
            // signals the caller. We do however need to pop the ns scope
            // and decrement the security depth counter.
            self.ns.pop_scope();
            self.parser_input.decrement_depth();
        } else {
            self.element_stack.push(name);
            self.depth += 1;
            self.in_element_content = true;
        }

        Ok(node)
    }

    fn parse_end_tag(&mut self) -> Result<ReaderNode, ParseError> {
        self.parser_input.expect_str(b"</")?;
        let name = self.parser_input.parse_name()?;
        self.parser_input.skip_whitespace();
        self.parser_input.expect_byte(b'>')?;

        // Match against the open element stack.
        if let Some(expected) = self.element_stack.last() {
            if *expected != name {
                if self.options.recover {
                    self.parser_input.push_diagnostic(
                        ErrorSeverity::Error,
                        format!("mismatched end tag: expected </{expected}>, found </{name}>"),
                    );
                } else {
                    return Err(self.parser_input.fatal(format!(
                        "mismatched end tag: expected </{expected}>, found </{name}>"
                    )));
                }
            }
        }

        self.element_stack.pop();
        self.depth -= 1;
        self.parser_input.decrement_depth();
        self.ns.pop_scope();
        self.in_element_content = !self.element_stack.is_empty();

        let (prefix, local_name) = split_name(&name);
        let mut node = ReaderNode::new(XmlNodeType::EndElement);
        node.local_name = local_name.to_string();
        node.prefix = prefix.map(String::from);
        node.depth = self.depth;

        Ok(node)
    }

    fn parse_char_data(&mut self) -> Result<ReaderNode, ParseError> {
        let mut text = String::new();
        while !self.parser_input.at_end() {
            if self.parser_input.peek() == Some(b'<') {
                break;
            }

            // XML 1.0 §2.4: "]]>" is forbidden in character data
            if self.parser_input.looking_at(b"]]>") {
                if self.options.recover {
                    self.parser_input.push_diagnostic(
                        ErrorSeverity::Error,
                        "']]>' not allowed in character data".to_string(),
                    );
                    text.push_str("]]>");
                    self.parser_input.advance(3);
                    continue;
                }
                return Err(self
                    .parser_input
                    .fatal("']]>' not allowed in character data"));
            }

            if self.parser_input.peek() == Some(b'&') {
                self.parser_input.parse_reference_into(&mut text)?;
            } else {
                let ch = self.parser_input.next_char()?;
                text.push(ch);
            }
        }

        let is_whitespace = text
            .chars()
            .all(|c| c == ' ' || c == '\t' || c == '\n' || c == '\r');

        let node_type = if is_whitespace {
            XmlNodeType::Whitespace
        } else {
            XmlNodeType::Text
        };

        let mut node = ReaderNode::new(node_type);
        node.value = Some(text);
        node.depth = self.depth;
        Ok(node)
    }

    fn parse_comment(&mut self) -> Result<ReaderNode, ParseError> {
        let content = parse_comment_content(&mut self.parser_input)?;
        let mut node = ReaderNode::new(XmlNodeType::Comment);
        node.value = Some(content);
        node.depth = self.depth;
        Ok(node)
    }

    fn parse_cdata(&mut self) -> Result<ReaderNode, ParseError> {
        let content = parse_cdata_content(&mut self.parser_input)?;
        let mut node = ReaderNode::new(XmlNodeType::CData);
        node.value = Some(content);
        node.depth = self.depth;
        Ok(node)
    }

    fn parse_processing_instruction(&mut self) -> Result<ReaderNode, ParseError> {
        let (target, data) = parse_pi_content(&mut self.parser_input)?;
        let mut node = ReaderNode::new(XmlNodeType::ProcessingInstruction);
        node.local_name = target;
        node.value = data;
        node.depth = self.depth;
        Ok(node)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // --- Helper ---

    fn read_all_types(input: &str) -> Vec<(XmlNodeType, String)> {
        let mut reader = XmlReader::new(input);
        let mut result = Vec::new();
        while reader.read().unwrap() {
            let label = match reader.node_type() {
                XmlNodeType::Element | XmlNodeType::EndElement => {
                    reader.name().unwrap_or("").to_string()
                }
                XmlNodeType::Text
                | XmlNodeType::CData
                | XmlNodeType::Comment
                | XmlNodeType::Whitespace
                | XmlNodeType::XmlDeclaration => reader.value().unwrap_or("").to_string(),
                XmlNodeType::ProcessingInstruction => {
                    let target = reader.name().unwrap_or("").to_string();
                    match reader.value() {
                        Some(data) => format!("{target} {data}"),
                        None => target,
                    }
                }
                XmlNodeType::DocumentType => reader.name().unwrap_or("").to_string(),
                _ => String::new(),
            };
            result.push((reader.node_type(), label));
        }
        result
    }

    // === Test: basic element ===

    #[test]
    fn test_read_empty_element() {
        let mut reader = XmlReader::new("<root/>");
        assert!(reader.read().unwrap());
        assert_eq!(reader.node_type(), XmlNodeType::Element);
        assert_eq!(reader.name(), Some("root"));
        assert!(reader.is_empty_element());
        assert_eq!(reader.depth(), 0);

        // No EndElement for empty elements.
        assert!(!reader.read().unwrap());
    }

    #[test]
    fn test_read_element_with_content() {
        let nodes = read_all_types("<root>Hello</root>");
        assert_eq!(
            nodes,
            vec![
                (XmlNodeType::Element, "root".to_string()),
                (XmlNodeType::Text, "Hello".to_string()),
                (XmlNodeType::EndElement, "root".to_string()),
            ]
        );
    }

    #[test]
    fn test_read_nested_elements() {
        let nodes = read_all_types("<a><b>text</b></a>");
        assert_eq!(
            nodes,
            vec![
                (XmlNodeType::Element, "a".to_string()),
                (XmlNodeType::Element, "b".to_string()),
                (XmlNodeType::Text, "text".to_string()),
                (XmlNodeType::EndElement, "b".to_string()),
                (XmlNodeType::EndElement, "a".to_string()),
            ]
        );
    }

    // === Test: depth tracking ===

    #[test]
    fn test_read_depth_tracking() {
        let mut reader = XmlReader::new("<a><b><c/></b></a>");

        reader.read().unwrap(); // <a>
        assert_eq!(reader.depth(), 0);
        assert_eq!(reader.name(), Some("a"));

        reader.read().unwrap(); // <b>
        assert_eq!(reader.depth(), 1);
        assert_eq!(reader.name(), Some("b"));

        reader.read().unwrap(); // <c/>
        assert_eq!(reader.depth(), 2);
        assert_eq!(reader.name(), Some("c"));
        assert!(reader.is_empty_element());

        reader.read().unwrap(); // </b>
        assert_eq!(reader.depth(), 1);
        assert_eq!(reader.node_type(), XmlNodeType::EndElement);

        reader.read().unwrap(); // </a>
        assert_eq!(reader.depth(), 0);
        assert_eq!(reader.node_type(), XmlNodeType::EndElement);

        assert!(!reader.read().unwrap()); // EOF
    }

    // === Test: attributes ===

    #[test]
    fn test_read_attributes() {
        let mut reader = XmlReader::new("<root id=\"1\" class=\"big\"/>");
        reader.read().unwrap();

        assert_eq!(reader.attribute_count(), 2);
        assert_eq!(reader.get_attribute("id"), Some("1"));
        assert_eq!(reader.get_attribute("class"), Some("big"));
        assert_eq!(reader.get_attribute("missing"), None);
    }

    #[test]
    fn test_attribute_navigation() {
        let mut reader = XmlReader::new("<root a=\"1\" b=\"2\" c=\"3\"/>");
        reader.read().unwrap();
        assert_eq!(reader.node_type(), XmlNodeType::Element);

        // Move to first attribute.
        assert!(reader.move_to_first_attribute());
        assert_eq!(reader.node_type(), XmlNodeType::Attribute);
        assert_eq!(reader.name(), Some("a"));
        assert_eq!(reader.value(), Some("1"));

        // Move to second attribute.
        assert!(reader.move_to_next_attribute());
        assert_eq!(reader.name(), Some("b"));
        assert_eq!(reader.value(), Some("2"));

        // Move to third attribute.
        assert!(reader.move_to_next_attribute());
        assert_eq!(reader.name(), Some("c"));
        assert_eq!(reader.value(), Some("3"));

        // No more attributes.
        assert!(!reader.move_to_next_attribute());

        // Move back to element.
        assert!(reader.move_to_element());
        assert_eq!(reader.node_type(), XmlNodeType::Element);
        assert_eq!(reader.name(), Some("root"));
    }

    // === Test: text and whitespace ===

    #[test]
    fn test_read_text_content() {
        let mut reader = XmlReader::new("<p>Hello &amp; world</p>");
        reader.read().unwrap(); // <p>
        reader.read().unwrap(); // text
        assert_eq!(reader.node_type(), XmlNodeType::Text);
        assert_eq!(reader.value(), Some("Hello & world"));
        assert!(reader.has_value());
    }

    #[test]
    fn test_read_whitespace_only_text() {
        let mut reader = XmlReader::new("<root>  \n  </root>");
        reader.read().unwrap(); // <root>
        reader.read().unwrap(); // whitespace
        assert_eq!(reader.node_type(), XmlNodeType::Whitespace);
        assert_eq!(reader.value(), Some("  \n  "));
    }

    #[test]
    fn test_read_no_blanks_option() {
        let opts = ParseOptions::default().no_blanks(true);
        let mut reader = XmlReader::with_options("<root>  <child/>  </root>", opts);

        reader.read().unwrap(); // <root>
        assert_eq!(reader.name(), Some("root"));

        reader.read().unwrap(); // <child/> (whitespace skipped)
        assert_eq!(reader.node_type(), XmlNodeType::Element);
        assert_eq!(reader.name(), Some("child"));

        reader.read().unwrap(); // </root> (whitespace skipped)
        assert_eq!(reader.node_type(), XmlNodeType::EndElement);
        assert_eq!(reader.name(), Some("root"));
    }

    // === Test: comments, CDATA, PI ===

    #[test]
    fn test_read_comment() {
        let nodes = read_all_types("<root><!-- hello --></root>");
        assert_eq!(
            nodes,
            vec![
                (XmlNodeType::Element, "root".to_string()),
                (XmlNodeType::Comment, " hello ".to_string()),
                (XmlNodeType::EndElement, "root".to_string()),
            ]
        );
    }

    #[test]
    fn test_read_cdata() {
        let nodes = read_all_types("<root><![CDATA[raw & data]]></root>");
        assert_eq!(
            nodes,
            vec![
                (XmlNodeType::Element, "root".to_string()),
                (XmlNodeType::CData, "raw & data".to_string()),
                (XmlNodeType::EndElement, "root".to_string()),
            ]
        );
    }

    #[test]
    fn test_read_processing_instruction() {
        let nodes = read_all_types("<?target data?><root/>");
        assert_eq!(
            nodes,
            vec![
                (
                    XmlNodeType::ProcessingInstruction,
                    "target data".to_string()
                ),
                (XmlNodeType::Element, "root".to_string()),
            ]
        );
    }

    // === Test: XML declaration and doctype ===

    #[test]
    fn test_read_xml_declaration() {
        let nodes = read_all_types("<?xml version=\"1.0\" encoding=\"UTF-8\"?><root/>");
        assert_eq!(
            nodes,
            vec![
                (
                    XmlNodeType::XmlDeclaration,
                    "version=1.0 encoding=UTF-8".to_string()
                ),
                (XmlNodeType::Element, "root".to_string()),
            ]
        );
    }

    #[test]
    fn test_read_doctype() {
        let nodes = read_all_types("<!DOCTYPE html><html/>");
        assert_eq!(
            nodes,
            vec![
                (XmlNodeType::DocumentType, "html".to_string()),
                (XmlNodeType::Element, "html".to_string()),
            ]
        );
    }

    // === Test: namespaces ===

    #[test]
    fn test_read_namespace() {
        let mut reader = XmlReader::new("<root xmlns=\"http://example.com\"/>");
        reader.read().unwrap();
        assert_eq!(reader.name(), Some("root"));
        assert_eq!(reader.namespace_uri(), Some("http://example.com"));
        assert_eq!(reader.prefix(), None);
    }

    #[test]
    fn test_read_prefixed_namespace() {
        let mut reader = XmlReader::new("<ns:root xmlns:ns=\"http://example.com\"/>");
        reader.read().unwrap();
        assert_eq!(reader.name(), Some("root"));
        assert_eq!(reader.prefix(), Some("ns"));
        assert_eq!(reader.namespace_uri(), Some("http://example.com"));
    }

    #[test]
    fn test_read_attribute_ns() {
        let mut reader = XmlReader::new("<root xmlns:x=\"http://x.com\" x:attr=\"val\"/>");
        reader.read().unwrap();
        assert_eq!(reader.get_attribute("x:attr"), Some("val"));
        assert_eq!(reader.get_attribute_ns("attr", "http://x.com"), Some("val"));
        assert_eq!(reader.get_attribute_ns("attr", "http://other.com"), None);
    }

    // === Test: mixed content ===

    #[test]
    fn test_read_mixed_content() {
        let nodes = read_all_types("<p>Hello <b>world</b>!</p>");
        assert_eq!(
            nodes,
            vec![
                (XmlNodeType::Element, "p".to_string()),
                (XmlNodeType::Text, "Hello ".to_string()),
                (XmlNodeType::Element, "b".to_string()),
                (XmlNodeType::Text, "world".to_string()),
                (XmlNodeType::EndElement, "b".to_string()),
                (XmlNodeType::Text, "!".to_string()),
                (XmlNodeType::EndElement, "p".to_string()),
            ]
        );
    }

    // === Test: entity references ===

    #[test]
    fn test_read_entity_references() {
        let mut reader = XmlReader::new("<root>&amp;&lt;&gt;&apos;&quot;</root>");
        reader.read().unwrap(); // <root>
        reader.read().unwrap(); // text
        assert_eq!(reader.value(), Some("&<>'\""));
    }

    // === Test: character references ===

    #[test]
    fn test_read_character_references() {
        let mut reader = XmlReader::new("<root>&#65;&#x42;</root>");
        reader.read().unwrap(); // <root>
        reader.read().unwrap(); // text "AB"
        assert_eq!(reader.value(), Some("AB"));
    }

    // === Test: error handling ===

    #[test]
    fn test_read_error_mismatched_tags() {
        let mut reader = XmlReader::new("<a></b>");
        reader.read().unwrap(); // <a>
        let result = reader.read(); // </b> should fail
                                    // The read of text between <a> and </b> will give us the end tag.
                                    // Actually there's no text, so we'll get the mismatched end tag error.
        assert!(result.is_err());
    }

    #[test]
    fn test_read_returns_false_after_end() {
        let mut reader = XmlReader::new("<root/>");
        assert!(reader.read().unwrap()); // <root/>
        assert!(!reader.read().unwrap()); // EOF
        assert!(!reader.read().unwrap()); // still EOF
    }

    // === Test: XmlNodeType Display ===

    #[test]
    fn test_node_type_display() {
        assert_eq!(XmlNodeType::Element.to_string(), "Element");
        assert_eq!(XmlNodeType::EndElement.to_string(), "EndElement");
        assert_eq!(XmlNodeType::Text.to_string(), "Text");
        assert_eq!(XmlNodeType::None.to_string(), "None");
        assert_eq!(XmlNodeType::EndDocument.to_string(), "EndDocument");
    }

    // === Test: has_value returns false for elements ===

    #[test]
    fn test_has_value_element() {
        let mut reader = XmlReader::new("<root/>");
        reader.read().unwrap();
        assert_eq!(reader.node_type(), XmlNodeType::Element);
        assert!(!reader.has_value());
    }

    // === Test: value returns None for element ===

    #[test]
    fn test_value_none_for_element() {
        let mut reader = XmlReader::new("<root/>");
        reader.read().unwrap();
        assert_eq!(reader.value(), None);
    }

    // === Test: initial state ===

    #[test]
    fn test_initial_state() {
        let reader = XmlReader::new("<root/>");
        assert_eq!(reader.node_type(), XmlNodeType::None);
        assert_eq!(reader.name(), None);
        assert_eq!(reader.value(), None);
        assert!(!reader.has_value());
        assert_eq!(reader.depth(), 0);
        assert_eq!(reader.attribute_count(), 0);
    }

    // === Test: complex document ===

    #[test]
    fn test_read_complex_document() {
        let xml = r#"<?xml version="1.0"?>
<!DOCTYPE doc>
<!-- prolog comment -->
<?style type="text/css"?>
<doc attr="val">
  <child>text</child>
  <![CDATA[raw]]>
  <!-- inner comment -->
  <empty/>
</doc>"#;
        let nodes = read_all_types(xml);
        // Verify we get all the expected node types.
        let types: Vec<XmlNodeType> = nodes.iter().map(|(t, _)| *t).collect();
        assert!(types.contains(&XmlNodeType::XmlDeclaration));
        assert!(types.contains(&XmlNodeType::DocumentType));
        assert!(types.contains(&XmlNodeType::Comment));
        assert!(types.contains(&XmlNodeType::ProcessingInstruction));
        assert!(types.contains(&XmlNodeType::Element));
        assert!(types.contains(&XmlNodeType::Text));
        assert!(types.contains(&XmlNodeType::CData));
        assert!(types.contains(&XmlNodeType::EndElement));
    }

    // === Test: prolog comments and PIs ===

    #[test]
    fn test_read_prolog_comment() {
        let nodes = read_all_types("<!-- prolog --><root/>");
        assert_eq!(
            nodes,
            vec![
                (XmlNodeType::Comment, " prolog ".to_string()),
                (XmlNodeType::Element, "root".to_string()),
            ]
        );
    }

    // === Test: trailing comments ===

    #[test]
    fn test_read_trailing_comment() {
        let nodes = read_all_types("<root/><!-- trailing -->");
        assert_eq!(
            nodes,
            vec![
                (XmlNodeType::Element, "root".to_string()),
                (XmlNodeType::Comment, " trailing ".to_string()),
            ]
        );
    }

    // === Test: move_to_element returns false when not on attribute ===

    #[test]
    fn test_move_to_element_when_not_on_attribute() {
        let mut reader = XmlReader::new("<root/>");
        reader.read().unwrap();
        assert!(!reader.move_to_element());
    }

    // === Test: empty document ===

    #[test]
    fn test_read_empty_input() {
        let mut reader = XmlReader::new("");
        assert!(!reader.read().unwrap());
    }

    // === Test: deeply nested ===

    #[test]
    fn test_read_deeply_nested() {
        let mut reader = XmlReader::new("<a><b><c><d><e>deep</e></d></c></b></a>");

        reader.read().unwrap(); // <a> depth=0
        assert_eq!(reader.depth(), 0);
        reader.read().unwrap(); // <b> depth=1
        assert_eq!(reader.depth(), 1);
        reader.read().unwrap(); // <c> depth=2
        assert_eq!(reader.depth(), 2);
        reader.read().unwrap(); // <d> depth=3
        assert_eq!(reader.depth(), 3);
        reader.read().unwrap(); // <e> depth=4
        assert_eq!(reader.depth(), 4);
        reader.read().unwrap(); // "deep"
        assert_eq!(reader.depth(), 5);
        assert_eq!(reader.value(), Some("deep"));
    }

    // === Test: single-quoted attributes ===

    #[test]
    fn test_read_single_quoted_attributes() {
        let mut reader = XmlReader::new("<root attr='value'/>");
        reader.read().unwrap();
        assert_eq!(reader.get_attribute("attr"), Some("value"));
    }
}
