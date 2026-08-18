//! `XInclude` 1.0 processing.
//!
//! This module implements the [XML Inclusions (XInclude) 1.0](https://www.w3.org/TR/xinclude/)
//! specification. `XInclude` allows XML documents to reference and include content from
//! other XML or text resources using `xi:include` elements.
//!
//! # Overview
//!
//! `XInclude` processing replaces `<xi:include>` elements (in the
//! `http://www.w3.org/2001/XInclude` namespace) with the content they reference.
//! The `href` attribute specifies the URI of the resource to include, and the
//! `parse` attribute determines whether the content is included as parsed XML
//! (`parse="xml"`, the default) or as a text node (`parse="text"`).
//!
//! If a resource cannot be resolved, the processor looks for an `<xi:fallback>`
//! child element and uses its content instead. If no fallback is provided, the
//! include is recorded as an error.
//!
//! # Design
//!
//! Since the core library does not perform I/O, the caller provides a resolver
//! callback (`Fn(&str) -> Option<String>`) that maps URIs to content. This
//! allows the library to be used in any environment (filesystem, network,
//! in-memory test fixtures, etc.).

use std::collections::HashSet;
use std::fmt;

use crate::tree::{Document, NodeId, NodeKind};

/// The `XInclude` namespace URI.
///
/// All `xi:include` and `xi:fallback` elements must be in this namespace
/// for `XInclude` processing to recognize them.
pub const XINCLUDE_NS: &str = "http://www.w3.org/2001/XInclude";

/// The local name of the include element.
const INCLUDE_ELEMENT: &str = "include";

/// The local name of the fallback element.
const FALLBACK_ELEMENT: &str = "fallback";

/// Options for `XInclude` processing.
///
/// Controls the behavior of [`process_xincludes`], such as the maximum
/// nesting depth for recursive includes.
///
/// # Examples
///
/// ```
/// use xmloxide::xinclude::XIncludeOptions;
///
/// let opts = XIncludeOptions::default();
/// assert_eq!(opts.max_depth, 50);
/// ```
#[derive(Debug, Clone)]
pub struct XIncludeOptions {
    /// Maximum nesting depth for recursive includes.
    ///
    /// When an included document itself contains `xi:include` elements,
    /// processing recurses. This limit prevents infinite recursion or
    /// excessively deep include chains. The default is 50.
    pub max_depth: usize,
}

impl Default for XIncludeOptions {
    fn default() -> Self {
        Self { max_depth: 50 }
    }
}

/// An error encountered during `XInclude` processing.
///
/// Errors are collected rather than stopping processing, so that as many
/// includes as possible are resolved even when some fail.
#[derive(Debug, Clone)]
pub struct XIncludeError {
    /// Human-readable description of the error.
    pub message: String,
    /// The `href` that caused the error, if applicable.
    pub href: Option<String>,
}

impl fmt::Display for XIncludeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.href {
            Some(href) => write!(f, "XInclude error for '{href}': {}", self.message),
            None => write!(f, "XInclude error: {}", self.message),
        }
    }
}

/// Result of `XInclude` processing.
///
/// Contains the processed document with all resolvable includes expanded,
/// along with statistics and any errors encountered.
pub struct XIncludeResult {
    /// Number of includes that were successfully processed.
    pub inclusions: usize,
    /// Errors encountered during processing.
    ///
    /// Each error corresponds to an `xi:include` element that could not
    /// be resolved and had no usable `xi:fallback`.
    pub errors: Vec<XIncludeError>,
}

/// Processes `XInclude` elements in a document.
///
/// Walks the document tree looking for elements in the `XInclude` namespace
/// (`http://www.w3.org/2001/XInclude`) with local name `include`. For each
/// such element:
///
/// 1. The `href` attribute is read to determine the resource URI.
/// 2. The `parse` attribute is read to determine how to interpret the content
///    (`"xml"` or `"text"`, defaulting to `"xml"`).
/// 3. The `resolver` callback is called with the href (minus any fragment) to
///    obtain the resource content.
/// 4. On success, the `xi:include` element is replaced with the included content.
/// 5. On failure, the `xi:fallback` child is used if present; otherwise an error
///    is recorded.
///
/// The resolver callback receives the href string and returns `Some(content)` if
/// the resource is available, or `None` if it cannot be resolved.
///
/// # Circular inclusion detection
///
/// The processor tracks which hrefs have been included in the current inclusion
/// chain and rejects any attempt to include an already-active href, preventing
/// infinite loops.
///
/// # Examples
///
/// ```
/// use xmloxide::Document;
/// use xmloxide::xinclude::{process_xincludes, XIncludeOptions};
///
/// let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude">
///   <xi:include href="greeting.xml"/>
/// </doc>"#;
///
/// let mut doc = Document::parse_str(xml).unwrap();
/// let result = process_xincludes(&mut doc, |href| {
///     match href {
///         "greeting.xml" => Some("<hello>world</hello>".to_string()),
///         _ => None,
///     }
/// }, &XIncludeOptions::default());
///
/// assert_eq!(result.inclusions, 1);
/// assert!(result.errors.is_empty());
/// ```
pub fn process_xincludes<F>(
    doc: &mut Document,
    resolver: F,
    options: &XIncludeOptions,
) -> XIncludeResult
where
    F: Fn(&str) -> Option<String>,
{
    let mut state = ProcessingState {
        inclusions: 0,
        errors: Vec::new(),
        active_hrefs: HashSet::new(),
        max_depth: options.max_depth,
    };

    process_node(doc, doc.root(), &resolver, &mut state, 0);

    XIncludeResult {
        inclusions: state.inclusions,
        errors: state.errors,
    }
}

/// Internal mutable state carried through the `XInclude` processing pass.
struct ProcessingState {
    /// Number of successfully processed includes.
    inclusions: usize,
    /// Accumulated errors.
    errors: Vec<XIncludeError>,
    /// Set of hrefs currently in the inclusion chain (for cycle detection).
    active_hrefs: HashSet<String>,
    /// Maximum allowed nesting depth.
    max_depth: usize,
}

/// Recursively processes `XInclude` elements under the given node.
///
/// We collect the list of children first (as a `Vec<NodeId>`) to avoid
/// borrowing issues while mutating the document.
fn process_node<F>(
    doc: &mut Document,
    node: NodeId,
    resolver: &F,
    state: &mut ProcessingState,
    depth: usize,
) where
    F: Fn(&str) -> Option<String>,
{
    // Collect children before iteration, since we may mutate the tree.
    let children: Vec<NodeId> = doc.children(node).collect();

    for child in children {
        if is_xinclude_element(doc, child) {
            process_include_element(doc, child, resolver, state, depth);
        } else {
            // Recurse into non-include elements to find nested xi:include.
            process_node(doc, child, resolver, state, depth);
        }
    }
}

/// Checks whether a node is an `xi:include` element in the `XInclude` namespace.
fn is_xinclude_element(doc: &Document, node: NodeId) -> bool {
    if let NodeKind::Element {
        name, namespace, ..
    } = &doc.node(node).kind
    {
        name == INCLUDE_ELEMENT && namespace.as_deref() == Some(XINCLUDE_NS)
    } else {
        false
    }
}

/// Checks whether a node is an `xi:fallback` element in the `XInclude` namespace.
fn is_fallback_element(doc: &Document, node: NodeId) -> bool {
    if let NodeKind::Element {
        name, namespace, ..
    } = &doc.node(node).kind
    {
        name == FALLBACK_ELEMENT && namespace.as_deref() == Some(XINCLUDE_NS)
    } else {
        false
    }
}

/// Processes a single `xi:include` element.
///
/// Reads the `href` and `parse` attributes, resolves the content via the
/// resolver, and replaces the `xi:include` element with the result.
fn process_include_element<F>(
    doc: &mut Document,
    include_node: NodeId,
    resolver: &F,
    state: &mut ProcessingState,
    depth: usize,
) where
    F: Fn(&str) -> Option<String>,
{
    // Read attributes from the xi:include element.
    let href = doc.attribute(include_node, "href").map(str::to_owned);
    let parse = doc
        .attribute(include_node, "parse")
        .unwrap_or("xml")
        .to_owned();

    // Validate: href is required.
    let Some(href) = href else {
        state.errors.push(XIncludeError {
            message: "xi:include element is missing required 'href' attribute".to_string(),
            href: None,
        });
        // Remove the xi:include element.
        doc.detach(include_node);
        return;
    };

    // Validate: parse must be "xml" or "text".
    if parse != "xml" && parse != "text" {
        state.errors.push(XIncludeError {
            message: format!("invalid parse attribute value '{parse}'; expected 'xml' or 'text'"),
            href: Some(href),
        });
        doc.detach(include_node);
        return;
    }

    // Check depth limit.
    if depth >= state.max_depth {
        state.errors.push(XIncludeError {
            message: format!(
                "maximum XInclude nesting depth ({}) exceeded",
                state.max_depth
            ),
            href: Some(href),
        });
        doc.detach(include_node);
        return;
    }

    // Strip fragment identifier for resolution (but keep it for potential
    // XPointer processing later).
    let (base_href, _fragment) = split_fragment(&href);

    // Check for circular inclusion.
    if state.active_hrefs.contains(base_href) {
        state.errors.push(XIncludeError {
            message: "circular inclusion detected".to_string(),
            href: Some(href),
        });
        doc.detach(include_node);
        return;
    }

    // Resolve the resource.
    let content = resolver(base_href);

    match content {
        Some(content) => {
            // Mark this href as active in the inclusion chain.
            state.active_hrefs.insert(base_href.to_owned());

            let success = match parse.as_str() {
                "xml" => process_xml_include(doc, include_node, &content, resolver, state, depth),
                "text" => process_text_include(doc, include_node, &content),
                _ => false, // Already validated above.
            };

            // Remove from active set after processing.
            state.active_hrefs.remove(base_href);

            if success {
                state.inclusions += 1;
            }
        }
        None => {
            // Resource not found — try fallback.
            if !try_fallback(doc, include_node, resolver, state, depth) {
                state.errors.push(XIncludeError {
                    message: "resource not found and no xi:fallback provided".to_string(),
                    href: Some(href),
                });
                doc.detach(include_node);
            }
        }
    }
}

/// Processes an XML include: parses the content as XML and replaces the
/// `xi:include` element with the parsed children.
///
/// Returns `true` on success.
fn process_xml_include<F>(
    doc: &mut Document,
    include_node: NodeId,
    content: &str,
    resolver: &F,
    state: &mut ProcessingState,
    depth: usize,
) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    // Parse the included content as an XML document.
    let included_doc = match Document::parse_str(content) {
        Ok(d) => d,
        Err(e) => {
            // Parse failure — try fallback, otherwise record error.
            if try_fallback(doc, include_node, resolver, state, depth) {
                return false;
            }
            state.errors.push(XIncludeError {
                message: format!("failed to parse included XML: {e}"),
                href: None,
            });
            doc.detach(include_node);
            return false;
        }
    };

    // Copy nodes from the included document into the main document.
    // We need to deep-copy because the nodes live in a different arena.
    let included_root = included_doc.root();
    let included_children: Vec<NodeId> = included_doc.children(included_root).collect();

    // Get the parent of the xi:include element so we can insert siblings.
    let parent = doc.parent(include_node);

    // Insert each child of the included document's root before the
    // xi:include element, then remove the xi:include element.
    let mut inserted_nodes = Vec::new();
    for inc_child in &included_children {
        let new_node = deep_copy_node(doc, &included_doc, *inc_child);
        inserted_nodes.push(new_node);
    }

    // Insert all new nodes before the include element.
    for new_node in &inserted_nodes {
        doc.insert_before(include_node, *new_node);
    }

    // Detach and discard the xi:include element.
    doc.detach(include_node);

    // Recursively process XInclude elements in the newly inserted content.
    if parent.is_some() {
        // We only need to process the newly inserted nodes.
        for new_node in inserted_nodes {
            process_node(doc, new_node, resolver, state, depth + 1);
        }
    }

    true
}

/// Processes a text include: creates a text node with the content and replaces
/// the `xi:include` element.
///
/// Returns `true` on success.
fn process_text_include(doc: &mut Document, include_node: NodeId, content: &str) -> bool {
    let text_node = doc.create_node(NodeKind::Text {
        content: content.to_string(),
    });

    doc.insert_before(include_node, text_node);
    doc.detach(include_node);

    true
}

/// Tries to use an `xi:fallback` child of the include element.
///
/// If a fallback is found, its children are moved to replace the `xi:include`
/// element. Returns `true` if a fallback was found and applied.
fn try_fallback<F>(
    doc: &mut Document,
    include_node: NodeId,
    resolver: &F,
    state: &mut ProcessingState,
    depth: usize,
) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    // Find the first xi:fallback child.
    let fallback_node = {
        let children: Vec<NodeId> = doc.children(include_node).collect();
        children
            .into_iter()
            .find(|&child| is_fallback_element(doc, child))
    };

    let Some(fallback) = fallback_node else {
        return false;
    };

    // Collect the fallback's children.
    let fallback_children: Vec<NodeId> = doc.children(fallback).collect();

    // Detach each fallback child and insert before the xi:include element.
    let mut inserted_nodes = Vec::new();
    for child in fallback_children {
        doc.detach(child);
        doc.insert_before(include_node, child);
        inserted_nodes.push(child);
    }

    // Remove the xi:include element (which still contains the now-empty fallback).
    doc.detach(include_node);

    // Recursively process the inserted fallback content.
    for node in inserted_nodes {
        process_node(doc, node, resolver, state, depth + 1);
    }

    true
}

/// Deep-copies a node (and all its descendants) from one document's arena
/// into another.
///
/// This is necessary because nodes in different `Document`s live in separate
/// arenas and cannot share `NodeId`s.
fn deep_copy_node(target: &mut Document, source: &Document, source_id: NodeId) -> NodeId {
    let source_node = source.node(source_id);
    let new_id = target.create_node(source_node.kind.clone());

    // Recursively copy children.
    let children: Vec<NodeId> = source.children(source_id).collect();
    for child_id in children {
        let new_child = deep_copy_node(target, source, child_id);
        target.append_child(new_id, new_child);
    }

    new_id
}

/// Splits a URI into the base part and optional fragment identifier.
///
/// For example, `"file.xml#section1"` returns `("file.xml", Some("section1"))`.
/// If there is no fragment, returns `(href, None)`.
fn split_fragment(href: &str) -> (&str, Option<&str>) {
    if let Some(pos) = href.find('#') {
        let (base, frag) = href.split_at(pos);
        // frag starts with '#', skip it.
        (base, Some(&frag[1..]))
    } else {
        (href, None)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // Helper: parse XML, process XIncludes with the given resolver, return the
    // document and result.
    fn process_with_resolver<F>(xml: &str, resolver: F) -> (Document, XIncludeResult)
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut doc = Document::parse_str(xml).unwrap();
        let result = process_xincludes(&mut doc, resolver, &XIncludeOptions::default());
        (doc, result)
    }

    // Helper: serialize the document to a string for comparison.
    fn doc_text_content(doc: &Document) -> String {
        let root_elem = doc.root_element().unwrap();
        doc.text_content(root_elem)
    }

    #[test]
    fn test_basic_xml_include() {
        let xml =
            r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="inc.xml"/></doc>"#;
        let (doc, result) = process_with_resolver(xml, |href| match href {
            "inc.xml" => Some("<greeting>hello</greeting>".to_string()),
            _ => None,
        });

        assert_eq!(result.inclusions, 1);
        assert!(result.errors.is_empty());

        // The included <greeting> element should be a child of <doc>.
        let root = doc.root_element().unwrap();
        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children.len(), 1);
        assert_eq!(doc.node_name(children[0]), Some("greeting"));
        assert_eq!(doc.text_content(children[0]), "hello");
    }

    #[test]
    fn test_basic_text_include() {
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="msg.txt" parse="text"/></doc>"#;
        let (doc, result) = process_with_resolver(xml, |href| match href {
            "msg.txt" => Some("Hello, World!".to_string()),
            _ => None,
        });

        assert_eq!(result.inclusions, 1);
        assert!(result.errors.is_empty());
        assert_eq!(doc_text_content(&doc), "Hello, World!");
    }

    #[test]
    fn test_fallback_when_resource_not_found() {
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="missing.xml"><xi:fallback><alt>fallback content</alt></xi:fallback></xi:include></doc>"#;
        let (doc, result) = process_with_resolver(xml, |_| None);

        assert_eq!(result.inclusions, 0);
        assert!(result.errors.is_empty());

        let root = doc.root_element().unwrap();
        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children.len(), 1);
        assert_eq!(doc.node_name(children[0]), Some("alt"));
        assert_eq!(doc.text_content(children[0]), "fallback content");
    }

    #[test]
    fn test_fallback_with_text_content() {
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="missing.xml"><xi:fallback>plain fallback</xi:fallback></xi:include></doc>"#;
        let (doc, result) = process_with_resolver(xml, |_| None);

        assert_eq!(result.inclusions, 0);
        assert!(result.errors.is_empty());
        assert_eq!(doc_text_content(&doc), "plain fallback");
    }

    #[test]
    fn test_missing_href_attribute() {
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include/></doc>"#;
        let (_doc, result) = process_with_resolver(xml, |_| None);

        assert_eq!(result.inclusions, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].message.contains("missing required 'href'"));
        assert!(result.errors[0].href.is_none());
    }

    #[test]
    fn test_circular_inclusion_detection() {
        // "a.xml" includes "b.xml" which includes "a.xml" again.
        let xml =
            r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="a.xml"/></doc>"#;
        let (_, result) = process_with_resolver(xml, |href| match href {
            "a.xml" => Some(
                r#"<a xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="a.xml"/></a>"#
                    .to_string(),
            ),
            _ => None,
        });

        // The first include succeeds, the second (circular) fails.
        assert_eq!(result.inclusions, 1);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].message.contains("circular inclusion"));
    }

    #[test]
    fn test_max_depth_exceeded() {
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="deep.xml"/></doc>"#;
        let mut doc = Document::parse_str(xml).unwrap();
        let opts = XIncludeOptions { max_depth: 2 };

        // Each level includes another level.
        let result = process_xincludes(
            &mut doc,
            |href| {
                match href {
                "deep.xml" => Some(
                    r#"<level xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="deeper.xml"/></level>"#
                        .to_string(),
                ),
                "deeper.xml" => Some(
                    r#"<level xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="deepest.xml"/></level>"#
                        .to_string(),
                ),
                "deepest.xml" => Some("<leaf/>".to_string()),
                _ => None,
            }
            },
            &opts,
        );

        // depth 0 -> deep.xml succeeds, depth 1 -> deeper.xml succeeds,
        // depth 2 -> deepest.xml exceeds max_depth=2.
        assert!(result.errors.iter().any(|e| e.message.contains("depth")));
    }

    #[test]
    fn test_multiple_includes_in_same_document() {
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="a.xml"/><xi:include href="b.xml"/></doc>"#;
        let (doc, result) = process_with_resolver(xml, |href| match href {
            "a.xml" => Some("<first/>".to_string()),
            "b.xml" => Some("<second/>".to_string()),
            _ => None,
        });

        assert_eq!(result.inclusions, 2);
        assert!(result.errors.is_empty());

        let root = doc.root_element().unwrap();
        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(children.len(), 2);
        assert_eq!(doc.node_name(children[0]), Some("first"));
        assert_eq!(doc.node_name(children[1]), Some("second"));
    }

    #[test]
    fn test_nested_includes() {
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="outer.xml"/></doc>"#;
        let (doc, result) = process_with_resolver(xml, |href| {
            match href {
            "outer.xml" => Some(
                r#"<outer xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="inner.xml"/></outer>"#
                    .to_string(),
            ),
            "inner.xml" => Some("<inner>nested</inner>".to_string()),
            _ => None,
        }
        });

        assert_eq!(result.inclusions, 2);
        assert!(result.errors.is_empty());

        let root = doc.root_element().unwrap();
        let outer: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(doc.node_name(outer[0]), Some("outer"));

        let inner: Vec<NodeId> = doc.children(outer[0]).collect();
        assert_eq!(doc.node_name(inner[0]), Some("inner"));
        assert_eq!(doc.text_content(inner[0]), "nested");
    }

    #[test]
    fn test_default_parse_attribute_is_xml() {
        // When parse is not specified, it defaults to "xml".
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="data.xml"/></doc>"#;
        let (doc, result) = process_with_resolver(xml, |href| match href {
            "data.xml" => Some("<item>value</item>".to_string()),
            _ => None,
        });

        assert_eq!(result.inclusions, 1);
        assert!(result.errors.is_empty());

        let root = doc.root_element().unwrap();
        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(doc.node_name(children[0]), Some("item"));
    }

    #[test]
    fn test_include_replaces_entire_xi_include_element() {
        // Verify that the xi:include element itself is completely removed.
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><before/><xi:include href="mid.xml"/><after/></doc>"#;
        let (doc, result) = process_with_resolver(xml, |href| match href {
            "mid.xml" => Some("<middle/>".to_string()),
            _ => None,
        });

        assert_eq!(result.inclusions, 1);

        let root = doc.root_element().unwrap();
        let names: Vec<Option<&str>> = doc.children(root).map(|c| doc.node_name(c)).collect();
        assert_eq!(names, vec![Some("before"), Some("middle"), Some("after")]);
    }

    #[test]
    fn test_text_include_preserves_whitespace() {
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="ws.txt" parse="text"/></doc>"#;
        let content = "  line1\n  line2\n";
        let (doc, result) = process_with_resolver(xml, |href| match href {
            "ws.txt" => Some(content.to_string()),
            _ => None,
        });

        assert_eq!(result.inclusions, 1);
        assert_eq!(doc_text_content(&doc), content);
    }

    #[test]
    fn test_empty_include_content() {
        // Including content that parses to an empty document root.
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="empty.txt" parse="text"/></doc>"#;
        let (doc, result) = process_with_resolver(xml, |href| match href {
            "empty.txt" => Some(String::new()),
            _ => None,
        });

        assert_eq!(result.inclusions, 1);
        assert!(result.errors.is_empty());
        assert_eq!(doc_text_content(&doc), "");
    }

    #[test]
    fn test_include_with_fragment_identifier() {
        // Fragment identifiers are stripped for resolution; the base href
        // is used to fetch the content.
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="data.xml#section1"/></doc>"#;
        let (doc, result) = process_with_resolver(xml, |href| match href {
            "data.xml" => Some("<section>content</section>".to_string()),
            _ => None,
        });

        assert_eq!(result.inclusions, 1);
        assert!(result.errors.is_empty());

        let root = doc.root_element().unwrap();
        let children: Vec<NodeId> = doc.children(root).collect();
        assert_eq!(doc.node_name(children[0]), Some("section"));
    }

    #[test]
    fn test_xinclude_namespace_detection() {
        // An "include" element NOT in the XInclude namespace should be ignored.
        let xml = r#"<doc><include href="should-ignore.xml"/></doc>"#;
        let (_, result) = process_with_resolver(xml, |_| {
            panic!("resolver should not be called for non-XInclude elements");
        });

        assert_eq!(result.inclusions, 0);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_split_fragment() {
        assert_eq!(split_fragment("file.xml#sec"), ("file.xml", Some("sec")));
        assert_eq!(split_fragment("file.xml"), ("file.xml", None));
        assert_eq!(split_fragment("file.xml#"), ("file.xml", Some("")));
        assert_eq!(split_fragment("#frag"), ("", Some("frag")));
    }

    #[test]
    fn test_no_fallback_records_error() {
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="nope.xml"/></doc>"#;
        let (_, result) = process_with_resolver(xml, |_| None);

        assert_eq!(result.inclusions, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].message.contains("resource not found"));
        assert_eq!(result.errors[0].href.as_deref(), Some("nope.xml"));
    }

    #[test]
    fn test_invalid_parse_attribute() {
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="x.xml" parse="json"/></doc>"#;
        let (_, result) = process_with_resolver(xml, |_| None);

        assert_eq!(result.errors.len(), 1);
        assert!(result.errors[0].message.contains("invalid parse attribute"));
    }

    #[test]
    fn test_xml_include_with_wrapper_element() {
        // Included document has a root element with multiple children.
        let xml = r#"<doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="multi.xml"/></doc>"#;
        let (doc, result) = process_with_resolver(xml, |href| match href {
            "multi.xml" => Some("<wrapper><first/><second/></wrapper>".to_string()),
            _ => None,
        });

        assert_eq!(result.inclusions, 1);
        assert!(result.errors.is_empty());

        let root = doc.root_element().unwrap();
        let children: Vec<NodeId> = doc.children(root).collect();
        // The <wrapper> element is inserted as a child of <doc>.
        assert_eq!(children.len(), 1);
        assert_eq!(doc.node_name(children[0]), Some("wrapper"));

        let wrapper_children: Vec<NodeId> = doc.children(children[0]).collect();
        assert_eq!(wrapper_children.len(), 2);
        assert_eq!(doc.node_name(wrapper_children[0]), Some("first"));
        assert_eq!(doc.node_name(wrapper_children[1]), Some("second"));
    }

    #[test]
    fn test_options_default() {
        let opts = XIncludeOptions::default();
        assert_eq!(opts.max_depth, 50);
    }

    #[test]
    fn test_error_display() {
        let err = XIncludeError {
            message: "resource not found".to_string(),
            href: Some("file.xml".to_string()),
        };
        assert_eq!(
            err.to_string(),
            "XInclude error for 'file.xml': resource not found"
        );

        let err_no_href = XIncludeError {
            message: "bad element".to_string(),
            href: None,
        };
        assert_eq!(err_no_href.to_string(), "XInclude error: bad element");
    }
}
