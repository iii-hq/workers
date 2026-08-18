//! SAX2 streaming event handler API.
//!
//! SAX (Simple API for XML) is a streaming, event-driven API for processing
//! XML. Instead of building a tree in memory, the parser fires callbacks as it
//! encounters elements, text, comments, and other XML constructs.
//!
//! This is useful for large documents where building a full tree would be
//! wasteful, or when you only need to extract specific data.
//!
//! # Examples
//!
//! ```
//! use xmloxide::sax::{SaxHandler, parse_sax, DefaultHandler};
//! use xmloxide::parser::ParseOptions;
//!
//! struct MyHandler {
//!     element_count: usize,
//! }
//!
//! impl SaxHandler for MyHandler {
//!     fn start_element(
//!         &mut self,
//!         local_name: &str,
//!         _prefix: Option<&str>,
//!         _namespace: Option<&str>,
//!         _attributes: &[(String, String, Option<String>, Option<String>)],
//!     ) {
//!         self.element_count += 1;
//!     }
//! }
//!
//! let mut handler = MyHandler { element_count: 0 };
//! parse_sax("<root><a/><b/><c/></root>", &ParseOptions::default(), &mut handler).unwrap();
//! assert_eq!(handler.element_count, 4);
//! ```

use crate::error::{ErrorSeverity, ParseError, SourceLocation};
use crate::parser::input::{
    parse_cdata_content, parse_comment_content, parse_pi_content, parse_xml_decl, split_name,
    NamespaceResolver, ParserInput,
};
use crate::parser::ParseOptions;

/// A SAX2 event handler trait.
///
/// Implement the callbacks you care about; all methods have default no-op
/// implementations so you only need to override what you need.
///
/// # Attribute tuples
///
/// Attributes are passed as `(local_name, value, prefix, namespace_uri)` tuples.
#[allow(unused_variables)]
pub trait SaxHandler {
    /// Called at the start of the document, before any other events.
    fn start_document(&mut self) {}

    /// Called at the end of the document, after all other events.
    fn end_document(&mut self) {}

    /// Called when an element start tag is encountered.
    ///
    /// `attributes` contains `(local_name, value, prefix, namespace_uri)` tuples.
    fn start_element(
        &mut self,
        local_name: &str,
        prefix: Option<&str>,
        namespace: Option<&str>,
        attributes: &[(String, String, Option<String>, Option<String>)],
    ) {
    }

    /// Called when an element end tag is encountered (or a self-closing tag ends).
    fn end_element(&mut self, local_name: &str, prefix: Option<&str>, namespace: Option<&str>) {}

    /// Called for character data (text content).
    fn characters(&mut self, content: &str) {}

    /// Called for CDATA sections.
    fn cdata(&mut self, content: &str) {}

    /// Called for XML comments.
    fn comment(&mut self, content: &str) {}

    /// Called for processing instructions.
    fn processing_instruction(&mut self, target: &str, data: Option<&str>) {}

    /// Called when a warning is encountered during parsing.
    fn warning(&mut self, message: &str, location: SourceLocation) {}

    /// Called when a recoverable error is encountered during parsing.
    fn error(&mut self, message: &str, location: SourceLocation) {}
}

/// A default no-op SAX handler. Useful as a base or for testing.
pub struct DefaultHandler;

impl SaxHandler for DefaultHandler {}

/// Parses XML from a string, firing SAX events on the provided handler.
///
/// # Errors
///
/// Returns `ParseError` if the input is not well-formed XML and recovery
/// mode is not enabled.
///
/// # Examples
///
/// ```
/// use xmloxide::sax::{parse_sax, DefaultHandler};
/// use xmloxide::parser::ParseOptions;
///
/// let mut handler = DefaultHandler;
/// parse_sax("<root/>", &ParseOptions::default(), &mut handler).unwrap();
/// ```
pub fn parse_sax(
    input: &str,
    options: &ParseOptions,
    handler: &mut dyn SaxHandler,
) -> Result<(), ParseError> {
    let mut parser = SaxParser::new(input, options, handler);
    let result = parser.parse();

    // Transfer diagnostics from the shared input into any error that is
    // returned, so callers see the full diagnostic trail.
    if let Err(ref _e) = result {
        // The error already contains diagnostics from ParserInput::fatal().
    }

    result
}

/// The SAX-driven XML parser.
///
/// Reuses the same parsing logic as the tree-building parser but fires
/// SAX events instead of constructing nodes.
struct SaxParser<'a, 'h> {
    /// Shared low-level input state (position, peek, advance, name parsing, etc.).
    input: ParserInput<'a>,
    /// Parser options.
    options: ParseOptions,
    /// SAX event handler.
    handler: &'h mut dyn SaxHandler,
    /// Namespace resolver managing the scope stack.
    ns: NamespaceResolver,
}

impl<'a, 'h> SaxParser<'a, 'h> {
    fn new(input: &'a str, options: &ParseOptions, handler: &'h mut dyn SaxHandler) -> Self {
        let mut pi = ParserInput::new(input);
        pi.set_recover(options.recover);
        pi.set_max_depth(options.max_depth);
        pi.set_max_name_length(options.max_name_length);
        pi.set_max_entity_expansions(options.max_entity_expansions);
        pi.set_entity_resolver(options.entity_resolver.clone());

        Self {
            input: pi,
            options: options.clone(),
            handler,
            ns: NamespaceResolver::new(),
        }
    }

    fn parse(&mut self) -> Result<(), ParseError> {
        self.handler.start_document();

        // Parse optional XML declaration
        self.input.skip_whitespace();
        if self.input.looking_at(b"<?xml ")
            || self.input.looking_at(b"<?xml\t")
            || self.input.looking_at(b"<?xml\r")
        {
            self.parse_xml_declaration()?;
        }

        // Parse prolog misc
        self.parse_misc()?;

        // Parse optional DOCTYPE
        if self.input.looking_at(b"<!DOCTYPE") || self.input.looking_at(b"<!doctype") {
            self.skip_doctype()?;
            self.parse_misc()?;
        }

        // Parse root element
        if self.input.peek() == Some(b'<')
            && self
                .input
                .peek_at(1)
                .is_some_and(|b| b != b'!' && b != b'?')
        {
            self.parse_element()?;
        }

        // Parse trailing misc
        self.parse_misc()?;

        self.input.skip_whitespace();
        if !self.input.at_end() && !self.options.recover {
            return Err(self.input.fatal("content after document element"));
        }

        self.handler.end_document();
        Ok(())
    }

    // --- XML Declaration ---
    // See XML 1.0 §2.8: [23] XMLDecl

    fn parse_xml_declaration(&mut self) -> Result<(), ParseError> {
        // Delegate to the shared XML declaration parser; we discard the
        // parsed values because the SAX API does not expose them.
        let _decl = parse_xml_decl(&mut self.input)?;
        Ok(())
    }

    // --- Misc (comments, PIs, whitespace) ---

    fn parse_misc(&mut self) -> Result<(), ParseError> {
        loop {
            self.input.skip_whitespace();
            if self.input.at_end() {
                break;
            }
            if self.input.looking_at(b"<!--") {
                self.parse_comment()?;
            } else if self.input.looking_at(b"<?") {
                self.parse_processing_instruction()?;
            } else {
                break;
            }
        }
        Ok(())
    }

    // --- DOCTYPE Declaration ---
    // See XML 1.0 §2.8: [28] doctypedecl

    fn skip_doctype(&mut self) -> Result<(), ParseError> {
        self.input.expect_str(b"<!DOCTYPE")?;
        self.input.skip_whitespace_required()?;
        self.input.parse_name()?;
        self.input.skip_whitespace();

        if self.input.looking_at(b"SYSTEM") {
            self.input.expect_str(b"SYSTEM")?;
            self.input.skip_whitespace_required()?;
            self.input.parse_quoted_value()?;
            self.input.skip_whitespace();
        } else if self.input.looking_at(b"PUBLIC") {
            self.input.expect_str(b"PUBLIC")?;
            self.input.skip_whitespace_required()?;
            self.input.parse_quoted_value()?;
            self.input.skip_whitespace_required()?;
            self.input.parse_quoted_value()?;
            self.input.skip_whitespace();
        }

        if self.input.peek() == Some(b'[') {
            self.input.advance(1);
            let start = self.input.pos();
            let mut depth: u32 = 1;
            while !self.input.at_end() && depth > 0 {
                if self.input.looking_at(b"<!--") {
                    self.input.advance(4);
                    while !self.input.at_end() && !self.input.looking_at(b"-->") {
                        self.input.advance(1);
                    }
                    if !self.input.at_end() {
                        self.input.advance(3);
                    }
                } else if let Some(b'"' | b'\'') = self.input.peek() {
                    let quote = self.input.peek().unwrap_or(b'"');
                    self.input.advance(1);
                    while !self.input.at_end() && self.input.peek() != Some(quote) {
                        self.input.advance(1);
                    }
                    if !self.input.at_end() {
                        self.input.advance(1);
                    }
                } else if self.input.peek() == Some(b'[') {
                    depth += 1;
                    self.input.advance(1);
                } else if self.input.peek() == Some(b']') {
                    depth -= 1;
                    self.input.advance(1);
                } else {
                    self.input.advance(1);
                }
            }

            // Parse DTD internal subset for entity declarations
            let end = self.input.pos() - 1;
            let subset_text = std::str::from_utf8(self.input.slice(start, end))
                .ok()
                .map(str::to_string);
            if let Some(subset_text) = subset_text {
                if subset_text.contains('%') {
                    self.input.has_pe_references = true;
                }
                if let Ok(dtd) = crate::validation::dtd::parse_dtd(&subset_text) {
                    for (ent_name, ent_decl) in &dtd.entities {
                        match &ent_decl.kind {
                            crate::validation::dtd::EntityKind::Internal(value) => {
                                self.input
                                    .entity_map
                                    .insert(ent_name.clone(), value.clone());
                            }
                            crate::validation::dtd::EntityKind::External {
                                system_id,
                                public_id,
                            } => {
                                self.input.entity_external.insert(
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

            self.input.skip_whitespace();
        }

        self.input.expect_byte(b'>')?;
        Ok(())
    }

    // --- Elements ---
    // See XML 1.0 §3.1: [40] STag, [42] ETag, [44] EmptyElemTag

    fn parse_element(&mut self) -> Result<(), ParseError> {
        self.input.increment_depth()?;
        self.input.expect_byte(b'<')?;
        let name = self.input.parse_name()?;

        // Parse attributes as (full_name, value) pairs first
        let mut raw_attrs: Vec<(String, String)> = Vec::new();
        loop {
            let had_ws = self.input.skip_whitespace();
            if self.input.peek() == Some(b'>') || self.input.looking_at(b"/>") {
                break;
            }
            if !had_ws {
                return Err(self.input.fatal("whitespace required between attributes"));
            }
            let attr_name = self.input.parse_name()?;
            self.input.skip_whitespace();
            self.input.expect_byte(b'=')?;
            self.input.skip_whitespace();
            let attr_value = self.input.parse_attribute_value()?;
            raw_attrs.push((attr_name, attr_value));
        }

        // Namespace processing: push scope and bind declarations.
        self.ns.push_scope();
        for (attr_name, attr_value) in &raw_attrs {
            if attr_name == "xmlns" {
                self.ns.bind(None, attr_value.clone());
            } else if let Some(prefix) = attr_name.strip_prefix("xmlns:") {
                self.ns.bind(Some(prefix.to_string()), attr_value.clone());
            }
        }

        // Resolve element namespace
        let (prefix, local_name) = split_name(&name);
        let elem_ns = self.ns.resolve(prefix).map(String::from);

        // Build attribute tuples: (local_name, value, prefix, namespace)
        let attributes: Vec<(String, String, Option<String>, Option<String>)> = raw_attrs
            .iter()
            .map(|(attr_name, attr_value)| {
                let (attr_prefix, attr_local) = split_name(attr_name);
                let attr_ns = if attr_prefix == Some("xmlns")
                    || (attr_prefix.is_none() && attr_local == "xmlns")
                {
                    None
                } else {
                    attr_prefix.and_then(|p| self.ns.resolve(Some(p)).map(String::from))
                };
                (
                    attr_local.to_string(),
                    attr_value.clone(),
                    attr_prefix.map(String::from),
                    attr_ns,
                )
            })
            .collect();

        // Fire start_element
        self.handler
            .start_element(local_name, prefix, elem_ns.as_deref(), &attributes);

        let is_empty = self.input.looking_at(b"/>");
        if is_empty {
            self.input.advance(2);
        } else {
            self.input.expect_byte(b'>')?;
            self.parse_content()?;
            self.input.expect_str(b"</")?;
            let end_name = self.input.parse_name()?;
            if end_name != name {
                if self.options.recover {
                    self.input.push_diagnostic(
                        crate::error::ErrorSeverity::Error,
                        format!("mismatched end tag: expected </{name}>, found </{end_name}>"),
                    );
                } else {
                    return Err(self.input.fatal(format!(
                        "mismatched end tag: expected </{name}>, found </{end_name}>"
                    )));
                }
            }
            self.input.skip_whitespace();
            self.input.expect_byte(b'>')?;
        }

        // Fire end_element
        self.handler
            .end_element(local_name, prefix, elem_ns.as_deref());

        self.ns.pop_scope();
        self.input.decrement_depth();
        Ok(())
    }

    // --- Content ---
    // See XML 1.0 §3.1: [43] content

    fn parse_content(&mut self) -> Result<(), ParseError> {
        loop {
            if self.input.at_end() {
                if self.options.recover {
                    break;
                }
                return Err(self
                    .input
                    .fatal("unexpected end of input in element content"));
            }
            if self.input.looking_at(b"</") {
                break;
            }
            if self.input.looking_at(b"<![CDATA[") {
                self.parse_cdata()?;
            } else if self.input.looking_at(b"<!--") {
                self.parse_comment()?;
            } else if self.input.looking_at(b"<?") {
                self.parse_processing_instruction()?;
            } else if self.input.peek() == Some(b'<') {
                self.parse_element()?;
            } else {
                self.parse_char_data()?;
            }
        }
        Ok(())
    }

    // --- Character Data ---
    // See XML 1.0 §2.4: [14] CharData

    fn parse_char_data(&mut self) -> Result<(), ParseError> {
        let mut text = String::new();
        while !self.input.at_end() {
            // Bulk scan: find the next `<`, `&`, or `]]>` boundary
            let safe_len = self.input.scan_char_data();
            if safe_len > 0 {
                let start = self.input.pos();
                let chunk_bytes = self.input.slice(start, start + safe_len);
                if chunk_bytes.contains(&b'\r') {
                    let chunk = std::str::from_utf8(chunk_bytes)
                        .map_err(|_| self.input.fatal("invalid UTF-8 in character data"))?;
                    let mut chars = chunk.chars().peekable();
                    while let Some(ch) = chars.next() {
                        if ch == '\r' {
                            if chars.peek() == Some(&'\n') {
                                chars.next();
                            }
                            text.push('\n');
                        } else {
                            text.push(ch);
                        }
                    }
                } else {
                    let chunk = std::str::from_utf8(chunk_bytes)
                        .map_err(|_| self.input.fatal("invalid UTF-8 in character data"))?;
                    text.push_str(chunk);
                }
                self.input.advance_counting_lines(safe_len);
                continue;
            }

            if self.input.peek() == Some(b'<') {
                break;
            }

            // XML 1.0 §2.4: "]]>" is forbidden in character data
            if self.input.looking_at(b"]]>") {
                if self.options.recover {
                    self.input.push_diagnostic(
                        ErrorSeverity::Error,
                        "']]>' not allowed in character data".to_string(),
                    );
                    text.push_str("]]>");
                    self.input.advance(3);
                    continue;
                }
                return Err(self.input.fatal("']]>' not allowed in character data"));
            }

            if self.input.peek() == Some(b'&') {
                self.input.parse_reference_into(&mut text)?;
            } else {
                let ch = self.input.next_char()?;
                text.push(ch);
            }
        }
        if !text.is_empty() {
            if self.options.no_blanks && text.chars().all(char::is_whitespace) {
                return Ok(());
            }
            self.handler.characters(&text);
        }
        Ok(())
    }

    // --- Comments ---
    // See XML 1.0 §2.5: [15] Comment

    fn parse_comment(&mut self) -> Result<(), ParseError> {
        let content = parse_comment_content(&mut self.input)?;
        self.handler.comment(&content);
        Ok(())
    }

    // --- CDATA Sections ---
    // See XML 1.0 §2.7: [18] CDSect

    fn parse_cdata(&mut self) -> Result<(), ParseError> {
        let content = parse_cdata_content(&mut self.input)?;
        self.handler.cdata(&content);
        Ok(())
    }

    // --- Processing Instructions ---
    // See XML 1.0 §2.6: [16] PI

    fn parse_processing_instruction(&mut self) -> Result<(), ParseError> {
        let (target, data) = parse_pi_content(&mut self.input)?;
        self.handler
            .processing_instruction(&target, data.as_deref());
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // --- Test handler that records events ---

    #[derive(Debug, Default)]
    struct RecordingHandler {
        events: Vec<String>,
    }

    impl SaxHandler for RecordingHandler {
        fn start_document(&mut self) {
            self.events.push("start_document".to_string());
        }

        fn end_document(&mut self) {
            self.events.push("end_document".to_string());
        }

        fn start_element(
            &mut self,
            local_name: &str,
            prefix: Option<&str>,
            namespace: Option<&str>,
            attributes: &[(String, String, Option<String>, Option<String>)],
        ) {
            use std::fmt::Write;
            let mut event = format!("start_element({local_name}");
            if let Some(pfx) = prefix {
                let _ = write!(event, ", prefix={pfx}");
            }
            if let Some(ns) = namespace {
                let _ = write!(event, ", ns={ns}");
            }
            for (name, value, _, _) in attributes {
                let _ = write!(event, ", {name}={value}");
            }
            event.push(')');
            self.events.push(event);
        }

        fn end_element(
            &mut self,
            local_name: &str,
            prefix: Option<&str>,
            _namespace: Option<&str>,
        ) {
            let event = match prefix {
                Some(pfx) => format!("end_element({pfx}:{local_name})"),
                None => format!("end_element({local_name})"),
            };
            self.events.push(event);
        }

        fn characters(&mut self, content: &str) {
            self.events.push(format!("characters({content})"));
        }

        fn cdata(&mut self, content: &str) {
            self.events.push(format!("cdata({content})"));
        }

        fn comment(&mut self, content: &str) {
            self.events.push(format!("comment({content})"));
        }

        fn processing_instruction(&mut self, target: &str, data: Option<&str>) {
            match data {
                Some(d) => self.events.push(format!("pi({target}, {d})")),
                None => self.events.push(format!("pi({target})")),
            }
        }

        fn warning(&mut self, message: &str, _location: SourceLocation) {
            self.events.push(format!("warning({message})"));
        }

        fn error(&mut self, message: &str, _location: SourceLocation) {
            self.events.push(format!("error({message})"));
        }
    }

    fn parse_events(input: &str) -> Vec<String> {
        let mut handler = RecordingHandler::default();
        parse_sax(input, &ParseOptions::default(), &mut handler).unwrap();
        handler.events
    }

    #[test]
    fn test_sax_empty_element() {
        let events = parse_events("<root/>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(root)",
                "end_element(root)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_element_with_text() {
        let events = parse_events("<root>Hello</root>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(root)",
                "characters(Hello)",
                "end_element(root)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_nested_elements() {
        let events = parse_events("<a><b>text</b></a>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(a)",
                "start_element(b)",
                "characters(text)",
                "end_element(b)",
                "end_element(a)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_attributes() {
        let events = parse_events("<root id=\"1\" class=\"big\"/>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(root, id=1, class=big)",
                "end_element(root)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_comment() {
        let events = parse_events("<root><!-- hello --></root>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(root)",
                "comment( hello )",
                "end_element(root)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_cdata() {
        let events = parse_events("<root><![CDATA[raw & data]]></root>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(root)",
                "cdata(raw & data)",
                "end_element(root)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_processing_instruction() {
        let events = parse_events("<?target data?><root/>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "pi(target, data)",
                "start_element(root)",
                "end_element(root)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_entity_references() {
        let events = parse_events("<root>&amp;&lt;&gt;</root>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(root)",
                "characters(&<>)",
                "end_element(root)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_mixed_content() {
        let events = parse_events("<p>Hello <b>world</b>!</p>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(p)",
                "characters(Hello )",
                "start_element(b)",
                "characters(world)",
                "end_element(b)",
                "characters(!)",
                "end_element(p)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_namespace() {
        let events = parse_events("<root xmlns=\"http://example.com\"><child/></root>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(root, ns=http://example.com, xmlns=http://example.com)",
                "start_element(child, ns=http://example.com)",
                "end_element(child)",
                "end_element(root)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_prefixed_namespace() {
        let events = parse_events("<ns:root xmlns:ns=\"http://example.com\"/>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(root, prefix=ns, ns=http://example.com, ns=http://example.com)",
                "end_element(ns:root)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_doctype() {
        // DOCTYPE should be silently skipped, not cause an error
        let events = parse_events("<!DOCTYPE html><html/>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(html)",
                "end_element(html)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_xml_declaration() {
        let events = parse_events("<?xml version=\"1.0\" encoding=\"UTF-8\"?><root/>");
        assert_eq!(
            events,
            vec![
                "start_document",
                "start_element(root)",
                "end_element(root)",
                "end_document",
            ]
        );
    }

    #[test]
    fn test_sax_element_count() {
        struct Counter {
            count: usize,
        }
        impl SaxHandler for Counter {
            fn start_element(
                &mut self,
                _local_name: &str,
                _prefix: Option<&str>,
                _namespace: Option<&str>,
                _attributes: &[(String, String, Option<String>, Option<String>)],
            ) {
                self.count += 1;
            }
        }

        let mut counter = Counter { count: 0 };
        parse_sax(
            "<root><a/><b><c/></b><d/></root>",
            &ParseOptions::default(),
            &mut counter,
        )
        .unwrap();
        assert_eq!(counter.count, 5);
    }

    #[test]
    fn test_sax_text_extraction() {
        struct TextCollector {
            text: String,
        }
        impl SaxHandler for TextCollector {
            fn characters(&mut self, content: &str) {
                self.text.push_str(content);
            }
        }

        let mut collector = TextCollector {
            text: String::new(),
        };
        parse_sax(
            "<root>Hello <b>world</b>!</root>",
            &ParseOptions::default(),
            &mut collector,
        )
        .unwrap();
        assert_eq!(collector.text, "Hello world!");
    }

    #[test]
    fn test_sax_default_handler() {
        // DefaultHandler should just work without panicking
        let mut handler = DefaultHandler;
        parse_sax(
            "<root><child/></root>",
            &ParseOptions::default(),
            &mut handler,
        )
        .unwrap();
    }

    #[test]
    fn test_sax_error_mismatched_tags() {
        let mut handler = DefaultHandler;
        let result = parse_sax("<a></b>", &ParseOptions::default(), &mut handler);
        assert!(result.is_err());
    }
}
