//! Streaming SAX-like API for HTML5 parsing.
//!
//! Wraps the WHATWG HTML5 tokenizer to fire callbacks for each token
//! without building a DOM tree in memory. This is useful for large HTML
//! documents where you only need to extract specific data.
//!
//! # Examples
//!
//! ```
//! use xmloxide::html5::sax::{Html5SaxHandler, parse_html5_sax};
//!
//! struct Counter { elements: usize }
//!
//! impl Html5SaxHandler for Counter {
//!     fn start_element(
//!         &mut self,
//!         name: &str,
//!         attributes: &[(String, String)],
//!         self_closing: bool,
//!     ) {
//!         self.elements += 1;
//!     }
//! }
//!
//! let mut handler = Counter { elements: 0 };
//! parse_html5_sax("<div><p>Hello</p></div>", &mut handler);
//! assert_eq!(handler.elements, 2);
//! ```

use super::tokenizer::{Token, Tokenizer, TokenizerError};

/// An event handler for streaming HTML5 parsing.
///
/// Implement the callbacks you care about; all methods have default no-op
/// implementations so you only need to override what you need.
///
/// # Attribute tuples
///
/// Attributes are passed as `(name, value)` tuples matching the HTML5
/// tokenizer's attribute representation.
#[allow(unused_variables)]
pub trait Html5SaxHandler {
    /// Called when a start tag is encountered.
    ///
    /// `attributes` contains `(name, value)` tuples.
    /// `self_closing` is true for self-closing tags like `<br/>`.
    fn start_element(&mut self, name: &str, attributes: &[(String, String)], self_closing: bool) {}

    /// Called when an end tag is encountered.
    fn end_element(&mut self, name: &str) {}

    /// Called for character data (text content).
    ///
    /// Note: the HTML5 tokenizer emits one character at a time; this API
    /// coalesces consecutive characters into a single callback for efficiency.
    fn characters(&mut self, content: &str) {}

    /// Called for HTML comments.
    fn comment(&mut self, content: &str) {}

    /// Called for DOCTYPE declarations.
    fn doctype(&mut self, name: Option<&str>, public_id: Option<&str>, system_id: Option<&str>) {}

    /// Called when a tokenizer error is encountered.
    fn error(&mut self, error: &TokenizerError) {}
}

/// A default no-op HTML5 SAX handler. Useful as a base or for testing.
pub struct DefaultHtml5Handler;

impl Html5SaxHandler for DefaultHtml5Handler {}

/// Parse HTML5 from a string, firing SAX events on the provided handler.
///
/// This drives the WHATWG HTML5 tokenizer and calls the appropriate handler
/// methods for each token. No DOM tree is built.
///
/// # Examples
///
/// ```
/// use xmloxide::html5::sax::{Html5SaxHandler, parse_html5_sax};
///
/// struct Links { hrefs: Vec<String> }
///
/// impl Html5SaxHandler for Links {
///     fn start_element(
///         &mut self,
///         name: &str,
///         attributes: &[(String, String)],
///         self_closing: bool,
///     ) {
///         if name == "a" {
///             if let Some((_, href)) = attributes.iter().find(|(n, _)| n == "href") {
///                 self.hrefs.push(href.clone());
///             }
///         }
///     }
/// }
///
/// let mut handler = Links { hrefs: Vec::new() };
/// parse_html5_sax(
///     r#"<a href="https://example.com">Link</a>"#,
///     &mut handler,
/// );
/// assert_eq!(handler.hrefs, vec!["https://example.com"]);
/// ```
pub fn parse_html5_sax(input: &str, handler: &mut dyn Html5SaxHandler) {
    let mut tokenizer = Tokenizer::new(input);
    let mut char_buf = String::new();

    loop {
        let token = tokenizer.next_token();
        match token {
            Token::Character(c) => {
                char_buf.push(c);
                continue;
            }
            _ => {
                // Flush any accumulated characters before handling the
                // non-character token.
                if !char_buf.is_empty() {
                    handler.characters(&char_buf);
                    char_buf.clear();
                }
            }
        }

        match token {
            Token::StartTag {
                ref name,
                ref attributes,
                self_closing,
            } => {
                let attrs: Vec<(String, String)> = attributes
                    .iter()
                    .map(|a| (a.name.clone(), a.value.clone()))
                    .collect();
                handler.start_element(name, &attrs, self_closing);
            }
            Token::EndTag { ref name } => {
                handler.end_element(name);
            }
            Token::Comment(ref text) => {
                handler.comment(text);
            }
            Token::Doctype {
                ref name,
                ref public_id,
                ref system_id,
                ..
            } => {
                handler.doctype(name.as_deref(), public_id.as_deref(), system_id.as_deref());
            }
            Token::Eof => break,
            Token::Character(_) => unreachable!(),
        }
    }

    // Report any tokenizer errors
    for error in tokenizer.errors() {
        handler.error(error);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_start_and_end_elements() {
        struct Recorder {
            events: Vec<String>,
        }
        impl Html5SaxHandler for Recorder {
            fn start_element(
                &mut self,
                name: &str,
                _attributes: &[(String, String)],
                _self_closing: bool,
            ) {
                self.events.push(format!("start:{name}"));
            }
            fn end_element(&mut self, name: &str) {
                self.events.push(format!("end:{name}"));
            }
        }

        let mut handler = Recorder { events: Vec::new() };
        parse_html5_sax("<div><p>text</p></div>", &mut handler);
        assert_eq!(
            handler.events,
            vec!["start:div", "start:p", "end:p", "end:div"]
        );
    }

    #[test]
    fn test_characters_coalesced() {
        struct TextCollector {
            texts: Vec<String>,
        }
        impl Html5SaxHandler for TextCollector {
            fn characters(&mut self, content: &str) {
                self.texts.push(content.to_string());
            }
        }

        let mut handler = TextCollector { texts: Vec::new() };
        parse_html5_sax("<p>Hello World</p>", &mut handler);
        // Should be a single coalesced text event, not one per character
        assert_eq!(handler.texts.len(), 1);
        assert_eq!(handler.texts[0], "Hello World");
    }

    #[test]
    fn test_attributes() {
        struct AttrCollector {
            attrs: Vec<Vec<(String, String)>>,
        }
        impl Html5SaxHandler for AttrCollector {
            fn start_element(
                &mut self,
                _name: &str,
                attributes: &[(String, String)],
                _self_closing: bool,
            ) {
                self.attrs.push(attributes.to_vec());
            }
        }

        let mut handler = AttrCollector { attrs: Vec::new() };
        parse_html5_sax(
            r#"<a href="http://example.com" class="link">x</a>"#,
            &mut handler,
        );
        assert_eq!(handler.attrs.len(), 1);
        assert_eq!(handler.attrs[0].len(), 2);
        assert_eq!(handler.attrs[0][0].0, "href");
        assert_eq!(handler.attrs[0][0].1, "http://example.com");
        assert_eq!(handler.attrs[0][1].0, "class");
        assert_eq!(handler.attrs[0][1].1, "link");
    }

    #[test]
    fn test_comment() {
        struct CommentCollector {
            comments: Vec<String>,
        }
        impl Html5SaxHandler for CommentCollector {
            fn comment(&mut self, content: &str) {
                self.comments.push(content.to_string());
            }
        }

        let mut handler = CommentCollector {
            comments: Vec::new(),
        };
        parse_html5_sax("<!-- hello --><p>text</p>", &mut handler);
        assert_eq!(handler.comments, vec![" hello "]);
    }

    #[test]
    fn test_doctype() {
        struct DoctypeCollector {
            name: Option<String>,
        }
        impl Html5SaxHandler for DoctypeCollector {
            fn doctype(
                &mut self,
                name: Option<&str>,
                _public_id: Option<&str>,
                _system_id: Option<&str>,
            ) {
                self.name = name.map(String::from);
            }
        }

        let mut handler = DoctypeCollector { name: None };
        parse_html5_sax("<!DOCTYPE html><html></html>", &mut handler);
        assert_eq!(handler.name, Some("html".to_string()));
    }

    #[test]
    fn test_self_closing() {
        struct SelfClosingChecker {
            self_closing_tags: Vec<String>,
        }
        impl Html5SaxHandler for SelfClosingChecker {
            fn start_element(
                &mut self,
                name: &str,
                _attributes: &[(String, String)],
                self_closing: bool,
            ) {
                if self_closing {
                    self.self_closing_tags.push(name.to_string());
                }
            }
        }

        let mut handler = SelfClosingChecker {
            self_closing_tags: Vec::new(),
        };
        parse_html5_sax("<br/><img/><p>text</p>", &mut handler);
        assert_eq!(handler.self_closing_tags, vec!["br", "img"]);
    }

    #[test]
    fn test_default_handler() {
        let mut handler = DefaultHtml5Handler;
        parse_html5_sax("<p>test</p>", &mut handler);
        // Should not panic — all callbacks are no-ops
    }

    #[test]
    fn test_error_reporting() {
        struct ErrorCounter {
            count: usize,
        }
        impl Html5SaxHandler for ErrorCounter {
            fn error(&mut self, _error: &TokenizerError) {
                self.count += 1;
            }
        }

        let mut handler = ErrorCounter { count: 0 };
        // EOF inside a tag triggers eof-in-tag error
        parse_html5_sax("<p><div attr=", &mut handler);
        assert!(handler.count >= 1);
    }

    #[test]
    fn test_element_counter() {
        struct Counter {
            elements: usize,
        }
        impl Html5SaxHandler for Counter {
            fn start_element(
                &mut self,
                _name: &str,
                _attributes: &[(String, String)],
                _self_closing: bool,
            ) {
                self.elements += 1;
            }
        }

        let mut handler = Counter { elements: 0 };
        parse_html5_sax(
            "<html><head><title>Test</title></head><body><p>Hello</p></body></html>",
            &mut handler,
        );
        assert_eq!(handler.elements, 5); // html, head, title, body, p
    }

    #[test]
    fn test_multiple_text_segments() {
        struct TextCollector {
            texts: Vec<String>,
        }
        impl Html5SaxHandler for TextCollector {
            fn characters(&mut self, content: &str) {
                self.texts.push(content.to_string());
            }
        }

        let mut handler = TextCollector { texts: Vec::new() };
        parse_html5_sax("<p>Hello</p><p>World</p>", &mut handler);
        assert_eq!(handler.texts, vec!["Hello", "World"]);
    }

    #[test]
    fn test_link_extractor() {
        struct LinkExtractor {
            hrefs: Vec<String>,
        }
        impl Html5SaxHandler for LinkExtractor {
            fn start_element(
                &mut self,
                name: &str,
                attributes: &[(String, String)],
                _self_closing: bool,
            ) {
                if name == "a" {
                    if let Some((_, href)) = attributes.iter().find(|(n, _)| n == "href") {
                        self.hrefs.push(href.clone());
                    }
                }
            }
        }

        let mut handler = LinkExtractor { hrefs: Vec::new() };
        parse_html5_sax(
            r#"<a href="/one">1</a><a href="/two">2</a><span>no link</span>"#,
            &mut handler,
        );
        assert_eq!(handler.hrefs, vec!["/one", "/two"]);
    }
}
