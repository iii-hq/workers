//! Error-tolerant HTML parser.
//!
//! This module implements an error-tolerant HTML 4.01 parser, similar to
//! libxml2's `HTMLparser.c`. Unlike the strict XML parser, this parser handles
//! common HTML patterns that are technically malformed:
//!
//! - Missing closing tags (auto-closed based on HTML content model rules)
//! - Unquoted attribute values (`<div class=main>`)
//! - Void elements that never need closing (`<br>`, `<img>`, `<hr>`, etc.)
//! - Case-insensitive tag name matching
//! - Bare `&` characters (not just `&amp;`)
//! - Missing doctype
//! - Boolean attributes without values (`<input disabled>`)
//!
//! The parser produces the same `Document` tree structure as the XML parser.
//!
//! # Examples
//!
//! ```
//! use xmloxide::html::parse_html;
//!
//! let doc = parse_html("<p>Hello <b>world</b>").unwrap();
//! let root = doc.root_element().unwrap();
//! assert_eq!(doc.node_name(root), Some("html"));
//! ```

pub mod entities;

use crate::error::{ErrorSeverity, ParseDiagnostic, ParseError};
use crate::parser::input::ParserInput;
use crate::tree::{Attribute, Document, NodeId, NodeKind};

/// Options controlling HTML parser behavior.
///
/// Use the builder pattern to configure options:
///
/// ```
/// use xmloxide::html::HtmlParseOptions;
///
/// let opts = HtmlParseOptions::default()
///     .recover(true)
///     .no_blanks(true)
///     .no_implied(true);
/// ```
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct HtmlParseOptions {
    /// If true, attempt to recover from errors and produce a partial tree.
    /// This is always effectively true for HTML parsing since HTML is
    /// inherently error-tolerant, but setting this to false makes the parser
    /// stricter about certain issues.
    pub recover: bool,
    /// If true, strip ignorable whitespace-only text nodes.
    pub no_blanks: bool,
    /// If true, do not add implied `html`, `head`, and `body` elements.
    pub no_implied: bool,
    /// If true, suppress warning diagnostics.
    pub no_warnings: bool,
}

impl Default for HtmlParseOptions {
    fn default() -> Self {
        Self {
            recover: true,
            no_blanks: false,
            no_implied: false,
            no_warnings: false,
        }
    }
}

impl HtmlParseOptions {
    /// Enables or disables error recovery mode.
    ///
    /// When enabled, the parser attempts to produce a partial tree even when
    /// it encounters malformed markup, collecting errors as diagnostics on
    /// the resulting [`Document`]. HTML parsing is inherently error-tolerant,
    /// but disabling this makes the parser stricter about certain issues.
    /// Enabled by default.
    #[must_use]
    pub fn recover(mut self, yes: bool) -> Self {
        self.recover = yes;
        self
    }

    /// Enables or disables stripping of blank (whitespace-only) text nodes.
    ///
    /// When enabled, text nodes that contain only whitespace (spaces, tabs,
    /// newlines) are discarded during parsing. This is useful for reducing
    /// tree size when whitespace between elements is not significant.
    /// Disabled by default.
    #[must_use]
    pub fn no_blanks(mut self, yes: bool) -> Self {
        self.no_blanks = yes;
        self
    }

    /// Enables or disables generation of implied `<html>`, `<head>`, and
    /// `<body>` elements.
    ///
    /// By default, the HTML parser automatically wraps content in these
    /// structural elements when they are missing (matching browser behavior).
    /// When enabled, the parser omits these implied wrappers, producing a
    /// tree that more closely reflects the literal input.
    /// Disabled by default.
    #[must_use]
    pub fn no_implied(mut self, yes: bool) -> Self {
        self.no_implied = yes;
        self
    }

    /// Enables or disables suppression of warning-level diagnostics.
    ///
    /// When enabled, only errors and fatal issues are recorded in
    /// [`Document::diagnostics`]; warnings about non-critical issues (e.g.,
    /// missing optional closing tags) are silently discarded.
    /// Disabled by default.
    #[must_use]
    pub fn no_warnings(mut self, yes: bool) -> Self {
        self.no_warnings = yes;
        self
    }
}

/// Parses an HTML string into a `Document` with default options.
///
/// The parser is error-tolerant and will produce a tree even for malformed
/// HTML. Diagnostics about any issues found during parsing are stored in
/// `Document::diagnostics`.
///
/// # Errors
///
/// Returns `ParseError` only for truly unrecoverable errors (e.g., empty input
/// when recovery is disabled).
///
/// # Examples
///
/// ```
/// use xmloxide::html::parse_html;
///
/// let doc = parse_html("<p>Hello <b>world</b>").unwrap();
/// let root = doc.root_element().unwrap();
/// assert_eq!(doc.node_name(root), Some("html"));
/// ```
pub fn parse_html(input: &str) -> Result<Document, ParseError> {
    parse_html_with_options(input, &HtmlParseOptions::default())
}

/// Parses an HTML string into a `Document` with the given options.
///
/// # Errors
///
/// Returns `ParseError` if the input cannot be parsed and recovery mode is
/// disabled.
///
/// # Examples
///
/// ```
/// use xmloxide::html::{parse_html_with_options, HtmlParseOptions};
///
/// let opts = HtmlParseOptions::default().no_blanks(true);
/// let doc = parse_html_with_options("<html><body><p>Hi</p></body></html>", &opts).unwrap();
/// ```
pub fn parse_html_with_options(
    input: &str,
    options: &HtmlParseOptions,
) -> Result<Document, ParseError> {
    let mut parser = HtmlParser::new(input, options);
    parser.parse()
}

// --- Void elements (elements that must not have content) ---
// See HTML 4.01 and common usage. These elements are self-closing.

/// Returns true if the given tag name (lowercase) is a void element that
/// must not have content.
pub(crate) fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
            | "basefont"
            | "frame"
            | "isindex"
    )
}

/// HTML4 attributes libxml serializes in minimized form regardless of value.
pub(crate) fn is_boolean_attribute(name: &str) -> bool {
    matches!(
        name,
        "checked"
            | "compact"
            | "declare"
            | "defer"
            | "disabled"
            | "ismap"
            | "multiple"
            | "nohref"
            | "noresize"
            | "noshade"
            | "nowrap"
            | "readonly"
            | "selected"
    )
}

fn html_char_reference(value: u32) -> char {
    match value {
        0 | 0xD800..=0xDFFF | 0x11_0000.. => '\u{FFFD}',
        0x80 => '\u{20AC}',
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}',
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        _ => char::from_u32(value).unwrap_or('\u{FFFD}'),
    }
}

/// Returns true if `tag` is an element whose opening auto-closes `open_tag`.
///
/// For example, a `<p>` auto-closes a previous `<p>`, and a `<li>` auto-closes
/// a previous `<li>`.
///
/// See HTML 4.01 DTD for the optional end tag rules.
fn auto_closes(open_tag: &str, tag: &str) -> bool {
    match open_tag {
        "a" => matches!(tag, "fieldset" | "table" | "td" | "th" | "a"),
        "b" | "i" | "u" => matches!(tag, "p" | "td" | "th"),
        "s" | "small" | "big" => tag == "p",
        "span" | "font" => matches!(tag, "td" | "th"),
        "p" => matches!(
            tag,
            "p" | "div"
                | "ul"
                | "ol"
                | "dl"
                | "pre"
                | "table"
                | "blockquote"
                | "address"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "hr"
                | "form"
                | "fieldset"
                | "section"
                | "article"
                | "aside"
                | "header"
                | "footer"
                | "nav"
                | "figure"
                | "figcaption"
                | "main"
                | "details"
                | "summary"
                | "tr"
                | "td"
                | "th"
        ),
        "li" => tag == "li",
        "dt" => matches!(tag, "dt" | "dd"),
        "dd" => tag == "dt",
        "tr" => tag == "tr",
        "td" => matches!(tag, "td" | "th" | "tr"),
        "th" => matches!(tag, "td" | "th" | "tr"),
        "thead" => matches!(tag, "tbody" | "tfoot"),
        "tbody" => matches!(tag, "tbody" | "tfoot"),
        "tfoot" => tag == "tbody",
        "option" => matches!(tag, "option" | "optgroup"),
        "optgroup" => tag == "optgroup",
        "colgroup" => {
            // colgroup is auto-closed by most things that are not col
            tag != "col" && matches!(tag, "thead" | "tbody" | "tfoot" | "tr" | "colgroup")
        }
        "head" => matches!(tag, "body" | "frameset"),
        _ => false,
    }
}

/// Returns true if `tag` is a raw text element whose content is not parsed
/// as HTML (script, style).
pub(crate) fn is_raw_text_element(tag: &str) -> bool {
    matches!(tag, "script" | "style")
}

/// Returns true if `tag` is an element that belongs in `<head>`.
fn is_head_content_element(tag: &str) -> bool {
    matches!(
        tag,
        "title" | "meta" | "link" | "base" | "style" | "script" | "noscript"
    )
}

// --- The HTML Parser ---

/// The core HTML parser state machine.
///
/// Implements a hand-rolled, error-tolerant parser for HTML 4.01 that produces
/// a `Document` tree. Unlike the XML parser, this parser:
/// - Normalizes tag names to lowercase
/// - Handles void elements (self-closing elements like `<br>`)
/// - Auto-closes elements based on HTML content model rules
/// - Accepts unquoted and boolean attributes
/// - Resolves HTML named character references
/// - Adds implied elements (html, head, body) when missing
struct HtmlParser<'a> {
    /// Shared low-level input state (position, peek, advance, etc.).
    input: ParserInput<'a>,
    /// The document being built.
    doc: Document,
    /// Parser options.
    options: HtmlParseOptions,
    /// Stack of open element node IDs and their lowercase tag names.
    open_elements: Vec<(NodeId, String)>,
    /// Set when a fatal error (e.g. depth limit) occurs; stops parsing.
    fatal_error: Option<ParseError>,
}

impl<'a> HtmlParser<'a> {
    fn new(input: &'a str, options: &HtmlParseOptions) -> Self {
        let mut pi = ParserInput::new(input);
        pi.set_recover(options.recover);

        Self {
            input: pi,
            doc: Document::new(),
            options: options.clone(),
            open_elements: Vec::new(),
            fatal_error: None,
        }
    }

    /// Main parse entry point. Parses the entire HTML document.
    fn parse(&mut self) -> Result<Document, ParseError> {
        self.input.skip_whitespace();

        // Track whether a DOCTYPE was found in the input
        let mut has_doctype = false;

        // Parse optional DOCTYPE
        if self.input.looking_at_ci(b"<!doctype") {
            self.parse_doctype();
            self.input.skip_whitespace();
            has_doctype = true;
        }

        // Parse the body content
        self.parse_content();

        // If a fatal error was recorded (e.g. depth limit), return it.
        if let Some(err) = self.fatal_error.take() {
            return Err(err);
        }

        // Close any remaining open elements
        while let Some((_, tag)) = self.open_elements.pop() {
            self.push_warning(format!("unclosed element <{tag}> at end of document"));
        }

        // Add default DOCTYPE if none was in the input and not disabled
        if !has_doctype && !self.options.no_implied {
            let doctype_id = self.doc.create_node(NodeKind::DocumentType {
                name: "html".to_string(),
                public_id: Some("-//W3C//DTD HTML 4.0 Transitional//EN".to_string()),
                system_id: Some("http://www.w3.org/TR/REC-html40/loose.dtd".to_string()),
                internal_subset: None,
            });
            // Prepend DOCTYPE before the first child of the document root
            let root = self.doc.root();
            self.doc.prepend_child(root, doctype_id);
        }

        Ok(std::mem::take(&mut self.doc))
    }

    /// Ensures an implied `<html>` element exists at the document root.
    /// Returns its `NodeId`.
    fn ensure_html(&mut self) -> NodeId {
        let root = self.doc.root();
        // Look for existing html element
        for id in self.doc.children(root) {
            if matches!(&self.doc.node(id).kind, NodeKind::Element { name, .. } if name == "html") {
                return id;
            }
        }
        // Create implied html
        let html_id = self.doc.create_node(NodeKind::Element {
            name: "html".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        self.doc.append_child(root, html_id);
        self.open_elements.push((html_id, "html".to_string()));
        html_id
    }

    /// Ensures an implied `<body>` element exists under `<html>`.
    /// Returns its `NodeId`.
    fn ensure_body(&mut self) -> NodeId {
        let html_id = self.ensure_html();
        // Look for existing body element
        for id in self.doc.children(html_id) {
            if matches!(&self.doc.node(id).kind, NodeKind::Element { name, .. } if name == "body") {
                return id;
            }
        }
        // Create implied body
        let body_id = self.doc.create_node(NodeKind::Element {
            name: "body".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        self.doc.append_child(html_id, body_id);
        self.open_elements.push((body_id, "body".to_string()));
        body_id
    }

    /// Ensures an implied `<head>` element exists under `<html>`.
    /// Returns its `NodeId`.
    fn ensure_head(&mut self) -> NodeId {
        let html_id = self.ensure_html();
        // Look for existing head element
        for id in self.doc.children(html_id) {
            if matches!(&self.doc.node(id).kind, NodeKind::Element { name, .. } if name == "head") {
                return id;
            }
        }
        // Create implied head. Insert before body if body exists,
        // otherwise append to html.
        let head_id = self.doc.create_node(NodeKind::Element {
            name: "head".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        // Find body — if it exists, insert head before it
        let body_id = self.doc.children(html_id).find(|&id| {
            matches!(&self.doc.node(id).kind, NodeKind::Element { name, .. } if name == "body")
        });
        if let Some(body) = body_id {
            self.doc.insert_before(body, head_id);
        } else {
            self.doc.append_child(html_id, head_id);
        }
        head_id
    }

    /// Returns the current insertion point (the innermost open element, or
    /// the document root).
    fn current_parent(&self) -> NodeId {
        self.open_elements
            .last()
            .map_or_else(|| self.doc.root(), |&(id, _)| id)
    }

    /// Parses the HTML content: elements, text, comments, etc.
    fn parse_content(&mut self) {
        while !self.input.at_end() && self.fatal_error.is_none() {
            if self.input.looking_at(b"<!--") {
                self.parse_comment();
            } else if self.input.looking_at_ci(b"<!doctype") {
                // Ignore extra doctypes
                self.skip_to_gt();
            } else if self.input.looking_at(b"</") {
                self.parse_end_tag();
            } else if self.input.peek() == Some(b'<')
                && self
                    .input
                    .peek_at(1)
                    .is_some_and(|b| b.is_ascii_alphabetic())
            {
                self.parse_start_tag();
            } else if self.input.peek() == Some(b'<') && self.input.peek_at(1) == Some(b'!') {
                // Could be <![if ...]> (IE conditional comment) or other malformed markup
                if self.input.peek_at(2) == Some(b'[') {
                    self.skip_conditional_comment();
                } else {
                    self.push_warning("malformed markup".to_string());
                    self.input.advance(1);
                }
            } else if self.input.peek() == Some(b'<') && self.input.peek_at(1) == Some(b'?') {
                self.parse_processing_instruction();
            } else if self.input.peek() == Some(b'<') {
                // Bare '<' not followed by alpha, '/', '!', or '?' — treat as text.
                // We must consume it here to avoid an infinite loop, since
                // parse_text() breaks on '<' without advancing.
                self.input.advance(1);
                if !self.options.no_implied && self.open_elements.is_empty() {
                    self.ensure_body();
                }
                let parent = self.current_parent();
                // Merge with previous text node if possible
                if let Some(last_child) = self.doc.last_child(parent) {
                    if let NodeKind::Text { content } = &mut self.doc.node_mut(last_child).kind {
                        content.push('<');
                        continue;
                    }
                }
                let text_id = self.doc.create_node(NodeKind::Text {
                    content: "<".to_string(),
                });
                self.doc.append_child(parent, text_id);
            } else {
                self.parse_text();
            }
        }
    }

    // --- DOCTYPE ---

    fn parse_doctype(&mut self) {
        // Skip <!DOCTYPE (case-insensitive)
        self.input.advance(9); // "<!DOCTYPE" or "<!doctype"
        self.input.skip_whitespace();

        // Read the root element name
        let name = self.parse_tag_name();

        self.input.skip_whitespace();

        let mut system_id = None;
        let mut public_id = None;

        if self.input.looking_at_ci(b"system") {
            self.input.advance(6);
            self.input.skip_whitespace();
            system_id = self.try_parse_quoted_value();
            self.input.skip_whitespace();
        } else if self.input.looking_at_ci(b"public") {
            self.input.advance(6);
            self.input.skip_whitespace();
            public_id = self.try_parse_quoted_value();
            self.input.skip_whitespace();
            system_id = self.try_parse_quoted_value();
            self.input.skip_whitespace();
        }

        // Skip to end of DOCTYPE
        while !self.input.at_end() && self.input.peek() != Some(b'>') {
            self.input.advance(1);
        }
        if !self.input.at_end() {
            self.input.advance(1); // consume '>'
        }

        let doctype_id = self.doc.create_node(NodeKind::DocumentType {
            name,
            system_id,
            public_id,
            internal_subset: None,
        });
        self.doc.append_child(self.doc.root(), doctype_id);
    }

    // --- Start Tag ---

    #[allow(clippy::too_many_lines)]
    fn parse_start_tag(&mut self) {
        let tag_start = self.input.save_position();
        self.input.advance(1); // consume '<'
        if let Err(e) = self.input.increment_depth() {
            self.fatal_error = Some(e);
            return;
        }
        let tag = self.parse_tag_name();

        if tag.is_empty() {
            self.push_warning("empty tag name".to_string());
            self.input.decrement_depth();
            self.skip_to_gt();
            return;
        }

        let lower_tag = tag.to_ascii_lowercase();

        // Parse attributes
        let attributes = self.parse_attributes();

        self.input.skip_whitespace();

        // libxml discards an unterminated start tag and everything consumed
        // since '<' instead of synthesizing an element at EOF.
        if self.input.at_end() {
            self.input.restore_position(tag_start);
            self.input.advance(self.input.remaining().len());
            self.input.decrement_depth();
            return;
        }

        // Handle self-closing slash
        let explicit_self_close = self.input.peek() == Some(b'/');
        if explicit_self_close {
            self.input.advance(1);
        }

        // Consume '>'
        if self.input.peek() == Some(b'>') {
            self.input.advance(1);
        } else if !self.input.at_end() {
            self.push_warning(format!("expected '>' after tag <{lower_tag}>"));
            self.skip_to_gt();
        }

        // Handle structural elements by merging with existing implied elements
        if !self.options.no_implied {
            if lower_tag == "html" {
                let html_id = self.ensure_html();
                self.merge_attributes(html_id, attributes);
                // Ensure html is on the open_elements stack
                if !self.open_elements.iter().any(|(_, t)| t == "html") {
                    self.open_elements.push((html_id, "html".to_string()));
                }
                self.input.decrement_depth();
                return;
            }
            if lower_tag == "head" {
                let head_id = self.ensure_head();
                self.merge_attributes(head_id, attributes);
                // Push head to open_elements so child elements go inside it
                if !self.open_elements.iter().any(|(_, t)| t == "head") {
                    self.open_elements.push((head_id, "head".to_string()));
                }
                self.input.decrement_depth();
                return;
            }
            if lower_tag == "body" && !self.is_in_frameset() {
                // Close head if open
                self.close_head_if_open();
                let body_id = self.ensure_body();
                self.merge_attributes(body_id, attributes);
                if !self.open_elements.iter().any(|(_, t)| t == "body") {
                    self.open_elements.push((body_id, "body".to_string()));
                }
                self.input.decrement_depth();
                return;
            }
        }

        // Handle auto-closing: if the new element auto-closes an open one,
        // pop the stack accordingly.
        self.handle_auto_close(&lower_tag);

        // For non-structural elements, ensure proper containment
        if !self.options.no_implied {
            if lower_tag == "frameset" {
                // <frameset> replaces <body> in HTML 4.01 — place it
                // directly under <html>, not inside <body>
                self.close_head_if_open();
                self.ensure_html();
            } else if is_head_content_element(&lower_tag) && !self.is_in_body() {
                // Head-content elements go under <head>
                let head_id = self.ensure_head();
                if !self.open_elements.iter().any(|(_, t)| t == "head") {
                    self.open_elements.push((head_id, "head".to_string()));
                }
            } else if self.is_in_frameset() {
                // Inside a frameset context, don't auto-create body.
                // Elements like <frame>, <noframes> stay inside frameset.
            } else {
                // Body-content elements: close head, ensure body exists
                self.close_head_if_open();
                self.ensure_body();
            }
        }

        let parent = self.current_parent();

        let id_value = attributes.iter().find_map(|a| {
            if a.name == "id" {
                Some(a.value.clone())
            } else {
                None
            }
        });

        let elem_id = self.doc.create_node(NodeKind::Element {
            name: lower_tag.clone(),
            prefix: None,
            namespace: None,
            attributes,
        });
        self.doc.append_child(parent, elem_id);

        if let Some(id_val) = id_value {
            self.doc.set_id(&id_val, elem_id);
        }

        // Void elements and explicit self-close don't get pushed
        if is_void_element(&lower_tag) || explicit_self_close {
            self.input.decrement_depth();
            return;
        }

        // Raw text elements (script, style) need special handling
        if is_raw_text_element(&lower_tag) {
            self.open_elements.push((elem_id, lower_tag.clone()));
            self.parse_raw_text(&lower_tag);
            // pop the element after raw text
            self.open_elements.pop();
            self.input.decrement_depth();
            return;
        }

        // HTML4/libxml treats `<plaintext>` as an irreversible switch: every
        // remaining byte, including apparent end tags and entity references,
        // belongs to one text node. The serializer escapes that text and adds
        // the element's synthetic closing tag.
        if lower_tag == "plaintext" {
            self.open_elements.push((elem_id, lower_tag));
            let remaining = self.input.remaining();
            if !remaining.is_empty() {
                let content = String::from_utf8_lossy(remaining).into_owned();
                let text_id = self.doc.create_node(NodeKind::Text { content });
                self.doc.append_child(elem_id, text_id);
                self.input.advance(remaining.len());
            }
            self.input.decrement_depth();
            return;
        }

        self.open_elements.push((elem_id, lower_tag));
    }

    /// Merges attributes from a parsed tag into an existing element node.
    fn merge_attributes(&mut self, elem_id: NodeId, attrs: Vec<Attribute>) {
        if attrs.is_empty() {
            return;
        }
        if let NodeKind::Element { attributes, .. } = &mut self.doc.node_mut(elem_id).kind {
            for attr in attrs {
                if !attributes.iter().any(|a| a.name == attr.name) {
                    attributes.push(attr);
                }
            }
        }
    }

    /// Closes `<head>` if it's currently open on the element stack.
    fn close_head_if_open(&mut self) {
        if self.open_elements.last().is_some_and(|(_, t)| t == "head") {
            self.open_elements.pop();
        }
    }

    /// Returns true if the parser is currently inside `<body>`.
    fn is_in_body(&self) -> bool {
        self.open_elements.iter().any(|(_, t)| t == "body")
    }

    /// Returns true if the parser is currently inside a `<frameset>`.
    fn is_in_frameset(&self) -> bool {
        self.open_elements.iter().any(|(_, t)| t == "frameset")
    }

    /// Handles auto-closing of open elements when a new element is encountered.
    fn handle_auto_close(&mut self, new_tag: &str) {
        // Walk the open elements stack from top to find elements that should
        // be auto-closed by the new tag.
        loop {
            let should_close = self
                .open_elements
                .last()
                .is_some_and(|(_, open_tag)| auto_closes(open_tag, new_tag));
            if should_close {
                self.open_elements.pop();
                self.input.decrement_depth();
            } else {
                break;
            }
        }
    }

    // --- End Tag ---

    fn parse_end_tag(&mut self) {
        self.input.advance(2); // consume '</'
        let tag = self.parse_tag_name();
        let lower_tag = tag.to_ascii_lowercase();

        // Skip to '>'
        self.input.skip_whitespace();
        if self.input.peek() == Some(b'>') {
            self.input.advance(1);
        } else if !self.input.at_end() {
            self.push_warning(format!("expected '>' after end tag </{lower_tag}>"));
            self.skip_to_gt();
        }

        // Void elements should not have end tags; ignore them.
        if is_void_element(&lower_tag) {
            self.push_warning(format!("end tag for void element </{lower_tag}> ignored"));
            return;
        }

        // Find the matching open element in the stack
        let found = self
            .open_elements
            .iter()
            .rposition(|(_, name)| *name == lower_tag);

        if let Some(idx) = found {
            // libxml's HTML4 recovery refuses an ancestor end tag when a
            // still-open structural element above it cannot be implicitly
            // closed. The unmatched ancestor then remains open to EOF.
            let incompatible = |tag: &str| match lower_tag.as_str() {
                "table" => false,
                "thead" => matches!(tag, "table" | "tbody" | "tfoot"),
                "tbody" => matches!(tag, "table" | "tfoot"),
                "tfoot" => matches!(tag, "table" | "tbody"),
                "tr" => matches!(tag, "table" | "thead" | "tbody" | "tfoot"),
                "td" | "th" => matches!(
                    tag,
                    "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
                ),
                _ => matches!(
                    tag,
                    "div" | "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
                ),
            };
            if self.open_elements[idx + 1..]
                .iter()
                .any(|(_, tag)| incompatible(tag))
            {
                self.push_warning(format!("stray end tag </{lower_tag}>"));
                return;
            }
            // Pop everything above and including the matched element
            // (auto-closing any intervening elements)
            let count = self.open_elements.len() - idx;
            for i in (0..count).rev() {
                let stack_idx = idx + i;
                if stack_idx < self.open_elements.len() {
                    let (_, ref closed_tag) = self.open_elements[stack_idx];
                    if *closed_tag != lower_tag {
                        self.push_warning(format!(
                            "implicitly closing <{closed_tag}> before </{lower_tag}>"
                        ));
                    }
                }
            }
            // Decrement depth for each element being closed
            for _ in 0..count {
                self.input.decrement_depth();
            }
            self.open_elements.truncate(idx);
        } else {
            // No matching open element — stray end tag
            self.push_warning(format!("stray end tag </{lower_tag}>"));
        }
    }

    // --- Attributes ---

    fn parse_attributes(&mut self) -> Vec<Attribute> {
        let mut attributes = Vec::new();

        loop {
            self.input.skip_whitespace();

            // Check for end of tag
            if self.input.at_end()
                || self.input.peek() == Some(b'>')
                || self.input.peek() == Some(b'/')
                || self.input.looking_at(b"/>")
            {
                break;
            }

            // Parse attribute name
            let name = self.parse_attr_name();
            if name.is_empty() {
                // Skip the bad character and continue
                self.input.advance(1);
                continue;
            }

            let lower_name = name.to_ascii_lowercase();

            self.input.skip_whitespace();

            // Check for = (attribute with value)
            let (value, minimized) = if self.input.peek() == Some(b'=') {
                self.input.advance(1); // consume '='
                self.input.skip_whitespace();
                (self.parse_attr_value(), false)
            } else {
                // libxml exposes HTML4 boolean attributes as their own name;
                // other minimized attributes have an empty DOM value.
                (
                    if is_boolean_attribute(&lower_name) {
                        lower_name.clone()
                    } else {
                        String::new()
                    },
                    true,
                )
            };

            // libxml keeps the first duplicate attribute and discards later
            // occurrences without disturbing source order.
            if !attributes
                .iter()
                .any(|attr: &Attribute| attr.name == lower_name)
            {
                attributes.push(Attribute {
                    name: lower_name,
                    value,
                    prefix: None,
                    namespace: None,
                    // An empty raw value cannot be produced by entity
                    // expansion, so it safely remembers source minimization
                    // for the libxml-compatible HTML serializer.
                    raw_value: minimized.then(String::new),
                });
            }
        }

        attributes
    }

    /// Parses an attribute name (sequence of non-whitespace, non-special chars).
    fn parse_attr_name(&mut self) -> String {
        let start = self.input.pos();
        while let Some(b) = self.input.peek() {
            if b == b' '
                || b == b'\t'
                || b == b'\r'
                || b == b'\n'
                || b == b'='
                || b == b'>'
                || b == b'/'
                || b == b'<'
                || b == b'"'
                || b == b'\''
            {
                break;
            }
            self.input.advance(1);
        }
        String::from_utf8_lossy(self.input.slice(start, self.input.pos())).to_string()
    }

    /// Parses an attribute value. Handles quoted and unquoted values.
    fn parse_attr_value(&mut self) -> String {
        if self.input.at_end() {
            return String::new();
        }

        let b = self.input.peek();
        if b == Some(b'"') || b == Some(b'\'') {
            // Quoted attribute value — we know `b` is Some from the check above
            let quote = b.unwrap_or(b'"');
            self.input.advance(1); // consume opening quote
            let mut value = String::new();
            while !self.input.at_end() {
                let ch = self.input.peek();
                if ch == Some(quote) {
                    self.input.advance(1);
                    break;
                }
                if ch == Some(b'&') {
                    let resolved = self.parse_html_reference();
                    value.push_str(&resolved);
                } else {
                    let c = self.next_char_html();
                    value.push(c);
                }
            }
            value
        } else {
            // Unquoted attribute value
            let mut value = String::new();
            while let Some(b) = self.input.peek() {
                if b == b' '
                    || b == b'\t'
                    || b == b'\r'
                    || b == b'\n'
                    || b == b'>'
                    || b == b'<'
                    || b == b'`'
                {
                    break;
                }
                // Backslash escaping: \c includes both chars literally
                // (libxml2 treats \" as literal backslash+quote)
                if b == b'\\' {
                    let c1 = self.next_char_html();
                    value.push(c1);
                    if !self.input.at_end() {
                        let c2 = self.next_char_html();
                        value.push(c2);
                    }
                    continue;
                }
                if b == b'"' || b == b'\'' {
                    break;
                }
                if b == b'&' {
                    let resolved = self.parse_html_reference();
                    value.push_str(&resolved);
                } else {
                    let c = self.next_char_html();
                    value.push(c);
                }
            }
            value
        }
    }

    // --- Text Content ---

    fn parse_text(&mut self) {
        let mut text = String::new();

        while !self.input.at_end() {
            if self.input.peek() == Some(b'<') {
                break;
            }

            if self.input.peek() == Some(b'&') {
                let resolved = self.parse_html_reference();
                text.push_str(&resolved);
            } else {
                let ch = self.next_char_html();
                text.push(ch);
            }
        }

        if !text.is_empty() {
            // Strip blank text nodes if configured
            if self.options.no_blanks && text.chars().all(char::is_whitespace) {
                return;
            }
            // Ensure body exists for text content (unless whitespace between
            // structural elements or we're already inside an element)
            if !self.options.no_implied && self.open_elements.is_empty() {
                // Whitespace-only text before any elements is ignored
                if text.chars().all(char::is_whitespace) {
                    return;
                }
                self.ensure_body();
            }
            let parent = self.current_parent();
            let text_id = self.doc.create_node(NodeKind::Text { content: text });
            self.doc.append_child(parent, text_id);
        }
    }

    // --- Raw text (script/style) ---

    fn parse_raw_text(&mut self, tag: &str) {
        let mut content = String::new();
        let end_tag_bytes: Vec<u8> = format!("</{tag}").bytes().collect();

        while !self.input.at_end() {
            if self.input.looking_at_ci(&end_tag_bytes) {
                break;
            }
            let ch = self.next_char_html();
            content.push(ch);
        }

        if !content.is_empty() {
            let parent = self.current_parent();
            let text_id = self.doc.create_node(NodeKind::Text { content });
            self.doc.append_child(parent, text_id);
        }

        // Consume the end tag
        if !self.input.at_end() {
            self.input.advance(end_tag_bytes.len());
            self.input.skip_whitespace();
            if self.input.peek() == Some(b'>') {
                self.input.advance(1);
            }
        }
    }

    // --- Comments ---

    fn parse_comment(&mut self) {
        self.input.advance(4); // consume '<!--'

        // Handle abrupt closing: <!-->  and <!--->
        if self.input.peek() == Some(b'>') {
            self.input.advance(1);
            let parent = self.current_parent();
            let comment_id = self.doc.create_node(NodeKind::Comment {
                content: String::new(),
            });
            self.doc.append_child(parent, comment_id);
            return;
        }
        if self.input.looking_at(b"->") {
            self.input.advance(2);
            let parent = self.current_parent();
            let comment_id = self.doc.create_node(NodeKind::Comment {
                content: String::new(),
            });
            self.doc.append_child(parent, comment_id);
            return;
        }

        let mut content = String::new();
        let mut terminated = false;

        loop {
            if self.input.at_end() {
                self.push_warning("unterminated comment".to_string());
                break;
            }
            // Standard comment end: -->
            if self.input.looking_at(b"-->") {
                self.input.advance(3);
                terminated = true;
                break;
            }
            // Quirky comment end: --!> (libxml2 treats this as -->)
            if self.input.looking_at(b"--!>") {
                self.input.advance(4);
                terminated = true;
                break;
            }
            let ch = self.next_char_html();
            content.push(ch);
        }

        // libxml2 drops unterminated comments (those that reach EOF)
        if !terminated {
            return;
        }

        let parent = self.current_parent();
        let comment_id = self.doc.create_node(NodeKind::Comment { content });
        self.doc.append_child(parent, comment_id);
    }

    // --- Processing Instructions ---

    fn parse_processing_instruction(&mut self) {
        self.input.advance(2); // consume '<?'
        let target = self.parse_tag_name();
        self.input.skip_whitespace();

        let mut data = String::new();
        // In HTML mode, libxml2 reads PI content up to '>' only.
        // The '?' before '>' (if present) is included in the content,
        // so <?pi data?> stores data as "data?" and serializes as "<?pi data?>".
        loop {
            if self.input.at_end() {
                self.push_warning("unterminated processing instruction".to_string());
                break;
            }
            if self.input.peek() == Some(b'>') {
                self.input.advance(1);
                break;
            }
            let ch = self.next_char_html();
            data.push(ch);
        }

        let pi_data = if data.is_empty() { None } else { Some(data) };

        let parent = self.current_parent();
        let pi_id = self.doc.create_node(NodeKind::ProcessingInstruction {
            target,
            data: pi_data,
        });
        self.doc.append_child(parent, pi_id);
    }

    // --- HTML Entity/Character References ---

    /// Parses an HTML character or entity reference, or returns a bare `&`
    /// if it doesn't look like a valid reference (error-tolerant).
    fn parse_html_reference(&mut self) -> String {
        // Save position for backtracking
        let saved = self.input.save_position();

        self.input.advance(1); // consume '&'

        if self.input.peek() == Some(b'#') {
            // Character reference
            self.input.advance(1);
            if self.input.peek() == Some(b'x') || self.input.peek() == Some(b'X') {
                // Hex
                self.input.advance(1);
                let hex = self.take_while_ascii(|b| b.is_ascii_hexdigit());
                if !hex.is_empty() {
                    if self.input.peek() == Some(b';') {
                        self.input.advance(1);
                    }
                    if let Ok(value) = u32::from_str_radix(&hex, 16) {
                        return html_char_reference(value).to_string();
                    }
                }
                // Invalid — backtrack
                self.input.restore_position(saved);
                self.input.advance(1);
                return "&".to_string();
            }
            // Decimal
            let dec = self.take_while_ascii(|b| b.is_ascii_digit());
            if !dec.is_empty() {
                if self.input.peek() == Some(b';') {
                    self.input.advance(1);
                }
                if let Ok(value) = dec.parse::<u32>() {
                    return html_char_reference(value).to_string();
                }
            }
            // Invalid — backtrack
            self.input.restore_position(saved);
            self.input.advance(1);
            return "&".to_string();
        }

        // Named entity reference
        let name = self.take_while_ascii(|b| b.is_ascii_alphanumeric());
        if !name.is_empty() {
            // Try with semicolon
            if self.input.peek() == Some(b';') {
                self.input.advance(1);
                if let Some(value) = entities::lookup_entity(&name) {
                    return value.to_string();
                }
                // libxml's HTML parser accepts the longest known entity
                // prefix even when the complete name is unknown.
                for split in (1..name.len()).rev() {
                    if !name.is_char_boundary(split) {
                        continue;
                    }
                    if let Some(value) = entities::lookup_entity(&name[..split]) {
                        return format!("{value}{};", &name[split..]);
                    }
                }
                // Unknown entity — return it as-is with the & and ;
                self.push_warning(format!("unknown entity reference: &{name};"));
                return format!("&{name};");
            }
            // Try without semicolon (error-tolerant: some HTML uses &amp without ;)
            if let Some(value) = entities::lookup_entity(&name) {
                self.push_warning(format!("entity reference &{name} missing semicolon"));
                return value.to_string();
            }
        }

        // Not a valid reference — backtrack and return bare &
        self.input.restore_position(saved);
        self.input.advance(1);
        "&".to_string()
    }

    // --- Tag Name Parsing ---

    /// Parses an HTML tag name. Tag names can contain letters, digits, hyphens,
    /// and a few other characters. Returns the name as-is (caller normalizes case).
    fn parse_tag_name(&mut self) -> String {
        let start = self.input.pos();
        while let Some(b) = self.input.peek() {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':' || b == b'.' {
                self.input.advance(1);
            } else {
                break;
            }
        }
        String::from_utf8_lossy(self.input.slice(start, self.input.pos())).to_string()
    }

    /// Tries to parse a quoted value for DOCTYPE SYSTEM/PUBLIC identifiers.
    fn try_parse_quoted_value(&mut self) -> Option<String> {
        let quote = self.input.peek()?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        self.input.advance(1);
        let start = self.input.pos();
        while !self.input.at_end() && self.input.peek() != Some(quote) {
            self.input.advance(1);
        }
        let value = String::from_utf8_lossy(self.input.slice(start, self.input.pos())).to_string();
        if !self.input.at_end() {
            self.input.advance(1); // closing quote
        }
        Some(value)
    }

    // --- HTML-specific low-level helpers ---
    // These methods are NOT shared with the XML parser because HTML has
    // different rules (error tolerance, different character handling, etc.).

    /// Reads the next character, handling CR/LF normalization.
    /// Returns `'\0'` at end of input (error-tolerant, unlike the XML parser
    /// which returns an error).
    fn next_char_html(&mut self) -> char {
        if self.input.at_end() {
            return '\0';
        }
        if let Some(ch) = self.input.peek_char() {
            self.input.advance_char(ch);
            // Handle \r\n normalization
            if ch == '\r' {
                if self.input.peek() == Some(b'\n') {
                    self.input.advance(1);
                }
                return '\n';
            }
            ch
        } else {
            // Invalid UTF-8 — skip the byte and return replacement character
            self.input.advance(1);
            '\u{FFFD}'
        }
    }

    /// Skips an IE conditional comment marker (`<![if ...]>` or `<![endif]>`).
    ///
    /// These are Microsoft extensions to HTML. Each `<![...>` marker is
    /// individually consumed, but the content between them flows as normal
    /// text, matching libxml2 behavior.
    fn skip_conditional_comment(&mut self) {
        self.push_warning("incorrectly opened comment".to_string());
        self.skip_to_gt();
    }

    /// Skips forward to and past the next `>` character.
    fn skip_to_gt(&mut self) {
        while !self.input.at_end() {
            if self.input.peek() == Some(b'>') {
                self.input.advance(1);
                return;
            }
            self.input.advance(1);
        }
    }

    /// Takes characters while they match the predicate (ASCII only).
    fn take_while_ascii(&mut self, pred: impl Fn(u8) -> bool) -> String {
        let start = self.input.pos();
        while let Some(b) = self.input.peek() {
            if pred(b) {
                self.input.advance(1);
            } else {
                break;
            }
        }
        String::from_utf8_lossy(self.input.slice(start, self.input.pos())).to_string()
    }

    /// Pushes a warning diagnostic to the document. Respects the
    /// `no_warnings` option.
    fn push_warning(&mut self, message: String) {
        if self.options.no_warnings {
            return;
        }
        self.doc.diagnostics.push(ParseDiagnostic {
            severity: ErrorSeverity::Warning,
            message,
            location: self.input.location(),
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Document {
        parse_html(input).unwrap_or_else(|e| panic!("parse failed: {e}"))
    }

    fn parse_no_implied(input: &str) -> Document {
        let opts = HtmlParseOptions::default().no_implied(true);
        parse_html_with_options(input, &opts).unwrap_or_else(|e| panic!("parse failed: {e}"))
    }

    // --- Basic parsing ---

    #[test]
    fn test_parse_simple_html() {
        let doc = parse("<html><body><p>Hello</p></body></html>");
        let html = doc.root_element().unwrap();
        assert_eq!(doc.node_name(html), Some("html"));
    }

    #[test]
    fn test_parse_implied_structure() {
        // Even without explicit html/body, the parser adds them
        let doc = parse("<p>Hello</p>");
        let html = doc.root_element().unwrap();
        assert_eq!(doc.node_name(html), Some("html"));

        // Should have body child (empty head is removed)
        let children: Vec<_> = doc.children(html).collect();
        assert!(!children.is_empty());
        // Body should contain the <p>
        let body = children.last().unwrap();
        assert_eq!(doc.node_name(*body), Some("body"));
    }

    #[test]
    fn test_parse_no_implied_option() {
        let doc = parse_no_implied("<p>Hello</p>");
        let root = doc.root();
        let first = doc.first_child(root).unwrap();
        // Without implied structure, p should be directly under root
        assert_eq!(doc.node_name(first), Some("p"));
        assert_eq!(doc.text_content(first), "Hello");
    }

    // --- Void elements ---

    #[test]
    fn test_void_elements() {
        let doc = parse_no_implied("<p>line1<br>line2</p>");
        let root = doc.root();
        let p = doc.first_child(root).unwrap();
        let children: Vec<_> = doc.children(p).collect();
        assert_eq!(children.len(), 3); // "line1", <br>, "line2"
        assert_eq!(doc.node_name(children[1]), Some("br"));
        assert!(doc.first_child(children[1]).is_none()); // br has no children
    }

    #[test]
    fn test_void_element_with_end_tag() {
        let doc = parse_no_implied("<p>text<br></br>more</p>");
        let root = doc.root();
        let p = doc.first_child(root).unwrap();
        // The </br> should be ignored
        let children: Vec<_> = doc.children(p).collect();
        assert_eq!(children.len(), 3); // "text", <br>, "more"
    }

    #[test]
    fn test_img_void_element() {
        let doc = parse_no_implied("<img src=\"test.jpg\" alt=\"test\">");
        let root = doc.root();
        let img = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(img), Some("img"));
        assert_eq!(doc.attribute(img, "src"), Some("test.jpg"));
        assert_eq!(doc.attribute(img, "alt"), Some("test"));
    }

    // --- Case-insensitive tags ---

    #[test]
    fn test_case_insensitive_tags() {
        let doc = parse_no_implied("<DIV><P>Hello</P></DIV>");
        let root = doc.root();
        let div = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(div), Some("div"));
        let p = doc.first_child(div).unwrap();
        assert_eq!(doc.node_name(p), Some("p"));
        assert_eq!(doc.text_content(p), "Hello");
    }

    #[test]
    fn test_mixed_case_tags() {
        let doc = parse_no_implied("<Div><SPAN>Hi</span></div>");
        let root = doc.root();
        let div = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(div), Some("div"));
    }

    // --- Unquoted attributes ---

    #[test]
    fn test_unquoted_attributes() {
        let doc = parse_no_implied("<div class=main id=content>text</div>");
        let root = doc.root();
        let div = doc.first_child(root).unwrap();
        assert_eq!(doc.attribute(div, "class"), Some("main"));
        assert_eq!(doc.attribute(div, "id"), Some("content"));
    }

    #[test]
    fn test_boolean_attributes() {
        let doc = parse_no_implied("<input disabled readonly>");
        let root = doc.root();
        let input = doc.first_child(root).unwrap();
        assert_eq!(doc.attribute(input, "disabled"), Some("disabled"));
        assert_eq!(doc.attribute(input, "readonly"), Some("readonly"));
    }

    // --- Auto-closing ---

    #[test]
    fn test_p_auto_closes_p() {
        let doc = parse_no_implied("<p>First<p>Second");
        let root = doc.root();
        let children: Vec<_> = doc.children(root).collect();
        // Should have two p elements as siblings, not nested
        assert_eq!(children.len(), 2);
        assert_eq!(doc.node_name(children[0]), Some("p"));
        assert_eq!(doc.text_content(children[0]), "First");
        assert_eq!(doc.node_name(children[1]), Some("p"));
        assert_eq!(doc.text_content(children[1]), "Second");
    }

    #[test]
    fn test_li_auto_closes_li() {
        let doc = parse_no_implied("<ul><li>A<li>B<li>C</ul>");
        let root = doc.root();
        let ul = doc.first_child(root).unwrap();
        let items: Vec<_> = doc.children(ul).collect();
        assert_eq!(items.len(), 3);
        assert_eq!(doc.text_content(items[0]), "A");
        assert_eq!(doc.text_content(items[1]), "B");
        assert_eq!(doc.text_content(items[2]), "C");
    }

    #[test]
    fn test_dd_dt_auto_close() {
        let doc = parse_no_implied("<dl><dt>Term<dd>Def<dt>Term2<dd>Def2</dl>");
        let root = doc.root();
        let dl = doc.first_child(root).unwrap();
        let items: Vec<_> = doc.children(dl).collect();
        assert_eq!(items.len(), 4);
        assert_eq!(doc.node_name(items[0]), Some("dt"));
        assert_eq!(doc.node_name(items[1]), Some("dd"));
        assert_eq!(doc.node_name(items[2]), Some("dt"));
        assert_eq!(doc.node_name(items[3]), Some("dd"));
    }

    // --- Entity references ---

    #[test]
    fn test_html_entities() {
        let doc = parse_no_implied("<p>&copy; 2024 &mdash; All rights reserved</p>");
        let root = doc.root();
        let p = doc.first_child(root).unwrap();
        let text = doc.text_content(p);
        assert!(text.contains('\u{00A9}')); // copyright sign
        assert!(text.contains('\u{2014}')); // em dash
    }

    #[test]
    fn test_bare_ampersand() {
        let doc = parse_no_implied("<p>A & B</p>");
        let root = doc.root();
        let p = doc.first_child(root).unwrap();
        assert_eq!(doc.text_content(p), "A & B");
    }

    #[test]
    fn test_numeric_character_reference() {
        let doc = parse_no_implied("<p>&#65; &#x42;</p>");
        let root = doc.root();
        let p = doc.first_child(root).unwrap();
        assert_eq!(doc.text_content(p), "A B");
    }

    #[test]
    fn test_libxml_numeric_reference_recovery() {
        let doc = parse_no_implied("<p>&#x80; &#128 &#0 &#x110000</p>");
        let p = doc.first_child(doc.root()).unwrap();
        assert_eq!(doc.text_content(p), "€ € � �");
    }

    #[test]
    fn test_minimized_attribute_values() {
        let doc = parse_no_implied("<div foo checked hidden=hidden></div>");
        let div = doc.first_child(doc.root()).unwrap();
        assert_eq!(doc.attribute(div, "foo"), Some(""));
        assert_eq!(doc.attribute(div, "checked"), Some("checked"));
        assert_eq!(doc.attribute(div, "hidden"), Some("hidden"));
    }

    // --- Missing closing tags ---

    #[test]
    fn test_missing_closing_tags() {
        let doc = parse_no_implied("<div><p>Hello");
        let root = doc.root();
        let div = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(div), Some("div"));
        let p = doc.first_child(div).unwrap();
        assert_eq!(doc.node_name(p), Some("p"));
        assert_eq!(doc.text_content(p), "Hello");
    }

    // --- Comments ---

    #[test]
    fn test_html_comment() {
        let doc = parse_no_implied("<!-- hello --><p>text</p>");
        let root = doc.root();
        let first = doc.first_child(root).unwrap();
        assert_eq!(doc.node_text(first), Some(" hello "));
    }

    // --- Script/style raw text ---

    #[test]
    fn test_script_raw_text() {
        let doc = parse_no_implied("<script>var x = 1 < 2 && true;</script>");
        let root = doc.root();
        let script = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(script), Some("script"));
        assert_eq!(doc.text_content(script), "var x = 1 < 2 && true;");
    }

    #[test]
    fn test_style_raw_text() {
        let doc = parse_no_implied("<style>p > span { color: red; }</style>");
        let root = doc.root();
        let style = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(style), Some("style"));
        assert_eq!(doc.text_content(style), "p > span { color: red; }");
    }

    // --- DOCTYPE ---

    #[test]
    fn test_html_doctype() {
        let doc = parse_no_implied("<!DOCTYPE html><p>text</p>");
        let root = doc.root();
        let children: Vec<_> = doc.children(root).collect();
        assert!(children.len() >= 2);
        match &doc.node(children[0]).kind {
            NodeKind::DocumentType { name, .. } => {
                assert_eq!(name, "html");
            }
            other => panic!("expected DocumentType, got {other:?}"),
        }
    }

    // --- Single-quoted and double-quoted attributes ---

    #[test]
    fn test_single_quoted_attributes() {
        let doc = parse_no_implied("<div class='main'>text</div>");
        let root = doc.root();
        let div = doc.first_child(root).unwrap();
        assert_eq!(doc.attribute(div, "class"), Some("main"));
    }

    // --- Processing instructions ---

    #[test]
    fn test_html_processing_instruction() {
        let doc = parse_no_implied("<?xml-stylesheet type=\"text/css\"?><p>text</p>");
        let root = doc.root();
        let first = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(first), Some("xml-stylesheet"));
    }

    // --- Attribute case normalization ---

    #[test]
    fn test_attribute_case_normalized() {
        let doc = parse_no_implied("<div CLASS=\"main\" ID=\"1\">text</div>");
        let root = doc.root();
        let div = doc.first_child(root).unwrap();
        assert_eq!(doc.attribute(div, "class"), Some("main"));
        assert_eq!(doc.attribute(div, "id"), Some("1"));
    }

    // --- Whitespace stripping ---

    #[test]
    fn test_no_blanks_option() {
        let opts = HtmlParseOptions::default().no_blanks(true).no_implied(true);
        let doc = parse_html_with_options("<div>  \n  <p>text</p>  \n  </div>", &opts).unwrap();
        let root = doc.root();
        let div = doc.first_child(root).unwrap();
        // With no_blanks, whitespace-only text nodes are stripped
        let children: Vec<_> = doc.children(div).collect();
        assert_eq!(children.len(), 1);
        assert_eq!(doc.node_name(children[0]), Some("p"));
    }

    // --- Stray end tags ---

    #[test]
    fn test_stray_end_tag() {
        let doc = parse_no_implied("</div><p>text</p>");
        let root = doc.root();
        let p = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(p), Some("p"));
        assert_eq!(doc.text_content(p), "text");
        // Should have a diagnostic about the stray end tag
        assert!(!doc.diagnostics.is_empty());
    }

    // --- Complex document ---

    #[test]
    fn test_complex_html_document() {
        let doc = parse(
            r#"<!DOCTYPE html>
<html>
<head><title>Test</title></head>
<body>
<h1>Hello</h1>
<p>A paragraph with <b>bold</b> and <em>emphasis</em>.</p>
<ul>
<li>Item 1
<li>Item 2
<li>Item 3
</ul>
<img src="test.jpg">
</body>
</html>"#,
        );
        let html = doc.root_element().unwrap();
        assert_eq!(doc.node_name(html), Some("html"));
    }

    // --- Self-closing syntax ---

    #[test]
    fn test_self_closing_syntax() {
        let doc = parse_no_implied("<br/>");
        let root = doc.root();
        let br = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(br), Some("br"));
        assert!(doc.first_child(br).is_none());
    }

    // --- Entity in attribute ---

    #[test]
    fn test_entity_in_attribute() {
        let doc = parse_no_implied("<a href=\"page?a=1&amp;b=2\">link</a>");
        let root = doc.root();
        let a = doc.first_child(root).unwrap();
        assert_eq!(doc.attribute(a, "href"), Some("page?a=1&b=2"));
    }
}
