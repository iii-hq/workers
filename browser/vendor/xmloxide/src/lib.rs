//! # xmloxide
//!
//! A pure Rust reimplementation of libxml2 — the de facto standard XML/HTML
//! parsing library. Memory-safe, high-performance, and conformant with the
//! W3C XML 1.0 (Fifth Edition) specification.
//!
//! ## Modules
//!
//! - [`tree`] — DOM tree representation with arena-allocated nodes ([`Document`], [`NodeId`])
//! - [`parser`] — XML 1.0 parser with error recovery and push/incremental parsing
//! - [`html`] — Error-tolerant HTML 4.01 parser
//! - [`html5`] — WHATWG HTML5 parser (tokenizer + tree construction)
//! - [`html5::sax`] — Streaming SAX-like API for HTML5 (no DOM tree built)
//! - [`css`] — CSS selector engine for querying document trees
//! - [`sax`] — SAX2 event-driven streaming parser
//! - [`reader`] — `XmlReader` pull-based parsing API
//! - [`xpath`] — `XPath` 1.0+ expression evaluation (includes key `XPath` 2.0 functions)
//! - [`validation`] — DTD, `RelaxNG`, XML Schema (XSD), and ISO Schematron validation
//! - [`serial`] — XML/HTML serialization and Canonical XML (C14N)
//! - [`encoding`] — Character encoding detection and conversion
//! - [`xinclude`] — `XInclude` 1.0 document inclusion
//! - [`catalog`] — OASIS XML Catalogs for URI resolution
//! - [`error`] — Error types and diagnostics
//! - [`serde_xml`] — Serde XML (de)serialization (requires `serde` feature)
//! - [`async_xml`] — Async parsing via `tokio::io::AsyncRead` (requires `async` feature)
//!
//! ## Quick Start
//!
//! ```
//! use xmloxide::Document;
//!
//! let doc = Document::parse_str("<root><child>Hello</child></root>").unwrap();
//! let root = doc.root_element().unwrap();
//! assert_eq!(doc.node_name(root), Some("root"));
//! ```

#[cfg(feature = "async")]
pub mod async_xml;
pub mod catalog;
pub mod css;
pub mod encoding;
pub mod error;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod html;
pub mod html5;
pub mod parser;
pub mod reader;
pub mod sax;
#[cfg(feature = "serde")]
pub mod serde_xml;
pub mod serial;
pub mod tree;
#[allow(dead_code)]
pub(crate) mod util;
pub mod validation;
pub mod xinclude;
pub mod xpath;

// Re-export primary types at the crate root for convenience.
pub use tree::{Attribute, Document, NodeId};
