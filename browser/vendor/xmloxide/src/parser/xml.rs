//! Core XML 1.0 parser state machine.
//!
//! Implements a hand-rolled recursive descent parser for XML 1.0 (Fifth Edition).
//! See <https://www.w3.org/TR/xml/> for the specification.

use std::collections::HashMap;

use crate::error::{ErrorSeverity, ParseError};
use crate::tree::{Attribute, Document, NodeId, NodeKind};
use crate::validation::dtd::{parse_dtd, serialize_dtd, AttributeDecl, AttributeType, EntityKind};

use super::input::{
    find_invalid_xml_char, may_contain_invalid_xml_chars, parse_cdata_content,
    parse_comment_content, parse_pi_content, parse_xml_decl, split_name, split_owned_name,
    validate_pubid, ExternalEntityInfo, NamespaceResolver, ParserInput, XMLNS_NAMESPACE,
    XML_NAMESPACE,
};
use super::ParseOptions;

/// Default maximum amplification factor for entity/attribute expansion.
///
/// libxml2 uses a factor of 5 — if the expanded output exceeds 5x the input
/// size due to default attribute application or entity expansion, the document
/// is rejected as a potential denial-of-service attack.
const DEFAULT_MAX_AMPLIFICATION: usize = 5;

/// Maximum nesting depth of general entity expansion in content.
///
/// Bounds the recursion when an entity's replacement text references other
/// entities (XML 1.0 §4.4 "Included"). Recursion is already rejected by the
/// DTD parser (WFC: No Recursion), so this is defense-in-depth for
/// resolver-provided external entities and stack safety.
const MAX_ENTITY_DEPTH: u32 = 32;

/// The core XML parser.
pub(crate) struct XmlParser<'a> {
    /// Shared low-level input state (position, peek, advance, name parsing, etc.).
    input: ParserInput<'a>,
    /// The document being built.
    doc: Document,
    /// Parser options.
    options: ParseOptions,
    /// Namespace resolver managing the scope stack.
    ns: NamespaceResolver,
    /// DTD attribute type declarations, keyed by `(element_name, attr_name)`.
    /// Used for attribute value normalization of namespace URIs.
    attr_types: HashMap<(String, String), AttributeType>,
    /// DTD attribute default value declarations, keyed by element name.
    /// Used for applying default/fixed attributes from ATTLIST.
    attr_defaults: HashMap<String, Vec<AttributeDecl>>,
    /// Total input size in bytes, used for amplification factor checking.
    input_size: usize,
    /// Running total of bytes added through default attribute expansion.
    expansion_size: usize,
    /// Current nesting depth of general entity expansion (0 = document level).
    entity_depth: u32,
}

impl<'a> XmlParser<'a> {
    pub fn new(input: &'a str, options: &ParseOptions) -> Self {
        let mut pi = ParserInput::new(input);
        pi.set_recover(options.recover);
        pi.set_max_depth(options.max_depth);
        pi.set_max_name_length(options.max_name_length);
        pi.set_max_entity_expansions(options.max_entity_expansions);
        pi.set_entity_resolver(options.entity_resolver.clone());

        // Pre-size the node arena — roughly 1 node per 30 bytes of input.
        let estimated_nodes = (input.len() / 30).max(64);
        Self {
            input: pi,
            doc: Document::with_capacity(estimated_nodes),
            options: options.clone(),
            ns: NamespaceResolver::new(),
            attr_types: HashMap::new(),
            attr_defaults: HashMap::new(),
            input_size: input.len(),
            expansion_size: 0,
            entity_depth: 0,
        }
    }

    /// Main parse entry point. Parses the entire document.
    pub fn parse(&mut self) -> Result<Document, ParseError> {
        // Parse optional XML declaration — must be at the very start of the
        // document with no leading whitespace (XML 1.0 §2.8).
        if self.input.looking_at(b"<?xml ")
            || self.input.looking_at(b"<?xml\t")
            || self.input.looking_at(b"<?xml\r")
        {
            self.parse_xml_declaration()?;
            // Skip whitespace immediately after the XML declaration — the
            // serializer always emits its own newline after the declaration.
            self.input.skip_whitespace();
        } else if !self.input.at_end() {
            // If there's no XML declaration, skip any leading whitespace.
            // Leading whitespace before a non-declaration is tolerated
            // (it will be handled as misc content).
            let had_leading_ws = self.input.skip_whitespace();
            // But if the whitespace was hiding an XML declaration, that's an error
            if had_leading_ws
                && (self.input.looking_at(b"<?xml ")
                    || self.input.looking_at(b"<?xml\t")
                    || self.input.looking_at(b"<?xml\r"))
            {
                return Err(self
                    .input
                    .fatal("XML declaration must be at the start of the document"));
            }
        }

        // Parse prolog content (comments, PIs, whitespace before root element)
        self.parse_misc(self.doc.root())?;

        // Parse optional DOCTYPE declaration
        if self.input.looking_at(b"<!DOCTYPE") || self.input.looking_at(b"<!doctype") {
            self.parse_doctype(self.doc.root())?;
            self.parse_misc(self.doc.root())?; // more misc after doctype
        }

        // Parse root element (required by XML 1.0 §2.1)
        if self.input.peek() == Some(b'<')
            && self
                .input
                .peek_at(1)
                .is_some_and(|b| b != b'!' && b != b'?')
        {
            self.parse_element(self.doc.root())?;
        } else if self.options.recover {
            self.input
                .push_diagnostic(ErrorSeverity::Error, "missing root element".to_string());
        } else {
            return Err(self.input.fatal("missing root element"));
        }

        // Parse trailing content (comments, PIs after root element)
        self.parse_misc(self.doc.root())?;

        self.input.skip_whitespace();
        if !self.input.at_end() && !self.options.recover {
            return Err(self.input.fatal("content after document element"));
        }

        // Sync diagnostics from input to document before returning.
        self.doc.diagnostics = std::mem::take(&mut self.input.diagnostics);

        Ok(std::mem::take(&mut self.doc))
    }

    // --- XML Declaration ---
    // See XML 1.0 §2.8: [23] XMLDecl

    fn parse_xml_declaration(&mut self) -> Result<(), ParseError> {
        let decl = parse_xml_decl(&mut self.input)?;
        self.doc.version = Some(decl.version);
        self.doc.encoding = decl.encoding;
        self.doc.standalone = decl.standalone;
        Ok(())
    }

    // --- Misc (comments, PIs, whitespace) ---

    fn parse_misc(&mut self, parent: NodeId) -> Result<(), ParseError> {
        loop {
            // Preserve document-level whitespace as text nodes (matches libxml2).
            // libxml2 normalizes prolog/epilog whitespace to a single `\n`
            // regardless of how many blank lines appear in the source.
            let ws = self.input.consume_whitespace();
            if !ws.is_empty() {
                let ws_node = self.doc.create_node(NodeKind::Text {
                    content: "\n".to_string(),
                });
                self.doc.append_child(parent, ws_node);
            }
            if self.input.at_end() {
                break;
            }
            if self.input.looking_at(b"<!--") {
                self.parse_comment(parent)?;
            } else if self.input.looking_at(b"<?") {
                self.parse_processing_instruction(parent)?;
            } else {
                break;
            }
        }
        Ok(())
    }

    // --- DOCTYPE Declaration ---
    // See XML 1.0 §2.8: [28] doctypedecl

    #[allow(clippy::too_many_lines)]
    fn parse_doctype(&mut self, parent: NodeId) -> Result<(), ParseError> {
        // Consume <!DOCTYPE (case-insensitive match already checked by caller)
        self.input.expect_str(b"<!DOCTYPE")?;
        self.input.skip_whitespace_required()?;

        // Read the root element name
        let name = self.input.parse_name()?;

        self.input.skip_whitespace();

        // Check for external ID: SYSTEM or PUBLIC
        let mut system_id = None;
        let mut public_id = None;

        if self.input.looking_at(b"SYSTEM") {
            self.input.expect_str(b"SYSTEM")?;
            self.input.skip_whitespace_required()?;
            system_id = Some(self.input.parse_quoted_value()?);
            self.input.skip_whitespace();
        } else if self.input.looking_at(b"PUBLIC") {
            self.input.expect_str(b"PUBLIC")?;
            self.input.skip_whitespace_required()?;
            let pid = self.input.parse_quoted_value()?;
            // Validate public ID characters per XML 1.0 §2.3 [13].
            if let Some(msg) = validate_pubid(&pid) {
                if self.options.recover {
                    self.input.push_diagnostic(ErrorSeverity::Warning, msg);
                } else {
                    return Err(self.input.fatal(msg));
                }
            }
            public_id = Some(pid);
            self.input.skip_whitespace_required()?;
            system_id = Some(self.input.parse_quoted_value()?);
            self.input.skip_whitespace();
        }

        // Flag whether there's an external DTD subset. Per XML 1.0 §4.1 WFC:
        // Entity Declared, undeclared entities are not WF errors when the
        // document references an unread external DTD subset.
        if system_id.is_some() || public_id.is_some() {
            self.input.has_external_dtd = true;
        }

        // Parse optional internal subset: [ ... ]
        let mut internal_subset = None;
        if self.input.peek() == Some(b'[') {
            self.input.advance(1);
            let start = self.input.pos();

            // Scan to matching ']', tracking depth for bracket chars inside
            // entity values. Quoted strings and comments are skipped to avoid
            // misinterpreting brackets or apostrophes in comments.
            let mut depth: u32 = 1;
            while !self.input.at_end() && depth > 0 {
                if self.input.looking_at(b"<!--") {
                    // Skip XML comments (may contain apostrophes/quotes)
                    self.input.advance(4);
                    while !self.input.at_end() && !self.input.looking_at(b"-->") {
                        self.input.advance(1);
                    }
                    if !self.input.at_end() {
                        self.input.advance(3); // consume -->
                    }
                } else if let Some(b'"' | b'\'') = self.input.peek() {
                    let quote = self.input.peek().unwrap_or(b'"');
                    self.input.advance(1);
                    while !self.input.at_end() && self.input.peek() != Some(quote) {
                        self.input.advance(1);
                    }
                    if !self.input.at_end() {
                        self.input.advance(1); // closing quote
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
            if depth > 0 {
                return Err(self
                    .input
                    .fatal("unexpected end of input in internal subset"));
            }

            // Extract the internal subset text (between '[' and ']').
            let end = self.input.pos() - 1; // exclude the closing ']'
            let subset_text = std::str::from_utf8(self.input.slice(start, end))
                .ok()
                .map(str::to_string);

            if let Some(subset_text) = subset_text {
                // Detect parameter entity references in the internal subset.
                // Per XML 1.0 §4.1 WFC: Entity Declared, their presence
                // means undeclared general entities are not WF errors.
                if subset_text.contains('%') {
                    self.input.has_pe_references = true;
                }

                match parse_dtd(&subset_text) {
                    Ok(dtd) => {
                        // Wire entity declarations into the parser input
                        // for entity reference resolution.
                        for (ent_name, ent_decl) in &dtd.entities {
                            match &ent_decl.kind {
                                EntityKind::Internal(value) => {
                                    self.input
                                        .entity_map
                                        .insert(ent_name.clone(), value.clone());
                                }
                                EntityKind::External {
                                    system_id,
                                    public_id,
                                } => {
                                    self.input.entity_external.insert(
                                        ent_name.clone(),
                                        ExternalEntityInfo {
                                            system_id: system_id.clone(),
                                            public_id: public_id.clone(),
                                        },
                                    );
                                }
                            }
                        }

                        // Wire attribute type declarations for namespace
                        // URI normalization (XML 1.0 §3.3.3).
                        for (element_name, attrs) in &dtd.attributes {
                            for attr_decl in attrs {
                                self.attr_types.insert(
                                    (element_name.clone(), attr_decl.attribute_name.clone()),
                                    attr_decl.attribute_type.clone(),
                                );
                            }
                        }

                        // Store attribute default values for later application.
                        for (element_name, attrs) in &dtd.attributes {
                            for attr_decl in attrs {
                                self.attr_defaults
                                    .entry(element_name.clone())
                                    .or_default()
                                    .push(attr_decl.clone());
                            }
                        }

                        // Re-serialize the DTD from parsed structures for
                        // consistent formatting (matches libxml2 behavior).
                        let serialized = serialize_dtd(&dtd);
                        if !serialized.is_empty() {
                            internal_subset = Some(serialized);
                        }
                    }
                    Err(e) => {
                        if self.options.recover {
                            self.input.push_diagnostic(
                                ErrorSeverity::Warning,
                                format!("error parsing DTD internal subset: {}", e.message),
                            );
                        } else {
                            return Err(self
                                .input
                                .fatal(format!("error in DTD internal subset: {}", e.message)));
                        }
                    }
                }
            }

            self.input.skip_whitespace();
        }

        self.input.expect_byte(b'>')?;

        let doctype_id = self.doc.create_node(NodeKind::DocumentType {
            name,
            system_id,
            public_id,
            internal_subset,
        });
        self.doc.append_child(parent, doctype_id);
        Ok(())
    }

    // --- Elements ---
    // See XML 1.0 §3.1: [40] STag, [42] ETag, [44] EmptyElemTag

    #[allow(clippy::too_many_lines)]
    fn parse_element(&mut self, parent: NodeId) -> Result<NodeId, ParseError> {
        self.input.increment_depth()?;
        self.input.expect_byte(b'<')?;
        let name = self.input.parse_name()?;
        let mut attributes = Vec::new();

        // Parse attributes
        loop {
            let had_ws = self.input.skip_whitespace();
            if self.input.peek() == Some(b'>') || self.input.looking_at(b"/>") {
                break;
            }
            if !had_ws {
                return Err(self.input.fatal("whitespace required between attributes"));
            }
            let attr = self.parse_attribute()?;
            attributes.push(attr);
        }

        // Check for duplicate attributes (XML 1.0 §3.1 WFC: Unique Att Spec)
        // Skip for 0 or 1 attributes (no duplicates possible).
        if attributes.len() >= 2 {
            // O(n²) comparison avoids HashSet allocation for small attribute lists.
            let mut found_dup = false;
            'outer: for i in 1..attributes.len() {
                for j in 0..i {
                    if attributes[i].name == attributes[j].name
                        && attributes[i].prefix == attributes[j].prefix
                    {
                        let full_name = if let Some(ref pfx) = attributes[i].prefix {
                            format!("{pfx}:{}", attributes[i].name)
                        } else {
                            attributes[i].name.clone()
                        };
                        if self.options.recover {
                            self.input.push_diagnostic(
                                ErrorSeverity::Error,
                                format!("duplicate attribute: '{full_name}'"),
                            );
                            found_dup = true;
                        } else {
                            return Err(self
                                .input
                                .fatal(format!("duplicate attribute: '{full_name}'")));
                        }
                        break 'outer;
                    }
                }
            }
            let _ = found_dup; // suppress unused warning
        }

        // --- Apply DTD ATTLIST default attributes (#FIXED and #DEFAULT) ---
        // Per XML 1.0 §3.3.2, when an attribute declared in an ATTLIST is not
        // present on the element, the parser must add it with the declared
        // default value. This applies to both `#FIXED "v"` and bare `"v"`
        // (so-called #DEFAULT) declarations. libxml2 applies both during
        // normal parsing — verifiable via `xmllint --c14n` on a document
        // with an ATTLIST default, which emits the default attribute in the
        // canonical form.
        // Namespace declarations (xmlns, xmlns:prefix) are inserted before
        // other attributes to match libxml2's attribute ordering.
        if let Some(defaults) = if self.attr_defaults.is_empty() {
            None
        } else {
            self.attr_defaults.get(&name).cloned()
        } {
            let mut insert_pos = 0; // insertion point for namespace declarations
            for attr_decl in &defaults {
                let (default_value, is_fixed) = match &attr_decl.default {
                    crate::validation::dtd::AttributeDefault::Fixed(v) => (Some(v.clone()), true),
                    crate::validation::dtd::AttributeDefault::Default(v) => {
                        (Some(v.clone()), false)
                    }
                    _ => (None, false),
                };
                if let Some(value) = default_value {
                    // Check if the attribute is already present by comparing
                    // prefix:local components directly, avoiding format!/clone.
                    let attr_name = &attr_decl.attribute_name;
                    let (decl_pfx, decl_local) = split_name(attr_name);
                    let already_present = attributes
                        .iter()
                        .any(|a| a.name == decl_local && a.prefix.as_deref() == decl_pfx);
                    if !already_present {
                        // Track expansion for amplification factor check
                        // (both #FIXED and #DEFAULT contribute to expansion).
                        self.expansion_size += attr_name.len() + value.len();

                        // Insert both #FIXED and #DEFAULT defaults into the
                        // tree. The `is_fixed` flag is no longer used to gate
                        // insertion; it would only matter if we additionally
                        // validated source attributes against #FIXED values,
                        // which is a separate validation step.
                        let _ = is_fixed;
                        let (decl_prefix, decl_local) = split_name(attr_name);
                        let attr = Attribute {
                            name: decl_local.to_string(),
                            value,
                            prefix: decl_prefix.map(String::from),
                            namespace: None,
                            raw_value: None,
                        };
                        let is_ns_decl = attr_name == "xmlns" || attr_name.starts_with("xmlns:");
                        if is_ns_decl {
                            attributes.insert(insert_pos, attr);
                            insert_pos += 1;
                        } else {
                            attributes.push(attr);
                        }
                    }
                }
            }

            // Check amplification factor: reject if default attribute
            // expansion would exceed the input size by more than the
            // maximum factor (matching libxml2's xmlCtxtSetMaxAmplification).
            if self.expansion_size > self.input_size.saturating_mul(DEFAULT_MAX_AMPLIFICATION) {
                return Err(self
                    .input
                    .fatal("maximum entity amplification factor exceeded"));
            }
        }

        // --- Namespace processing (Namespaces in XML 1.0 section 3) ---

        // Check for namespace declarations to skip namespace scope push/pop
        // when not needed. Skip the scan entirely when there are no attributes.
        let has_ns_decls = !attributes.is_empty()
            && attributes.iter().any(|a| {
                a.prefix.as_deref() == Some("xmlns") || (a.prefix.is_none() && a.name == "xmlns")
            });
        if has_ns_decls {
            self.ns.push_scope();
        }

        // Split into prefix and local name for namespace processing.
        let (prefix, local_name) = split_name(&name);

        // Validate QName syntax: check for multiple colons (the local part
        // should not contain a colon after split_name).
        if prefix.is_some() && local_name.contains(':') {
            let msg = "QName contains multiple colons";
            if self.options.recover {
                self.input
                    .push_diagnostic(ErrorSeverity::Error, msg.to_string());
            } else {
                return Err(self.input.fatal(msg));
            }
        }

        // Reject element names with "xmlns" prefix (Namespaces in XML 1.0 §3).
        if prefix == Some("xmlns") {
            if self.options.recover {
                self.input.push_diagnostic(
                    ErrorSeverity::Error,
                    "elements must not have the prefix 'xmlns'".to_string(),
                );
            } else {
                return Err(self
                    .input
                    .fatal("elements must not have the prefix 'xmlns'"));
            }
        }

        // Scan attributes for namespace declarations and bind them,
        // with validation of namespace constraints.
        // In recovery mode, some invalid attributes may be stripped.
        let mut strip_attr_indices: Vec<usize> = Vec::new();
        if has_ns_decls {
            for (attr_idx, attr) in attributes.iter().enumerate() {
                if attr.prefix.as_deref() == Some("xmlns") {
                    // Prefixed namespace declaration: xmlns:prefix="uri"
                    let declared_prefix = &attr.name;

                    // Validate QName: the local part (declared_prefix) must be
                    // a non-empty NCName (no colon, not empty). An empty local
                    // part means the attribute was `xmlns:` with nothing after
                    // the colon, which is not a valid QName.
                    if declared_prefix.is_empty() {
                        let msg = "namespace prefix must not be empty (invalid QName 'xmlns:')";
                        if self.options.recover {
                            self.input
                                .push_diagnostic(ErrorSeverity::Error, msg.to_string());
                            continue;
                        }
                        return Err(self.input.fatal(msg));
                    }
                    if declared_prefix.contains(':') {
                        let msg = "QName contains multiple colons";
                        if self.options.recover {
                            self.input
                                .push_diagnostic(ErrorSeverity::Error, msg.to_string());
                        } else {
                            return Err(self.input.fatal(msg));
                        }
                    }

                    // Normalize namespace URI based on DTD-declared attribute
                    // type. For non-CDATA types (e.g., NMTOKEN), whitespace is
                    // collapsed per XML 1.0 §3.3.3. Construct the full attribute
                    // name only when attr_types is non-empty (DTD present).
                    let ns_value = if self.attr_types.is_empty() {
                        attr.value.clone()
                    } else {
                        let attr_qname = format!("xmlns:{declared_prefix}");
                        self.normalize_attr_value_by_type(&name, &attr_qname, &attr.value)
                    };

                    // XML 1.0 Namespaces: cannot unbind a prefix (xmlns:prefix="").
                    if ns_value.is_empty() {
                        if self.options.recover {
                            self.input.push_diagnostic(
                                ErrorSeverity::Error,
                                format!("namespace prefix '{declared_prefix}' cannot be undeclared in XML 1.0"),
                            );
                        } else {
                            return Err(self.input.fatal(format!(
                                "namespace prefix '{declared_prefix}' cannot be undeclared in XML 1.0"
                            )));
                        }
                    }

                    // Cannot declare the 'xmlns' prefix itself.
                    if declared_prefix == "xmlns" {
                        if self.options.recover {
                            self.input.push_diagnostic(
                                ErrorSeverity::Error,
                                "the 'xmlns' prefix must not be declared".to_string(),
                            );
                        } else {
                            return Err(self
                                .input
                                .fatal("the 'xmlns' prefix must not be declared"));
                        }
                    }

                    // 'xml' prefix must map to the XML namespace URI and vice versa.
                    if declared_prefix == "xml" && ns_value != XML_NAMESPACE {
                        if self.options.recover {
                            self.input.push_diagnostic(
                                ErrorSeverity::Error,
                                "the 'xml' prefix must be bound to the XML namespace".to_string(),
                            );
                            // In recovery mode, strip the invalid rebinding
                            // (matches libxml2: output is <tst/> not <tst xmlns:xml="..."/>).
                            strip_attr_indices.push(attr_idx);
                            continue;
                        }
                        return Err(self
                            .input
                            .fatal("the 'xml' prefix must be bound to the XML namespace"));
                    }

                    // No other prefix may be bound to the XML namespace URI.
                    if declared_prefix != "xml" && ns_value == XML_NAMESPACE {
                        if self.options.recover {
                            self.input.push_diagnostic(
                                ErrorSeverity::Error,
                                "only the 'xml' prefix may be bound to the XML namespace"
                                    .to_string(),
                            );
                        } else {
                            return Err(self
                                .input
                                .fatal("only the 'xml' prefix may be bound to the XML namespace"));
                        }
                    }

                    // No prefix may be bound to the xmlns namespace URI.
                    if ns_value == XMLNS_NAMESPACE {
                        if self.options.recover {
                            self.input.push_diagnostic(
                                ErrorSeverity::Error,
                                "the xmlns namespace must not be bound to any prefix".to_string(),
                            );
                        } else {
                            return Err(self
                                .input
                                .fatal("the xmlns namespace must not be bound to any prefix"));
                        }
                    }

                    self.ns.bind(Some(attr.name.clone()), ns_value);
                } else if attr.prefix.is_none() && attr.name == "xmlns" {
                    // Default namespace declaration: xmlns="uri"

                    // Normalize namespace URI based on DTD-declared attribute type.
                    let ns_value = if self.attr_types.is_empty() {
                        attr.value.clone()
                    } else {
                        self.normalize_attr_value_by_type(&name, "xmlns", &attr.value)
                    };

                    // Cannot bind default namespace to the XML or xmlns namespace URIs.
                    if ns_value == XML_NAMESPACE {
                        if self.options.recover {
                            self.input.push_diagnostic(
                                ErrorSeverity::Error,
                                "the xml namespace must not be declared as the default namespace"
                                    .to_string(),
                            );
                        } else {
                            return Err(self.input.fatal(
                                "the xml namespace must not be declared as the default namespace",
                            ));
                        }
                    }
                    if ns_value == XMLNS_NAMESPACE {
                        if self.options.recover {
                            self.input.push_diagnostic(
                                ErrorSeverity::Error,
                                "the xmlns namespace must not be declared as the default namespace"
                                    .to_string(),
                            );
                        } else {
                            return Err(self.input.fatal(
                                "the xmlns namespace must not be declared as the default namespace",
                            ));
                        }
                    }
                    self.ns.bind(None, ns_value);
                } else if attr.prefix.is_some() && attr.name.contains(':') {
                    // Validate QName syntax for prefixed non-namespace attributes.
                    let msg = "QName contains multiple colons";
                    if self.options.recover {
                        self.input
                            .push_diagnostic(ErrorSeverity::Error, msg.to_string());
                    } else {
                        return Err(self.input.fatal(msg));
                    }
                }
            }
        } else {
            // No namespace declarations — only validate QName syntax for
            // prefixed attributes (checking for multiple colons).
            for attr in &attributes {
                if attr.prefix.is_some() && attr.name.contains(':') {
                    let msg = "QName contains multiple colons";
                    if self.options.recover {
                        self.input
                            .push_diagnostic(ErrorSeverity::Error, msg.to_string());
                    } else {
                        return Err(self.input.fatal(msg));
                    }
                }
            }
        }

        // Resolve the element's namespace URI from its prefix.
        let elem_ns = self.ns.resolve(prefix).map(String::from);

        // Check for unbound element prefix.
        if let Some(pfx) = prefix {
            if pfx != "xml" && elem_ns.is_none() {
                if self.options.recover {
                    self.input.push_diagnostic(
                        ErrorSeverity::Error,
                        format!("unbound namespace prefix '{pfx}'"),
                    );
                } else {
                    return Err(self
                        .input
                        .fatal(format!("unbound namespace prefix '{pfx}'")));
                }
            }
        }

        // Resolve namespace URIs for non-xmlns prefixed attributes.
        // Unprefixed attributes do NOT inherit the default namespace (per spec).
        // Skip entirely when there are no prefixed non-xmlns attributes.
        let has_prefixed_attrs = !attributes.is_empty()
            && attributes
                .iter()
                .any(|a| a.prefix.is_some() && a.prefix.as_deref() != Some("xmlns"));
        if has_prefixed_attrs {
            for attr in &mut attributes {
                if let Some(pfx) = &attr.prefix {
                    if pfx == "xmlns" {
                        continue; // namespace declaration, not a real attribute prefix
                    }
                    let resolved = self.ns.resolve(Some(pfx.as_str())).map(String::from);
                    if pfx != "xml" && resolved.is_none() {
                        if self.options.recover {
                            self.input.push_diagnostic(
                                ErrorSeverity::Error,
                                format!("unbound namespace prefix '{pfx}' on attribute"),
                            );
                        } else {
                            return Err(self
                                .input
                                .fatal(format!("unbound namespace prefix '{pfx}' on attribute")));
                        }
                    }
                    attr.namespace = resolved;
                }
            }
        }

        // Namespace-aware attribute uniqueness: two attributes with the same
        // namespace URI and local name are duplicates, even if they use different
        // prefixes (Namespaces in XML 1.0 §6.3).
        // Only meaningful when there are 2+ namespaced (non-xmlns) attributes.
        // Skip entirely when no prefixed attributes exist (no namespaces were
        // resolved, so ns_attr_count is guaranteed to be 0).
        if has_prefixed_attrs {
            let ns_attr_count = attributes.iter().filter(|a| a.namespace.is_some()).count();
            if ns_attr_count >= 2 {
                // O(n²) comparison avoids HashSet allocation
                'ns_outer: for i in 1..attributes.len() {
                    if attributes[i].namespace.is_none() {
                        continue;
                    }
                    for j in 0..i {
                        if attributes[j].namespace.is_none() {
                            continue;
                        }
                        if attributes[i].namespace == attributes[j].namespace
                            && attributes[i].name == attributes[j].name
                        {
                            let display = if let Some(ns) = &attributes[i].namespace {
                                format!("{{{}}}:{}", ns, attributes[i].name)
                            } else {
                                attributes[i].name.clone()
                            };
                            if self.options.recover {
                                self.input.push_diagnostic(
                                    ErrorSeverity::Error,
                                    format!("namespace-aware duplicate attribute: '{display}'"),
                                );
                            } else {
                                return Err(self.input.fatal(format!(
                                    "namespace-aware duplicate attribute: '{display}'"
                                )));
                            }
                            break 'ns_outer;
                        }
                    }
                }
            }
        }

        // Remove stripped attributes (e.g., invalid xmlns:xml rebindings).
        if !strip_attr_indices.is_empty() {
            // Remove in reverse order to preserve indices.
            for &idx in strip_attr_indices.iter().rev() {
                attributes.remove(idx);
            }
        }

        // Consume the original name String via split_owned_name, avoiding a
        // re-allocation for unprefixed names (the common case). For unprefixed
        // names, split_owned_name returns (None, name) — just a move, zero copy.
        let (elem_prefix_owned, elem_local_owned) = split_owned_name(name);
        // Auto-populate id_map for "id" attributes (enables element_by_id
        // and fast CSS #id selectors without requiring DTD validation).
        let id_value = attributes.iter().find_map(|a| {
            if a.prefix.is_none() && a.name == "id" {
                Some(a.value.clone())
            } else {
                None
            }
        });

        let elem_id = self.doc.create_node(NodeKind::Element {
            name: elem_local_owned,
            prefix: elem_prefix_owned,
            namespace: elem_ns,
            attributes,
        });
        self.doc.append_child(parent, elem_id);

        if let Some(id_val) = id_value {
            self.doc.set_id(&id_val, elem_id);
        }

        // Empty element tag <foo/>
        if self.input.looking_at(b"/>") {
            self.input.advance(2);
            if has_ns_decls {
                self.ns.pop_scope();
            }
            self.input.decrement_depth();
            return Ok(elem_id);
        }

        // Start tag close >
        self.input.expect_byte(b'>')?;

        // Parse element content
        self.parse_content(elem_id)?;

        // Parse end tag — read back the stored name from the tree node
        // for matching, since the original name was consumed by split_owned_name.
        self.input.expect_str(b"</")?;
        let (match_prefix, match_local) = {
            let node = self.doc.node(elem_id);
            match &node.kind {
                NodeKind::Element { name, prefix, .. } => (prefix.as_deref(), name.as_str()),
                _ => unreachable!(),
            }
        };
        if let Some(end_name) = self.input.parse_name_eq_parts(match_prefix, match_local)? {
            let expected = match match_prefix {
                Some(pfx) => format!("{pfx}:{match_local}"),
                None => match_local.to_string(),
            };
            if self.options.recover {
                self.input.push_diagnostic(
                    ErrorSeverity::Error,
                    format!("mismatched end tag: expected </{expected}>, found </{end_name}>"),
                );
            } else {
                return Err(self.input.fatal(format!(
                    "mismatched end tag: expected </{expected}>, found </{end_name}>"
                )));
            }
        }
        self.input.skip_whitespace();
        self.input.expect_byte(b'>')?;

        // Pop the namespace scope when leaving this element (only if we pushed).
        if has_ns_decls {
            self.ns.pop_scope();
        }
        self.input.decrement_depth();

        Ok(elem_id)
    }

    /// Normalizes an attribute value based on its DTD-declared type.
    ///
    /// For non-CDATA types (e.g., NMTOKEN, ID, IDREF), collapses whitespace:
    /// trim leading/trailing whitespace, reduce internal whitespace sequences
    /// to single spaces (XML 1.0 §3.3.3).
    fn normalize_attr_value_by_type(
        &self,
        element_name: &str,
        attr_name: &str,
        value: &str,
    ) -> String {
        // Fast path: skip lookup when no DTD attribute types are declared
        // (the common case for documents without a DTD).
        if !self.attr_types.is_empty() {
            let key = (element_name.to_string(), attr_name.to_string());
            if let Some(attr_type) = self.attr_types.get(&key) {
                if !matches!(attr_type, AttributeType::CData) {
                    return value.split_whitespace().collect::<Vec<_>>().join(" ");
                }
            }
        }
        value.to_string()
    }

    // --- Content ---
    // See XML 1.0 §3.1: [43] content

    fn parse_content(&mut self, parent: NodeId) -> Result<(), ParseError> {
        loop {
            if self.input.at_end() {
                if self.options.recover {
                    break;
                }
                return Err(self
                    .input
                    .fatal("unexpected end of input in element content"));
            }

            // End tag starts
            if self.input.looking_at(b"</") {
                break;
            }

            if self.input.looking_at(b"<![CDATA[") {
                self.parse_cdata(parent)?;
            } else if self.input.looking_at(b"<!--") {
                self.parse_comment(parent)?;
            } else if self.input.looking_at(b"<?") {
                self.parse_processing_instruction(parent)?;
            } else if self.input.peek() == Some(b'<') {
                self.parse_element(parent)?;
            } else {
                self.parse_char_data(parent)?;
            }
        }
        Ok(())
    }

    // --- Character Data ---
    // See XML 1.0 §2.4: [14] CharData

    #[allow(clippy::too_many_lines)]
    fn parse_char_data(&mut self, parent: NodeId) -> Result<(), ParseError> {
        let mut text = String::new();

        while !self.input.at_end() {
            // Bulk scan: find the next `<`, `&`, or `]]>` boundary and
            // consume all safe bytes in one go.
            let safe_len = self.input.scan_char_data();
            if safe_len > 0 {
                let start = self.input.pos();
                let chunk = std::str::from_utf8(self.input.slice(start, start + safe_len))
                    .map_err(|_| self.input.fatal("invalid UTF-8 in character data"))?;
                // Fast byte-level pre-check for invalid XML chars (0x7F,
                // U+FFFE, U+FFFF). Skips the expensive char-by-char
                // validation for the 99.9% of chunks that are clean.
                let bad_char = if may_contain_invalid_xml_chars(chunk.as_bytes()) {
                    find_invalid_xml_char(chunk)
                } else {
                    None
                };
                // Append text with CR normalization if needed (XML 1.0 §2.11)
                if chunk.as_bytes().contains(&b'\r') {
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
                    text.push_str(chunk);
                }
                // chunk borrow released — safe to mutably borrow self.input
                self.input.advance_counting_lines(safe_len);
                if let Some(bad) = bad_char {
                    if self.options.recover {
                        self.input.push_diagnostic(
                            ErrorSeverity::Error,
                            format!("invalid XML character: U+{:04X}", bad as u32),
                        );
                    } else {
                        return Err(self
                            .input
                            .fatal(format!("invalid XML character: U+{:04X}", bad as u32)));
                    }
                }
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
                // A named general entity reference in content (not a char ref,
                // not a builtin) is included per XML 1.0 §4.4: an EntityRef
                // node is created and the entity's replacement text is parsed
                // as content, attached as children of the EntityRef node.
                // Skip the peek for builtins at byte level to avoid String allocation.
                if self.input.peek_at(1) != Some(b'#')
                    && !self.is_looking_at_builtin_entity_ref()
                    && self.peek_entity_ref_name().is_some()
                {
                    // Flush accumulated text before the entity ref
                    if !text.is_empty() {
                        let text_id = self.doc.create_node(NodeKind::Text {
                            content: std::mem::take(&mut text),
                        });
                        self.doc.append_child(parent, text_id);
                    }
                    self.parse_entity_ref_in_content(parent)?;
                    continue;
                }
                self.input.parse_reference_into(&mut text)?;
            } else {
                let ch = self.input.next_char()?;
                text.push(ch);
            }
        }

        if !text.is_empty() {
            // Strip blank text nodes if configured
            if self.options.no_blanks && text.chars().all(char::is_whitespace) {
                return Ok(());
            }
            let text_id = self.doc.create_node(NodeKind::Text { content: text });
            self.doc.append_child(parent, text_id);
        }

        Ok(())
    }

    /// Parses a general entity reference (`&name;`) appearing in element
    /// content per XML 1.0 §4.4 ("Included").
    ///
    /// Creates an `EntityRef` node under `parent`, then parses the entity's
    /// replacement text (XML 1.0 §4.5: character references expanded at
    /// declaration time) as content, attaching the resulting nodes as
    /// children of the `EntityRef` node.
    fn parse_entity_ref_in_content(&mut self, parent: NodeId) -> Result<(), ParseError> {
        // Consume `&name;`
        self.input.advance(1); // '&'
        let name = self.input.parse_name()?;
        self.input.expect_byte(b';')?;

        // Count this reference against the expansion limit.
        self.input.count_entity_expansion()?;

        // Resolve the replacement text (XML 1.0 §4.5).
        let replacement: Option<String> = if let Some(raw) = self.input.entity_map.get(&name) {
            Some(crate::validation::dtd::expand_char_refs_only(raw))
        } else if let Some(info) = self.input.entity_external.get(&name).cloned() {
            if let Some(resolver) = self.options.entity_resolver.clone() {
                let request = crate::parser::ExternalEntityRequest {
                    name: &name,
                    system_id: &info.system_id,
                    public_id: info.public_id.as_deref(),
                };
                if let Some(resolved) = resolver(request) {
                    // External parsed entities may begin with a text
                    // declaration (XML 1.0 §4.3.1) which is not content.
                    Some(strip_text_declaration(&resolved).to_string())
                } else {
                    return Err(self.input.fatal(format!(
                        "reference to external entity '{name}' is not supported"
                    )));
                }
            } else {
                return Err(self.input.fatal(format!(
                    "reference to external entity '{name}' is not supported"
                )));
            }
        } else if self.input.has_pe_references || self.input.has_external_dtd {
            // Undeclared entity in tolerant mode (external DTD or PE refs
            // present, XML 1.0 §4.1 WFC: Entity Declared): preserve the
            // reference without a value.
            None
        } else if self.options.recover {
            self.input.push_diagnostic(
                ErrorSeverity::Warning,
                format!("unknown entity reference: &{name};"),
            );
            None
        } else {
            return Err(self
                .input
                .fatal(format!("unknown entity reference: &{name};")));
        };

        let ref_id = self.doc.create_node(NodeKind::EntityRef {
            name: name.clone(),
            value: replacement.clone(),
        });
        self.doc.append_child(parent, ref_id);

        let Some(replacement) = replacement else {
            return Ok(());
        };
        if replacement.is_empty() {
            return Ok(());
        }

        // Amplification guard (matches libxml2's max amplification factor).
        self.expansion_size += replacement.len();
        if self.expansion_size > self.input_size.saturating_mul(DEFAULT_MAX_AMPLIFICATION) {
            return Err(self
                .input
                .fatal("maximum entity amplification factor exceeded"));
        }

        if !replacement.contains('<') && !replacement.contains('&') {
            // Fast path: plain character data — no markup, no references.
            let content = if replacement.contains('\r') {
                normalize_line_ends(&replacement)
            } else {
                replacement
            };
            let text_id = self.doc.create_node(NodeKind::Text { content });
            self.doc.append_child(ref_id, text_id);
        } else {
            self.parse_replacement_as_content(&name, &replacement, ref_id)?;
        }
        Ok(())
    }

    /// Parses an entity's replacement text as XML content (XML 1.0 §4.3.2:
    /// the replacement text must match the `content` production), attaching
    /// the parsed nodes as children of `ref_id`.
    ///
    /// A nested sub-parser is used so the replacement text is processed with
    /// the same entity declarations, namespace scope, options, and security
    /// counters as the outer document.
    fn parse_replacement_as_content(
        &mut self,
        entity_name: &str,
        replacement: &str,
        ref_id: NodeId,
    ) -> Result<(), ParseError> {
        if self.entity_depth >= MAX_ENTITY_DEPTH {
            return Err(self.input.fatal(format!(
                "entity '{entity_name}' exceeds maximum entity nesting depth"
            )));
        }

        let mut sub = XmlParser::new(replacement, &self.options);
        sub.entity_depth = self.entity_depth + 1;
        sub.input_size = self.input_size;
        sub.expansion_size = self.expansion_size;
        // Inherit the element nesting depth so total depth across entity
        // expansions stays bounded by max_depth (a fresh counter per
        // sub-parser would allow MAX_ENTITY_DEPTH * max_depth stack frames —
        // enough to overflow the stack).
        sub.input.set_depth(self.input.depth());
        // Move shared state into the sub-parser (returned below).
        sub.input.entity_map = std::mem::take(&mut self.input.entity_map);
        sub.input.entity_external = std::mem::take(&mut self.input.entity_external);
        sub.input.has_pe_references = self.input.has_pe_references;
        sub.input.has_external_dtd = self.input.has_external_dtd;
        sub.input.entity_expansions = self.input.entity_expansions;
        sub.attr_types = std::mem::take(&mut self.attr_types);
        sub.attr_defaults = std::mem::take(&mut self.attr_defaults);
        std::mem::swap(&mut sub.ns, &mut self.ns);

        let sub_root = sub.doc.root();
        let result = sub.parse_content_fragment(sub_root);

        // Restore shared state regardless of outcome.
        self.input.entity_map = std::mem::take(&mut sub.input.entity_map);
        self.input.entity_external = std::mem::take(&mut sub.input.entity_external);
        self.input.entity_expansions = sub.input.entity_expansions;
        self.attr_types = std::mem::take(&mut sub.attr_types);
        self.attr_defaults = std::mem::take(&mut sub.attr_defaults);
        std::mem::swap(&mut self.ns, &mut sub.ns);
        self.expansion_size = sub.expansion_size;
        if !sub.input.diagnostics.is_empty() {
            self.input.diagnostics.append(&mut sub.input.diagnostics);
        }

        if let Err(e) = result {
            // Propagate security-limit errors and already-wrapped entity
            // errors unchanged to avoid nested message wrapping.
            if e.message.contains("exceeded")
                || e.message.contains("replacement text is not well-formed")
            {
                return Err(e);
            }
            return Err(self.input.fatal(format!(
                "entity '{entity_name}' replacement text is not well-formed \
                 XML content: {}",
                e.message
            )));
        }

        // Graft the parsed nodes into our arena under the EntityRef node.
        let sub_doc = sub.doc;
        let top_level: Vec<NodeId> = sub_doc.children(sub_doc.root()).collect();
        for child in top_level {
            Self::import_subtree(&mut self.doc, &sub_doc, child, ref_id);
        }
        Ok(())
    }

    /// Parses content until end of input (used for entity replacement text,
    /// which must match the `content` production — XML 1.0 §4.3.2).
    ///
    /// Unlike [`parse_content`](Self::parse_content), end of input is the
    /// normal termination and an end tag (`</`) is an error (it would close
    /// an element opened outside the entity — WFC violation).
    fn parse_content_fragment(&mut self, parent: NodeId) -> Result<(), ParseError> {
        while !self.input.at_end() {
            if self.input.looking_at(b"</") {
                return Err(self.input.fatal("unbalanced end tag"));
            }
            if self.input.looking_at(b"<![CDATA[") {
                self.parse_cdata(parent)?;
            } else if self.input.looking_at(b"<!--") {
                self.parse_comment(parent)?;
            } else if self.input.looking_at(b"<?") {
                self.parse_processing_instruction(parent)?;
            } else if self.input.peek() == Some(b'<') {
                self.parse_element(parent)?;
            } else {
                self.parse_char_data(parent)?;
            }
        }
        Ok(())
    }

    /// Recursively deep-copies a subtree from `src` into `dst`, appending
    /// the copy under `dst_parent`.
    fn import_subtree(dst: &mut Document, src: &Document, src_id: NodeId, dst_parent: NodeId) {
        let kind = src.node(src_id).kind.clone();
        let new_id = dst.create_node(kind);
        dst.append_child(dst_parent, new_id);
        for child in src.children(src_id) {
            Self::import_subtree(dst, src, child, new_id);
        }
    }

    /// Checks if the input is positioned at a builtin entity reference
    /// (`&amp;`, `&lt;`, `&gt;`, `&apos;`, `&quot;`) using byte-level
    /// checks. This avoids the `String` allocation of `peek_entity_ref_name`.
    #[inline]
    fn is_looking_at_builtin_entity_ref(&self) -> bool {
        let remaining = self.input.remaining();
        if remaining.len() < 4 || remaining[0] != b'&' {
            return false;
        }
        let after_amp = &remaining[1..];
        after_amp.starts_with(b"lt;")
            || after_amp.starts_with(b"gt;")
            || after_amp.starts_with(b"amp;")
            || after_amp.starts_with(b"apos;")
            || after_amp.starts_with(b"quot;")
    }

    /// Peeks ahead to extract the entity name from `&name;` without consuming
    /// any input. Returns `None` if the next bytes don't form a valid entity
    /// reference pattern.
    fn peek_entity_ref_name(&self) -> Option<String> {
        // We're at `&` — look ahead past it to find the name and `;`
        let remaining = self.input.remaining();
        if remaining.len() < 2 || remaining[0] != b'&' {
            return None;
        }
        let mut i = 1;
        // Collect name bytes
        let name_start = i;
        while i < remaining.len()
            && (remaining[i].is_ascii_alphanumeric()
                || remaining[i] == b'_'
                || remaining[i] == b':'
                || remaining[i] == b'-'
                || remaining[i] == b'.')
        {
            i += 1;
        }
        if i == name_start || i >= remaining.len() || remaining[i] != b';' {
            return None;
        }
        std::str::from_utf8(&remaining[name_start..i])
            .ok()
            .map(String::from)
    }

    /// Checks if all entity references (`&name;`) in a raw attribute value
    /// text are declared in the entity map. Returns false if any undeclared
    /// entity ref is found (these should not be preserved via `raw_value`).
    fn all_entity_refs_declared(&self, raw: &str) -> bool {
        let bytes = raw.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'&' && i + 1 < bytes.len() && bytes[i + 1] != b'#' {
                // Named entity reference — extract the name
                let mut j = i + 1;
                while j < bytes.len()
                    && (bytes[j].is_ascii_alphanumeric()
                        || bytes[j] == b'_'
                        || bytes[j] == b':'
                        || bytes[j] == b'-'
                        || bytes[j] == b'.')
                {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b';' {
                    let name = std::str::from_utf8(&bytes[i + 1..j]).unwrap_or("");
                    if !is_builtin_entity(name) && !self.input.entity_map.contains_key(name) {
                        return false;
                    }
                    i = j + 1;
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        true
    }

    // --- Attributes ---
    // See XML 1.0 §3.1: [41] Attribute

    fn parse_attribute(&mut self) -> Result<Attribute, ParseError> {
        let name = self.input.parse_name()?;
        self.input.skip_whitespace();
        self.input.expect_byte(b'=')?;
        self.input.skip_whitespace();

        // Capture raw attribute value text (before entity expansion) so we
        // can preserve entity references during serialization.
        let raw_start = self.input.pos();
        let value = self.input.parse_attribute_value()?;
        let raw_end = self.input.pos();

        // Extract raw value (between quotes) — the raw slice includes the
        // outer quote chars, so trim them. Skip entirely when the raw bytes
        // contain no '&' (most attributes have no entity references).
        let raw_value = if raw_end > raw_start + 2 {
            let raw_bytes = self.input.slice(raw_start + 1, raw_end - 1);
            if raw_bytes.contains(&b'&') {
                let raw_str = std::str::from_utf8(raw_bytes).ok().map(str::to_string);
                // Only store raw_value if it differs from the expanded value
                // (i.e., it contained entity references that got expanded) AND
                // all entity references in the raw value are declared (not
                // undeclared entities that expanded to empty string).
                raw_str.filter(|raw| *raw != value && self.all_entity_refs_declared(raw))
            } else {
                None
            }
        } else {
            None
        };

        let (prefix, local_name) = split_owned_name(name);

        Ok(Attribute {
            name: local_name,
            value,
            prefix,
            namespace: None,
            raw_value,
        })
    }

    // --- Comments ---
    // See XML 1.0 §2.5: [15] Comment

    fn parse_comment(&mut self, parent: NodeId) -> Result<(), ParseError> {
        let content = parse_comment_content(&mut self.input)?;
        let comment_id = self.doc.create_node(NodeKind::Comment { content });
        self.doc.append_child(parent, comment_id);
        Ok(())
    }

    // --- CDATA Sections ---
    // See XML 1.0 §2.7: [18] CDSect

    fn parse_cdata(&mut self, parent: NodeId) -> Result<(), ParseError> {
        let content = parse_cdata_content(&mut self.input)?;
        let cdata_id = self.doc.create_node(NodeKind::CData { content });
        self.doc.append_child(parent, cdata_id);
        Ok(())
    }

    // --- Processing Instructions ---
    // See XML 1.0 §2.6: [16] PI

    fn parse_processing_instruction(&mut self, parent: NodeId) -> Result<(), ParseError> {
        let (target, data) = parse_pi_content(&mut self.input)?;
        let pi_id = self
            .doc
            .create_node(NodeKind::ProcessingInstruction { target, data });
        self.doc.append_child(parent, pi_id);
        Ok(())
    }
}

/// Normalizes line ends per XML 1.0 §2.11: `\r\n` and lone `\r` become `\n`.
fn normalize_line_ends(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Strips an optional leading text declaration (`<?xml ...?>`) from external
/// parsed entity text (XML 1.0 §4.3.1: `TextDecl`).
fn strip_text_declaration(s: &str) -> &str {
    let rest = s.strip_prefix("<?xml").filter(|r| {
        r.starts_with(' ') || r.starts_with('\t') || r.starts_with('\r') || r.starts_with('\n')
    });
    if let Some(rest) = rest {
        if let Some(end) = rest.find("?>") {
            return &rest[end + 2..];
        }
    }
    s
}

/// Returns true if the entity name is one of the five XML builtin entities.
fn is_builtin_entity(name: &str) -> bool {
    matches!(name, "amp" | "lt" | "gt" | "apos" | "quot")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::tree::Document;

    fn parse(input: &str) -> Document {
        Document::parse_str(input).unwrap_or_else(|e| panic!("parse failed: {e}"))
    }

    #[test]
    fn test_parse_empty_element() {
        let doc = parse("<root/>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
        assert_eq!(doc.first_child(root), None);
    }

    #[test]
    fn test_parse_element_with_text() {
        let doc = parse("<greeting>Hello, world!</greeting>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("greeting"));
        assert_eq!(doc.text_content(root), "Hello, world!");
    }

    #[test]
    fn test_parse_nested_elements() {
        let doc = parse("<a><b><c/></b></a>");
        let a = doc.root_element().unwrap();
        assert_eq!(doc.node_name(a), Some("a"));

        let b = doc.first_child(a).unwrap();
        assert_eq!(doc.node_name(b), Some("b"));

        let c = doc.first_child(b).unwrap();
        assert_eq!(doc.node_name(c), Some("c"));
    }

    #[test]
    fn test_parse_attributes() {
        let doc = parse("<div id=\"main\" class=\"big\"/>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.attribute(root, "id"), Some("main"));
        assert_eq!(doc.attribute(root, "class"), Some("big"));
    }

    #[test]
    fn test_parse_single_quoted_attributes() {
        let doc = parse("<div id='main'/>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.attribute(root, "id"), Some("main"));
    }

    #[test]
    fn test_parse_xml_declaration() {
        let doc = parse("<?xml version=\"1.0\" encoding=\"UTF-8\"?><root/>");
        assert_eq!(doc.version.as_deref(), Some("1.0"));
        assert_eq!(doc.encoding.as_deref(), Some("UTF-8"));
    }

    #[test]
    fn test_parse_xml_declaration_standalone() {
        let doc = parse("<?xml version=\"1.0\" standalone=\"yes\"?><root/>");
        assert_eq!(doc.standalone, Some(true));
    }

    #[test]
    fn test_parse_comment() {
        let doc = parse("<root><!-- hello --></root>");
        let root = doc.root_element().unwrap();
        let child = doc.first_child(root).unwrap();
        assert_eq!(doc.node_text(child), Some(" hello "));
    }

    #[test]
    fn test_parse_cdata() {
        let doc = parse("<root><![CDATA[x < 1 && y > 2]]></root>");
        let root = doc.root_element().unwrap();
        let child = doc.first_child(root).unwrap();
        assert_eq!(doc.node_text(child), Some("x < 1 && y > 2"));
    }

    #[test]
    fn test_parse_processing_instruction() {
        let doc = parse("<?my-pi some data?><root/>");
        let pi = doc.first_child(doc.root()).unwrap();
        assert_eq!(doc.node_name(pi), Some("my-pi"));
        assert_eq!(doc.node_text(pi), Some("some data"));
    }

    #[test]
    fn test_parse_entity_references() {
        let doc = parse("<root>&amp; &lt; &gt; &apos; &quot;</root>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.text_content(root), "& < > ' \"");
    }

    #[test]
    fn test_parse_char_reference_decimal() {
        let doc = parse("<root>&#65;</root>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.text_content(root), "A");
    }

    #[test]
    fn test_parse_char_reference_hex() {
        let doc = parse("<root>&#x41;</root>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.text_content(root), "A");
    }

    #[test]
    fn test_parse_mixed_content() {
        let doc = parse("<p>Hello <b>world</b>!</p>");
        let p = doc.root_element().unwrap();
        let children: Vec<_> = doc.children(p).collect();
        assert_eq!(children.len(), 3); // "Hello ", <b>, "!"

        assert_eq!(doc.node_text(children[0]), Some("Hello "));
        assert_eq!(doc.node_name(children[1]), Some("b"));
        assert_eq!(doc.text_content(children[1]), "world");
        assert_eq!(doc.node_text(children[2]), Some("!"));
    }

    #[test]
    fn test_parse_prefixed_element() {
        let doc = parse("<svg:rect xmlns:svg=\"http://www.w3.org/2000/svg\"/>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("rect"));
        match &doc.node(root).kind {
            NodeKind::Element { prefix, .. } => {
                assert_eq!(prefix.as_deref(), Some("svg"));
            }
            _ => panic!("expected element"),
        }
    }

    #[test]
    fn test_parse_prefixed_attribute() {
        let doc = parse("<root xml:lang=\"en\"/>");
        let root = doc.root_element().unwrap();
        let attrs = doc.attributes(root);
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].name, "lang");
        assert_eq!(attrs[0].prefix.as_deref(), Some("xml"));
        assert_eq!(attrs[0].value, "en");
    }

    #[test]
    fn test_parse_error_mismatched_tags() {
        let result = Document::parse_str("<a></b>");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_error_unexpected_eof() {
        let result = Document::parse_str("<a>");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_error_no_root() {
        let result = Document::parse_str("");
        // XML 1.0 §2.1 requires a root element
        assert!(result.is_err());
    }

    /// The XML declaration prefix that `serialize()` always emits.
    const DECL: &str = "<?xml version=\"1.0\"?>\n";

    #[test]
    fn test_roundtrip_simple() {
        let input = "<root><child>text</child></root>";
        let doc = parse(input);
        let output = crate::serial::serialize(&doc);
        assert_eq!(output, format!("{DECL}{input}\n"));
    }

    #[test]
    fn test_roundtrip_attributes() {
        let input = "<root attr=\"value\"><child id=\"1\"/></root>";
        let doc = parse(input);
        let output = crate::serial::serialize(&doc);
        assert_eq!(output, format!("{DECL}{input}\n"));
    }

    #[test]
    fn test_roundtrip_entities() {
        let input = "<root>&amp; &lt; &gt;</root>";
        let doc = parse(input);
        let output = crate::serial::serialize(&doc);
        // After parsing, entities are resolved to characters.
        // Serialization re-escapes them.
        assert_eq!(output, format!("{DECL}<root>&amp; &lt; &gt;</root>\n"));
    }

    #[test]
    fn test_roundtrip_comment() {
        let input = "<root><!-- comment --></root>";
        let doc = parse(input);
        let output = crate::serial::serialize(&doc);
        assert_eq!(output, format!("{DECL}{input}\n"));
    }

    #[test]
    fn test_roundtrip_cdata() {
        let input = "<root><![CDATA[data & stuff]]></root>";
        let doc = parse(input);
        let output = crate::serial::serialize(&doc);
        assert_eq!(output, format!("{DECL}{input}\n"));
    }

    #[test]
    fn test_roundtrip_pi() {
        let input = "<?target data?><root/>";
        let doc = parse(input);
        let output = crate::serial::serialize(&doc);
        assert_eq!(output, format!("{DECL}{input}\n"));
    }

    #[test]
    fn test_roundtrip_xml_declaration() {
        let input = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><root/>";
        let doc = parse(input);
        let output = crate::serial::serialize(&doc);
        assert_eq!(
            output,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<root/>\n"
        );
    }

    #[test]
    fn test_whitespace_in_attribute_value() {
        let doc = parse("<root attr=\"a\tb\nc\"/>");
        let root = doc.root_element().unwrap();
        // Tabs and newlines in attribute values are normalized to spaces
        assert_eq!(doc.attribute(root, "attr"), Some("a b c"));
    }

    #[test]
    fn test_name_chars() {
        use super::super::input::{is_name_char, is_name_start_char};

        assert!(is_name_start_char('A'));
        assert!(is_name_start_char('z'));
        assert!(is_name_start_char('_'));
        assert!(is_name_start_char(':'));
        assert!(!is_name_start_char('0'));
        assert!(!is_name_start_char('-'));

        assert!(is_name_char('A'));
        assert!(is_name_char('0'));
        assert!(is_name_char('-'));
        assert!(is_name_char('.'));
        assert!(!is_name_char(' '));
    }

    #[test]
    fn test_parse_doctype_simple() {
        let doc = parse("<!DOCTYPE html><html/>");
        let root = doc.root();
        let children: Vec<_> = doc.children(root).collect();
        assert_eq!(children.len(), 2);

        match &doc.node(children[0]).kind {
            NodeKind::DocumentType {
                name,
                system_id,
                public_id,
                ..
            } => {
                assert_eq!(name, "html");
                assert_eq!(*system_id, None);
                assert_eq!(*public_id, None);
            }
            other => panic!("expected DocumentType, got {other:?}"),
        }

        assert_eq!(doc.node_name(children[1]), Some("html"));
    }

    #[test]
    fn test_parse_doctype_system() {
        let doc = parse("<!DOCTYPE root SYSTEM \"root.dtd\"><root/>");
        let root = doc.root();
        let children: Vec<_> = doc.children(root).collect();
        assert_eq!(children.len(), 2);

        match &doc.node(children[0]).kind {
            NodeKind::DocumentType {
                name,
                system_id,
                public_id,
                ..
            } => {
                assert_eq!(name, "root");
                assert_eq!(system_id.as_deref(), Some("root.dtd"));
                assert_eq!(*public_id, None);
            }
            other => panic!("expected DocumentType, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_doctype_public() {
        let doc = parse(
            "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0//EN\" \
             \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\"><html/>",
        );
        let root = doc.root();
        let children: Vec<_> = doc.children(root).collect();
        assert_eq!(children.len(), 2);

        match &doc.node(children[0]).kind {
            NodeKind::DocumentType {
                name,
                system_id,
                public_id,
                ..
            } => {
                assert_eq!(name, "html");
                assert_eq!(public_id.as_deref(), Some("-//W3C//DTD XHTML 1.0//EN"));
                assert_eq!(
                    system_id.as_deref(),
                    Some("http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd")
                );
            }
            other => panic!("expected DocumentType, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_doctype_internal_subset() {
        let doc = parse("<!DOCTYPE root [<!ELEMENT root (#PCDATA)>]><root/>");
        let root = doc.root();
        let children: Vec<_> = doc.children(root).collect();
        assert_eq!(children.len(), 2);

        match &doc.node(children[0]).kind {
            NodeKind::DocumentType {
                name,
                system_id,
                public_id,
                ..
            } => {
                assert_eq!(name, "root");
                assert_eq!(*system_id, None);
                assert_eq!(*public_id, None);
            }
            other => panic!("expected DocumentType, got {other:?}"),
        }

        assert_eq!(doc.node_name(children[1]), Some("root"));
    }

    #[test]
    fn test_parse_doctype_multiline_internal_subset() {
        let input =
            "<!DOCTYPE root [\n<!ELEMENT y (#PCDATA|x|x)*>\n<!ELEMENT root ANY>\n]>\n\n<root/>";
        let doc = parse(input);
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
    }

    #[test]
    fn test_parse_doctype_with_entity() {
        let input = "<!DOCTYPE doc [\n<!ELEMENT doc (#PCDATA)>\n<!ENTITY rsqb \"]\">\n]>\n<doc>&rsqb;</doc>";
        let doc = parse(input);
        let root = doc.root_element().unwrap();
        assert_eq!(doc.text_content(root), "]");
    }

    #[test]
    fn test_parse_doctype_content_model() {
        let input = "<!DOCTYPE violation [\n<!ELEMENT violation (a,a,a,b)>\n<!ELEMENT a EMPTY>\n<!ELEMENT b EMPTY>\n]>\n<violation>\n    <a/>\n    <a/>\n    <b/>\n</violation>";
        let doc = parse(input);
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("violation"));
    }

    #[test]
    fn test_parse_doctype_with_crlf() {
        // Test with CRLF line endings (like the OASIS conformance tests)
        let input =
            "<!DOCTYPE doc\r\n[\r\n<!ELEMENT doc ANY>\r\n<!ELEMENT a (doc?)>\r\n]>\r\n<doc/>";
        let doc = parse(input);
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("doc"));
    }

    #[test]
    fn test_parse_doctype_attlist() {
        let input = "<!DOCTYPE root [\n<!ELEMENT root EMPTY>\n<!ATTLIST root\n    token\tNMTOKEN\t\t#REQUIRED\n    >\n\n    <!-- comment -->\n]>\n<root token=\"dev@null\"/>";
        let doc = parse(input);
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
    }

    #[test]
    fn test_parse_doctype_comment_with_apostrophe() {
        // Apostrophes in DTD comments must not confuse the bracket scanner
        let input = "<!DOCTYPE root [\n<!ELEMENT root ANY>\n<!-- can't break -->\n]>\n<root/>";
        let doc = parse(input);
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
    }

    #[test]
    fn test_roundtrip_doctype() {
        // Simple DOCTYPE (no whitespace between DOCTYPE and root in input)
        let input = "<!DOCTYPE html><html/>";
        let doc = parse(input);
        let output = crate::serial::serialize(&doc);
        assert_eq!(output, format!("{DECL}<!DOCTYPE html><html/>\n"));

        // DOCTYPE with SYSTEM
        let input = "<!DOCTYPE root SYSTEM \"root.dtd\"><root/>";
        let doc = parse(input);
        let output = crate::serial::serialize(&doc);
        assert_eq!(
            output,
            format!("{DECL}<!DOCTYPE root SYSTEM \"root.dtd\"><root/>\n")
        );

        // DOCTYPE with PUBLIC
        let input = "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0//EN\" \
                      \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\"><html/>";
        let doc = parse(input);
        let output = crate::serial::serialize(&doc);
        assert_eq!(
            output,
            format!(
                "{DECL}<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0//EN\" \
                 \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd\"><html/>\n"
            )
        );
    }

    // --- Namespace resolution tests ---

    #[test]
    fn test_parse_default_namespace() {
        let doc = parse("<root xmlns=\"http://example.com\"/>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_namespace(root), Some("http://example.com"));
    }

    #[test]
    fn test_parse_prefixed_namespace() {
        let doc = parse("<ns:root xmlns:ns=\"http://example.com\"/>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_name(root), Some("root"));
        assert_eq!(doc.node_namespace(root), Some("http://example.com"));
        match &doc.node(root).kind {
            NodeKind::Element { prefix, .. } => {
                assert_eq!(prefix.as_deref(), Some("ns"));
            }
            _ => panic!("expected element"),
        }
    }

    #[test]
    fn test_parse_nested_namespace() {
        // Child elements inherit the default namespace from the parent.
        let doc = parse("<root xmlns=\"http://example.com\"><child/></root>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_namespace(root), Some("http://example.com"));

        let child = doc.first_child(root).unwrap();
        assert_eq!(doc.node_name(child), Some("child"));
        assert_eq!(doc.node_namespace(child), Some("http://example.com"));
    }

    #[test]
    fn test_parse_namespace_override() {
        // A child element can override the parent's default namespace.
        let doc = parse(
            "<root xmlns=\"http://example.com\">\
             <child xmlns=\"http://other.com\"/>\
             </root>",
        );
        let root = doc.root_element().unwrap();
        assert_eq!(doc.node_namespace(root), Some("http://example.com"));

        let child = doc.first_child(root).unwrap();
        assert_eq!(doc.node_namespace(child), Some("http://other.com"));
    }

    #[test]
    fn test_parse_xml_namespace() {
        // The xml: prefix is always bound to the XML namespace URI.
        let doc = parse("<root xml:lang=\"en\"/>");
        let root = doc.root_element().unwrap();
        let attrs = doc.attributes(root);
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].name, "lang");
        assert_eq!(attrs[0].prefix.as_deref(), Some("xml"));
        assert_eq!(
            attrs[0].namespace.as_deref(),
            Some("http://www.w3.org/XML/1998/namespace")
        );
    }

    #[test]
    fn test_parse_attribute_namespace() {
        // Prefixed attributes get their namespace resolved.
        let doc = parse("<root xmlns:app=\"http://example.com/app\" app:version=\"2.0\"/>");
        let root = doc.root_element().unwrap();
        let attrs = doc.attributes(root);

        // Find the app:version attribute
        let version_attr = attrs.iter().find(|a| a.name == "version").unwrap();
        assert_eq!(version_attr.prefix.as_deref(), Some("app"));
        assert_eq!(
            version_attr.namespace.as_deref(),
            Some("http://example.com/app")
        );

        // The xmlns:app attribute should not have a resolved namespace itself.
        let xmlns_attr = attrs.iter().find(|a| a.name == "app").unwrap();
        assert_eq!(xmlns_attr.prefix.as_deref(), Some("xmlns"));
        assert_eq!(xmlns_attr.namespace, None);
    }

    // -- General entity inclusion in content (issue #43) --------------------
    //
    // XML 1.0 §4.4 "Included": a general entity referenced in content has
    // its replacement text parsed as content. §4.5: character references in
    // the declaration are expanded when the replacement text is built.

    #[test]
    fn test_parse_entity_charref_replacement() {
        let doc = parse("<!DOCTYPE d [<!ENTITY e \"caf&#233;\">]><d>&e;</d>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.text_content(root), "caf\u{e9}");
    }

    #[test]
    fn test_parse_entity_nested_reference() {
        let doc = parse("<!DOCTYPE d [<!ENTITY a \"XYZ\"><!ENTITY b \"&a;\">]><d>&b;</d>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.text_content(root), "XYZ");
        // Tree shape: EntityRef(b) -> EntityRef(a) -> Text("XYZ").
        let outer = doc.first_child(root).unwrap();
        let NodeKind::EntityRef { ref name, .. } = doc.node(outer).kind else {
            panic!("expected EntityRef, got {:?}", doc.node(outer).kind);
        };
        assert_eq!(name, "b");
        let inner = doc.first_child(outer).unwrap();
        let NodeKind::EntityRef { ref name, .. } = doc.node(inner).kind else {
            panic!("expected EntityRef, got {:?}", doc.node(inner).kind);
        };
        assert_eq!(name, "a");
    }

    #[test]
    fn test_parse_entity_amp_double_reference() {
        // "&#38;amp;" builds replacement text "&amp;", which is then parsed
        // as content, yielding "&".
        let doc = parse("<!DOCTYPE d [<!ENTITY e \"&#38;amp;\">]><d>&e;</d>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.text_content(root), "&");
    }

    #[test]
    fn test_parse_entity_markup_content() {
        // Markup in the replacement text becomes real element children of
        // the EntityRef node — not escaped text.
        let doc = parse("<!DOCTYPE d [<!ENTITY e \"<b>hi</b>\">]><d>&e;</d>");
        let root = doc.root_element().unwrap();
        assert_eq!(doc.text_content(root), "hi");
        let entity_ref = doc.first_child(root).unwrap();
        let element = doc.first_child(entity_ref).unwrap();
        assert_eq!(doc.node_name(element), Some("b"));
        // Serialization re-emits the entity reference, preserving roundtrip.
        let out = crate::serial::serialize(&doc);
        assert!(
            out.contains("<d>&e;</d>"),
            "unexpected serialization: {out}"
        );
    }

    #[test]
    fn test_parse_entity_charref_markup() {
        // "&#60;b>hi&#60;/b>" expands to "<b>hi</b>" at declaration time
        // (§4.5) and must be parsed as markup at reference time.
        let doc = parse("<!DOCTYPE d [<!ENTITY e \"&#60;b>hi&#60;/b>\">]><d>&e;</d>");
        let root = doc.root_element().unwrap();
        let entity_ref = doc.first_child(root).unwrap();
        let element = doc.first_child(entity_ref).unwrap();
        assert_eq!(doc.node_name(element), Some("b"));
        assert_eq!(doc.text_content(root), "hi");
    }

    #[test]
    fn test_parse_entity_markup_roundtrip() {
        // parse -> serialize -> parse must preserve the element structure
        // inside the entity expansion.
        let input = "<!DOCTYPE d [<!ENTITY e \"<b>hi</b>\">]><d>&e;</d>";
        let doc = parse(input);
        let out = crate::serial::serialize(&doc);
        let doc2 = parse(&out);
        let root2 = doc2.root_element().unwrap();
        assert_eq!(doc2.text_content(root2), "hi");
        let entity_ref = doc2.first_child(root2).unwrap();
        let element = doc2.first_child(entity_ref).unwrap();
        assert_eq!(doc2.node_name(element), Some("b"));
    }

    #[test]
    fn test_parse_entity_unbalanced_markup_rejected() {
        // §4.3.2: replacement text must match the content production; an
        // unbalanced start tag is not well-formed.
        let result = Document::parse_str("<!DOCTYPE d [<!ENTITY e \"<b>hi\">]><d>&e;</d>");
        assert!(
            result.is_err(),
            "unbalanced entity content must be rejected"
        );
    }

    #[test]
    fn test_parse_entity_split_tags_rejected() {
        // A start tag in one entity and its end tag in another violates
        // WFC: Element Type Match within the replacement text.
        let result = Document::parse_str(
            "<!DOCTYPE d [<!ENTITY open \"<b>\"><!ENTITY close \"</b>\">]><d>&open;hi&close;</d>",
        );
        assert!(result.is_err(), "split tag pair must be rejected");
    }

    #[test]
    fn test_parse_entity_namespace_inherited() {
        // Elements produced by entity expansion resolve namespaces in scope
        // at the point of reference.
        let doc = parse("<!DOCTYPE d [<!ENTITY e \"<b/>\">]><d xmlns=\"urn:x\">&e;</d>");
        let root = doc.root_element().unwrap();
        let entity_ref = doc.first_child(root).unwrap();
        let element = doc.first_child(entity_ref).unwrap();
        let NodeKind::Element { ref namespace, .. } = doc.node(element).kind else {
            panic!("expected Element, got {:?}", doc.node(element).kind);
        };
        assert_eq!(namespace.as_deref(), Some("urn:x"));
    }

    #[test]
    fn test_parse_entity_nesting_depth_limit() {
        // A 40-deep reference chain exceeds MAX_ENTITY_DEPTH when expanded.
        use std::fmt::Write as _;
        let mut dtd = String::from("<!ENTITY e0 \"<b/>\">");
        for i in 1..40u32 {
            let _ = write!(dtd, "<!ENTITY e{i} \"<b>&e{};</b>\">", i - 1);
        }
        let xml = format!("<!DOCTYPE d [{dtd}]><d>&e39;</d>");
        let result = Document::parse_str(&xml);
        let Err(err) = result else {
            panic!("expected depth-limited parse to fail");
        };
        assert!(
            err.message.contains("depth"),
            "unexpected error message: {}",
            err.message
        );
    }

    #[test]
    fn test_parse_entity_amplification_rejected() {
        // Many references to a large entity trip the amplification guard
        // (or the expansion counter) — billion-laughs protection.
        let mut xml = String::from(
            "<!DOCTYPE d [<!ENTITY a \"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\">\
             <!ENTITY b \"&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;\">\
             <!ENTITY c \"&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;\">\
             <!ENTITY e \"&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;\">]><d>",
        );
        for _ in 0..20 {
            xml.push_str("&e;");
        }
        xml.push_str("</d>");
        let result = Document::parse_str(&xml);
        let Err(err) = result else {
            panic!("expected amplification attack to be rejected");
        };
        assert!(
            err.message.contains("amplification") || err.message.contains("expansion limit"),
            "unexpected error message: {}",
            err.message
        );
    }
}
