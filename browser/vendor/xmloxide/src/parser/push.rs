//! Push/incremental XML parser.
//!
//! Provides a chunk-oriented parsing interface inspired by libxml2's push parser
//! (`xmlCreatePushParserCtxt` / `xmlParseChunk`). Data can be fed to the parser
//! in arbitrarily sized chunks via [`PushParser::push`], and the final document
//! is obtained by calling [`PushParser::finish`].
//!
//! This is useful for scenarios where XML data arrives incrementally, such as
//! reading from a network socket or streaming from another process.
//!
//! # Design
//!
//! The current implementation buffers all pushed data internally and performs
//! the full parse on [`PushParser::finish`]. This provides correct chunk-boundary
//! handling with minimal complexity. A future optimization may parse eagerly
//! after each [`PushParser::push`] call.
//!
//! # Examples
//!
//! ```
//! use xmloxide::parser::PushParser;
//!
//! let mut parser = PushParser::new();
//! parser.push(b"<root>");
//! parser.push(b"<child>Hello</child>");
//! parser.push(b"</root>");
//!
//! let doc = parser.finish().unwrap();
//! let root = doc.root_element().unwrap();
//! assert_eq!(doc.node_name(root), Some("root"));
//! ```

use crate::encoding::decode_to_utf8;
use crate::error::{ParseError, SourceLocation};
use crate::parser::ParseOptions;
use crate::tree::Document;

/// A push-based (incremental) XML parser.
///
/// Accepts XML data in arbitrarily sized chunks and builds a [`Document`] tree
/// when parsing is finalized. This mirrors libxml2's push parser interface
/// (`xmlCreatePushParserCtxt` / `xmlParseChunk`).
///
/// # Construction
///
/// Use [`PushParser::new`] for default options, or [`PushParser::with_options`]
/// to configure parser behavior.
///
/// # Examples
///
/// Basic usage with multiple chunks:
///
/// ```
/// use xmloxide::parser::PushParser;
///
/// let mut parser = PushParser::new();
/// parser.push(b"<?xml version=\"1.0\"?>");
/// parser.push(b"<root attr=\"value\">");
/// parser.push(b"Hello, world!");
/// parser.push(b"</root>");
///
/// let doc = parser.finish().unwrap();
/// let root = doc.root_element().unwrap();
/// assert_eq!(doc.node_name(root), Some("root"));
/// assert_eq!(doc.text_content(root), "Hello, world!");
/// ```
///
/// With parse options:
///
/// ```
/// use xmloxide::parser::{ParseOptions, PushParser};
///
/// let opts = ParseOptions::default().recover(true);
/// let mut parser = PushParser::with_options(opts);
/// parser.push(b"<root>");
/// parser.push(b"</root>");
///
/// let doc = parser.finish().unwrap();
/// assert!(doc.root_element().is_some());
/// ```
pub struct PushParser {
    /// Accumulated raw bytes from all `push()` calls.
    buffer: Vec<u8>,
    /// Parser options.
    options: ParseOptions,
    /// Whether `finish()` has already been called.
    finished: bool,
}

impl PushParser {
    /// Creates a new push parser with default options.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::parser::PushParser;
    ///
    /// let mut parser = PushParser::new();
    /// parser.push(b"<root/>");
    /// let doc = parser.finish().unwrap();
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            options: ParseOptions::default(),
            finished: false,
        }
    }

    /// Creates a new push parser with the specified options.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::parser::{ParseOptions, PushParser};
    ///
    /// let parser = PushParser::with_options(
    ///     ParseOptions::default().recover(true).no_blanks(true),
    /// );
    /// ```
    #[must_use]
    pub fn with_options(options: ParseOptions) -> Self {
        Self {
            buffer: Vec::new(),
            options,
            finished: false,
        }
    }

    /// Feeds a chunk of raw XML bytes into the parser.
    ///
    /// Data is accumulated in an internal buffer. The chunk can be any size
    /// and may split tokens, elements, or even multi-byte characters at
    /// arbitrary boundaries.
    ///
    /// # Panics
    ///
    /// Panics if called after [`finish`](PushParser::finish) has been invoked.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::parser::PushParser;
    ///
    /// let mut parser = PushParser::new();
    /// parser.push(b"<ro");
    /// parser.push(b"ot/>");
    /// let doc = parser.finish().unwrap();
    /// ```
    pub fn push(&mut self, data: &[u8]) {
        assert!(
            !self.finished,
            "push() called after finish() — parser has already been consumed"
        );
        self.buffer.extend_from_slice(data);
    }

    /// Finalizes parsing and returns the constructed [`Document`].
    ///
    /// This consumes the parser. All buffered data is decoded (with automatic
    /// encoding detection) and parsed as a complete XML document.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] if the accumulated data is not well-formed XML
    /// (unless recovery mode is enabled via [`ParseOptions::recover`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::parser::PushParser;
    ///
    /// let mut parser = PushParser::new();
    /// parser.push(b"<root><child/></root>");
    /// let doc = parser.finish().unwrap();
    /// ```
    pub fn finish(mut self) -> Result<Document, ParseError> {
        self.finished = true;

        let utf8 = decode_to_utf8(&self.buffer).map_err(|e| ParseError {
            message: e.message,
            location: SourceLocation::default(),
            diagnostics: Vec::new(),
        })?;

        crate::parser::parse_str_with_options(&utf8, &self.options)
    }

    /// Returns the number of bytes currently buffered.
    ///
    /// This is the total number of bytes received via [`push`](PushParser::push)
    /// that have not yet been parsed (parsing occurs on [`finish`](PushParser::finish)).
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::parser::PushParser;
    ///
    /// let mut parser = PushParser::new();
    /// assert_eq!(parser.buffered_bytes(), 0);
    /// parser.push(b"<root/>");
    /// assert_eq!(parser.buffered_bytes(), 7);
    /// ```
    #[must_use]
    pub fn buffered_bytes(&self) -> usize {
        self.buffer.len()
    }

    /// Returns `true` if no data has been pushed yet.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::parser::PushParser;
    ///
    /// let mut parser = PushParser::new();
    /// assert!(parser.is_empty());
    /// parser.push(b"<root/>");
    /// assert!(!parser.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Resets the parser, discarding all buffered data.
    ///
    /// After calling this method, the parser is in the same state as a
    /// newly created one (with the same options). This allows reusing
    /// the parser for a new document without allocating a new instance.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::parser::PushParser;
    ///
    /// let mut parser = PushParser::new();
    /// parser.push(b"<root/>");
    /// parser.reset();
    /// assert!(parser.is_empty());
    /// parser.push(b"<other/>");
    /// let doc = parser.finish().unwrap();
    /// ```
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.finished = false;
    }
}

impl Default for PushParser {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for PushParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushParser")
            .field("buffered_bytes", &self.buffer.len())
            .field("options", &self.options)
            .field("finished", &self.finished)
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_push_parser_single_chunk() {
        let mut parser = PushParser::new();
        parser.push(b"<root/>");
        let doc = parser.finish().unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
    }

    #[test]
    fn test_push_parser_multiple_chunks() {
        let mut parser = PushParser::new();
        parser.push(b"<root>");
        parser.push(b"<child>text</child>");
        parser.push(b"</root>");
        let doc = parser.finish().unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));

        let child = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(child), Some("child"));
        assert_eq!(doc.text_content(child), "text");
    }

    #[test]
    fn test_push_parser_split_token() {
        // Split an element tag across chunk boundaries.
        let mut parser = PushParser::new();
        parser.push(b"<ro");
        parser.push(b"ot att");
        parser.push(b"r=\"val");
        parser.push(b"ue\"/>");
        let doc = parser.finish().unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
        assert_eq!(doc.attribute(root, "attr"), Some("value"));
    }

    #[test]
    fn test_push_parser_byte_at_a_time() {
        let xml = b"<root><child/></root>";
        let mut parser = PushParser::new();
        for &byte in xml {
            parser.push(&[byte]);
        }
        let doc = parser.finish().unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
        let child = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(child), Some("child"));
    }

    #[test]
    fn test_push_parser_xml_declaration_split() {
        let mut parser = PushParser::new();
        parser.push(b"<?xml ver");
        parser.push(b"sion=\"1.0\" encoding=\"UTF-8\"?>");
        parser.push(b"<root/>");
        let doc = parser.finish().unwrap();
        assert_eq!(doc.version.as_deref(), Some("1.0"));
        assert_eq!(doc.encoding.as_deref(), Some("UTF-8"));
    }

    #[test]
    fn test_push_parser_empty_input() {
        let parser = PushParser::new();
        let result = parser.finish();
        // XML 1.0 §2.1 requires a root element
        assert!(result.is_err());
    }

    #[test]
    fn test_push_parser_with_options_recover() {
        let opts = ParseOptions::default().recover(true);
        let mut parser = PushParser::with_options(opts);
        // Mismatched tags — should succeed in recovery mode.
        parser.push(b"<a></b>");
        let result = parser.finish();
        assert!(result.is_ok());
    }

    #[test]
    fn test_push_parser_with_options_no_blanks() {
        let opts = ParseOptions::default().no_blanks(true);
        let mut parser = PushParser::with_options(opts);
        parser.push(b"<root>  <child/>  </root>");
        let doc = parser.finish().unwrap();
        let root = doc.root_element().unwrap();
        // The blank text nodes ("  ") should have been stripped.
        let children: Vec<_> = doc.children(root).collect();
        assert_eq!(children.len(), 1);
        assert_eq!(doc.node_name(children[0]), Some("child"));
    }

    #[test]
    fn test_push_parser_error_malformed() {
        let mut parser = PushParser::new();
        parser.push(b"<a></b>");
        let result = parser.finish();
        assert!(result.is_err());
    }

    #[test]
    fn test_push_parser_buffered_bytes() {
        let mut parser = PushParser::new();
        assert_eq!(parser.buffered_bytes(), 0);
        parser.push(b"<root>");
        assert_eq!(parser.buffered_bytes(), 6);
        parser.push(b"</root>");
        assert_eq!(parser.buffered_bytes(), 13);
    }

    #[test]
    fn test_push_parser_is_empty() {
        let mut parser = PushParser::new();
        assert!(parser.is_empty());
        parser.push(b"<root/>");
        assert!(!parser.is_empty());
    }

    #[test]
    fn test_push_parser_reset() {
        let mut parser = PushParser::new();
        parser.push(b"<invalid");
        parser.reset();
        assert!(parser.is_empty());
        assert_eq!(parser.buffered_bytes(), 0);
        parser.push(b"<root/>");
        let doc = parser.finish().unwrap();
        assert!(doc.root_element().is_some());
    }

    #[test]
    fn test_push_parser_default_trait() {
        let parser = PushParser::default();
        assert!(parser.is_empty());
    }

    #[test]
    fn test_push_parser_debug_trait() {
        let mut parser = PushParser::new();
        parser.push(b"<root/>");
        let debug_str = format!("{parser:?}");
        assert!(debug_str.contains("PushParser"));
        assert!(debug_str.contains("buffered_bytes: 7"));
    }

    #[test]
    fn test_push_parser_utf8_bom() {
        let mut parser = PushParser::new();
        // Push UTF-8 BOM followed by XML.
        parser.push(b"\xEF\xBB\xBF");
        parser.push(b"<root/>");
        let doc = parser.finish().unwrap();
        assert!(doc.root_element().is_some());
    }

    #[test]
    fn test_push_parser_comment_split() {
        let mut parser = PushParser::new();
        parser.push(b"<root><!-");
        parser.push(b"- comment -");
        parser.push(b"-></root>");
        let doc = parser.finish().unwrap();
        let root = doc.root_element().unwrap();
        let child = doc.first_child(root).unwrap();
        assert_eq!(doc.node_text(child), Some(" comment "));
    }

    #[test]
    fn test_push_parser_cdata_split() {
        let mut parser = PushParser::new();
        parser.push(b"<root><![CDA");
        parser.push(b"TA[some data]]");
        parser.push(b"></root>");
        let doc = parser.finish().unwrap();
        let root = doc.root_element().unwrap();
        let child = doc.first_child(root).unwrap();
        assert_eq!(doc.node_text(child), Some("some data"));
    }

    #[test]
    fn test_push_parser_entity_references() {
        let mut parser = PushParser::new();
        parser.push(b"<root>&am");
        parser.push(b"p; &lt; &gt;</root>");
        let doc = parser.finish().unwrap();
        let root = doc.root_element().unwrap();
        assert_eq!(doc.text_content(root), "& < >");
    }

    #[test]
    fn test_push_parser_roundtrip() {
        let input = b"<root><child attr=\"val\">text</child></root>";
        let mut parser = PushParser::new();
        parser.push(&input[..10]);
        parser.push(&input[10..25]);
        parser.push(&input[25..]);
        let doc = parser.finish().unwrap();
        let output = crate::serial::serialize(&doc);
        let expected = format!(
            "<?xml version=\"1.0\"?>\n{}\n",
            std::str::from_utf8(input).unwrap()
        );
        assert_eq!(output, expected);
    }

    #[test]
    #[should_panic(expected = "push() called after finish()")]
    fn test_push_parser_push_after_finish_panics() {
        let mut parser = PushParser::new();
        parser.push(b"<root/>");
        // Simulate calling push after finish by using an unsafe trick.
        // Actually we need to keep a reference — but finish() consumes self.
        // The assertion in push() uses self.finished, so we test indirectly.
        //
        // We cannot directly test this because `finish()` takes self by value.
        // Instead, test the assert fires when finished is set.
        parser.finished = true;
        parser.push(b"more data");
    }

    #[test]
    fn test_push_parser_large_document() {
        let mut parser = PushParser::new();
        parser.push(b"<root>");
        for i in 0..100 {
            let chunk = format!("<item id=\"{i}\">value {i}</item>");
            parser.push(chunk.as_bytes());
        }
        parser.push(b"</root>");

        let doc = parser.finish().unwrap();
        let root = doc.root_element().unwrap();
        let children: Vec<_> = doc.children(root).collect();
        assert_eq!(children.len(), 100);
    }
}
