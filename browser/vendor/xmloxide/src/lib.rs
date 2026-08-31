//! # xmloxide
//!
//! A pure Rust reimplementation of libxml2 — the de facto standard XML/HTML
//! parsing library. Memory-safe, high-performance, and conformant with the
//! W3C XML 1.0 (Fifth Edition) specification.
//!
//! ## Modules
//!
//! This Scrapling-compatibility fork is trimmed to the slice the browser
//! worker consumes. Upstream's WHATWG html5 parser, CSS engine, SAX/reader
//! streaming APIs, RelaxNG/XSD/Schematron validators, XInclude, catalogs,
//! serde integration, async parsing, FFI layer, and xmllint CLI are removed;
//! restore them from upstream xmloxide if ever needed.
//!
//! - [`tree`] — DOM tree representation with arena-allocated nodes ([`Document`], [`NodeId`])
//! - [`parser`] — XML 1.0 parser with error recovery and push/incremental parsing
//! - [`html`] — Error-tolerant HTML 4.01 parser
//! - [`xpath`] — `XPath` 1.0+ expression evaluation (includes key `XPath` 2.0 functions)
//! - [`validation`] — DTD processing (`validation::dtd`; the parser depends on it)
//! - [`serial`] — XML/HTML serialization and Canonical XML (C14N)
//! - [`encoding`] — Character encoding detection and conversion
//! - [`error`] — Error types and diagnostics
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

pub mod encoding;
pub mod error;
pub mod html;
pub mod parser;
pub mod serial;
pub mod tree;
#[allow(dead_code)]
pub(crate) mod util;
pub mod validation;
pub mod xpath;

// Re-export primary types at the crate root for convenience.
pub use tree::{Attribute, Document, NodeId};
