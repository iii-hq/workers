//! CSS selector engine for querying [`Document`] trees.
//!
//! Provides a familiar CSS selector API for finding elements in XML/HTML
//! documents, as an alternative to [`XPath`](crate::xpath).
//!
//! # Supported Selectors
//!
//! | Selector | Example | Description |
//! |----------|---------|-------------|
//! | Tag | `div` | Matches elements by tag name |
//! | Class | `.intro` | Matches elements with a class |
//! | ID | `#main` | Matches elements by id attribute |
//! | Universal | `*` | Matches any element |
//! | Attribute | `[href]` | Matches elements with an attribute |
//! | Attr value | `[type="text"]` | Exact attribute value match |
//! | Attr prefix | `[href^="https"]` | Attribute starts with value |
//! | Attr suffix | `[src$=".png"]` | Attribute ends with value |
//! | Attr substr | `[title*="hello"]` | Attribute contains value |
//! | Attr word | `[class~="active"]` | Whitespace-separated word match |
//! | Attr dash | `[lang\|="en"]` | Exact or dash-prefix match |
//! | Descendant | `div p` | `p` inside `div` (any depth) |
//! | Child | `div > p` | `p` directly inside `div` |
//! | Adjacent | `h1 + p` | `p` immediately after `h1` |
//! | General sibling | `h1 ~ p` | `p` after `h1` (same parent) |
//! | Group | `div, p` | Matches `div` or `p` |
//! | `:first-child` | `p:first-child` | First child element |
//! | `:last-child` | `p:last-child` | Last child element |
//! | `:only-child` | `p:only-child` | Only child element |
//! | `:empty` | `div:empty` | Element with no children |
//! | `:not()` | `:not(.hidden)` | Negation |
//! | `:nth-child()` | `:nth-child(2n+1)` | Position-based matching |
//!
//! # Examples
//!
//! ```
//! use xmloxide::css::select;
//! use xmloxide::Document;
//!
//! let doc = Document::parse_str(r#"
//!     <html>
//!       <body>
//!         <div class="content">
//!           <p id="intro">Hello</p>
//!           <p class="highlight">World</p>
//!         </div>
//!       </body>
//!     </html>
//! "#).unwrap();
//!
//! let root = doc.root_element().unwrap();
//!
//! // Find all paragraphs
//! let paragraphs = select(&doc, root, "p").unwrap();
//! assert_eq!(paragraphs.len(), 2);
//!
//! // Find by class
//! let highlighted = select(&doc, root, ".highlight").unwrap();
//! assert_eq!(highlighted.len(), 1);
//! assert_eq!(doc.text_content(highlighted[0]), "World");
//!
//! // Find by ID
//! let intro = select(&doc, root, "#intro").unwrap();
//! assert_eq!(intro.len(), 1);
//!
//! // Complex selector
//! let result = select(&doc, root, "div.content > p").unwrap();
//! assert_eq!(result.len(), 2);
//! ```

mod eval;
pub mod parser;
pub mod types;

pub use parser::CssSelectorError;
pub use types::SelectorGroup;

use crate::tree::{Document, NodeId};

/// Select all descendant elements matching a CSS selector string.
///
/// Parses the selector and evaluates it against all descendants of `scope`.
/// Returns matching nodes in document order.
///
/// # Errors
///
/// Returns a [`CssSelectorError`] if the selector string is malformed.
///
/// # Examples
///
/// ```
/// use xmloxide::css::select;
/// use xmloxide::Document;
///
/// let doc = Document::parse_str("<ul><li class=\"a\">1</li><li>2</li></ul>").unwrap();
/// let root = doc.root_element().unwrap();
/// let items = select(&doc, root, "li.a").unwrap();
/// assert_eq!(items.len(), 1);
/// ```
pub fn select(
    doc: &Document,
    scope: NodeId,
    selector: &str,
) -> Result<Vec<NodeId>, CssSelectorError> {
    let group = parser::parse_selector(selector)?;
    Ok(eval::select(doc, scope, &group))
}

/// Select all descendant elements matching a pre-parsed selector group.
///
/// Use this when evaluating the same selector against multiple documents
/// or scopes to avoid re-parsing the selector string.
pub fn select_with(doc: &Document, scope: NodeId, group: &SelectorGroup) -> Vec<NodeId> {
    eval::select(doc, scope, group)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_doc() -> Document {
        Document::parse_str(
            r#"<html>
            <body>
                <div id="main" class="container wide">
                    <h1>Title</h1>
                    <p class="intro">Hello</p>
                    <p class="body">World</p>
                    <ul>
                        <li class="active">One</li>
                        <li>Two</li>
                        <li>Three</li>
                    </ul>
                    <a href="https://example.com">Link</a>
                    <img src="photo.png"/>
                    <span lang="en-US">English</span>
                </div>
                <div class="sidebar">
                    <p>Side</p>
                </div>
            </body>
        </html>"#,
        )
        .unwrap()
    }

    #[test]
    fn test_select_by_tag() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let ps = select(&doc, root, "p").unwrap();
        assert_eq!(ps.len(), 3);
    }

    #[test]
    fn test_select_by_class() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, ".intro").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "Hello");
    }

    #[test]
    fn test_select_by_id() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "#main").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("div"));
    }

    #[test]
    fn test_select_descendant() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "div p").unwrap();
        assert_eq!(result.len(), 3); // 2 in main + 1 in sidebar
    }

    #[test]
    fn test_select_child() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "#main > p").unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_select_adjacent_sibling() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "h1 + p").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "Hello");
    }

    #[test]
    fn test_select_general_sibling() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "h1 ~ p").unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_select_group() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "h1, img").unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_select_attr_existence() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "[href]").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("a"));
    }

    #[test]
    fn test_select_attr_prefix() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "[href^=\"https\"]").unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_select_attr_suffix() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "[src$=\".png\"]").unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_select_attr_dash_prefix() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "[lang|=\"en\"]").unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_select_first_child() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "li:first-child").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "One");
    }

    #[test]
    fn test_select_last_child() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "li:last-child").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "Three");
    }

    #[test]
    fn test_select_not() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "li:not(.active)").unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_select_nth_child_odd() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "li:nth-child(odd)").unwrap();
        assert_eq!(result.len(), 2); // 1st and 3rd
    }

    #[test]
    fn test_select_empty() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, ":empty").unwrap();
        // img is self-closing / empty
        assert!(result.iter().any(|&n| doc.node_name(n) == Some("img")));
    }

    #[test]
    fn test_select_universal() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "#main > *").unwrap();
        // All direct children of #main
        assert!(result.len() >= 5);
    }

    #[test]
    fn test_select_multiple_classes() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, ".container.wide").unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_select_complex() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "div.container > ul li.active").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "One");
    }

    #[test]
    fn test_select_error() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        assert!(select(&doc, root, ">>>").is_err());
    }

    #[test]
    fn test_id_map_auto_populated() {
        // Verify element_by_id works without DTD validation
        let doc = test_doc();
        let node = doc.element_by_id("main").unwrap();
        assert_eq!(doc.node_name(node), Some("div"));
    }

    #[test]
    fn test_fast_id_select() {
        // Pure #id selector should use the fast path
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, "#main").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("div"));
    }
}
