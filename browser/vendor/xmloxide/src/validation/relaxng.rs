//! `RelaxNG` schema validation for XML documents.
//!
//! This module implements the `RelaxNG` specification
//! (<https://relaxng.org/spec-20011203.html>) for validating XML documents
//! against `RelaxNG` schemas. `RelaxNG` schemas are themselves XML documents
//! that describe the structure and content of valid XML.
//!
//! # Architecture
//!
//! The implementation is split into three layers:
//!
//! 1. **Data model** ([`Pattern`], [`NameClass`], [`RelaxNgSchema`]) — an
//!    algebraic representation of the schema grammar.
//! 2. **Schema parser** ([`parse_relaxng`]) — reads a `RelaxNG` XML schema
//!    document and produces a `RelaxNgSchema`.
//! 3. **Validator** ([`validate`]) — checks an XML document tree against a
//!    compiled schema using a recursive pattern-matching approach.
//!
//! # Examples
//!
//! ```
//! use xmloxide::Document;
//! use xmloxide::validation::relaxng::{parse_relaxng, validate};
//!
//! let schema_xml = r#"
//!   <element name="greeting" xmlns="http://relaxng.org/ns/structure/1.0">
//!     <text/>
//!   </element>
//! "#;
//!
//! let schema = parse_relaxng(schema_xml).unwrap();
//! let doc = Document::parse_str("<greeting>Hello!</greeting>").unwrap();
//! let result = validate(&doc, &schema);
//! assert!(result.is_valid);
//! ```

use std::collections::HashMap;
use std::fmt;

use crate::tree::{Document, NodeId, NodeKind};
use crate::validation::{ValidationError, ValidationResult};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A `RelaxNG` pattern — the core building block of a schema grammar.
///
/// Patterns form a tree that describes the allowed structure and content
/// of XML documents. The variants correspond to the grammar constructs
/// defined in the `RelaxNG` specification.
///
/// See <https://relaxng.org/spec-20011203.html#section:patterns>.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    /// Matches empty content (no elements, no text).
    Empty,
    /// Matches nothing — always fails. Used as an identity element for choice.
    NotAllowed,
    /// Matches arbitrary text content.
    Text,
    /// Matches an element whose name satisfies the [`NameClass`] and whose
    /// content matches the inner pattern.
    Element {
        /// Name constraint for the element.
        name: NameClass,
        /// Pattern that the element's content must match.
        pattern: Box<Pattern>,
    },
    /// Matches an attribute whose name satisfies the [`NameClass`] and whose
    /// value matches the inner pattern.
    Attribute {
        /// Name constraint for the attribute.
        name: NameClass,
        /// Pattern that the attribute value must match.
        pattern: Box<Pattern>,
    },
    /// Sequential composition — first pattern then second pattern.
    Group(Box<Pattern>, Box<Pattern>),
    /// Interleave — both patterns must match but in any order.
    Interleave(Box<Pattern>, Box<Pattern>),
    /// Choice — one of the two patterns must match.
    Choice(Box<Pattern>, Box<Pattern>),
    /// Optional — zero or one occurrence.
    Optional(Box<Pattern>),
    /// Zero or more occurrences.
    ZeroOrMore(Box<Pattern>),
    /// One or more occurrences.
    OneOrMore(Box<Pattern>),
    /// A named reference to a `<define>` block in the grammar.
    Ref(String),
    /// Matches a whitespace-separated list of tokens against the inner pattern.
    List(Box<Pattern>),
    /// Matches an exact string value.
    Value(String),
    /// Matches a value against a named datatype from a datatype library.
    Data {
        /// The datatype name (e.g., `"integer"`, `"string"`).
        datatype: String,
        /// The datatype library URI (e.g., the XML Schema datatypes namespace).
        library: String,
    },
    /// Matches a mixed content model (interleave of text and a pattern).
    Mixed(Box<Pattern>),
}

impl fmt::Display for Pattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty"),
            Self::NotAllowed => write!(f, "notAllowed"),
            Self::Text => write!(f, "text"),
            Self::Element { name, .. } => write!(f, "element {name}"),
            Self::Attribute { name, .. } => write!(f, "attribute {name}"),
            Self::Group(a, b) => write!(f, "group({a}, {b})"),
            Self::Interleave(a, b) => write!(f, "interleave({a}, {b})"),
            Self::Choice(a, b) => write!(f, "choice({a}, {b})"),
            Self::Optional(p) => write!(f, "optional({p})"),
            Self::ZeroOrMore(p) => write!(f, "zeroOrMore({p})"),
            Self::OneOrMore(p) => write!(f, "oneOrMore({p})"),
            Self::Ref(name) => write!(f, "ref({name})"),
            Self::List(p) => write!(f, "list({p})"),
            Self::Value(v) => write!(f, "value(\"{v}\")"),
            Self::Data { datatype, .. } => write!(f, "data({datatype})"),
            Self::Mixed(p) => write!(f, "mixed({p})"),
        }
    }
}

/// A name class — constrains which element or attribute names are allowed.
///
/// Name classes can match specific names, any name in a namespace,
/// any name at all, or combinations via choice and exclusion.
///
/// See <https://relaxng.org/spec-20011203.html#section:name-classes>.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameClass {
    /// Matches a specific name (namespace URI + local name).
    Name {
        /// The namespace URI (empty string for no namespace).
        ns: String,
        /// The local name of the element or attribute.
        local: String,
    },
    /// Matches any name regardless of namespace.
    AnyName,
    /// Matches any name except those matching the excluded name class.
    AnyNameExcept(Box<NameClass>),
    /// Matches any name in the given namespace.
    NsName {
        /// The namespace URI to match.
        ns: String,
    },
    /// Matches any name in the given namespace except those matching the
    /// excluded name class.
    NsNameExcept {
        /// The namespace URI to match.
        ns: String,
        /// Names to exclude.
        except: Box<NameClass>,
    },
    /// Choice of two name classes — matches if either matches.
    Choice(Box<NameClass>, Box<NameClass>),
}

impl NameClass {
    /// Tests whether this name class matches the given namespace and local name.
    #[must_use]
    pub fn matches(&self, ns: &str, local: &str) -> bool {
        match self {
            Self::Name {
                ns: expected_ns,
                local: expected_local,
            } => expected_ns == ns && expected_local == local,
            Self::AnyName => true,
            Self::AnyNameExcept(except) => !except.matches(ns, local),
            Self::NsName { ns: expected_ns } => expected_ns == ns,
            Self::NsNameExcept {
                ns: expected_ns,
                except,
            } => expected_ns == ns && !except.matches(ns, local),
            Self::Choice(a, b) => a.matches(ns, local) || b.matches(ns, local),
        }
    }
}

impl fmt::Display for NameClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name { ns, local } => {
                if ns.is_empty() {
                    write!(f, "{local}")
                } else {
                    write!(f, "{{{ns}}}{local}")
                }
            }
            Self::AnyName => write!(f, "*"),
            Self::AnyNameExcept(except) => write!(f, "* - {except}"),
            Self::NsName { ns } => write!(f, "{{{ns}}}*"),
            Self::NsNameExcept { ns, except } => write!(f, "{{{ns}}}* - {except}"),
            Self::Choice(a, b) => write!(f, "{a} | {b}"),
        }
    }
}

/// A compiled `RelaxNG` schema ready for validation.
///
/// Contains the start pattern (the entry point for validation) and a map
/// of named definitions that can be referenced via `Ref` patterns.
#[derive(Debug, Clone)]
pub struct RelaxNgSchema {
    /// The start pattern — the root document must match this.
    pub start: Pattern,
    /// Named definitions (`<define name="...">` blocks).
    pub defines: HashMap<String, Pattern>,
}

// ---------------------------------------------------------------------------
// Schema parsing errors
// ---------------------------------------------------------------------------

/// Error type for schema parsing failures.
#[derive(Debug, Clone)]
pub struct SchemaParseError {
    /// Human-readable description of what went wrong.
    pub message: String,
}

impl fmt::Display for SchemaParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RelaxNG schema error: {}", self.message)
    }
}

impl std::error::Error for SchemaParseError {}

// ---------------------------------------------------------------------------
// Schema parser
// ---------------------------------------------------------------------------

/// Parses a `RelaxNG` XML schema string into a [`RelaxNgSchema`].
///
/// The input must be a valid XML document conforming to the `RelaxNG` XML
/// syntax (<https://relaxng.org/spec-20011203.html>). Both compact-element
/// form (e.g., `<element name="foo">`) and verbose form with child `<name>`
/// elements are supported.
///
/// # Errors
///
/// Returns [`SchemaParseError`] if the input is not well-formed XML or does
/// not conform to the expected `RelaxNG` structure.
///
/// # Examples
///
/// ```
/// use xmloxide::validation::relaxng::parse_relaxng;
///
/// let schema = parse_relaxng(r#"
///   <element name="root" xmlns="http://relaxng.org/ns/structure/1.0">
///     <empty/>
///   </element>
/// "#).unwrap();
/// ```
pub fn parse_relaxng(schema_xml: &str) -> Result<RelaxNgSchema, SchemaParseError> {
    let doc = Document::parse_str(schema_xml).map_err(|e| SchemaParseError {
        message: format!("failed to parse schema XML: {e}"),
    })?;

    let root_el = doc.root_element().ok_or_else(|| SchemaParseError {
        message: "schema has no root element".to_string(),
    })?;

    let root_name = doc.node_name(root_el).unwrap_or("");

    // Determine default namespace for elements from the `ns` attribute
    // on the root schema element.
    let default_ns = element_ns_attr(&doc, root_el);

    if strip_rng_prefix(root_name) == "grammar" {
        parse_grammar(&doc, root_el, &default_ns)
    } else if strip_rng_prefix(root_name) == "element" {
        // Top-level element pattern (short form, no grammar wrapper).
        let pattern = parse_element_pattern(&doc, root_el, &default_ns)?;
        Ok(RelaxNgSchema {
            start: pattern,
            defines: HashMap::new(),
        })
    } else {
        Err(SchemaParseError {
            message: format!("expected <grammar> or <element> as root, found <{root_name}>"),
        })
    }
}

/// Strips a `rng:` prefix from a name, if present, returning the local part.
fn strip_rng_prefix(name: &str) -> &str {
    name.strip_prefix("rng:").unwrap_or(name)
}

/// Returns the value of the `ns` attribute on an element, defaulting to
/// the empty string.
fn element_ns_attr(doc: &Document, node: NodeId) -> String {
    doc.attribute(node, "ns").unwrap_or("").to_string()
}

/// Parses a `<grammar>` element containing `<start>` and `<define>` children.
fn parse_grammar(
    doc: &Document,
    grammar_el: NodeId,
    parent_ns: &str,
) -> Result<RelaxNgSchema, SchemaParseError> {
    let mut start: Option<Pattern> = None;
    let mut defines: HashMap<String, Pattern> = HashMap::new();

    let ns = resolve_ns(doc, grammar_el, parent_ns);

    for child in doc.children(grammar_el) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let child_name = doc.node_name(child).unwrap_or("");
        let local = strip_rng_prefix(child_name);

        match local {
            "start" => {
                let inner = parse_pattern_children(doc, child, &ns)?;
                start = Some(inner);
            }
            "define" => {
                let name = doc
                    .attribute(child, "name")
                    .ok_or_else(|| SchemaParseError {
                        message: "<define> missing 'name' attribute".to_string(),
                    })?
                    .to_string();
                let inner = parse_pattern_children(doc, child, &ns)?;
                defines.insert(name, inner);
            }
            _ => {
                // Ignore unknown elements (includes, divs, etc. — not yet
                // implemented).
            }
        }
    }

    let start = start.ok_or_else(|| SchemaParseError {
        message: "<grammar> has no <start> element".to_string(),
    })?;

    Ok(RelaxNgSchema { start, defines })
}

/// Resolves the effective namespace for pattern children. Uses the `ns`
/// attribute on the current element if present, otherwise inherits.
fn resolve_ns(doc: &Document, el: NodeId, parent_ns: &str) -> String {
    doc.attribute(el, "ns")
        .map_or_else(|| parent_ns.to_string(), String::from)
}

/// Parses the child patterns of a container element (e.g., `<start>`,
/// `<group>`, `<choice>`). If there are multiple children, they are
/// implicitly grouped.
fn parse_pattern_children(
    doc: &Document,
    container: NodeId,
    ns: &str,
) -> Result<Pattern, SchemaParseError> {
    let patterns = collect_child_patterns(doc, container, ns)?;
    combine_patterns(patterns)
}

/// Collects all child pattern elements from a container node.
fn collect_child_patterns(
    doc: &Document,
    container: NodeId,
    ns: &str,
) -> Result<Vec<Pattern>, SchemaParseError> {
    let mut patterns = Vec::new();
    for child in doc.children(container) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let p = parse_pattern(doc, child, ns)?;
        patterns.push(p);
    }
    Ok(patterns)
}

/// Combines a list of patterns using implicit `Group` (sequential composition).
fn combine_patterns(patterns: Vec<Pattern>) -> Result<Pattern, SchemaParseError> {
    if patterns.is_empty() {
        return Ok(Pattern::Empty);
    }

    let mut iter = patterns.into_iter();
    let first = iter.next().ok_or_else(|| SchemaParseError {
        message: "internal error: empty pattern list".to_string(),
    })?;

    Ok(iter.fold(first, |acc, p| Pattern::Group(Box::new(acc), Box::new(p))))
}

/// Parses a single pattern element.
fn parse_pattern(doc: &Document, el: NodeId, parent_ns: &str) -> Result<Pattern, SchemaParseError> {
    let name = doc.node_name(el).unwrap_or("");
    let local = strip_rng_prefix(name);
    let ns = resolve_ns(doc, el, parent_ns);

    match local {
        "element" => parse_element_pattern(doc, el, &ns),
        "attribute" => parse_attribute_pattern(doc, el, &ns),
        "group" => {
            let children = collect_child_patterns(doc, el, &ns)?;
            combine_patterns(children)
        }
        "interleave" => {
            let children = collect_child_patterns(doc, el, &ns)?;
            combine_interleave(children)
        }
        "choice" => {
            let children = collect_child_patterns(doc, el, &ns)?;
            combine_choice(children)
        }
        "optional" => {
            let inner = parse_pattern_children(doc, el, &ns)?;
            Ok(Pattern::Optional(Box::new(inner)))
        }
        "zeroOrMore" => {
            let inner = parse_pattern_children(doc, el, &ns)?;
            Ok(Pattern::ZeroOrMore(Box::new(inner)))
        }
        "oneOrMore" => {
            let inner = parse_pattern_children(doc, el, &ns)?;
            Ok(Pattern::OneOrMore(Box::new(inner)))
        }
        "mixed" => {
            let inner = parse_pattern_children(doc, el, &ns)?;
            Ok(Pattern::Mixed(Box::new(inner)))
        }
        "ref" => {
            let ref_name = doc
                .attribute(el, "name")
                .ok_or_else(|| SchemaParseError {
                    message: "<ref> missing 'name' attribute".to_string(),
                })?
                .to_string();
            Ok(Pattern::Ref(ref_name))
        }
        "text" => Ok(Pattern::Text),
        "empty" => Ok(Pattern::Empty),
        "notAllowed" => Ok(Pattern::NotAllowed),
        "value" => {
            let text = doc.text_content(el);
            Ok(Pattern::Value(text))
        }
        "data" => {
            let datatype = doc.attribute(el, "type").unwrap_or("string").to_string();
            let library = doc
                .attribute(el, "datatypeLibrary")
                .unwrap_or("")
                .to_string();
            Ok(Pattern::Data { datatype, library })
        }
        "list" => {
            let inner = parse_pattern_children(doc, el, &ns)?;
            Ok(Pattern::List(Box::new(inner)))
        }
        other => Err(SchemaParseError {
            message: format!("unknown pattern element <{other}>"),
        }),
    }
}

/// Parses an `<element>` pattern, extracting the name class and content pattern.
fn parse_element_pattern(
    doc: &Document,
    el: NodeId,
    ns: &str,
) -> Result<Pattern, SchemaParseError> {
    let name_class = parse_name_class_from_element(doc, el, ns)?;
    let el_ns = resolve_ns(doc, el, ns);

    // Collect content patterns (everything except <name>, <anyName>, <nsName>,
    // <choice> when used as name class).
    let mut content_patterns = Vec::new();
    for child in doc.children(el) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let child_name = doc.node_name(child).unwrap_or("");
        let child_local = strip_rng_prefix(child_name);
        // Skip name-class children — they're handled by parse_name_class.
        if is_name_class_element(child_local) && doc.attribute(el, "name").is_none() {
            continue;
        }
        let p = parse_pattern(doc, child, &el_ns)?;
        content_patterns.push(p);
    }

    let content = combine_patterns(content_patterns)?;

    Ok(Pattern::Element {
        name: name_class,
        pattern: Box::new(content),
    })
}

/// Parses an `<attribute>` pattern, extracting the name class and value pattern.
fn parse_attribute_pattern(
    doc: &Document,
    el: NodeId,
    ns: &str,
) -> Result<Pattern, SchemaParseError> {
    let name_class = parse_name_class_from_element(doc, el, ns)?;
    let attr_ns = resolve_ns(doc, el, ns);

    let mut content_patterns = Vec::new();
    for child in doc.children(el) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let child_name = doc.node_name(child).unwrap_or("");
        let child_local = strip_rng_prefix(child_name);
        if is_name_class_element(child_local) && doc.attribute(el, "name").is_none() {
            continue;
        }
        let p = parse_pattern(doc, child, &attr_ns)?;
        content_patterns.push(p);
    }

    let content = if content_patterns.is_empty() {
        Pattern::Text // default: attribute value is text
    } else {
        combine_patterns(content_patterns)?
    };

    Ok(Pattern::Attribute {
        name: name_class,
        pattern: Box::new(content),
    })
}

/// Determines whether an element local name is a name-class element.
fn is_name_class_element(local: &str) -> bool {
    matches!(local, "name" | "anyName" | "nsName" | "choice")
}

/// Extracts a [`NameClass`] from an element or attribute pattern element.
///
/// If the element has a `name` attribute, uses that directly. Otherwise,
/// looks for a child `<name>`, `<anyName>`, or `<nsName>` element.
fn parse_name_class_from_element(
    doc: &Document,
    el: NodeId,
    ns: &str,
) -> Result<NameClass, SchemaParseError> {
    // Check for `name` attribute shorthand.
    if let Some(name_attr) = doc.attribute(el, "name") {
        let el_ns = resolve_ns(doc, el, ns);
        return Ok(NameClass::Name {
            ns: el_ns,
            local: name_attr.to_string(),
        });
    }

    // Look for a name-class child element.
    for child in doc.children(el) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let child_name = doc.node_name(child).unwrap_or("");
        let child_local = strip_rng_prefix(child_name);
        match child_local {
            "name" => {
                let local_name = doc.text_content(child);
                let child_ns = resolve_ns(doc, child, ns);
                return Ok(NameClass::Name {
                    ns: child_ns,
                    local: local_name.trim().to_string(),
                });
            }
            "anyName" => {
                return parse_any_name_class(doc, child, ns);
            }
            "nsName" => {
                return parse_ns_name_class(doc, child, ns);
            }
            "choice" => {
                return parse_name_class_choice(doc, child, ns);
            }
            _ => {}
        }
    }

    Err(SchemaParseError {
        message: "element/attribute pattern has no name or name class".to_string(),
    })
}

/// Parses an `<anyName>` name class, possibly with `<except>`.
fn parse_any_name_class(
    doc: &Document,
    el: NodeId,
    ns: &str,
) -> Result<NameClass, SchemaParseError> {
    for child in doc.children(el) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let child_name = doc.node_name(child).unwrap_or("");
        if strip_rng_prefix(child_name) == "except" {
            let except = parse_name_class_children(doc, child, ns)?;
            return Ok(NameClass::AnyNameExcept(Box::new(except)));
        }
    }
    Ok(NameClass::AnyName)
}

/// Parses an `<nsName>` name class, possibly with `<except>`.
fn parse_ns_name_class(
    doc: &Document,
    el: NodeId,
    ns: &str,
) -> Result<NameClass, SchemaParseError> {
    let target_ns = resolve_ns(doc, el, ns);
    for child in doc.children(el) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let child_name = doc.node_name(child).unwrap_or("");
        if strip_rng_prefix(child_name) == "except" {
            let except = parse_name_class_children(doc, child, ns)?;
            return Ok(NameClass::NsNameExcept {
                ns: target_ns,
                except: Box::new(except),
            });
        }
    }
    Ok(NameClass::NsName { ns: target_ns })
}

/// Parses a `<choice>` element used as a name class.
fn parse_name_class_choice(
    doc: &Document,
    el: NodeId,
    ns: &str,
) -> Result<NameClass, SchemaParseError> {
    let mut classes = Vec::new();
    for child in doc.children(el) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let child_name = doc.node_name(child).unwrap_or("");
        let child_local = strip_rng_prefix(child_name);
        let nc = match child_local {
            "name" => {
                let local_name = doc.text_content(child).trim().to_string();
                let child_ns = resolve_ns(doc, child, ns);
                NameClass::Name {
                    ns: child_ns,
                    local: local_name,
                }
            }
            "anyName" => parse_any_name_class(doc, child, ns)?,
            "nsName" => parse_ns_name_class(doc, child, ns)?,
            "choice" => parse_name_class_choice(doc, child, ns)?,
            _ => {
                continue;
            }
        };
        classes.push(nc);
    }
    combine_name_classes(classes)
}

/// Parses name class children inside an `<except>` or similar container.
fn parse_name_class_children(
    doc: &Document,
    container: NodeId,
    ns: &str,
) -> Result<NameClass, SchemaParseError> {
    let mut classes = Vec::new();
    for child in doc.children(container) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let child_name = doc.node_name(child).unwrap_or("");
        let child_local = strip_rng_prefix(child_name);
        let nc = match child_local {
            "name" => {
                let local_name = doc.text_content(child).trim().to_string();
                let child_ns = resolve_ns(doc, child, ns);
                NameClass::Name {
                    ns: child_ns,
                    local: local_name,
                }
            }
            "anyName" => parse_any_name_class(doc, child, ns)?,
            "nsName" => parse_ns_name_class(doc, child, ns)?,
            "choice" => parse_name_class_choice(doc, child, ns)?,
            _ => {
                continue;
            }
        };
        classes.push(nc);
    }
    combine_name_classes(classes)
}

/// Combines multiple name classes using `Choice`.
fn combine_name_classes(classes: Vec<NameClass>) -> Result<NameClass, SchemaParseError> {
    if classes.is_empty() {
        return Err(SchemaParseError {
            message: "empty name class".to_string(),
        });
    }
    let mut iter = classes.into_iter();
    let first = iter.next().ok_or_else(|| SchemaParseError {
        message: "internal error: empty name class list".to_string(),
    })?;
    Ok(iter.fold(first, |acc, nc| {
        NameClass::Choice(Box::new(acc), Box::new(nc))
    }))
}

/// Combines patterns using `Interleave`.
fn combine_interleave(patterns: Vec<Pattern>) -> Result<Pattern, SchemaParseError> {
    if patterns.is_empty() {
        return Ok(Pattern::Empty);
    }
    let mut iter = patterns.into_iter();
    let first = iter.next().ok_or_else(|| SchemaParseError {
        message: "internal error: empty interleave list".to_string(),
    })?;
    Ok(iter.fold(first, |acc, p| {
        Pattern::Interleave(Box::new(acc), Box::new(p))
    }))
}

/// Combines patterns using `Choice`.
fn combine_choice(patterns: Vec<Pattern>) -> Result<Pattern, SchemaParseError> {
    if patterns.is_empty() {
        return Ok(Pattern::NotAllowed);
    }
    let mut iter = patterns.into_iter();
    let first = iter.next().ok_or_else(|| SchemaParseError {
        message: "internal error: empty choice list".to_string(),
    })?;
    Ok(iter.fold(first, |acc, p| Pattern::Choice(Box::new(acc), Box::new(p))))
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validates an XML document against a compiled [`RelaxNgSchema`].
///
/// The validator checks that the document's root element matches the
/// schema's start pattern, recursively verifying element names, attributes,
/// text content, and structural constraints.
///
/// # Examples
///
/// ```
/// use xmloxide::Document;
/// use xmloxide::validation::relaxng::{parse_relaxng, validate};
///
/// let schema = parse_relaxng(r#"
///   <element name="root" xmlns="http://relaxng.org/ns/structure/1.0">
///     <empty/>
///   </element>
/// "#).unwrap();
///
/// let doc = Document::parse_str("<root/>").unwrap();
/// let result = validate(&doc, &schema);
/// assert!(result.is_valid);
/// ```
#[must_use]
pub fn validate(doc: &Document, schema: &RelaxNgSchema) -> ValidationResult {
    let mut errors = Vec::new();

    let Some(root_el) = doc.root_element() else {
        return ValidationResult {
            is_valid: false,
            errors: vec![ValidationError {
                message: "document has no root element".to_string(),
                line: None,
                column: None,
            }],
            warnings: Vec::new(),
        };
    };

    let ctx = ValidationContext {
        doc,
        defines: &schema.defines,
    };

    let ok = ctx.validate_node(root_el, &schema.start, &mut errors);

    ValidationResult {
        is_valid: ok && errors.is_empty(),
        errors,
        warnings: Vec::new(),
    }
}

/// Internal validation context — carries references to the document and schema.
struct ValidationContext<'a> {
    doc: &'a Document,
    defines: &'a HashMap<String, Pattern>,
}

impl ValidationContext<'_> {
    /// Validates a node against a pattern. Returns `true` if the node matches.
    fn validate_node(
        &self,
        node: NodeId,
        pattern: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        match pattern {
            Pattern::Element {
                name,
                pattern: inner,
            } => self.validate_element(node, name, inner, errors),
            Pattern::Choice(a, b) => {
                // Try the first alternative silently; if it fails, try the second.
                let mut a_errors = Vec::new();
                if self.validate_node(node, a, &mut a_errors) {
                    return true;
                }
                let mut b_errors = Vec::new();
                if self.validate_node(node, b, &mut b_errors) {
                    return true;
                }
                // Both failed — report the second branch errors (usually more
                // informative for the "expected" case).
                errors.extend(b_errors);
                false
            }
            Pattern::Ref(ref_name) => {
                if let Some(def) = self.defines.get(ref_name) {
                    self.validate_node(node, def, errors)
                } else {
                    errors.push(ValidationError {
                        message: format!("undefined reference: {ref_name}"),
                        line: None,
                        column: None,
                    });
                    false
                }
            }
            _ => {
                // For top-level validation, only Element patterns make sense
                // as the start. If a non-element pattern appears at the root,
                // it means the schema is unusual.
                errors.push(ValidationError {
                    message: format!(
                        "expected document root to match {pattern}, \
                         but root validation requires an element pattern"
                    ),
                    line: None,
                    column: None,
                });
                false
            }
        }
    }

    /// Validates an element node against an element pattern.
    fn validate_element(
        &self,
        node: NodeId,
        name_class: &NameClass,
        content_pattern: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        let node_kind = &self.doc.node(node).kind;

        // Ensure the node is actually an element.
        let (el_name, el_ns) = if let NodeKind::Element {
            name, namespace, ..
        } = node_kind
        {
            (name.as_str(), namespace.as_deref().unwrap_or(""))
        } else {
            errors.push(ValidationError {
                message: "expected element node".to_string(),
                line: None,
                column: None,
            });
            return false;
        };

        // Check name.
        if !name_class.matches(el_ns, el_name) {
            errors.push(ValidationError {
                message: format!(
                    "element name mismatch: found <{el_name}>, \
                     expected {name_class}"
                ),
                line: None,
                column: None,
            });
            return false;
        }

        // Validate content (attributes + children).
        self.validate_content(node, content_pattern, errors)
    }

    /// Validates the content of an element (attributes and child nodes)
    /// against a content pattern.
    fn validate_content(
        &self,
        element: NodeId,
        pattern: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        let attrs = self.doc.attributes(element);
        let children: Vec<NodeId> = self.doc.children(element).collect();

        // Separate attribute patterns from child-content patterns.
        let (attr_patterns, content_pattern) = split_attributes(pattern);

        // Validate attributes.
        let mut attr_ok = true;
        let mut matched_attrs: Vec<bool> = vec![false; attrs.len()];

        for ap in &attr_patterns {
            if let Pattern::Attribute {
                name: name_class,
                pattern: value_pattern,
            } = ap
            {
                let found = self.validate_attribute(
                    attrs,
                    &mut matched_attrs,
                    name_class,
                    value_pattern,
                    errors,
                );
                if !found {
                    attr_ok = false;
                }
            } else if let Pattern::Optional(inner) = ap {
                if let Pattern::Attribute {
                    name: name_class,
                    pattern: value_pattern,
                } = inner.as_ref()
                {
                    // Optional attribute: try to match but don't report error
                    // if missing.
                    let mut tmp_errors = Vec::new();
                    let _ = self.validate_attribute(
                        attrs,
                        &mut matched_attrs,
                        name_class,
                        value_pattern,
                        &mut tmp_errors,
                    );
                    // Ignore "missing" errors for optional attributes but
                    // keep value-mismatch errors.
                    for err in tmp_errors {
                        if !err.message.contains("missing required") {
                            errors.push(err);
                            attr_ok = false;
                        }
                    }
                }
            }
        }

        // Check for unmatched (unexpected) attributes — but only if we had
        // attribute patterns. If the content pattern is AnyName-style, we
        // skip this check.
        if !attr_patterns.is_empty() || !has_wildcard_attribute(pattern) {
            for (i, attr) in attrs.iter().enumerate() {
                if !matched_attrs[i] && !is_xmlns_attribute(attr) {
                    errors.push(ValidationError {
                        message: format!(
                            "unexpected attribute '{}' on element '{}'",
                            attr.name,
                            self.doc.node_name(element).unwrap_or("<unknown>")
                        ),
                        line: None,
                        column: None,
                    });
                    attr_ok = false;
                }
            }
        }

        // Validate child content.
        let content_ok = self.validate_children(&children, &content_pattern, 0, errors);

        attr_ok && content_ok
    }

    /// Checks if a specific attribute is present and its value matches.
    fn validate_attribute(
        &self,
        attrs: &[crate::tree::Attribute],
        matched: &mut [bool],
        name_class: &NameClass,
        value_pattern: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        for (i, attr) in attrs.iter().enumerate() {
            if matched[i] {
                continue;
            }
            let attr_ns = attr.namespace.as_deref().unwrap_or("");
            if name_class.matches(attr_ns, &attr.name) {
                matched[i] = true;
                return self.validate_attribute_value(
                    &attr.value,
                    &attr.name,
                    value_pattern,
                    errors,
                );
            }
        }

        // Attribute not found.
        errors.push(ValidationError {
            message: format!("missing required attribute {name_class}"),
            line: None,
            column: None,
        });
        false
    }

    /// Validates an attribute value against a pattern.
    fn validate_attribute_value(
        &self,
        value: &str,
        attr_name: &str,
        pattern: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        match pattern {
            Pattern::Value(expected) => {
                if value == expected {
                    true
                } else {
                    errors.push(ValidationError {
                        message: format!(
                            "attribute '{attr_name}' has value \"{value}\", \
                             expected \"{expected}\""
                        ),
                        line: None,
                        column: None,
                    });
                    false
                }
            }
            Pattern::Choice(a, b) => {
                let mut tmp = Vec::new();
                if self.validate_attribute_value(value, attr_name, a, &mut tmp) {
                    return true;
                }
                self.validate_attribute_value(value, attr_name, b, errors)
            }
            Pattern::Data { datatype, library } => {
                validate_datatype(value, attr_name, datatype, library, errors)
            }
            Pattern::List(inner) => {
                let tokens: Vec<&str> = value.split_whitespace().collect();
                self.validate_list_tokens(&tokens, attr_name, inner, errors)
            }
            Pattern::Ref(ref_name) => {
                if let Some(def) = self.defines.get(ref_name) {
                    self.validate_attribute_value(value, attr_name, def, errors)
                } else {
                    errors.push(ValidationError {
                        message: format!("undefined reference: {ref_name}"),
                        line: None,
                        column: None,
                    });
                    false
                }
            }
            _ => true, // Be permissive for patterns we don't specifically handle.
        }
    }

    /// Validates a list of whitespace-separated tokens against a pattern.
    fn validate_list_tokens(
        &self,
        tokens: &[&str],
        attr_name: &str,
        pattern: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        match pattern {
            Pattern::OneOrMore(inner) => {
                if tokens.is_empty() {
                    errors.push(ValidationError {
                        message: format!(
                            "attribute '{attr_name}' list must have \
                             at least one token"
                        ),
                        line: None,
                        column: None,
                    });
                    return false;
                }
                tokens
                    .iter()
                    .all(|t| self.validate_attribute_value(t, attr_name, inner, errors))
            }
            Pattern::ZeroOrMore(inner) => tokens
                .iter()
                .all(|t| self.validate_attribute_value(t, attr_name, inner, errors)),
            _ => {
                // Single-token list: validate the first token.
                if let Some(t) = tokens.first() {
                    self.validate_attribute_value(t, attr_name, pattern, errors)
                } else {
                    true
                }
            }
        }
    }

    /// Validates child nodes against a content pattern.
    ///
    /// Returns `true` if the children from `start` onwards match the pattern.
    fn validate_children(
        &self,
        children: &[NodeId],
        pattern: &Pattern,
        start: usize,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        // Filter to significant children (elements and non-whitespace text).
        let significant: Vec<(usize, NodeId)> = children[start..]
            .iter()
            .enumerate()
            .filter(|(_, &id)| match &self.doc.node(id).kind {
                NodeKind::Text { content } => !content.trim().is_empty(),
                NodeKind::Element { .. } | NodeKind::CData { .. } => true,
                _ => false, // Skip comments, PIs
            })
            .map(|(i, &id)| (start + i, id))
            .collect();

        self.match_children(&significant, 0, pattern, errors)
    }

    /// Recursive child-pattern matcher. Returns `true` if the significant
    /// children from `pos` onwards match the given pattern.
    #[allow(clippy::too_many_lines)]
    fn match_children(
        &self,
        children: &[(usize, NodeId)],
        pos: usize,
        pattern: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        match pattern {
            Pattern::Empty => self.match_empty(children, pos, errors),
            Pattern::NotAllowed => {
                errors.push(ValidationError {
                    message: "content is not allowed here".to_string(),
                    line: None,
                    column: None,
                });
                false
            }
            Pattern::Text => self.match_text(children, pos, errors),
            Pattern::Element {
                name,
                pattern: inner,
            } => self.match_element_child(children, pos, name, inner, errors),
            Pattern::Group(a, b) => self.match_group(children, pos, a, b, errors),
            Pattern::Choice(a, b) => {
                let mut a_errors = Vec::new();
                if self.match_children(children, pos, a, &mut a_errors) {
                    return true;
                }
                let mut b_errors = Vec::new();
                if self.match_children(children, pos, b, &mut b_errors) {
                    return true;
                }
                errors.extend(b_errors);
                false
            }
            Pattern::Optional(inner) => {
                let mut tmp = Vec::new();
                if self.match_children(children, pos, inner, &mut tmp) {
                    return true;
                }
                // Optional: also allow zero matches.
                self.match_children(children, pos, &Pattern::Empty, errors)
            }
            Pattern::ZeroOrMore(inner) => self.match_zero_or_more(children, pos, inner, errors),
            Pattern::OneOrMore(inner) => self.match_one_or_more(children, pos, inner, errors),
            Pattern::Interleave(a, b) => self.match_interleave(children, pos, a, b, errors),
            Pattern::Mixed(inner) => {
                let elements: Vec<(usize, NodeId)> = children[pos..]
                    .iter()
                    .filter(|(_, id)| matches!(self.doc.node(*id).kind, NodeKind::Element { .. }))
                    .copied()
                    .collect();
                self.match_children(&elements, 0, inner, errors)
            }
            Pattern::Value(expected) => {
                let text = self.collect_children_text(children, pos);
                if text.trim() == expected.trim() {
                    true
                } else {
                    errors.push(ValidationError {
                        message: format!("expected value \"{expected}\", found \"{text}\""),
                        line: None,
                        column: None,
                    });
                    false
                }
            }
            Pattern::Data { datatype, library } => {
                let text = self.collect_children_text(children, pos);
                validate_datatype(text.trim(), "<text>", datatype, library, errors)
            }
            Pattern::List(inner) => {
                let text = self.collect_children_text(children, pos);
                let tokens: Vec<&str> = text.split_whitespace().collect();
                self.validate_list_tokens(&tokens, "<text>", inner, errors)
            }
            Pattern::Ref(ref_name) => {
                if let Some(def) = self.defines.get(ref_name) {
                    self.match_children(children, pos, def, errors)
                } else {
                    errors.push(ValidationError {
                        message: format!("undefined reference: {ref_name}"),
                        line: None,
                        column: None,
                    });
                    false
                }
            }
            Pattern::Attribute { .. } => {
                // Attribute patterns in content position are already handled
                // by the attribute validation pass. They match empty content.
                pos >= children.len()
            }
        }
    }

    /// Matches an `Empty` pattern against remaining children.
    fn match_empty(
        &self,
        children: &[(usize, NodeId)],
        pos: usize,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        if pos >= children.len() {
            return true;
        }
        for &(_, id) in &children[pos..] {
            if let NodeKind::Text { content } = &self.doc.node(id).kind {
                if content.trim().is_empty() {
                    continue;
                }
            }
            let desc = self.node_description(id);
            errors.push(ValidationError {
                message: format!("unexpected content: {desc} (expected empty)"),
                line: None,
                column: None,
            });
            return false;
        }
        true
    }

    /// Matches a `Text` pattern against remaining children.
    fn match_text(
        &self,
        children: &[(usize, NodeId)],
        pos: usize,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        for &(_, id) in &children[pos..] {
            match &self.doc.node(id).kind {
                NodeKind::Text { .. } | NodeKind::CData { .. } => {}
                _ => {
                    let desc = self.node_description(id);
                    errors.push(ValidationError {
                        message: format!("unexpected {desc} (expected text)"),
                        line: None,
                        column: None,
                    });
                    return false;
                }
            }
        }
        true
    }

    /// Matches an element child pattern at a given position.
    fn match_element_child(
        &self,
        children: &[(usize, NodeId)],
        pos: usize,
        name: &NameClass,
        inner: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        if pos >= children.len() {
            errors.push(ValidationError {
                message: format!("missing required element {name}"),
                line: None,
                column: None,
            });
            return false;
        }
        let (_, child_id) = children[pos];
        if !self.validate_element(child_id, name, inner, errors) {
            return false;
        }
        // Ensure no more children after this element.
        if pos + 1 < children.len() {
            for &(_, id) in &children[pos + 1..] {
                let desc = self.node_description(id);
                errors.push(ValidationError {
                    message: format!("unexpected content after element: {desc}"),
                    line: None,
                    column: None,
                });
            }
            return false;
        }
        true
    }

    /// Matches a group pattern (sequential). Tries to find a split point
    /// where the first pattern matches `children[pos..split]` and the second
    /// matches `children[split..]`.
    fn match_group(
        &self,
        children: &[(usize, NodeId)],
        pos: usize,
        a: &Pattern,
        b: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        // Try all possible split points.
        for split in pos..=children.len() {
            let slice_a = &children[..split];
            let mut a_errors = Vec::new();
            if self.match_children(slice_a, pos, a, &mut a_errors) {
                let mut b_errors = Vec::new();
                if self.match_children(children, split, b, &mut b_errors) {
                    return true;
                }
            }
        }

        // None of the split points worked. Generate an error.
        let mut a_errors = Vec::new();
        if self.match_children(children, pos, a, &mut a_errors) {
            // `a` matched some prefix but `b` couldn't match the rest.
            let mut b_errors = Vec::new();
            let _ = self.match_children(children, children.len(), b, &mut b_errors);
            errors.extend(b_errors);
        } else {
            errors.extend(a_errors);
        }
        false
    }

    /// Matches zero or more occurrences of a pattern.
    fn match_zero_or_more(
        &self,
        children: &[(usize, NodeId)],
        pos: usize,
        inner: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        // Base case: all children consumed.
        if pos >= children.len() {
            return true;
        }

        // Try to match one occurrence, then recurse for more.
        for split in (pos + 1)..=children.len() {
            let slice = &children[..split];
            let mut tmp = Vec::new();
            if self.match_children(slice, pos, inner, &mut tmp) {
                let mut rest_errors = Vec::new();
                if self.match_zero_or_more(children, split, inner, &mut rest_errors) {
                    return true;
                }
            }
        }

        // No match at all — check if remaining is empty (whitespace text).
        self.match_children(children, pos, &Pattern::Empty, errors)
    }

    /// Matches one or more occurrences of a pattern.
    fn match_one_or_more(
        &self,
        children: &[(usize, NodeId)],
        pos: usize,
        inner: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        // Must match at least once.
        for split in (pos + 1)..=children.len() {
            let slice = &children[..split];
            let mut tmp = Vec::new();
            if self.match_children(slice, pos, inner, &mut tmp) {
                let mut rest_errors = Vec::new();
                if self.match_zero_or_more(children, split, inner, &mut rest_errors) {
                    return true;
                }
            }
        }

        // Failed to match even once.
        let _ = self.match_children(children, pos, inner, errors);
        false
    }

    /// Matches an interleave pattern — both sub-patterns must match but
    /// in any order. Uses a subset-matching approach.
    fn match_interleave(
        &self,
        children: &[(usize, NodeId)],
        pos: usize,
        a: &Pattern,
        b: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        let remaining = &children[pos..];
        if remaining.is_empty() {
            // Both patterns must accept empty.
            let mut tmp = Vec::new();
            let a_ok = self.match_children(&[], 0, a, &mut tmp);
            let b_ok = self.match_children(&[], 0, b, &mut tmp);
            if !a_ok || !b_ok {
                errors.extend(tmp);
            }
            return a_ok && b_ok;
        }

        // Try each possible partitioning of remaining children into
        // two subsequences (maintaining order within each) that match
        // a and b respectively.
        //
        // For small numbers of children, we use a bitmask approach where
        // each bit indicates whether the child goes to partition A or B.
        let n = remaining.len();
        if n > 20 {
            // For very large child lists, fall back to a simpler heuristic:
            // try matching a first, giving it greedy first pick, then b on rest.
            return self.match_interleave_greedy(remaining, a, b, errors);
        }

        let total = 1u32 << n;
        for mask in 0..total {
            let mut a_children: Vec<(usize, NodeId)> = Vec::new();
            let mut b_children: Vec<(usize, NodeId)> = Vec::new();
            for (i, &child) in remaining.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    a_children.push(child);
                } else {
                    b_children.push(child);
                }
            }
            let mut tmp = Vec::new();
            if self.match_children(&a_children, 0, a, &mut tmp)
                && self.match_children(&b_children, 0, b, &mut tmp)
            {
                return true;
            }
        }

        errors.push(ValidationError {
            message: "content does not match interleave pattern".to_string(),
            line: None,
            column: None,
        });
        false
    }

    /// Greedy interleave matching for large child lists.
    fn match_interleave_greedy(
        &self,
        children: &[(usize, NodeId)],
        a: &Pattern,
        b: &Pattern,
        errors: &mut Vec<ValidationError>,
    ) -> bool {
        let mut a_children: Vec<(usize, NodeId)> = Vec::new();
        let mut b_children: Vec<(usize, NodeId)> = Vec::new();

        for &child in children {
            let single = &[child];
            let mut tmp = Vec::new();
            if self.match_children(single, 0, a, &mut tmp) {
                a_children.push(child);
            } else {
                b_children.push(child);
            }
        }

        let mut a_err = Vec::new();
        let mut b_err = Vec::new();
        let a_ok = self.match_children(&a_children, 0, a, &mut a_err);
        let b_ok = self.match_children(&b_children, 0, b, &mut b_err);

        if !a_ok {
            errors.extend(a_err);
        }
        if !b_ok {
            errors.extend(b_err);
        }
        a_ok && b_ok
    }

    /// Collects the text content of remaining children as a single string.
    fn collect_children_text(&self, children: &[(usize, NodeId)], pos: usize) -> String {
        let mut result = String::new();
        for &(_, id) in &children[pos..] {
            match &self.doc.node(id).kind {
                NodeKind::Text { content } | NodeKind::CData { content } => {
                    result.push_str(content);
                }
                _ => {}
            }
        }
        result
    }

    /// Returns a human-readable description of a node (for error messages).
    fn node_description(&self, id: NodeId) -> String {
        match &self.doc.node(id).kind {
            NodeKind::Element { name, .. } => format!("element <{name}>"),
            NodeKind::Text { content } => {
                let truncated = if content.len() > 30 {
                    format!("\"{}...\"", &content[..30])
                } else {
                    format!("\"{content}\"")
                };
                format!("text {truncated}")
            }
            NodeKind::CData { content } => {
                let truncated = if content.len() > 30 {
                    format!("\"{}...\"", &content[..30])
                } else {
                    format!("\"{content}\"")
                };
                format!("CDATA {truncated}")
            }
            NodeKind::Comment { .. } => "comment".to_string(),
            NodeKind::ProcessingInstruction { target, .. } => {
                format!("PI <?{target}?>")
            }
            _ => "node".to_string(),
        }
    }
}

/// Very basic datatype validation (token, string, integer).
fn validate_datatype(
    value: &str,
    attr_name: &str,
    datatype: &str,
    _library: &str,
    errors: &mut Vec<ValidationError>,
) -> bool {
    match datatype {
        "integer" | "int" | "long" | "short" | "byte" => {
            if value.trim().parse::<i64>().is_ok() {
                true
            } else {
                errors.push(ValidationError {
                    message: format!(
                        "attribute '{attr_name}' value \"{value}\" \
                         is not a valid {datatype}"
                    ),
                    line: None,
                    column: None,
                });
                false
            }
        }
        "positiveInteger" | "nonNegativeInteger" => match value.trim().parse::<i64>() {
            Ok(n) if n >= 0 => true,
            _ => {
                errors.push(ValidationError {
                    message: format!(
                        "attribute '{attr_name}' value \"{value}\" \
                         is not a valid {datatype}"
                    ),
                    line: None,
                    column: None,
                });
                false
            }
        },
        "boolean" => {
            let v = value.trim();
            if v == "true" || v == "false" || v == "1" || v == "0" {
                true
            } else {
                errors.push(ValidationError {
                    message: format!(
                        "attribute '{attr_name}' value \"{value}\" \
                         is not a valid boolean"
                    ),
                    line: None,
                    column: None,
                });
                false
            }
        }
        _ => true, // Unknown datatypes are accepted.
    }
}

/// Checks whether an attribute is an `xmlns` declaration (which are not
/// validated by `RelaxNG`).
fn is_xmlns_attribute(attr: &crate::tree::Attribute) -> bool {
    attr.name == "xmlns"
        || attr.prefix.as_deref() == Some("xmlns")
        || attr.namespace.as_deref() == Some("http://www.w3.org/2000/xmlns/")
}

/// Splits a pattern into attribute patterns and the remaining content pattern.
///
/// This walks the pattern tree and extracts all `Attribute` patterns,
/// returning them separately from the content pattern (which has the
/// attribute patterns replaced with `Empty`).
fn split_attributes(pattern: &Pattern) -> (Vec<Pattern>, Pattern) {
    let mut attrs = Vec::new();
    let content = extract_attrs(pattern, &mut attrs);
    (attrs, content)
}

/// Recursively extracts attribute patterns from a pattern tree.
fn extract_attrs(pattern: &Pattern, attrs: &mut Vec<Pattern>) -> Pattern {
    match pattern {
        Pattern::Attribute { .. } => {
            attrs.push(pattern.clone());
            Pattern::Empty
        }
        Pattern::Group(a, b) => {
            let a2 = extract_attrs(a, attrs);
            let b2 = extract_attrs(b, attrs);
            match (&a2, &b2) {
                (Pattern::Empty, _) => b2,
                (_, Pattern::Empty) => a2,
                _ => Pattern::Group(Box::new(a2), Box::new(b2)),
            }
        }
        Pattern::Interleave(a, b) => {
            let a2 = extract_attrs(a, attrs);
            let b2 = extract_attrs(b, attrs);
            match (&a2, &b2) {
                (Pattern::Empty, _) => b2,
                (_, Pattern::Empty) => a2,
                _ => Pattern::Interleave(Box::new(a2), Box::new(b2)),
            }
        }
        Pattern::Optional(inner) => {
            if matches!(inner.as_ref(), Pattern::Attribute { .. }) {
                // Optional attribute: still extract it but mark it optional
                // by wrapping in Optional.
                attrs.push(Pattern::Optional(inner.clone()));
                Pattern::Empty
            } else {
                let inner2 = extract_attrs(inner, attrs);
                Pattern::Optional(Box::new(inner2))
            }
        }
        _ => pattern.clone(),
    }
}

/// Checks whether a pattern tree contains a wildcard attribute pattern
/// (attribute with `AnyName` name class).
fn has_wildcard_attribute(pattern: &Pattern) -> bool {
    match pattern {
        Pattern::Attribute {
            name: NameClass::AnyName,
            ..
        } => true,
        Pattern::Group(a, b) | Pattern::Interleave(a, b) | Pattern::Choice(a, b) => {
            has_wildcard_attribute(a) || has_wildcard_attribute(b)
        }
        Pattern::Optional(p)
        | Pattern::ZeroOrMore(p)
        | Pattern::OneOrMore(p)
        | Pattern::Mixed(p) => has_wildcard_attribute(p),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // --- Name class matching tests ---

    #[test]
    fn test_name_class_specific_name_matches() {
        let nc = NameClass::Name {
            ns: String::new(),
            local: "foo".to_string(),
        };
        assert!(nc.matches("", "foo"));
        assert!(!nc.matches("", "bar"));
    }

    #[test]
    fn test_name_class_specific_name_with_ns() {
        let nc = NameClass::Name {
            ns: "http://example.com".to_string(),
            local: "foo".to_string(),
        };
        assert!(nc.matches("http://example.com", "foo"));
        assert!(!nc.matches("", "foo"));
        assert!(!nc.matches("http://example.com", "bar"));
    }

    #[test]
    fn test_name_class_any_name() {
        let nc = NameClass::AnyName;
        assert!(nc.matches("", "anything"));
        assert!(nc.matches("http://example.com", "anything"));
    }

    #[test]
    fn test_name_class_any_name_except() {
        let nc = NameClass::AnyNameExcept(Box::new(NameClass::Name {
            ns: String::new(),
            local: "secret".to_string(),
        }));
        assert!(nc.matches("", "foo"));
        assert!(!nc.matches("", "secret"));
    }

    #[test]
    fn test_name_class_ns_name() {
        let nc = NameClass::NsName {
            ns: "http://example.com".to_string(),
        };
        assert!(nc.matches("http://example.com", "anything"));
        assert!(!nc.matches("http://other.com", "anything"));
    }

    #[test]
    fn test_name_class_ns_name_except() {
        let nc = NameClass::NsNameExcept {
            ns: "http://example.com".to_string(),
            except: Box::new(NameClass::Name {
                ns: "http://example.com".to_string(),
                local: "secret".to_string(),
            }),
        };
        assert!(nc.matches("http://example.com", "foo"));
        assert!(!nc.matches("http://example.com", "secret"));
        assert!(!nc.matches("http://other.com", "foo"));
    }

    #[test]
    fn test_name_class_choice() {
        let nc = NameClass::Choice(
            Box::new(NameClass::Name {
                ns: String::new(),
                local: "a".to_string(),
            }),
            Box::new(NameClass::Name {
                ns: String::new(),
                local: "b".to_string(),
            }),
        );
        assert!(nc.matches("", "a"));
        assert!(nc.matches("", "b"));
        assert!(!nc.matches("", "c"));
    }

    // --- Schema parsing tests ---

    #[test]
    fn test_parse_simple_element_schema() {
        let schema_xml = r#"
            <element name="greeting" xmlns="http://relaxng.org/ns/structure/1.0">
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        assert!(matches!(schema.start, Pattern::Element { .. }));
        assert!(schema.defines.is_empty());
    }

    #[test]
    fn test_parse_grammar_with_start_and_define() {
        let schema_xml = r#"
            <grammar xmlns="http://relaxng.org/ns/structure/1.0">
                <start>
                    <ref name="root"/>
                </start>
                <define name="root">
                    <element name="root">
                        <text/>
                    </element>
                </define>
            </grammar>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        assert!(matches!(schema.start, Pattern::Ref(ref name) if name == "root"));
        assert!(schema.defines.contains_key("root"));
    }

    #[test]
    fn test_parse_element_with_attributes() {
        let schema_xml = r#"
            <element name="person" xmlns="http://relaxng.org/ns/structure/1.0">
                <attribute name="id"/>
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let Pattern::Element { pattern, .. } = &schema.start else {
            panic!("expected Element pattern, got {:?}", schema.start);
        };
        // Should be Group(Attribute, Text)
        assert!(matches!(pattern.as_ref(), Pattern::Group(_, _)));
    }

    #[test]
    fn test_parse_choice_pattern() {
        let schema_xml = r#"
            <element name="value" xmlns="http://relaxng.org/ns/structure/1.0">
                <choice>
                    <element name="a"><text/></element>
                    <element name="b"><text/></element>
                </choice>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let Pattern::Element { pattern, .. } = &schema.start else {
            panic!("expected Element pattern, got {:?}", schema.start);
        };
        assert!(matches!(pattern.as_ref(), Pattern::Choice(_, _)));
    }

    #[test]
    fn test_parse_zero_or_more() {
        let schema_xml = r#"
            <element name="list" xmlns="http://relaxng.org/ns/structure/1.0">
                <zeroOrMore>
                    <element name="item"><text/></element>
                </zeroOrMore>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let Pattern::Element { pattern, .. } = &schema.start else {
            panic!("expected Element pattern, got {:?}", schema.start);
        };
        assert!(matches!(pattern.as_ref(), Pattern::ZeroOrMore(_)));
    }

    #[test]
    fn test_parse_one_or_more() {
        let schema_xml = r#"
            <element name="list" xmlns="http://relaxng.org/ns/structure/1.0">
                <oneOrMore>
                    <element name="item"><text/></element>
                </oneOrMore>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let Pattern::Element { pattern, .. } = &schema.start else {
            panic!("expected Element pattern, got {:?}", schema.start);
        };
        assert!(matches!(pattern.as_ref(), Pattern::OneOrMore(_)));
    }

    #[test]
    fn test_parse_optional_pattern() {
        let schema_xml = r#"
            <element name="doc" xmlns="http://relaxng.org/ns/structure/1.0">
                <optional>
                    <attribute name="lang"/>
                </optional>
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let Pattern::Element { pattern, .. } = &schema.start else {
            panic!("expected Element pattern, got {:?}", schema.start);
        };
        assert!(matches!(pattern.as_ref(), Pattern::Group(_, _)));
    }

    #[test]
    fn test_parse_interleave_pattern() {
        let schema_xml = r#"
            <element name="doc" xmlns="http://relaxng.org/ns/structure/1.0">
                <interleave>
                    <element name="a"><text/></element>
                    <element name="b"><text/></element>
                </interleave>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let Pattern::Element { pattern, .. } = &schema.start else {
            panic!("expected Element pattern, got {:?}", schema.start);
        };
        assert!(matches!(pattern.as_ref(), Pattern::Interleave(_, _)));
    }

    #[test]
    fn test_parse_value_pattern() {
        let schema_xml = r#"
            <element name="status" xmlns="http://relaxng.org/ns/structure/1.0">
                <value>active</value>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let Pattern::Element { pattern, .. } = &schema.start else {
            panic!("expected Element pattern, got {:?}", schema.start);
        };
        assert!(matches!(pattern.as_ref(), Pattern::Value(v) if v == "active"));
    }

    #[test]
    fn test_parse_data_pattern() {
        let schema_xml = r#"
            <element name="count" xmlns="http://relaxng.org/ns/structure/1.0">
                <data type="integer"/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let Pattern::Element { pattern, .. } = &schema.start else {
            panic!("expected Element pattern, got {:?}", schema.start);
        };
        assert!(
            matches!(pattern.as_ref(), Pattern::Data { datatype, .. } if datatype == "integer")
        );
    }

    // --- Validation tests ---

    #[test]
    fn test_validate_simple_element_with_text() {
        let schema_xml = r#"
            <element name="greeting" xmlns="http://relaxng.org/ns/structure/1.0">
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<greeting>Hello!</greeting>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_wrong_root_element() {
        let schema_xml = r#"
            <element name="greeting" xmlns="http://relaxng.org/ns/structure/1.0">
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<salutation>Hi</salutation>").unwrap();
        let result = validate(&doc, &schema);
        assert!(!result.is_valid);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_validate_missing_required_attribute() {
        let schema_xml = r#"
            <element name="person" xmlns="http://relaxng.org/ns/structure/1.0">
                <attribute name="id"/>
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<person>John</person>").unwrap();
        let result = validate(&doc, &schema);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("attribute")),
            "expected attribute error, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_element_with_attribute() {
        let schema_xml = r#"
            <element name="person" xmlns="http://relaxng.org/ns/structure/1.0">
                <attribute name="id"/>
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str(r#"<person id="42">John</person>"#).unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_unexpected_attribute() {
        let schema_xml = r#"
            <element name="item" xmlns="http://relaxng.org/ns/structure/1.0">
                <empty/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str(r#"<item extra="oops"/>"#).unwrap();
        let result = validate(&doc, &schema);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("unexpected attribute")),
            "expected unexpected attribute error, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_choice_first_alternative() {
        let schema_xml = r#"
            <element name="value" xmlns="http://relaxng.org/ns/structure/1.0">
                <choice>
                    <element name="a"><text/></element>
                    <element name="b"><text/></element>
                </choice>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<value><a>hello</a></value>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_choice_second_alternative() {
        let schema_xml = r#"
            <element name="value" xmlns="http://relaxng.org/ns/structure/1.0">
                <choice>
                    <element name="a"><text/></element>
                    <element name="b"><text/></element>
                </choice>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<value><b>hello</b></value>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_choice_invalid() {
        let schema_xml = r#"
            <element name="value" xmlns="http://relaxng.org/ns/structure/1.0">
                <choice>
                    <element name="a"><text/></element>
                    <element name="b"><text/></element>
                </choice>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<value><c>hello</c></value>").unwrap();
        let result = validate(&doc, &schema);
        assert!(!result.is_valid);
    }

    #[test]
    fn test_validate_zero_or_more_empty() {
        let schema_xml = r#"
            <element name="list" xmlns="http://relaxng.org/ns/structure/1.0">
                <zeroOrMore>
                    <element name="item"><text/></element>
                </zeroOrMore>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<list/>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_zero_or_more_multiple() {
        let schema_xml = r#"
            <element name="list" xmlns="http://relaxng.org/ns/structure/1.0">
                <zeroOrMore>
                    <element name="item"><text/></element>
                </zeroOrMore>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc =
            Document::parse_str("<list><item>a</item><item>b</item><item>c</item></list>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_one_or_more_empty_fails() {
        let schema_xml = r#"
            <element name="list" xmlns="http://relaxng.org/ns/structure/1.0">
                <oneOrMore>
                    <element name="item"><text/></element>
                </oneOrMore>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<list/>").unwrap();
        let result = validate(&doc, &schema);
        assert!(!result.is_valid);
    }

    #[test]
    fn test_validate_one_or_more_with_items() {
        let schema_xml = r#"
            <element name="list" xmlns="http://relaxng.org/ns/structure/1.0">
                <oneOrMore>
                    <element name="item"><text/></element>
                </oneOrMore>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<list><item>a</item><item>b</item></list>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_optional_present() {
        let schema_xml = r#"
            <element name="doc" xmlns="http://relaxng.org/ns/structure/1.0">
                <optional>
                    <attribute name="lang"/>
                </optional>
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str(r#"<doc lang="en">Hello</doc>"#).unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_optional_absent() {
        let schema_xml = r#"
            <element name="doc" xmlns="http://relaxng.org/ns/structure/1.0">
                <optional>
                    <attribute name="lang"/>
                </optional>
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<doc>Hello</doc>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_interleave_any_order() {
        let schema_xml = r#"
            <element name="doc" xmlns="http://relaxng.org/ns/structure/1.0">
                <interleave>
                    <element name="a"><text/></element>
                    <element name="b"><text/></element>
                </interleave>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();

        // Order 1: a then b
        let doc1 = Document::parse_str("<doc><a>1</a><b>2</b></doc>").unwrap();
        let r1 = validate(&doc1, &schema);
        assert!(r1.is_valid, "a,b order failed: {:?}", r1.errors);

        // Order 2: b then a
        let doc2 = Document::parse_str("<doc><b>2</b><a>1</a></doc>").unwrap();
        let r2 = validate(&doc2, &schema);
        assert!(r2.is_valid, "b,a order failed: {:?}", r2.errors);
    }

    #[test]
    fn test_validate_ref_define_resolution() {
        let schema_xml = r#"
            <grammar xmlns="http://relaxng.org/ns/structure/1.0">
                <start>
                    <ref name="root"/>
                </start>
                <define name="root">
                    <element name="root">
                        <ref name="content"/>
                    </element>
                </define>
                <define name="content">
                    <element name="child"><text/></element>
                </define>
            </grammar>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<root><child>hello</child></root>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_nested_elements() {
        let schema_xml = r#"
            <element name="root" xmlns="http://relaxng.org/ns/structure/1.0">
                <element name="parent">
                    <element name="child">
                        <text/>
                    </element>
                </element>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<root><parent><child>text</child></parent></root>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_value_match() {
        let schema_xml = r#"
            <element name="status" xmlns="http://relaxng.org/ns/structure/1.0">
                <value>active</value>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();

        let doc1 = Document::parse_str("<status>active</status>").unwrap();
        let r1 = validate(&doc1, &schema);
        assert!(r1.is_valid, "errors: {:?}", r1.errors);

        let doc2 = Document::parse_str("<status>inactive</status>").unwrap();
        let r2 = validate(&doc2, &schema);
        assert!(!r2.is_valid);
    }

    #[test]
    fn test_validate_missing_element() {
        let schema_xml = r#"
            <element name="root" xmlns="http://relaxng.org/ns/structure/1.0">
                <element name="required"><text/></element>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<root/>").unwrap();
        let result = validate(&doc, &schema);
        assert!(!result.is_valid);
        assert!(
            result.errors.iter().any(|e| e.message.contains("missing")),
            "expected missing element error, got: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_unexpected_element() {
        let schema_xml = r#"
            <element name="root" xmlns="http://relaxng.org/ns/structure/1.0">
                <empty/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<root><surprise>oops</surprise></root>").unwrap();
        let result = validate(&doc, &schema);
        assert!(!result.is_valid);
    }

    #[test]
    fn test_validate_empty_element() {
        let schema_xml = r#"
            <element name="br" xmlns="http://relaxng.org/ns/structure/1.0">
                <empty/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<br/>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_attribute_value_mismatch() {
        let schema_xml = r#"
            <element name="item" xmlns="http://relaxng.org/ns/structure/1.0">
                <attribute name="type">
                    <value>book</value>
                </attribute>
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();

        // Correct value.
        let doc1 = Document::parse_str(r#"<item type="book">Title</item>"#).unwrap();
        let r1 = validate(&doc1, &schema);
        assert!(r1.is_valid, "errors: {:?}", r1.errors);

        // Wrong value.
        let doc2 = Document::parse_str(r#"<item type="dvd">Title</item>"#).unwrap();
        let r2 = validate(&doc2, &schema);
        assert!(!r2.is_valid);
    }

    #[test]
    fn test_validate_no_root_element() {
        let schema_xml = r#"
            <element name="root" xmlns="http://relaxng.org/ns/structure/1.0">
                <text/>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();

        let doc = Document::new();
        let result = validate(&doc, &schema);
        assert!(!result.is_valid);
        assert!(result
            .errors
            .iter()
            .any(|e| e.message.contains("no root element")),);
    }

    #[test]
    fn test_validate_sequence_of_elements() {
        let schema_xml = r#"
            <element name="root" xmlns="http://relaxng.org/ns/structure/1.0">
                <element name="first"><text/></element>
                <element name="second"><text/></element>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();

        let doc = Document::parse_str("<root><first>a</first><second>b</second></root>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_sequence_wrong_order() {
        let schema_xml = r#"
            <element name="root" xmlns="http://relaxng.org/ns/structure/1.0">
                <element name="first"><text/></element>
                <element name="second"><text/></element>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();

        let doc = Document::parse_str("<root><second>b</second><first>a</first></root>").unwrap();
        let result = validate(&doc, &schema);
        assert!(!result.is_valid);
    }

    #[test]
    fn test_pattern_display() {
        assert_eq!(Pattern::Empty.to_string(), "empty");
        assert_eq!(Pattern::Text.to_string(), "text");
        assert_eq!(Pattern::NotAllowed.to_string(), "notAllowed");
        assert_eq!(
            Pattern::Element {
                name: NameClass::Name {
                    ns: String::new(),
                    local: "div".to_string(),
                },
                pattern: Box::new(Pattern::Empty),
            }
            .to_string(),
            "element div"
        );
    }

    #[test]
    fn test_name_class_display() {
        assert_eq!(NameClass::AnyName.to_string(), "*");
        assert_eq!(
            NameClass::Name {
                ns: String::new(),
                local: "foo".to_string(),
            }
            .to_string(),
            "foo"
        );
        assert_eq!(
            NameClass::Name {
                ns: "http://example.com".to_string(),
                local: "foo".to_string(),
            }
            .to_string(),
            "{http://example.com}foo"
        );
    }

    #[test]
    fn test_schema_parse_error_display() {
        let err = SchemaParseError {
            message: "test error".to_string(),
        };
        assert_eq!(err.to_string(), "RelaxNG schema error: test error");
    }

    #[test]
    fn test_parse_complex_grammar_with_cross_refs() {
        let schema_xml = r#"
            <grammar xmlns="http://relaxng.org/ns/structure/1.0">
                <start>
                    <element name="addressBook">
                        <zeroOrMore>
                            <ref name="cardContent"/>
                        </zeroOrMore>
                    </element>
                </start>
                <define name="cardContent">
                    <element name="card">
                        <ref name="cardFields"/>
                    </element>
                </define>
                <define name="cardFields">
                    <element name="name"><text/></element>
                    <element name="email"><text/></element>
                </define>
            </grammar>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        assert!(schema.defines.contains_key("cardContent"));
        assert!(schema.defines.contains_key("cardFields"));

        let doc = Document::parse_str(
            "<addressBook>\
                <card><name>Alice</name><email>alice@example.com</email></card>\
                <card><name>Bob</name><email>bob@example.com</email></card>\
            </addressBook>",
        )
        .unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_mixed_content() {
        let schema_xml = r#"
            <element name="p" xmlns="http://relaxng.org/ns/structure/1.0">
                <mixed>
                    <zeroOrMore>
                        <element name="b"><text/></element>
                    </zeroOrMore>
                </mixed>
            </element>
        "#;
        let schema = parse_relaxng(schema_xml).unwrap();
        let doc = Document::parse_str("<p>Hello <b>world</b> and more</p>").unwrap();
        let result = validate(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }
}
