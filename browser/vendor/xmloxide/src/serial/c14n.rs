//! Canonical XML (C14N) serialization.
//!
//! Implements Canonical XML 1.0 per the W3C specification:
//! <https://www.w3.org/TR/xml-c14n/>
//!
//! Canonical XML produces a unique, deterministic byte sequence for logically
//! equivalent XML documents. This is critical for XML digital signatures, where
//! the canonical form must be identical regardless of insignificant variations
//! in the original serialization.
//!
//! # Key C14N rules
//!
//! - No XML declaration in output
//! - Attributes sorted by namespace URI then local name
//! - Namespace declarations sorted by prefix
//! - Empty elements always use start-end tag pairs (`<a></a>`, not `<a/>`)
//! - CDATA sections replaced with escaped text content
//! - Entity references expanded
//! - DOCTYPE declarations removed
//! - Specific character escaping rules for text content and attribute values
//!
//! # Examples
//!
//! ```
//! use xmloxide::Document;
//! use xmloxide::serial::c14n::{canonicalize, C14nOptions};
//!
//! let doc = Document::parse_str("<root><child/></root>").unwrap();
//! let c14n = canonicalize(&doc, &C14nOptions::default());
//! assert_eq!(c14n, "<root><child></child></root>");
//! ```

use std::collections::BTreeMap;

use crate::tree::{Document, NodeId, NodeKind};

/// Options for canonical XML serialization.
///
/// Controls the mode of canonicalization: inclusive or exclusive,
/// with or without comments.
///
/// # Examples
///
/// ```
/// use xmloxide::serial::c14n::C14nOptions;
///
/// // Default: inclusive C14N with comments
/// let opts = C14nOptions::default();
/// assert!(opts.with_comments);
/// assert!(!opts.exclusive);
/// ```
#[derive(Debug, Clone)]
pub struct C14nOptions {
    /// If true, include comments in output (C14N with comments).
    /// If false, strip comments (plain C14N).
    pub with_comments: bool,
    /// If true, use exclusive C14N (Exclusive XML Canonicalization 1.0).
    /// If false, use inclusive C14N.
    pub exclusive: bool,
    /// For exclusive C14N, the list of additional namespace prefixes to
    /// treat as visibly utilized (the `InclusiveNamespaces PrefixList`).
    pub inclusive_prefixes: Vec<String>,
}

impl Default for C14nOptions {
    fn default() -> Self {
        Self {
            with_comments: true,
            exclusive: false,
            inclusive_prefixes: Vec::new(),
        }
    }
}

/// Serializes a document to Canonical XML (C14N 1.0).
///
/// Processes the entire document according to the Canonical XML specification.
/// The output never includes an XML declaration, DOCTYPE declarations are
/// removed, and all other canonicalization rules are applied.
///
/// # Examples
///
/// ```
/// use xmloxide::Document;
/// use xmloxide::serial::c14n::{canonicalize, C14nOptions};
///
/// let doc = Document::parse_str("<root attr2=\"b\" attr1=\"a\"/>").unwrap();
/// let c14n = canonicalize(&doc, &C14nOptions::default());
/// // Attributes are sorted, empty element uses start-end tags
/// assert_eq!(c14n, "<root attr1=\"a\" attr2=\"b\"></root>");
/// ```
#[must_use]
pub fn canonicalize(doc: &Document, options: &C14nOptions) -> String {
    let mut ctx = C14nContext::new(doc, options);
    ctx.process_document();
    ctx.output
}

/// Serializes a subtree (specific node and its descendants) to Canonical XML.
///
/// This is useful when canonicalizing a portion of a document, for example
/// when computing a digest for a specific element in an XML signature.
///
/// # Examples
///
/// ```
/// use xmloxide::Document;
/// use xmloxide::serial::c14n::{canonicalize_subtree, C14nOptions};
///
/// let doc = Document::parse_str("<root><child>text</child></root>").unwrap();
/// let root = doc.root_element().unwrap();
/// let c14n = canonicalize_subtree(&doc, root, &C14nOptions::default());
/// assert_eq!(c14n, "<root><child>text</child></root>");
/// ```
#[must_use]
pub fn canonicalize_subtree(doc: &Document, node: NodeId, options: &C14nOptions) -> String {
    let mut ctx = C14nContext::new(doc, options);
    ctx.process_node(node);
    ctx.output
}

/// A namespace binding: prefix (empty string for default namespace) to URI.
type NsBinding = BTreeMap<String, String>;

/// Internal context for C14N serialization.
struct C14nContext<'a> {
    doc: &'a Document,
    options: &'a C14nOptions,
    output: String,
    /// Stack of namespace bindings currently in scope.
    /// Each entry maps prefix -> URI. The stack tracks what has been
    /// rendered so far, to avoid redundant re-declarations.
    rendered_ns_stack: Vec<NsBinding>,
}

impl<'a> C14nContext<'a> {
    fn new(doc: &'a Document, options: &'a C14nOptions) -> Self {
        // Seed the rendered namespace stack with the implicit `xml` prefix
        // binding. The XML Namespaces spec reserves `xml` as always bound to
        // `http://www.w3.org/XML/1998/namespace`. Canonical XML 1.0 §2.3
        // ("Processing Model") requires that this binding is never emitted:
        //     "omit namespace node with local name xml, which defines the
        //      xml prefix, if its string value is
        //      http://www.w3.org/XML/1998/namespace"
        // Exclusive C14N §3 defines itself as a variant of Canonical XML and
        // does not restate this rule — it inherits it. Pre-populating the
        // binding here makes the dedup check in `compute_ns_declarations`
        // filter it out automatically when an element uses `xml:lang`,
        // `xml:space`, or `xml:base`.
        let mut initial_bindings = NsBinding::new();
        initial_bindings.insert(
            "xml".to_string(),
            "http://www.w3.org/XML/1998/namespace".to_string(),
        );
        Self {
            doc,
            options,
            output: String::new(),
            rendered_ns_stack: vec![initial_bindings],
        }
    }

    /// Processes the entire document node.
    fn process_document(&mut self) {
        let root = self.doc.root();
        let children: Vec<NodeId> = self.doc.children(root).collect();
        let root_elem_index = children
            .iter()
            .position(|&id| matches!(self.doc.node(id).kind, NodeKind::Element { .. }));

        for (i, &child) in children.iter().enumerate() {
            match &self.doc.node(child).kind {
                NodeKind::Comment { .. } if !self.options.with_comments => {}
                NodeKind::Comment { content } => {
                    if let Some(root_idx) = root_elem_index {
                        if i < root_idx {
                            write_c14n_comment(&mut self.output, content);
                            self.output.push('\n');
                        } else if i > root_idx {
                            self.output.push('\n');
                            write_c14n_comment(&mut self.output, content);
                        }
                    } else {
                        write_c14n_comment(&mut self.output, content);
                    }
                }
                NodeKind::ProcessingInstruction { target, data } => {
                    if let Some(root_idx) = root_elem_index {
                        if i < root_idx {
                            write_c14n_pi(&mut self.output, target, data.as_deref());
                            self.output.push('\n');
                        } else if i > root_idx {
                            self.output.push('\n');
                            write_c14n_pi(&mut self.output, target, data.as_deref());
                        }
                    } else {
                        write_c14n_pi(&mut self.output, target, data.as_deref());
                    }
                }
                NodeKind::Element { .. } => {
                    self.process_element(child);
                }
                // DOCTYPE, Document, Text, CData, EntityRef nodes at the
                // document level are not output in C14N.
                _ => {}
            }
        }
    }

    /// Processes a single node, dispatching by kind.
    fn process_node(&mut self, id: NodeId) {
        match &self.doc.node(id).kind {
            NodeKind::Element { .. } => {
                self.process_element(id);
            }
            NodeKind::Text { content } => {
                write_c14n_text(&mut self.output, content);
            }
            NodeKind::CData { content } => {
                // CDATA sections are replaced with their escaped text content
                write_c14n_text(&mut self.output, content);
            }
            NodeKind::Comment { content } => {
                if self.options.with_comments {
                    write_c14n_comment(&mut self.output, content);
                }
            }
            NodeKind::ProcessingInstruction { target, data } => {
                write_c14n_pi(&mut self.output, target, data.as_deref());
            }
            NodeKind::EntityRef { name, .. } => {
                // Entity references are expanded. We output the entity
                // reference's children (the expansion). If there are no
                // children (unexpanded), output the reference as text.
                let has_children = self.doc.first_child(id).is_some();
                if has_children {
                    for child in self.doc.children(id) {
                        self.process_node(child);
                    }
                } else {
                    // Fallback: output the predefined entity expansion
                    let expanded = expand_predefined_entity(name);
                    write_c14n_text(&mut self.output, expanded);
                }
            }
            NodeKind::DocumentType { .. } | NodeKind::Document => {
                // DOCTYPE and Document nodes are not output
            }
        }
    }

    /// Processes an element node according to C14N rules.
    ///
    /// This is the core of the canonicalization algorithm. It:
    /// 1. Collects namespace declarations that need to be output
    /// 2. Sorts and outputs namespace declarations
    /// 3. Sorts and outputs attributes
    /// 4. Recursively processes children
    /// 5. Always uses start-end tag pairs (never self-closing)
    fn process_element(&mut self, id: NodeId) {
        let (name, prefix, namespace, attributes) = match &self.doc.node(id).kind {
            NodeKind::Element {
                name,
                prefix,
                namespace,
                attributes,
            } => (
                name.clone(),
                prefix.clone(),
                namespace.clone(),
                attributes.clone(),
            ),
            _ => return,
        };

        let qname = match &prefix {
            Some(pfx) => format!("{pfx}:{name}"),
            None => name.clone(),
        };

        let ns_to_output = self.compute_ns_declarations(
            id,
            &name,
            prefix.as_deref(),
            namespace.as_deref(),
            &attributes,
        );

        self.output.push('<');
        self.output.push_str(&qname);
        self.write_ns_declarations(&ns_to_output);
        self.write_sorted_attributes(&attributes);
        self.output.push('>');

        for child in self.doc.children(id) {
            self.process_node(child);
        }

        self.output.push_str("</");
        self.output.push_str(&qname);
        self.output.push('>');

        self.rendered_ns_stack.pop();
    }

    /// Computes which namespace declarations need to be output for an element,
    /// pushes a new rendered namespace scope, and returns the sorted list of
    /// (prefix, URI) pairs to emit.
    fn compute_ns_declarations(
        &mut self,
        id: NodeId,
        name: &str,
        prefix: Option<&str>,
        namespace: Option<&str>,
        attributes: &[crate::tree::Attribute],
    ) -> Vec<(String, String)> {
        let ns_decls = if self.options.exclusive {
            collect_exclusive_ns_decls(
                &self.options.inclusive_prefixes,
                prefix,
                namespace,
                attributes,
            )
        } else {
            collect_inclusive_ns_decls(attributes)
        };

        let mut current_rendered = self.rendered_ns_stack.last().cloned().unwrap_or_default();
        let mut ns_to_output: Vec<(String, String)> = Vec::new();

        for (ns_prefix, ns_uri) in &ns_decls {
            if current_rendered.get(ns_prefix) == Some(ns_uri) {
                continue;
            }

            // Special case for `xmlns=""`: per Canonical XML 1.0 §3.7 and
            // c14n11 §3.1, the empty default-namespace declaration is only
            // emitted to undeclare a *non-empty* inherited default. If no
            // non-empty default is currently in scope (parent has no default,
            // or the inherited default is itself empty), the `xmlns=""` from
            // the source must not appear in the canonical form.
            if ns_prefix.is_empty() && ns_uri.is_empty() {
                let has_nonempty_inherited_default =
                    current_rendered.get("").is_some_and(|s| !s.is_empty());
                if !has_nonempty_inherited_default {
                    continue;
                }
            }

            ns_to_output.push((ns_prefix.clone(), ns_uri.clone()));
            current_rendered.insert(ns_prefix.clone(), ns_uri.clone());
        }

        // The default-namespace undeclaration rule applies in both modes. If
        // the parent's default namespace is non-empty and visibly rendered in
        // scope, and the current element is in no namespace (the source has
        // an explicit `xmlns=""`), the canonical output must emit `xmlns=""`
        // to undeclare it. Canonical XML 1.0 §2.3 covers the inclusive case;
        // Exclusive C14N §3 inherits the rule (the default prefix is part of
        // the visibly-utilized set when an explicit undeclaration is present
        // in the source and the inherited default would otherwise propagate).
        self.check_default_ns_undeclaration(
            &ns_decls,
            attributes,
            &mut ns_to_output,
            &mut current_rendered,
        );

        // Suppress unused variable warnings for future use
        let _ = (id, name);

        ns_to_output.sort_by(|a, b| a.0.cmp(&b.0));
        self.rendered_ns_stack.push(current_rendered);
        ns_to_output
    }

    /// Checks whether a default namespace undeclaration (`xmlns=""`) is needed
    /// and adds it to the output list if so.
    fn check_default_ns_undeclaration(
        &self,
        ns_decls: &[(String, String)],
        attributes: &[crate::tree::Attribute],
        ns_to_output: &mut Vec<(String, String)>,
        current_rendered: &mut NsBinding,
    ) {
        let parent_default = self
            .rendered_ns_stack
            .last()
            .and_then(|m| m.get(""))
            .cloned();

        let has_current_default = ns_decls.iter().any(|(p, _)| p.is_empty());

        if !has_current_default && parent_default.is_some() && parent_default.as_deref() != Some("")
        {
            let has_explicit_undecl = attributes
                .iter()
                .any(|a| a.prefix.is_none() && a.name == "xmlns" && a.value.is_empty());
            if has_explicit_undecl {
                ns_to_output.push((String::new(), String::new()));
                current_rendered.insert(String::new(), String::new());
            }
        }
    }

    /// Writes sorted namespace declarations to the output.
    fn write_ns_declarations(&mut self, ns_to_output: &[(String, String)]) {
        for (ns_prefix, ns_uri) in ns_to_output {
            if ns_prefix.is_empty() {
                self.output.push_str(" xmlns=\"");
            } else {
                self.output.push_str(" xmlns:");
                self.output.push_str(ns_prefix);
                self.output.push_str("=\"");
            }
            write_c14n_attr_value(&mut self.output, ns_uri);
            self.output.push('"');
        }
    }

    /// Writes sorted non-namespace attributes to the output.
    fn write_sorted_attributes(&mut self, attributes: &[crate::tree::Attribute]) {
        let mut regular_attrs: Vec<_> = attributes
            .iter()
            .filter(|a| !is_ns_declaration(a))
            .collect();

        regular_attrs.sort_by(|a, b| {
            let a_ns = a.namespace.as_deref().unwrap_or("");
            let b_ns = b.namespace.as_deref().unwrap_or("");
            match a_ns.cmp(b_ns) {
                std::cmp::Ordering::Equal => a.name.cmp(&b.name),
                other => other,
            }
        });

        for attr in &regular_attrs {
            self.output.push(' ');
            if let Some(pfx) = &attr.prefix {
                self.output.push_str(pfx);
                self.output.push(':');
            }
            self.output.push_str(&attr.name);
            self.output.push_str("=\"");
            write_c14n_attr_value(&mut self.output, &attr.value);
            self.output.push('"');
        }
    }
}

/// Returns true if the attribute is a namespace declaration (`xmlns` or `xmlns:*`).
fn is_ns_declaration(attr: &crate::tree::Attribute) -> bool {
    attr.prefix.as_deref() == Some("xmlns") || (attr.prefix.is_none() && attr.name == "xmlns")
}

/// Collects namespace declarations for inclusive C14N.
///
/// In inclusive mode, all namespace declarations present on the element's
/// attributes are collected. The rendering stack handles deduplication.
fn collect_inclusive_ns_decls(attributes: &[crate::tree::Attribute]) -> Vec<(String, String)> {
    let mut decls = Vec::new();
    for attr in attributes {
        if attr.prefix.as_deref() == Some("xmlns") {
            decls.push((attr.name.clone(), attr.value.clone()));
        } else if attr.prefix.is_none() && attr.name == "xmlns" {
            decls.push((String::new(), attr.value.clone()));
        }
    }
    decls
}

/// Collects namespace declarations for exclusive C14N.
///
/// In exclusive mode, only "visibly utilized" namespace prefixes are output.
/// A namespace prefix is visibly utilized if it appears as the element's own
/// prefix, an attribute's prefix, or is listed in the `inclusive_prefixes`
/// option.
fn collect_exclusive_ns_decls(
    inclusive_prefixes: &[String],
    elem_prefix: Option<&str>,
    elem_ns: Option<&str>,
    attributes: &[crate::tree::Attribute],
) -> Vec<(String, String)> {
    let mut all_decls: BTreeMap<String, String> = BTreeMap::new();
    for attr in attributes {
        if attr.prefix.as_deref() == Some("xmlns") {
            all_decls.insert(attr.name.clone(), attr.value.clone());
        } else if attr.prefix.is_none() && attr.name == "xmlns" {
            all_decls.insert(String::new(), attr.value.clone());
        }
    }

    let mut utilized: Vec<String> = Vec::new();

    if let Some(pfx) = elem_prefix {
        utilized.push(pfx.to_string());
    } else if elem_ns.is_some() {
        utilized.push(String::new());
    }

    for attr in attributes {
        if is_ns_declaration(attr) {
            continue;
        }
        if let Some(pfx) = &attr.prefix {
            if !utilized.contains(pfx) {
                utilized.push(pfx.clone());
            }
        }
    }

    for pfx in inclusive_prefixes {
        let key = if pfx == "#default" {
            String::new()
        } else {
            pfx.clone()
        };
        if !utilized.contains(&key) {
            utilized.push(key);
        }
    }

    // Build a map of prefix -> URI from both explicit xmlns declarations
    // and the resolved namespace information on the element and attributes.
    // This handles the case where a subtree is canonicalized and the xmlns
    // declaration lives on an ancestor element.
    let mut available_bindings = all_decls;

    // Add the element's own namespace binding
    if let (Some(pfx), Some(uri)) = (elem_prefix, elem_ns) {
        available_bindings
            .entry(pfx.to_string())
            .or_insert_with(|| uri.to_string());
    } else if let (None, Some(uri)) = (elem_prefix, elem_ns) {
        available_bindings
            .entry(String::new())
            .or_insert_with(|| uri.to_string());
    }

    // Add attribute namespace bindings
    for attr in attributes {
        if is_ns_declaration(attr) {
            continue;
        }
        if let (Some(pfx), Some(uri)) = (&attr.prefix, &attr.namespace) {
            available_bindings
                .entry(pfx.clone())
                .or_insert_with(|| uri.clone());
        }
    }

    let mut result = Vec::new();
    for pfx in &utilized {
        if let Some(uri) = available_bindings.get(pfx) {
            result.push((pfx.clone(), uri.clone()));
        }
    }
    result
}

/// Writes a processing instruction in C14N form.
fn write_c14n_pi(out: &mut String, target: &str, data: Option<&str>) {
    out.push_str("<?");
    out.push_str(target);
    if let Some(d) = data {
        out.push(' ');
        out.push_str(d);
    }
    out.push_str("?>");
}

/// Writes a comment in C14N form.
fn write_c14n_comment(out: &mut String, content: &str) {
    out.push_str("<!--");
    out.push_str(content);
    out.push_str("-->");
}

/// Escapes text content per C14N rules.
///
/// C14N text escaping: `&` -> `&amp;`, `<` -> `&lt;`, `>` -> `&gt;`,
/// `\r` -> `&#xD;`
fn write_c14n_text(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(ch),
        }
    }
}

/// Escapes an attribute value per C14N rules.
///
/// C14N attribute value escaping: `&` -> `&amp;`, `<` -> `&lt;`,
/// `"` -> `&quot;`, `\t` -> `&#x9;`, `\n` -> `&#xA;`, `\r` -> `&#xD;`
fn write_c14n_attr_value(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(ch),
        }
    }
}

/// Expands a predefined XML entity name to its character value.
fn expand_predefined_entity(name: &str) -> &str {
    match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "apos" => "'",
        "quot" => "\"",
        _ => "",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::tree::Attribute;

    /// Helper: create a document from XML and return its C14N output.
    fn c14n(xml: &str) -> String {
        let doc = Document::parse_str(xml).unwrap();
        canonicalize(&doc, &C14nOptions::default())
    }

    /// Helper: create a C14N without comments.
    fn c14n_no_comments(xml: &str) -> String {
        let doc = Document::parse_str(xml).unwrap();
        canonicalize(
            &doc,
            &C14nOptions {
                with_comments: false,
                ..C14nOptions::default()
            },
        )
    }

    #[test]
    fn test_c14n_empty_element_uses_start_end_tags() {
        // C14N rule: empty elements always use start-end tag pairs, never
        // self-closing.
        let result = c14n("<root/>");
        assert_eq!(result, "<root></root>");

        let result = c14n("<root><child/></root>");
        assert_eq!(result, "<root><child></child></root>");
    }

    #[test]
    fn test_c14n_attribute_sorting() {
        // C14N rule: attributes sorted by namespace URI then local name.
        // Non-namespaced attributes (empty NS URI) come first.
        let result = c14n("<root z=\"1\" a=\"2\" m=\"3\"/>");
        assert_eq!(result, "<root a=\"2\" m=\"3\" z=\"1\"></root>");
    }

    #[test]
    fn test_c14n_namespace_declaration_ordering() {
        // C14N rule: namespace declarations sorted by prefix.
        let result = c14n("<root xmlns:z=\"http://z.example\" xmlns:a=\"http://a.example\"/>");
        assert_eq!(
            result,
            "<root xmlns:a=\"http://a.example\" xmlns:z=\"http://z.example\"></root>"
        );
    }

    #[test]
    fn test_c14n_text_content_escaping() {
        // C14N text escaping: & < > and \r
        let mut doc = Document::new();
        let root = doc.root();
        let elem = doc.create_node(NodeKind::Element {
            name: "root".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        let text = doc.create_node(NodeKind::Text {
            content: "a & b < c > d\re".to_string(),
        });
        doc.append_child(root, elem);
        doc.append_child(elem, text);
        let result = canonicalize(&doc, &C14nOptions::default());
        assert_eq!(result, "<root>a &amp; b &lt; c &gt; d&#xD;e</root>");
    }

    #[test]
    fn test_c14n_attribute_value_escaping() {
        // C14N attribute value escaping: & < " \t \n \r
        let mut doc = Document::new();
        let root = doc.root();
        let elem = doc.create_node(NodeKind::Element {
            name: "root".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![Attribute {
                name: "val".to_string(),
                value: "a&b<c\"d\te\nf\rg".to_string(),
                prefix: None,
                namespace: None,
                raw_value: None,
            }],
        });
        doc.append_child(root, elem);
        let result = canonicalize(&doc, &C14nOptions::default());
        assert_eq!(
            result,
            "<root val=\"a&amp;b&lt;c&quot;d&#x9;e&#xA;f&#xD;g\"></root>"
        );
    }

    #[test]
    fn test_c14n_no_xml_declaration() {
        // C14N rule: output never includes XML declaration.
        let result = c14n("<?xml version=\"1.0\" encoding=\"UTF-8\"?><root/>");
        assert_eq!(result, "<root></root>");
        assert!(!result.contains("<?xml"));
    }

    #[test]
    fn test_c14n_cdata_replaced_with_escaped_text() {
        // C14N rule: CDATA sections replaced with escaped text content.
        let mut doc = Document::new();
        let root = doc.root();
        let elem = doc.create_node(NodeKind::Element {
            name: "root".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        let cdata = doc.create_node(NodeKind::CData {
            content: "x < 1 && y > 2".to_string(),
        });
        doc.append_child(root, elem);
        doc.append_child(elem, cdata);
        let result = canonicalize(&doc, &C14nOptions::default());
        assert_eq!(result, "<root>x &lt; 1 &amp;&amp; y &gt; 2</root>");
    }

    #[test]
    fn test_c14n_comments_included_by_default() {
        // C14N with comments: comments are included.
        let result = c14n("<root><!-- hello --></root>");
        assert_eq!(result, "<root><!-- hello --></root>");
    }

    #[test]
    fn test_c14n_comments_excluded_when_option_set() {
        // C14N without comments: comments are stripped.
        let result = c14n_no_comments("<root><!-- hello --></root>");
        assert_eq!(result, "<root></root>");
    }

    #[test]
    fn test_c14n_doctype_removed() {
        // C14N rule: DOCTYPE declarations removed from output.
        let mut doc = Document::new();
        let root = doc.root();
        let doctype = doc.create_node(NodeKind::DocumentType {
            name: "html".to_string(),
            system_id: None,
            public_id: None,
            internal_subset: None,
        });
        let elem = doc.create_node(NodeKind::Element {
            name: "html".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        doc.append_child(root, doctype);
        doc.append_child(root, elem);
        let result = canonicalize(&doc, &C14nOptions::default());
        assert_eq!(result, "<html></html>");
        assert!(!result.contains("DOCTYPE"));
    }

    #[test]
    fn test_c14n_processing_instructions_preserved() {
        // C14N rule: processing instructions are preserved.
        let result = c14n("<root><?target data?></root>");
        assert_eq!(result, "<root><?target data?></root>");
    }

    #[test]
    fn test_c14n_simple_document_roundtrip() {
        // A simple well-formed document should canonicalize predictably.
        let result = c14n("<root><a>hello</a><b>world</b></root>");
        assert_eq!(result, "<root><a>hello</a><b>world</b></root>");
    }

    #[test]
    fn test_c14n_namespace_handling_default() {
        // Default namespace declaration should be output.
        let result = c14n("<root xmlns=\"http://example.com\"/>");
        assert_eq!(result, "<root xmlns=\"http://example.com\"></root>");
    }

    #[test]
    fn test_c14n_namespace_handling_prefixed() {
        // Prefixed namespace declaration should be output.
        let result = c14n("<ns:root xmlns:ns=\"http://example.com\"/>");
        assert_eq!(
            result,
            "<ns:root xmlns:ns=\"http://example.com\"></ns:root>"
        );
    }

    #[test]
    fn test_c14n_whitespace_only_text_preserved() {
        // Whitespace-only text nodes within elements are preserved.
        let result = c14n("<root> </root>");
        assert_eq!(result, "<root> </root>");

        let result = c14n("<root>  \n  </root>");
        assert_eq!(result, "<root>  \n  </root>");
    }

    #[test]
    fn test_c14n_exclusive_namespace_scoping() {
        // In exclusive C14N, only visibly utilized namespaces are output.
        let doc = Document::parse_str(
            "<root xmlns:a=\"http://a.example\" xmlns:b=\"http://b.example\">\
             <a:child/></root>",
        )
        .unwrap();

        let root_elem = doc.root_element().unwrap();
        let child = doc.first_child(root_elem).unwrap();

        let result = canonicalize_subtree(
            &doc,
            child,
            &C14nOptions {
                with_comments: true,
                exclusive: true,
                inclusive_prefixes: vec![],
            },
        );

        // In exclusive mode on the subtree, only the "a" namespace that is
        // visibly utilized should appear. The "b" namespace should not.
        assert!(result.contains("xmlns:a="));
        assert!(!result.contains("xmlns:b="));
    }

    #[test]
    fn test_c14n_complex_document_all_node_types() {
        // A document exercising many node types together.
        let mut doc = Document::new();
        let root = doc.root();

        // Comment before root element
        let comment_before = doc.create_node(NodeKind::Comment {
            content: " prologue comment ".to_string(),
        });
        doc.append_child(root, comment_before);

        // PI before root element
        let pi_before = doc.create_node(NodeKind::ProcessingInstruction {
            target: "app".to_string(),
            data: Some("start".to_string()),
        });
        doc.append_child(root, pi_before);

        // Root element with attributes
        let elem = doc.create_node(NodeKind::Element {
            name: "root".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![
                Attribute {
                    name: "z".to_string(),
                    value: "1".to_string(),
                    prefix: None,
                    namespace: None,
                    raw_value: None,
                },
                Attribute {
                    name: "a".to_string(),
                    value: "2".to_string(),
                    prefix: None,
                    namespace: None,
                    raw_value: None,
                },
            ],
        });
        doc.append_child(root, elem);

        // Text child
        let text = doc.create_node(NodeKind::Text {
            content: "hello".to_string(),
        });
        doc.append_child(elem, text);

        // CDATA child
        let cdata = doc.create_node(NodeKind::CData {
            content: "a<b".to_string(),
        });
        doc.append_child(elem, cdata);

        // Comment child
        let inner_comment = doc.create_node(NodeKind::Comment {
            content: " inner ".to_string(),
        });
        doc.append_child(elem, inner_comment);

        // PI child
        let inner_pi = doc.create_node(NodeKind::ProcessingInstruction {
            target: "proc".to_string(),
            data: None,
        });
        doc.append_child(elem, inner_pi);

        // Comment after root element
        let comment_after = doc.create_node(NodeKind::Comment {
            content: " epilogue ".to_string(),
        });
        doc.append_child(root, comment_after);

        let result = canonicalize(&doc, &C14nOptions::default());

        // Expected: comment + newline, PI + newline, root element
        // (sorted attrs), content, newline + trailing comment
        assert_eq!(
            result,
            "<!-- prologue comment -->\n\
             <?app start?>\n\
             <root a=\"2\" z=\"1\">helloa&lt;b<!-- inner --><?proc?></root>\n\
             <!-- epilogue -->"
        );
    }

    #[test]
    fn test_c14n_subtree_serialization() {
        // Canonicalize only a subtree of a larger document.
        let doc = Document::parse_str("<root><child attr=\"value\">text</child></root>").unwrap();
        let root_elem = doc.root_element().unwrap();
        let child = doc.first_child(root_elem).unwrap();

        let result = canonicalize_subtree(&doc, child, &C14nOptions::default());
        assert_eq!(result, "<child attr=\"value\">text</child>");
    }

    #[test]
    fn test_c14n_redundant_namespace_not_redeclared() {
        // When a child element inherits a namespace from a parent,
        // C14N should not re-declare it.
        let result = c14n(
            "<root xmlns=\"http://example.com\">\
             <child xmlns=\"http://example.com\"/></root>",
        );
        // The child should not have xmlns re-declared
        assert_eq!(
            result,
            "<root xmlns=\"http://example.com\"><child></child></root>"
        );
    }

    #[test]
    fn test_c14n_mixed_namespace_and_regular_attrs() {
        // Namespace declarations come before regular attributes,
        // sorted by prefix. Regular attributes sorted by ns URI then
        // local name.
        let result =
            c14n("<root xmlns:b=\"http://b\" xmlns:a=\"http://a\" b:y=\"1\" a:x=\"2\" c=\"3\"/>");
        // Namespace decls: xmlns:a, xmlns:b (sorted by prefix)
        // Regular attrs: c (no ns, empty URI), a:x (http://a), b:y (http://b)
        assert_eq!(
            result,
            "<root xmlns:a=\"http://a\" xmlns:b=\"http://b\" c=\"3\" a:x=\"2\" b:y=\"1\"></root>"
        );
    }

    #[test]
    fn test_c14n_pi_without_data() {
        // Processing instruction with no data.
        let result = c14n("<root><?target?></root>");
        assert_eq!(result, "<root><?target?></root>");
    }

    #[test]
    fn test_c14n_nested_elements() {
        // Deeply nested elements all use start-end tags.
        let result = c14n("<a><b><c/></b></a>");
        assert_eq!(result, "<a><b><c></c></b></a>");
    }

    #[test]
    fn test_c14n_exclusive_with_inclusive_prefixes() {
        // Exclusive C14N with additional inclusive prefixes.
        let doc = Document::parse_str(
            "<root xmlns:a=\"http://a\" xmlns:b=\"http://b\">\
             <child/></root>",
        )
        .unwrap();
        let root_elem = doc.root_element().unwrap();
        let child = doc.first_child(root_elem).unwrap();

        let result = canonicalize_subtree(
            &doc,
            child,
            &C14nOptions {
                with_comments: true,
                exclusive: true,
                inclusive_prefixes: vec!["b".to_string()],
            },
        );

        // The "b" prefix is forced via inclusive_prefixes even though
        // it's not visibly utilized on the child element.
        // However, the child element doesn't have the xmlns:b declaration
        // as an attribute, so it won't appear (it's on the parent).
        // This tests the boundary condition.
        assert_eq!(result, "<child></child>");
    }

    #[test]
    fn test_c14n_document_comments_and_pis_spacing() {
        // Comments and PIs before the root element get a trailing newline.
        // Comments and PIs after the root element get a leading newline.
        let mut doc = Document::new();
        let root = doc.root();

        let pi = doc.create_node(NodeKind::ProcessingInstruction {
            target: "before".to_string(),
            data: None,
        });
        doc.append_child(root, pi);

        let elem = doc.create_node(NodeKind::Element {
            name: "root".to_string(),
            prefix: None,
            namespace: None,
            attributes: vec![],
        });
        doc.append_child(root, elem);

        let pi_after = doc.create_node(NodeKind::ProcessingInstruction {
            target: "after".to_string(),
            data: None,
        });
        doc.append_child(root, pi_after);

        let result = canonicalize(&doc, &C14nOptions::default());
        assert_eq!(result, "<?before?>\n<root></root>\n<?after?>");
    }

    #[test]
    fn test_c14n_inclusive_xml_prefix_not_emitted() {
        // XML Namespaces reserves the `xml` prefix as implicitly bound to
        // `http://www.w3.org/XML/1998/namespace`. Canonical XML §2.3 requires
        // that this binding is never emitted as an `xmlns:xml` declaration.
        let result = c14n("<root xml:lang=\"en\">hello</root>");
        assert_eq!(result, "<root xml:lang=\"en\">hello</root>");
        assert!(
            !result.contains("xmlns:xml"),
            "implicit xml namespace should not be emitted, got: {result}"
        );
    }

    #[test]
    fn test_c14n_exclusive_xml_prefix_not_emitted_on_root() {
        // Exclusive C14N §3 is defined as a variant of Canonical XML and
        // inherits the §2.3 rule that suppresses the implicit `xml` prefix
        // binding from canonical output.
        let doc = Document::parse_str("<root xml:lang=\"en\">hello</root>").unwrap();
        let result = canonicalize(
            &doc,
            &C14nOptions {
                with_comments: false,
                exclusive: true,
                inclusive_prefixes: vec![],
            },
        );
        assert_eq!(result, "<root xml:lang=\"en\">hello</root>");
        assert!(!result.contains("xmlns:xml"));
    }

    #[test]
    fn test_c14n_exclusive_xml_prefix_not_emitted_on_subtree() {
        // When canonicalizing a subtree whose ancestor carries `xml:lang`,
        // the subtree must not spuriously declare `xmlns:xml` just because
        // an `xml:` attribute appears in scope.
        let doc = Document::parse_str(
            "<root xmlns=\"http://example.com\" xml:lang=\"en\">\
             <child xml:space=\"preserve\">hi</child></root>",
        )
        .unwrap();
        let root = doc.root_element().unwrap();
        let child = doc
            .children(root)
            .find(|&n| doc.node_name(n) == Some("child"))
            .unwrap();

        let result = canonicalize_subtree(
            &doc,
            child,
            &C14nOptions {
                with_comments: false,
                exclusive: true,
                inclusive_prefixes: vec![],
            },
        );

        assert!(
            !result.contains("xmlns:xml"),
            "implicit xml namespace should not be emitted in exclusive C14N, got: {result}"
        );
        // The default namespace from the root IS visibly utilized by the
        // child element's unprefixed name, so it should appear.
        assert!(result.contains("xmlns=\"http://example.com\""));
        assert!(result.contains("xml:space=\"preserve\""));
    }

    /// W3C Canonical XML §2.3 — when a child element is in no namespace under
    /// a parent that has a non-empty default namespace, the canonical output
    /// must emit `xmlns=""` to undeclare the inherited default. This is the
    /// inclusive-mode baseline; libxml2's `xmllint --c14n` exhibits the same.
    #[test]
    fn test_c14n_inclusive_emits_default_ns_undeclaration() {
        let xml =
            r#"<Envelope xmlns="http://example.org/usps"><NonNs xmlns="">child</NonNs></Envelope>"#;
        let result = c14n(xml);
        assert!(
            result.contains("<NonNs xmlns=\"\">"),
            "default namespace undeclaration missing, got: {result}"
        );
    }

    /// W3C Exclusive C14N §3 inherits Canonical XML's default-namespace
    /// undeclaration rule. Without it, the canonical form leaks the parent's
    /// default namespace into a child that explicitly has none, producing a
    /// digest that diverges from libxml2 / xmlsec.
    #[test]
    fn test_c14n_exclusive_emits_default_ns_undeclaration() {
        let xml =
            r#"<Envelope xmlns="http://example.org/usps"><NonNs xmlns="">child</NonNs></Envelope>"#;
        let doc = Document::parse_str(xml).unwrap();
        let result = canonicalize(
            &doc,
            &C14nOptions {
                with_comments: false,
                exclusive: true,
                inclusive_prefixes: vec![],
            },
        );
        assert!(
            result.contains("<NonNs xmlns=\"\">"),
            "exclusive C14N must emit xmlns=\"\" to undeclare inherited default ns, got: {result}"
        );
    }

    /// Negative companion: when no inherited default exists, the output must
    /// NOT emit a spurious `xmlns=""`.
    #[test]
    fn test_c14n_exclusive_no_undeclaration_when_no_inherited_default() {
        let xml = r"<root><child>x</child></root>";
        let doc = Document::parse_str(xml).unwrap();
        let result = canonicalize(
            &doc,
            &C14nOptions {
                with_comments: false,
                exclusive: true,
                inclusive_prefixes: vec![],
            },
        );
        assert!(
            !result.contains("xmlns=\"\""),
            "exclusive C14N must not emit xmlns=\"\" when no inherited default to undeclare, got: {result}"
        );
    }
}
