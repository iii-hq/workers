//! `XPath` 1.0 query language implementation.
//!
//! This module provides an implementation of the `XPath` 1.0 specification
//! (<https://www.w3.org/TR/xpath-10/>), including expression parsing and
//! evaluation against an XML document tree.
//!
//! # Quick Start
//!
//! ```
//! use xmloxide::Document;
//! use xmloxide::xpath::{evaluate, XPathValue};
//!
//! let doc = Document::parse_str("<root><a>1</a><b>2</b></root>").unwrap();
//! let root = doc.root_element().unwrap();
//! let result = evaluate(&doc, root, "count(*)").unwrap();
//! assert_eq!(result.to_number(), 2.0);
//! ```
//!
//! # Known Limitations
//!
//! - The `namespace::` axis returns the element's `NodeId` when in-scope
//!   namespaces match. Namespace nodes are not materialized as separate
//!   nodes (attribute nodes are — see [`types::XPathNode`]).
//!
//! # Submodules
//!
//! - [`ast`]: Abstract syntax tree types for parsed `XPath` expressions.
//! - [`lexer`]: Tokenizer for `XPath` expression strings.
//! - [`types`]: `XPath` value types and comparison helpers.
//! - [`parser`]: Recursive descent parser for `XPath` expressions.
//! - [`eval`]: Expression evaluator against a document tree.

pub mod ast;
pub mod eval;
pub mod lexer;
pub mod parser;
pub(crate) mod regex;
pub mod types;

pub use eval::XPathContext;
pub use types::{XPathError, XPathNode, XPathValue};

use crate::tree::{Document, NodeId};

/// Evaluates an `XPath` 1.0 expression against a document node.
///
/// This is a convenience function that parses the expression and evaluates it
/// in a single call. For evaluating the same expression against multiple
/// context nodes, use [`parser::parse`] and [`XPathContext::evaluate`]
/// separately to avoid re-parsing.
///
/// # Examples
///
/// ```
/// use xmloxide::Document;
/// use xmloxide::xpath::{evaluate, XPathValue};
///
/// let doc = Document::parse_str("<root><child>Hello</child></root>").unwrap();
/// let root = doc.root_element().unwrap();
///
/// // Count child elements
/// let result = evaluate(&doc, root, "count(*)").unwrap();
/// assert_eq!(result.to_number(), 1.0);
///
/// // Get text content
/// let result = evaluate(&doc, root, "string(child)").unwrap();
/// assert_eq!(result.to_xpath_string(), "Hello");
/// ```
///
/// # Errors
///
/// Returns [`XPathError`] if the expression is malformed or evaluation fails.
pub fn evaluate(
    doc: &Document,
    context_node: NodeId,
    expression: &str,
) -> Result<XPathValue, XPathError> {
    let expr = parser::parse(expression)?;
    let ctx = XPathContext::new(doc, context_node);
    ctx.evaluate(&expr)
}
