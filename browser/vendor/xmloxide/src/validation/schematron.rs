//! ISO Schematron validation for XML documents.
//!
//! This module implements a subset of the ISO Schematron specification
//! (ISO/IEC 19757-3) for rule-based XML document validation. Schematron
//! schemas express constraints as `XPath` assertions evaluated against
//! selected context nodes, complementing grammar-based schemas like DTD,
//! `RelaxNG`, and XSD.
//!
//! # Architecture
//!
//! The implementation is split into three layers:
//!
//! 1. **Data model** ([`SchematronSchema`], [`SchematronPattern`],
//!    [`SchematronRule`], [`SchematronCheck`]) — the parsed schema
//!    representation.
//! 2. **Schema parser** ([`parse_schematron`]) — reads a Schematron XML
//!    schema and produces a `SchematronSchema`.
//! 3. **Validator** ([`validate_schematron`], [`validate_schematron_with_phase`])
//!    — evaluates assertions against a document tree using the `XPath` engine.
//!
//! # Examples
//!
//! ```
//! use xmloxide::Document;
//! use xmloxide::validation::schematron::{parse_schematron, validate_schematron};
//!
//! let schema_xml = r#"
//!   <schema xmlns="http://purl.oclc.org/dml/schematron">
//!     <pattern>
//!       <rule context="/root">
//!         <assert test="child">root must have a child element</assert>
//!       </rule>
//!     </pattern>
//!   </schema>
//! "#;
//!
//! let schema = parse_schematron(schema_xml).unwrap();
//! let doc = Document::parse_str("<root><child/></root>").unwrap();
//! let result = validate_schematron(&doc, &schema);
//! assert!(result.is_valid);
//! ```
//!
//! # Limitations
//!
//! - Namespace-prefixed `XPath` name tests (e.g., `//inv:invoice`) do not
//!   match because the `XPath` evaluator compares against local names.
//!   Unprefixed names work. Workaround: use `local-name()`.
//! - Abstract patterns and `<sch:extends>` are not supported.
//! - `xsl:key` and XSLT-specific features are not supported.
//! - Variable forward references are not supported (evaluated in document order).

use std::collections::{HashMap, HashSet};

use crate::tree::{Document, NodeId, NodeKind};
use crate::validation::{ValidationError, ValidationResult};
use crate::xpath;
use crate::xpath::eval::XPathContext;
use crate::xpath::types::{XPathError, XPathNode, XPathValue};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A parsed ISO Schematron schema.
///
/// Contains patterns (each with rules and assertions), namespace bindings,
/// schema-level variables, and optional phase definitions.
#[derive(Debug, Clone)]
pub struct SchematronSchema {
    /// Namespace bindings from `<sch:ns>` elements.
    pub namespaces: Vec<NamespaceBinding>,
    /// Schema-level `<sch:let>` variable bindings.
    pub variables: Vec<LetBinding>,
    /// Patterns containing rules and assertions.
    pub patterns: Vec<SchematronPattern>,
    /// Named phases that activate subsets of patterns.
    pub phases: HashMap<String, Phase>,
    /// The default phase (`defaultPhase` attribute on the root element).
    pub default_phase: Option<String>,
}

/// A namespace binding declared via `<sch:ns prefix="..." uri="..."/>`.
#[derive(Debug, Clone)]
pub struct NamespaceBinding {
    /// The namespace prefix.
    pub prefix: String,
    /// The namespace URI.
    pub uri: String,
}

/// A `let` variable binding declared via `<sch:let name="..." value="..."/>`.
#[derive(Debug, Clone)]
pub struct LetBinding {
    /// The variable name (referenced as `$name` in `XPath` expressions).
    pub name: String,
    /// The `XPath` expression whose result is bound to the variable.
    pub value: String,
}

/// A pattern containing rules, with optional id and pattern-level variables.
#[derive(Debug, Clone)]
pub struct SchematronPattern {
    /// Optional pattern identifier (used by phases to activate subsets).
    pub id: Option<String>,
    /// Whether this is an abstract pattern (template for `is-a` instantiation).
    pub is_abstract: bool,
    /// Reference to an abstract pattern id (instantiates the abstract pattern
    /// with parameter substitutions).
    pub is_a: Option<String>,
    /// Parameter bindings for `is-a` instantiation (`<sch:param>`).
    pub params: Vec<(String, String)>,
    /// Pattern-level `<sch:let>` variable bindings.
    pub variables: Vec<LetBinding>,
    /// Rules within this pattern.
    pub rules: Vec<SchematronRule>,
}

/// A rule that selects context nodes and applies checks to them.
#[derive(Debug, Clone)]
pub struct SchematronRule {
    /// `XPath` expression selecting context nodes.
    pub context: String,
    /// Rule-level `<sch:let>` variable bindings.
    pub variables: Vec<LetBinding>,
    /// Assertions and reports to evaluate at each context node.
    pub checks: Vec<SchematronCheck>,
}

/// An individual assertion or report within a rule.
#[derive(Debug, Clone)]
pub enum SchematronCheck {
    /// An assertion: if `test` evaluates to false at the context node,
    /// a validation error is raised with `message`.
    Assert {
        /// `XPath` boolean expression.
        test: String,
        /// Human-readable message parts (may include `<sch:value-of>`).
        message: Vec<MessagePart>,
    },
    /// A report: if `test` evaluates to true at the context node,
    /// a validation warning is raised with `message`.
    Report {
        /// `XPath` boolean expression.
        test: String,
        /// Human-readable message parts (may include `<sch:value-of>`).
        message: Vec<MessagePart>,
    },
}

/// A segment of a Schematron message (plain text or interpolated value).
#[derive(Debug, Clone)]
pub enum MessagePart {
    /// Literal text.
    Text(String),
    /// An `XPath` expression whose string value is interpolated via
    /// `<sch:value-of select="..."/>`.
    ValueOf {
        /// The `XPath` `select` expression.
        select: String,
    },
}

/// A named phase that activates a subset of patterns.
#[derive(Debug, Clone)]
pub struct Phase {
    /// The phase identifier.
    pub id: String,
    /// Pattern ids activated by this phase.
    pub active_patterns: Vec<String>,
}

// ---------------------------------------------------------------------------
// Schematron namespace constants
// ---------------------------------------------------------------------------

/// ISO Schematron namespace URI.
const SCH_NS_ISO: &str = "http://purl.oclc.org/dml/schematron";

/// Classic Schematron 1.5 namespace URI.
const SCH_NS_CLASSIC: &str = "http://www.ascc.net/xml/schematron";

// ---------------------------------------------------------------------------
// Schema parser
// ---------------------------------------------------------------------------

/// Parses a Schematron schema from an XML string.
///
/// Supports both the ISO namespace (`http://purl.oclc.org/dml/schematron`)
/// and the classic 1.5 namespace (`http://www.ascc.net/xml/schematron`).
/// The schema can also use the `sch:` prefix convention with no namespace.
///
/// # Errors
///
/// Returns [`ValidationError`] if the XML cannot be parsed or the schema
/// structure is invalid (e.g., a rule missing its `context` attribute).
///
/// # Examples
///
/// ```
/// use xmloxide::validation::schematron::parse_schematron;
///
/// let schema = parse_schematron(r#"
///   <schema xmlns="http://purl.oclc.org/dml/schematron">
///     <pattern>
///       <rule context="/*">
///         <assert test="true()">always passes</assert>
///       </rule>
///     </pattern>
///   </schema>
/// "#).unwrap();
/// assert_eq!(schema.patterns.len(), 1);
/// ```
pub fn parse_schematron(schema_xml: &str) -> Result<SchematronSchema, ValidationError> {
    let doc = Document::parse_str(schema_xml).map_err(|e| ValidationError {
        message: format!("failed to parse Schematron schema XML: {e}"),
        line: None,
        column: None,
    })?;

    let root = doc.root_element().ok_or_else(|| ValidationError {
        message: "Schematron schema has no root element".to_string(),
        line: None,
        column: None,
    })?;

    let root_name = doc.node_name(root).unwrap_or("");
    if !is_sch_element(root_name, "schema") {
        return Err(ValidationError {
            message: format!("expected <schema> root element, found <{root_name}>"),
            line: None,
            column: None,
        });
    }

    let root_ns = doc.node_namespace(root).unwrap_or("");
    let ns_mode = detect_ns_mode(root_ns, root_name);

    let default_phase = doc.attribute(root, "defaultPhase").map(String::from);

    let mut namespaces = Vec::new();
    let mut variables = Vec::new();
    let mut patterns = Vec::new();
    let mut phases = HashMap::new();

    for child in doc.children(root) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let name = doc.node_name(child).unwrap_or("");
        let child_ns = doc.node_namespace(child).unwrap_or("");

        if !is_sch_name_in_mode(&ns_mode, child_ns) {
            continue;
        }

        let local = sch_local_name(name);
        match local {
            "ns" => {
                if let (Some(prefix), Some(uri)) =
                    (doc.attribute(child, "prefix"), doc.attribute(child, "uri"))
                {
                    namespaces.push(NamespaceBinding {
                        prefix: prefix.to_owned(),
                        uri: uri.to_owned(),
                    });
                }
            }
            "let" => {
                if let Some(binding) = parse_let_binding(&doc, child) {
                    variables.push(binding);
                }
            }
            "pattern" => {
                patterns.push(parse_pattern(&doc, &ns_mode, child)?);
            }
            "phase" => {
                if let Some(phase) = parse_phase(&doc, &ns_mode, child) {
                    phases.insert(phase.id.clone(), phase);
                }
            }
            _ => {}
        }
    }

    // Resolve abstract pattern instantiations (is-a references)
    let patterns = resolve_abstract_patterns(patterns);

    Ok(SchematronSchema {
        namespaces,
        variables,
        patterns,
        phases,
        default_phase,
    })
}

/// Resolves `is-a` references by copying rules from abstract patterns
/// and substituting `$param` placeholders in context and test expressions.
fn resolve_abstract_patterns(patterns: Vec<SchematronPattern>) -> Vec<SchematronPattern> {
    // Collect abstract patterns by id (cloned so we can move patterns below)
    let abstract_map: HashMap<String, SchematronPattern> = patterns
        .iter()
        .filter(|p| p.is_abstract)
        .filter_map(|p| p.id.as_ref().map(|id| (id.clone(), p.clone())))
        .collect();

    patterns
        .into_iter()
        .filter(|p| !p.is_abstract) // Exclude abstract patterns from validation
        .map(|mut p| {
            if let Some(ref abstract_id) = p.is_a {
                if let Some(abstract_pat) = abstract_map.get(abstract_id) {
                    // Copy rules from abstract pattern, substituting params
                    p.rules = abstract_pat
                        .rules
                        .iter()
                        .map(|rule| substitute_rule_params(rule, &p.params))
                        .collect();
                    // Also inherit variables from abstract pattern
                    let mut combined_vars = abstract_pat.variables.clone();
                    combined_vars.extend(p.variables.clone());
                    p.variables = combined_vars;
                }
            }
            p
        })
        .collect()
}

/// Substitutes `$param_name` placeholders in a rule's context and test
/// expressions with the corresponding parameter values.
fn substitute_rule_params(rule: &SchematronRule, params: &[(String, String)]) -> SchematronRule {
    SchematronRule {
        context: substitute_params(&rule.context, params),
        variables: rule
            .variables
            .iter()
            .map(|v| LetBinding {
                name: v.name.clone(),
                value: substitute_params(&v.value, params),
            })
            .collect(),
        checks: rule
            .checks
            .iter()
            .map(|check| match check {
                SchematronCheck::Assert { test, message } => SchematronCheck::Assert {
                    test: substitute_params(test, params),
                    message: message.clone(),
                },
                SchematronCheck::Report { test, message } => SchematronCheck::Report {
                    test: substitute_params(test, params),
                    message: message.clone(),
                },
            })
            .collect(),
    }
}

/// Replaces `$name` placeholders in `text` with parameter values.
fn substitute_params(text: &str, params: &[(String, String)]) -> String {
    let mut result = text.to_string();
    for (name, value) in params {
        let placeholder = format!("${name}");
        result = result.replace(&placeholder, value);
    }
    result
}

/// Namespace detection mode for parsing Schematron elements.
#[derive(Debug, Clone)]
enum NsMode {
    /// Elements are in the ISO namespace.
    Iso,
    /// Elements are in the classic 1.5 namespace.
    Classic,
    /// Elements use `sch:` prefix with no namespace (or unrecognized namespace).
    Prefix,
}

/// Detects which namespace mode to use based on the root element.
fn detect_ns_mode(ns: &str, _name: &str) -> NsMode {
    match ns {
        SCH_NS_ISO => NsMode::Iso,
        SCH_NS_CLASSIC => NsMode::Classic,
        _ => NsMode::Prefix,
    }
}

/// Checks if a given element name matches a Schematron local name.
fn is_sch_element(name: &str, local: &str) -> bool {
    name == local || name == format!("sch:{local}") || name.ends_with(&format!(":{local}"))
}

/// Checks if a child element is a Schematron element in the detected mode.
fn is_sch_name_in_mode(mode: &NsMode, ns: &str) -> bool {
    match mode {
        NsMode::Iso => ns == SCH_NS_ISO,
        NsMode::Classic => ns == SCH_NS_CLASSIC,
        NsMode::Prefix => {
            // Accept sch: prefix or bare names in schema context
            ns == SCH_NS_ISO || ns == SCH_NS_CLASSIC || ns.is_empty()
        }
    }
}

/// Extracts the local name from a potentially prefixed element name.
fn sch_local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Parses a `<sch:let>` binding.
fn parse_let_binding(doc: &Document, node: NodeId) -> Option<LetBinding> {
    let name = doc.attribute(node, "name")?;
    let value = doc.attribute(node, "value").unwrap_or("");
    Some(LetBinding {
        name: name.to_owned(),
        value: value.to_owned(),
    })
}

/// Parses a `<sch:pattern>` element.
fn parse_pattern(
    doc: &Document,
    ns_mode: &NsMode,
    node: NodeId,
) -> Result<SchematronPattern, ValidationError> {
    let id = doc.attribute(node, "id").map(String::from);
    let is_abstract = doc.attribute(node, "abstract") == Some("true");
    let is_a = doc.attribute(node, "is-a").map(String::from);
    let mut variables = Vec::new();
    let mut rules = Vec::new();
    let mut params = Vec::new();

    for child in doc.children(node) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let name = doc.node_name(child).unwrap_or("");
        let child_ns = doc.node_namespace(child).unwrap_or("");

        if !is_sch_name_in_mode(ns_mode, child_ns) {
            continue;
        }

        let local = sch_local_name(name);
        match local {
            "let" => {
                if let Some(binding) = parse_let_binding(doc, child) {
                    variables.push(binding);
                }
            }
            "rule" => {
                rules.push(parse_rule(doc, ns_mode, child)?);
            }
            "param" => {
                if let (Some(pname), Some(pvalue)) =
                    (doc.attribute(child, "name"), doc.attribute(child, "value"))
                {
                    params.push((pname.to_owned(), pvalue.to_owned()));
                }
            }
            _ => {}
        }
    }

    Ok(SchematronPattern {
        id,
        is_abstract,
        is_a,
        params,
        variables,
        rules,
    })
}

/// Parses a `<sch:rule>` element.
fn parse_rule(
    doc: &Document,
    ns_mode: &NsMode,
    node: NodeId,
) -> Result<SchematronRule, ValidationError> {
    let context = doc
        .attribute(node, "context")
        .ok_or_else(|| ValidationError {
            message: "rule element is missing required 'context' attribute".to_string(),
            line: None,
            column: None,
        })?
        .to_owned();

    let mut variables = Vec::new();
    let mut checks = Vec::new();

    for child in doc.children(node) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let name = doc.node_name(child).unwrap_or("");
        let child_ns = doc.node_namespace(child).unwrap_or("");

        if !is_sch_name_in_mode(ns_mode, child_ns) {
            continue;
        }

        let local = sch_local_name(name);
        match local {
            "let" => {
                if let Some(binding) = parse_let_binding(doc, child) {
                    variables.push(binding);
                }
            }
            "assert" => {
                if let Some(check) = parse_check(doc, ns_mode, child, true) {
                    checks.push(check);
                }
            }
            "report" => {
                if let Some(check) = parse_check(doc, ns_mode, child, false) {
                    checks.push(check);
                }
            }
            _ => {}
        }
    }

    Ok(SchematronRule {
        context,
        variables,
        checks,
    })
}

/// Parses a `<sch:assert>` or `<sch:report>` element.
fn parse_check(
    doc: &Document,
    ns_mode: &NsMode,
    node: NodeId,
    is_assert: bool,
) -> Option<SchematronCheck> {
    let test = doc.attribute(node, "test")?.to_owned();
    let message = parse_message_parts(doc, ns_mode, node);
    if is_assert {
        Some(SchematronCheck::Assert { test, message })
    } else {
        Some(SchematronCheck::Report { test, message })
    }
}

/// Parses the mixed content of an assert/report element into message parts.
fn parse_message_parts(doc: &Document, ns_mode: &NsMode, node: NodeId) -> Vec<MessagePart> {
    let mut parts = Vec::new();
    for child in doc.children(node) {
        match &doc.node(child).kind {
            NodeKind::Text { content } if !content.is_empty() => {
                parts.push(MessagePart::Text(content.clone()));
            }
            NodeKind::Element { .. } => {
                let name = doc.node_name(child).unwrap_or("");
                let child_ns = doc.node_namespace(child).unwrap_or("");
                if is_sch_name_in_mode(ns_mode, child_ns) && sch_local_name(name) == "value-of" {
                    if let Some(select) = doc.attribute(child, "select") {
                        parts.push(MessagePart::ValueOf {
                            select: select.to_owned(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    parts
}

/// Parses a `<sch:phase>` element.
fn parse_phase(doc: &Document, ns_mode: &NsMode, node: NodeId) -> Option<Phase> {
    let id = doc.attribute(node, "id")?.to_owned();
    let mut active_patterns = Vec::new();

    for child in doc.children(node) {
        if !matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            continue;
        }
        let name = doc.node_name(child).unwrap_or("");
        let child_ns = doc.node_namespace(child).unwrap_or("");
        if is_sch_name_in_mode(ns_mode, child_ns) && sch_local_name(name) == "active" {
            if let Some(pattern) = doc.attribute(child, "pattern") {
                active_patterns.push(pattern.to_owned());
            }
        }
    }

    Some(Phase {
        id,
        active_patterns,
    })
}

// ---------------------------------------------------------------------------
// Validator
// ---------------------------------------------------------------------------

/// Validates a document against a Schematron schema.
///
/// Evaluates all patterns (or the default phase's patterns) and returns
/// a [`ValidationResult`] with errors from failed assertions and warnings
/// from fired reports.
///
/// # Examples
///
/// ```
/// use xmloxide::Document;
/// use xmloxide::validation::schematron::{parse_schematron, validate_schematron};
///
/// let schema = parse_schematron(r#"
///   <schema xmlns="http://purl.oclc.org/dml/schematron">
///     <pattern>
///       <rule context="/root">
///         <assert test="child">root must have a child element</assert>
///       </rule>
///     </pattern>
///   </schema>
/// "#).unwrap();
///
/// let doc = Document::parse_str("<root><child/></root>").unwrap();
/// let result = validate_schematron(&doc, &schema);
/// assert!(result.is_valid);
/// ```
pub fn validate_schematron(doc: &Document, schema: &SchematronSchema) -> ValidationResult {
    if let Some(ref phase_id) = schema.default_phase {
        validate_schematron_with_phase(doc, schema, phase_id)
    } else {
        validate_patterns(doc, schema, &schema.patterns)
    }
}

/// Validates a document against a Schematron schema using a specific phase.
///
/// Only patterns referenced by `<sch:active>` elements within the named
/// phase are evaluated.
///
/// # Examples
///
/// ```
/// use xmloxide::Document;
/// use xmloxide::validation::schematron::{parse_schematron, validate_schematron_with_phase};
///
/// let schema = parse_schematron(r#"
///   <schema xmlns="http://purl.oclc.org/dml/schematron">
///     <phase id="quick">
///       <active pattern="basic"/>
///     </phase>
///     <pattern id="basic">
///       <rule context="/*">
///         <assert test="true()">always passes</assert>
///       </rule>
///     </pattern>
///     <pattern id="strict">
///       <rule context="/*">
///         <assert test="false()">always fails</assert>
///       </rule>
///     </pattern>
///   </schema>
/// "#).unwrap();
///
/// let doc = Document::parse_str("<root/>").unwrap();
/// let result = validate_schematron_with_phase(&doc, &schema, "quick");
/// assert!(result.is_valid);
/// ```
pub fn validate_schematron_with_phase(
    doc: &Document,
    schema: &SchematronSchema,
    phase_id: &str,
) -> ValidationResult {
    if let Some(phase) = schema.phases.get(phase_id) {
        let active_ids: HashSet<&str> = phase.active_patterns.iter().map(String::as_str).collect();
        let active_patterns: Vec<&SchematronPattern> = schema
            .patterns
            .iter()
            .filter(|p| {
                p.id.as_ref()
                    .is_some_and(|id| active_ids.contains(id.as_str()))
            })
            .collect();
        validate_pattern_refs(doc, schema, &active_patterns)
    } else {
        // Unknown phase — validate all patterns
        validate_patterns(doc, schema, &schema.patterns)
    }
}

/// Validates a set of patterns (owned references).
fn validate_patterns(
    doc: &Document,
    schema: &SchematronSchema,
    patterns: &[SchematronPattern],
) -> ValidationResult {
    let refs: Vec<&SchematronPattern> = patterns.iter().collect();
    validate_pattern_refs(doc, schema, &refs)
}

/// Core validation logic operating on a slice of pattern references.
fn validate_pattern_refs(
    doc: &Document,
    schema: &SchematronSchema,
    patterns: &[&SchematronPattern],
) -> ValidationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let root = doc.root();
    let ns = &schema.namespaces;

    // Evaluate schema-level variables at the document root
    let root_node = XPathNode::Node(root);
    let schema_vars = eval_variables(doc, root_node, &schema.variables, &HashMap::new(), ns);

    for pattern in patterns {
        // Per-pattern fired_nodes tracking (firing rule semantics)
        let mut fired_nodes: HashSet<XPathNode> = HashSet::new();

        // Evaluate pattern-level variables
        let mut pattern_vars = schema_vars.clone();
        let extra = eval_variables(doc, root_node, &pattern.variables, &pattern_vars, ns);
        pattern_vars.extend(extra);

        for rule in &pattern.rules {
            // Evaluate the context XPath to find matching nodes
            let context_nodes =
                match eval_context_xpath(doc, root, &rule.context, &pattern_vars, ns) {
                    Ok(nodes) => nodes,
                    Err(e) => {
                        errors.push(ValidationError {
                            message: format!(
                                "XPath error in rule context '{}': {}",
                                rule.context, e
                            ),
                            line: None,
                            column: None,
                        });
                        continue;
                    }
                };

            for &node in &context_nodes {
                // Firing rule: skip nodes already fired in this pattern
                if fired_nodes.contains(&node) {
                    continue;
                }
                fired_nodes.insert(node);

                // Evaluate rule-level variables at this context node
                let mut rule_vars = pattern_vars.clone();
                let extra = eval_variables(doc, node, &rule.variables, &rule_vars, ns);
                rule_vars.extend(extra);

                for check in &rule.checks {
                    match check {
                        SchematronCheck::Assert { test, message } => {
                            match eval_test(doc, node, test, &rule_vars, ns) {
                                Ok(true) => {} // assertion satisfied
                                Ok(false) => {
                                    let msg =
                                        interpolate_message(doc, node, message, &rule_vars, ns);
                                    errors.push(ValidationError {
                                        message: msg,
                                        line: None,
                                        column: None,
                                    });
                                }
                                Err(e) => {
                                    errors.push(ValidationError {
                                        message: format!(
                                            "XPath error in assert test '{test}': {e}"
                                        ),
                                        line: None,
                                        column: None,
                                    });
                                }
                            }
                        }
                        SchematronCheck::Report { test, message } => {
                            match eval_test(doc, node, test, &rule_vars, ns) {
                                Ok(true) => {
                                    let msg =
                                        interpolate_message(doc, node, message, &rule_vars, ns);
                                    warnings.push(ValidationError {
                                        message: msg,
                                        line: None,
                                        column: None,
                                    });
                                }
                                Ok(false) => {} // report condition not met
                                Err(e) => {
                                    errors.push(ValidationError {
                                        message: format!(
                                            "XPath error in report test '{test}': {e}"
                                        ),
                                        line: None,
                                        column: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    ValidationResult {
        is_valid: errors.is_empty(),
        errors,
        warnings,
    }
}

/// Creates an `XPathContext` with variables and namespace bindings.
///
/// The context node may be an attribute node (e.g., for rules whose
/// context expression selects attributes).
fn make_xpath_context<'a>(
    doc: &'a Document,
    node: XPathNode,
    variables: &HashMap<String, XPathValue>,
    ns_bindings: &[NamespaceBinding],
) -> XPathContext<'a> {
    let mut ctx = XPathContext::new_at(doc, node);
    for (name, value) in variables {
        ctx.set_variable(name, value.clone());
    }
    for ns in ns_bindings {
        ctx.set_namespace(&ns.prefix, &ns.uri);
    }
    ctx
}

/// Evaluates an `XPath` context expression and returns the matching nodes.
///
/// The result may contain attribute nodes: a rule whose context selects
/// attributes fires with the attribute itself as the context node.
fn eval_context_xpath(
    doc: &Document,
    root: NodeId,
    xpath_expr: &str,
    variables: &HashMap<String, XPathValue>,
    ns_bindings: &[NamespaceBinding],
) -> Result<Vec<XPathNode>, XPathError> {
    let expr = xpath::parser::parse(xpath_expr)?;
    let ctx = make_xpath_context(doc, XPathNode::Node(root), variables, ns_bindings);
    let result = ctx.evaluate(&expr)?;
    match result {
        XPathValue::NodeSet(nodes) => Ok(nodes),
        _ => Ok(vec![]),
    }
}

/// Evaluates a test expression at a context node, returning a boolean.
fn eval_test(
    doc: &Document,
    node: XPathNode,
    test_expr: &str,
    variables: &HashMap<String, XPathValue>,
    ns_bindings: &[NamespaceBinding],
) -> Result<bool, XPathError> {
    let expr = xpath::parser::parse(test_expr)?;
    let ctx = make_xpath_context(doc, node, variables, ns_bindings);
    let result = ctx.evaluate(&expr)?;
    Ok(result.to_boolean())
}

/// Evaluates `<sch:let>` bindings and returns the resulting variable map.
fn eval_variables(
    doc: &Document,
    context_node: XPathNode,
    bindings: &[LetBinding],
    existing: &HashMap<String, XPathValue>,
    ns_bindings: &[NamespaceBinding],
) -> HashMap<String, XPathValue> {
    let mut result = HashMap::new();
    // Accumulate so later bindings can reference earlier ones
    let mut combined = existing.clone();

    for binding in bindings {
        // Try to evaluate as XPath; fall back to string literal
        let value = if let Ok(expr) = xpath::parser::parse(&binding.value) {
            let ctx = make_xpath_context(doc, context_node, &combined, ns_bindings);
            ctx.evaluate(&expr)
                .unwrap_or_else(|_| XPathValue::String(binding.value.clone()))
        } else {
            XPathValue::String(binding.value.clone())
        };

        result.insert(binding.name.clone(), value.clone());
        combined.insert(binding.name.clone(), value);
    }

    result
}

/// Interpolates message parts by evaluating `<sch:value-of>` expressions.
fn interpolate_message(
    doc: &Document,
    node: XPathNode,
    parts: &[MessagePart],
    variables: &HashMap<String, XPathValue>,
    ns_bindings: &[NamespaceBinding],
) -> String {
    let mut result = String::new();
    for part in parts {
        match part {
            MessagePart::Text(text) => result.push_str(text),
            MessagePart::ValueOf { select } => {
                if let Ok(expr) = xpath::parser::parse(select) {
                    let ctx = make_xpath_context(doc, node, variables, ns_bindings);
                    if let Ok(val) = ctx.evaluate(&expr) {
                        result.push_str(&xpath_value_to_string(doc, &val));
                    }
                }
            }
        }
    }
    result
}

/// Converts an `XPath` value to a string, computing string-value for
/// node-sets using the document (unlike `to_xpath_string()` which returns
/// empty for node-sets without document access).
fn xpath_value_to_string(doc: &Document, val: &XPathValue) -> String {
    match val {
        XPathValue::NodeSet(nodes) => match nodes.first() {
            Some(&XPathNode::Attribute { owner, index }) => doc
                .attributes(owner)
                .get(index as usize)
                .map(|a| a.value.clone())
                .unwrap_or_default(),
            Some(&XPathNode::Node(id)) => doc.text_content(id),
            None => String::new(),
        },
        _ => val.to_xpath_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // ===================================================================
    // Phase 1: Parsing tests
    // ===================================================================

    #[test]
    fn test_parse_minimal_schema() {
        let schema =
            parse_schematron(r#"<schema xmlns="http://purl.oclc.org/dml/schematron"/>"#).unwrap();
        assert!(schema.patterns.is_empty());
        assert!(schema.namespaces.is_empty());
        assert!(schema.variables.is_empty());
        assert!(schema.phases.is_empty());
        assert!(schema.default_phase.is_none());
    }

    #[test]
    fn test_parse_single_assert() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <assert test="@id">root must have an id</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        assert_eq!(schema.patterns.len(), 1);
        assert_eq!(schema.patterns[0].rules.len(), 1);
        assert_eq!(schema.patterns[0].rules[0].context, "/root");
        assert_eq!(schema.patterns[0].rules[0].checks.len(), 1);
        match &schema.patterns[0].rules[0].checks[0] {
            SchematronCheck::Assert { test, message } => {
                assert_eq!(test, "@id");
                assert_eq!(message.len(), 1);
                match &message[0] {
                    MessagePart::Text(t) => assert_eq!(t, "root must have an id"),
                    MessagePart::ValueOf { .. } => panic!("expected Text message part"),
                }
            }
            SchematronCheck::Report { .. } => panic!("expected Assert check"),
        }
    }

    #[test]
    fn test_parse_report() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="//item">
                  <report test="@deprecated">item is deprecated</report>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        match &schema.patterns[0].rules[0].checks[0] {
            SchematronCheck::Report { test, message } => {
                assert_eq!(test, "@deprecated");
                assert_eq!(message.len(), 1);
            }
            SchematronCheck::Assert { .. } => panic!("expected Report check"),
        }
    }

    #[test]
    fn test_parse_multiple_patterns() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern id="p1">
                <rule context="/a">
                  <assert test="b">need b</assert>
                </rule>
              </pattern>
              <pattern id="p2">
                <rule context="/a">
                  <assert test="c">need c</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        assert_eq!(schema.patterns.len(), 2);
        assert_eq!(schema.patterns[0].id.as_deref(), Some("p1"));
        assert_eq!(schema.patterns[1].id.as_deref(), Some("p2"));
    }

    #[test]
    fn test_parse_ns_bindings() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <ns prefix="inv" uri="urn:invoice"/>
              <ns prefix="cbc" uri="urn:oasis:names:cbc"/>
            </schema>
            "#,
        )
        .unwrap();
        assert_eq!(schema.namespaces.len(), 2);
        assert_eq!(schema.namespaces[0].prefix, "inv");
        assert_eq!(schema.namespaces[0].uri, "urn:invoice");
        assert_eq!(schema.namespaces[1].prefix, "cbc");
        assert_eq!(schema.namespaces[1].uri, "urn:oasis:names:cbc");
    }

    #[test]
    fn test_parse_let_bindings() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <let name="threshold" value="100"/>
              <pattern>
                <let name="pat_var" value="'hello'"/>
                <rule context="/*">
                  <let name="rule_var" value="@count"/>
                  <assert test="$rule_var > $threshold">too low</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        assert_eq!(schema.variables.len(), 1);
        assert_eq!(schema.variables[0].name, "threshold");
        assert_eq!(schema.variables[0].value, "100");
        assert_eq!(schema.patterns[0].variables.len(), 1);
        assert_eq!(schema.patterns[0].variables[0].name, "pat_var");
        assert_eq!(schema.patterns[0].rules[0].variables.len(), 1);
        assert_eq!(schema.patterns[0].rules[0].variables[0].name, "rule_var");
    }

    #[test]
    fn test_parse_value_of_in_message() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <assert test="@id">element <value-of select="name()"/> must have an id</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let check = &schema.patterns[0].rules[0].checks[0];
        match check {
            SchematronCheck::Assert { message, .. } => {
                assert_eq!(message.len(), 3);
                match &message[0] {
                    MessagePart::Text(t) => assert_eq!(t, "element "),
                    MessagePart::ValueOf { .. } => panic!("expected Text"),
                }
                match &message[1] {
                    MessagePart::ValueOf { select } => assert_eq!(select, "name()"),
                    MessagePart::Text(_) => panic!("expected ValueOf"),
                }
                match &message[2] {
                    MessagePart::Text(t) => assert_eq!(t, " must have an id"),
                    MessagePart::ValueOf { .. } => panic!("expected Text"),
                }
            }
            SchematronCheck::Report { .. } => panic!("expected Assert"),
        }
    }

    #[test]
    fn test_parse_error_missing_context() {
        let result = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule>
                  <assert test="true()">ok</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("context"),
            "error should mention 'context': {}",
            err.message
        );
    }

    // ===================================================================
    // Phase 2: Basic validation tests
    // ===================================================================

    #[test]
    fn test_validate_assert_passes() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <assert test="child">root must have a child</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root><child/></root>").unwrap();
        let result = validate_schematron(&doc, &schema);
        assert!(result.is_valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validate_assert_fails() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <assert test="child">root must have a child</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root/>").unwrap();
        let result = validate_schematron(&doc, &schema);
        assert!(!result.is_valid);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].message, "root must have a child");
    }

    #[test]
    fn test_validate_report_fires() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <report test="@deprecated">element is deprecated</report>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str(r#"<root deprecated="true"/>"#).unwrap();
        let result = validate_schematron(&doc, &schema);
        // Reports produce warnings, not errors
        assert!(result.is_valid);
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].message, "element is deprecated");
    }

    #[test]
    fn test_validate_report_silent() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <report test="@deprecated">element is deprecated</report>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root/>").unwrap();
        let result = validate_schematron(&doc, &schema);
        assert!(result.is_valid);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_validate_multiple_asserts() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <assert test="@id">must have id</assert>
                  <assert test="child">must have child</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root/>").unwrap();
        let result = validate_schematron(&doc, &schema);
        assert!(!result.is_valid);
        assert_eq!(result.errors.len(), 2);
    }

    #[test]
    fn test_validate_context_multiple_nodes() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="//item">
                  <assert test="@name">item must have name</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc =
            Document::parse_str(r#"<root><item name="a"/><item/><item name="c"/></root>"#).unwrap();
        let result = validate_schematron(&doc, &schema);
        assert!(!result.is_valid);
        // Only the second <item> lacks @name
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn test_validate_no_matching_nodes() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="//nonexistent">
                  <assert test="false()">should never fire</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root/>").unwrap();
        let result = validate_schematron(&doc, &schema);
        assert!(result.is_valid);
    }

    #[test]
    fn test_validate_multiple_patterns() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <assert test="@id">need id</assert>
                </rule>
              </pattern>
              <pattern>
                <rule context="/root">
                  <assert test="child">need child</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root/>").unwrap();
        let result = validate_schematron(&doc, &schema);
        assert!(!result.is_valid);
        // Both patterns fire on the same node (different patterns = independent)
        assert_eq!(result.errors.len(), 2);
    }

    // ===================================================================
    // Phase 3: Firing rules tests
    // ===================================================================

    #[test]
    fn test_firing_rule_first_wins() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <assert test="true()">first rule passes</assert>
                </rule>
                <rule context="/root">
                  <assert test="false()">second rule would fail</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root/>").unwrap();
        let result = validate_schematron(&doc, &schema);
        // The second rule never fires because /root already fired in rule 1
        assert!(result.is_valid);
    }

    #[test]
    fn test_firing_rule_across_patterns() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <assert test="true()">pattern 1 passes</assert>
                </rule>
              </pattern>
              <pattern>
                <rule context="/root">
                  <assert test="false()">pattern 2 fails</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root/>").unwrap();
        let result = validate_schematron(&doc, &schema);
        // Same node fires independently in each pattern
        assert!(!result.is_valid);
        assert_eq!(result.errors.len(), 1);
    }

    // ===================================================================
    // Phase 4: Variables tests
    // ===================================================================

    #[test]
    fn test_variable_schema_level() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <let name="threshold" value="100"/>
              <pattern>
                <rule context="/root">
                  <assert test="@count >= $threshold">count must be at least 100</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        let doc_pass = Document::parse_str(r#"<root count="150"/>"#).unwrap();
        assert!(validate_schematron(&doc_pass, &schema).is_valid);

        let doc_fail = Document::parse_str(r#"<root count="50"/>"#).unwrap();
        assert!(!validate_schematron(&doc_fail, &schema).is_valid);
    }

    #[test]
    fn test_variable_rule_level() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <let name="n" value="@name"/>
                  <assert test="string-length($n) > 0">name must not be empty</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        let doc_pass = Document::parse_str(r#"<root name="hello"/>"#).unwrap();
        assert!(validate_schematron(&doc_pass, &schema).is_valid);

        let doc_fail = Document::parse_str(r#"<root name=""/>"#).unwrap();
        assert!(!validate_schematron(&doc_fail, &schema).is_valid);
    }

    #[test]
    fn test_variable_xpath_expression() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <let name="total" value="count(item)"/>
                  <assert test="$total > 0">must have at least one item</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        let doc_pass = Document::parse_str("<root><item/><item/></root>").unwrap();
        assert!(validate_schematron(&doc_pass, &schema).is_valid);

        let doc_fail = Document::parse_str("<root/>").unwrap();
        assert!(!validate_schematron(&doc_fail, &schema).is_valid);
    }

    // ===================================================================
    // Phase 5: Message interpolation tests
    // ===================================================================

    #[test]
    fn test_message_value_of() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <assert test="false()">element <value-of select="name()"/> failed</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root/>").unwrap();
        let result = validate_schematron(&doc, &schema);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].message, "element root failed");
    }

    #[test]
    fn test_message_mixed() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/order">
                  <assert test="false()">Order <value-of select="@id"/> has <value-of select="count(item)"/> items</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str(r#"<order id="42"><item/><item/><item/></order>"#).unwrap();
        let result = validate_schematron(&doc, &schema);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].message, "Order 42 has 3 items");
    }

    #[test]
    fn test_attribute_rule_context() {
        // A rule context selecting attribute nodes evaluates its asserts
        // with the ATTRIBUTE as the context node: `.` is the attribute
        // value, not the owner element's text content.
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="//@id">
                  <assert test=". = 'x'">id must be x (got <value-of select="."/>)</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        // Valid: the id attribute IS 'x' even though the element text differs.
        let doc = Document::parse_str(r#"<r><a id="x">element text</a></r>"#).unwrap();
        let result = validate_schematron(&doc, &schema);
        assert!(result.is_valid, "errors: {:?}", result.errors);

        // Invalid: the assert fails and value-of reads the attribute value.
        let doc2 = Document::parse_str(r#"<r><a id="y">element text</a></r>"#).unwrap();
        let result2 = validate_schematron(&doc2, &schema);
        assert!(!result2.is_valid);
        assert_eq!(result2.errors[0].message, "id must be x (got y)");
    }

    #[test]
    fn test_attribute_rule_context_does_not_mask_element_rules() {
        // Firing an attribute-context rule must not mark the owner ELEMENT
        // as fired for later rules in the same pattern.
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="//item/@code">
                  <assert test="string-length(.) >= 3">code too short</assert>
                </rule>
                <rule context="//item">
                  <assert test="@name">item must have a name attribute</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        // code is fine but @name is missing: exactly the element rule fires.
        let doc = Document::parse_str(r#"<order><item code="XYZ"/></order>"#).unwrap();
        let result = validate_schematron(&doc, &schema);
        assert_eq!(result.errors.len(), 1, "errors: {:?}", result.errors);
        assert_eq!(result.errors[0].message, "item must have a name attribute");
    }

    // ===================================================================
    // Phase 6: Phases + integration tests
    // ===================================================================

    #[test]
    fn test_phase_selective() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron" defaultPhase="quick">
              <phase id="quick">
                <active pattern="basic"/>
              </phase>
              <pattern id="basic">
                <rule context="/*">
                  <assert test="true()">basic passes</assert>
                </rule>
              </pattern>
              <pattern id="strict">
                <rule context="/*">
                  <assert test="false()">strict fails</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root/>").unwrap();

        // Default phase is "quick", which only activates "basic"
        let result = validate_schematron(&doc, &schema);
        assert!(result.is_valid);

        let schema2 = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <phase id="quick">
                <active pattern="basic"/>
              </phase>
              <phase id="full">
                <active pattern="basic"/>
                <active pattern="strict"/>
              </phase>
              <pattern id="basic">
                <rule context="/*">
                  <assert test="true()">basic passes</assert>
                </rule>
              </pattern>
              <pattern id="strict">
                <rule context="/*">
                  <assert test="false()">strict fails</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        let quick = validate_schematron_with_phase(&doc, &schema2, "quick");
        assert!(quick.is_valid);

        let full = validate_schematron_with_phase(&doc, &schema2, "full");
        assert!(!full.is_valid);
        assert_eq!(full.errors.len(), 1);
    }

    #[test]
    fn test_validate_invoice_schema() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <let name="min_items" value="1"/>
              <pattern id="structure">
                <rule context="/invoice">
                  <assert test="@id">Invoice must have an id</assert>
                  <assert test="customer">Invoice must have a customer</assert>
                  <assert test="count(item) >= $min_items">Invoice must have at least <value-of select="$min_items"/> item(s)</assert>
                </rule>
              </pattern>
              <pattern id="amounts">
                <rule context="//item">
                  <assert test="number(@amount) > 0">Item <value-of select="@name"/> amount must be positive</assert>
                </rule>
              </pattern>
              <pattern id="names">
                <rule context="//item">
                  <assert test="@name">Every item must have a name</assert>
                  <report test="@discount">Item <value-of select="@name"/> has a discount applied</report>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        // Valid invoice
        let valid_doc = Document::parse_str(
            r#"<invoice id="INV-001">
                 <customer>Acme Corp</customer>
                 <item name="Widget" amount="10"/>
                 <item name="Gadget" amount="20"/>
               </invoice>"#,
        )
        .unwrap();
        let result = validate_schematron(&valid_doc, &schema);
        assert!(
            result.is_valid,
            "valid invoice should pass: {:?}",
            result.errors
        );

        // Invalid: missing id, no customer, zero amount
        let invalid_doc = Document::parse_str(
            r#"<invoice>
                 <item name="Widget" amount="10"/>
                 <item name="Gadget" amount="0"/>
               </invoice>"#,
        )
        .unwrap();
        let result = validate_schematron(&invalid_doc, &schema);
        assert!(!result.is_valid);
        // Errors: missing id, missing customer, zero amount on Gadget
        assert!(
            result.errors.len() >= 3,
            "expected at least 3 errors, got {}: {:?}",
            result.errors.len(),
            result.errors
        );

        // Test report fires on discount attribute
        let discount_doc = Document::parse_str(
            r#"<invoice id="INV-002">
                 <customer>Beta Corp</customer>
                 <item name="Widget" amount="10" discount="5"/>
               </invoice>"#,
        )
        .unwrap();
        let result = validate_schematron(&discount_doc, &schema);
        assert!(result.is_valid);
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(
            result.warnings[0].message,
            "Item Widget has a discount applied"
        );
    }

    #[test]
    fn test_xpath_error_recovery() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/root">
                  <assert test="[[[invalid xpath">should not crash</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();
        let doc = Document::parse_str("<root/>").unwrap();
        let result = validate_schematron(&doc, &schema);
        // Should report an error about XPath, not panic
        assert!(!result.is_valid);
        assert!(
            result.errors[0].message.contains("XPath error"),
            "error should mention XPath: {}",
            result.errors[0].message
        );
    }

    #[test]
    fn test_validate_sum_attribute_path() {
        // Tests that sum(child/@attr) works correctly now that
        // attribute paths return proper NodeSets.
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/order">
                  <let name="total" value="sum(item/@price)"/>
                  <assert test="$total = @expected">Total <value-of select="$total"/> does not match expected <value-of select="@expected"/></assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        let doc_pass = Document::parse_str(
            r#"<order expected="30"><item price="10"/><item price="20"/></order>"#,
        )
        .unwrap();
        let result = validate_schematron(&doc_pass, &schema);
        assert!(result.is_valid, "sum should equal 30: {:?}", result.errors);

        let doc_fail = Document::parse_str(
            r#"<order expected="99"><item price="10"/><item price="20"/></order>"#,
        )
        .unwrap();
        let result = validate_schematron(&doc_fail, &schema);
        assert!(!result.is_valid);
    }

    // ===================================================================
    // Namespace-prefixed XPath tests
    // ===================================================================

    #[test]
    fn test_namespace_prefixed_xpath() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <ns prefix="inv" uri="urn:example:invoice"/>
              <pattern>
                <rule context="/inv:invoice">
                  <assert test="inv:customer">Invoice must have a customer</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        // Document with namespace
        let doc_pass = Document::parse_str(
            r#"<invoice xmlns="urn:example:invoice"><customer>Acme</customer></invoice>"#,
        )
        .unwrap();
        let result = validate_schematron(&doc_pass, &schema);
        assert!(
            result.is_valid,
            "namespace-prefixed XPath should match: {:?}",
            result.errors
        );

        // Document with namespace but missing customer
        let doc_fail = Document::parse_str(r#"<invoice xmlns="urn:example:invoice"/>"#).unwrap();
        let result = validate_schematron(&doc_fail, &schema);
        assert!(!result.is_valid);
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn test_namespace_prefix_wildcard() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <ns prefix="inv" uri="urn:example:invoice"/>
              <pattern>
                <rule context="/inv:*">
                  <assert test="@id">Root element must have an id</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        let doc = Document::parse_str(r#"<invoice xmlns="urn:example:invoice" id="1"/>"#).unwrap();
        let result = validate_schematron(&doc, &schema);
        assert!(result.is_valid);

        let doc_fail = Document::parse_str(r#"<invoice xmlns="urn:example:invoice"/>"#).unwrap();
        let result = validate_schematron(&doc_fail, &schema);
        assert!(!result.is_valid);
    }

    // ===================================================================
    // Additional edge case tests
    // ===================================================================

    #[test]
    fn test_matches_function() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern>
                <rule context="/order">
                  <assert test="matches(@country, '[A-Z]{2}')">Country must be a 2-letter ISO code</assert>
                  <assert test="matches(@id, '[A-Z]+-\d+')">ID must match format LETTERS-DIGITS</assert>
                </rule>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        let doc_pass = Document::parse_str(r#"<order country="US" id="INV-42"/>"#).unwrap();
        assert!(validate_schematron(&doc_pass, &schema).is_valid);

        let doc_fail = Document::parse_str(r#"<order country="usa" id="123"/>"#).unwrap();
        let result = validate_schematron(&doc_fail, &schema);
        assert!(!result.is_valid);
        assert_eq!(result.errors.len(), 2);
    }

    #[test]
    fn test_classic_namespace() {
        let schema = parse_schematron(
            r#"<schema xmlns="http://www.ascc.net/xml/schematron">
              <pattern>
                <rule context="/*">
                  <assert test="true()">ok</assert>
                </rule>
              </pattern>
            </schema>"#,
        )
        .unwrap();
        assert_eq!(schema.patterns.len(), 1);
        let doc = Document::parse_str("<root/>").unwrap();
        assert!(validate_schematron(&doc, &schema).is_valid);
    }

    #[test]
    fn test_prefixed_schema() {
        let schema = parse_schematron(
            r#"<sch:schema xmlns:sch="http://purl.oclc.org/dml/schematron">
              <sch:pattern>
                <sch:rule context="/*">
                  <sch:assert test="true()">ok</sch:assert>
                </sch:rule>
              </sch:pattern>
            </sch:schema>"#,
        )
        .unwrap();
        assert_eq!(schema.patterns.len(), 1);
    }

    // ===================================================================
    // Abstract pattern tests
    // ===================================================================

    #[test]
    fn test_abstract_pattern_basic() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern id="req_attr" abstract="true">
                <rule context="$element">
                  <assert test="@$attr">Element must have $attr attribute</assert>
                </rule>
              </pattern>
              <pattern id="check_id" is-a="req_attr">
                <param name="element" value="//item"/>
                <param name="attr" value="id"/>
              </pattern>
              <pattern id="check_name" is-a="req_attr">
                <param name="element" value="//item"/>
                <param name="attr" value="name"/>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        // Abstract pattern should be excluded, two concrete patterns remain
        assert_eq!(schema.patterns.len(), 2);

        // First pattern should have context "//item" and test "@id"
        assert_eq!(schema.patterns[0].rules[0].context, "//item");
        match &schema.patterns[0].rules[0].checks[0] {
            SchematronCheck::Assert { test, .. } => assert_eq!(test, "@id"),
            SchematronCheck::Report { .. } => panic!("expected assert"),
        }

        // Validate
        let doc_pass = Document::parse_str(r#"<root><item id="1" name="x"/></root>"#).unwrap();
        assert!(validate_schematron(&doc_pass, &schema).is_valid);

        let doc_fail = Document::parse_str(r#"<root><item id="1"/></root>"#).unwrap();
        let result = validate_schematron(&doc_fail, &schema);
        assert!(!result.is_valid);
        // Missing name attribute
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn test_abstract_pattern_multiple_rules() {
        let schema = parse_schematron(
            r#"
            <schema xmlns="http://purl.oclc.org/dml/schematron">
              <pattern id="has_content" abstract="true">
                <rule context="$ctx">
                  <assert test="string-length(normalize-space(.)) > 0">$ctx must not be empty</assert>
                </rule>
              </pattern>
              <pattern is-a="has_content">
                <param name="ctx" value="/doc/title"/>
              </pattern>
              <pattern is-a="has_content">
                <param name="ctx" value="/doc/body"/>
              </pattern>
            </schema>
            "#,
        )
        .unwrap();

        let doc_pass =
            Document::parse_str("<doc><title>Hi</title><body>Content</body></doc>").unwrap();
        assert!(validate_schematron(&doc_pass, &schema).is_valid);

        let doc_fail = Document::parse_str("<doc><title>Hi</title><body>  </body></doc>").unwrap();
        assert!(!validate_schematron(&doc_fail, &schema).is_valid);
    }
}
