//! DTD (Document Type Definition) data model, parser, and validator.
//!
//! This module implements DTD processing as defined in XML 1.0 (Fifth Edition)
//! sections 2.8, 3.2, 3.3, 3.4, and 4.2. It provides:
//!
//! - A data model for DTD declarations (elements, attributes, entities, notations)
//! - A parser that processes DTD internal subset content
//! - A validator that checks document conformance against a parsed DTD
//!
//! # Content Model Matching
//!
//! The validator implements deterministic content model matching for:
//! - `EMPTY`: element must have no element or text children
//! - `ANY`: any content is allowed
//! - Mixed content `(#PCDATA|a|b)*`: text and listed elements in any order
//! - Element content with sequences `(a,b,c)`, choices `(a|b|c)`, and
//!   occurrence indicators `?`, `*`, `+`
//!
//! See XML 1.0 section 3.2 for the full content model specification.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::error::{ParseError, SourceLocation};
use crate::tree::{Document, NodeId, NodeKind};

use super::{ValidationError, ValidationResult};

// ---------------------------------------------------------------------------
// DTD Data Model
// ---------------------------------------------------------------------------

/// A parsed DTD containing all declarations from the internal subset.
///
/// This is the result of [`parse_dtd`] and serves as input to [`validate`].
#[derive(Debug, Clone, Default)]
pub struct Dtd {
    /// Element declarations, keyed by element name.
    pub elements: HashMap<String, ElementDecl>,
    /// Attribute declarations, keyed by `(element_name, attribute_name)`.
    pub attributes: HashMap<String, Vec<AttributeDecl>>,
    /// General entity declarations, keyed by entity name.
    pub entities: HashMap<String, EntityDecl>,
    /// Parameter entity declarations, keyed by entity name.
    pub param_entities: HashMap<String, EntityDecl>,
    /// Notation declarations, keyed by notation name.
    pub notations: HashMap<String, NotationDecl>,
    /// Ordered list of all declarations (preserving source order and comments).
    pub declarations: Vec<DtdDeclaration>,
}

/// A single DTD declaration, preserving source order for re-serialization.
#[derive(Debug, Clone)]
pub enum DtdDeclaration {
    /// An element declaration.
    Element(ElementDecl),
    /// A single attribute declaration (one per attribute, even if the source
    /// used a multi-attribute ATTLIST).
    Attlist(AttributeDecl),
    /// A general entity declaration.
    Entity(EntityDecl),
    /// A notation declaration.
    Notation(NotationDecl),
    /// A comment.
    Comment(String),
    /// A processing instruction.
    Pi(String, Option<String>),
}

/// An element declaration from `<!ELEMENT name content-model>`.
///
/// See XML 1.0 section 3.2.
#[derive(Debug, Clone)]
pub struct ElementDecl {
    /// The element name.
    pub name: String,
    /// The declared content model.
    pub content_model: ContentModel,
}

/// The content model for an element declaration.
///
/// See XML 1.0 section 3.2 for the grammar:
/// - `contentspec ::= 'EMPTY' | 'ANY' | Mixed | children`
#[derive(Debug, Clone, PartialEq)]
pub enum ContentModel {
    /// The element must have no children (no elements, no text).
    /// Declared as `<!ELEMENT name EMPTY>`.
    Empty,
    /// Any content is allowed.
    /// Declared as `<!ELEMENT name ANY>`.
    Any,
    /// Mixed content: text and optionally listed elements in any order.
    /// Declared as `<!ELEMENT name (#PCDATA)>` or `<!ELEMENT name (#PCDATA|a|b)*>`.
    ///
    /// The `Vec<String>` contains the allowed element names (empty for `#PCDATA` only).
    Mixed(Vec<String>),
    /// Element-only content following a content spec pattern.
    /// Declared as `<!ELEMENT name (a,b,c)>` etc.
    Children(ContentSpec),
}

/// A content specification for element-only content models.
///
/// Represents the recursive structure of `(a,b)`, `(a|b)`, etc.
/// with occurrence indicators.
///
/// See XML 1.0 section 3.2.1 and 3.2.2.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentSpec {
    /// The content particle kind.
    pub kind: ContentSpecKind,
    /// How many times this particle may occur.
    pub occurrence: Occurrence,
}

/// The kind of a content specification particle.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentSpecKind {
    /// A single named element, e.g., `a`.
    Name(String),
    /// A sequence of particles, e.g., `(a, b, c)`.
    Seq(Vec<ContentSpec>),
    /// A choice among particles, e.g., `(a | b | c)`.
    Choice(Vec<ContentSpec>),
}

/// Occurrence indicator for a content particle.
///
/// See XML 1.0 section 3.2.1: `'?' | '*' | '+'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occurrence {
    /// Exactly once (no indicator).
    Once,
    /// Zero or one time (`?`).
    Optional,
    /// Zero or more times (`*`).
    ZeroOrMore,
    /// One or more times (`+`).
    OneOrMore,
}

/// An attribute declaration from `<!ATTLIST element-name attr-name type default>`.
///
/// See XML 1.0 section 3.3.
#[derive(Debug, Clone)]
pub struct AttributeDecl {
    /// The element this attribute belongs to.
    pub element_name: String,
    /// The attribute name.
    pub attribute_name: String,
    /// The attribute type.
    pub attribute_type: AttributeType,
    /// The default value specification.
    pub default: AttributeDefault,
}

/// The type of an attribute as declared in `<!ATTLIST>`.
///
/// See XML 1.0 section 3.3.1.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeType {
    /// Character data (`CDATA`).
    CData,
    /// A unique identifier (`ID`).
    Id,
    /// A reference to an ID (`IDREF`).
    IdRef,
    /// Space-separated list of ID references (`IDREFS`).
    IdRefs,
    /// An entity name (`ENTITY`).
    Entity,
    /// Space-separated list of entity names (`ENTITIES`).
    Entities,
    /// A name token (`NMTOKEN`).
    NmToken,
    /// Space-separated list of name tokens (`NMTOKENS`).
    NmTokens,
    /// A notation type with allowed notation names (`NOTATION (a|b|c)`).
    Notation(Vec<String>),
    /// An enumeration of allowed values (`(a|b|c)`).
    Enumeration(Vec<String>),
}

/// The default value specification for an attribute.
///
/// See XML 1.0 section 3.3.2.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeDefault {
    /// The attribute is required (`#REQUIRED`).
    Required,
    /// The attribute is optional with no default (`#IMPLIED`).
    Implied,
    /// The attribute has a fixed value (`#FIXED "value"`).
    Fixed(String),
    /// The attribute has a default value (`"value"`).
    Default(String),
}

/// A general entity declaration.
///
/// See XML 1.0 section 4.2.
#[derive(Debug, Clone)]
pub struct EntityDecl {
    /// The entity name.
    pub name: String,
    /// The entity's value, either internal or external.
    pub kind: EntityKind,
}

/// Whether an entity is internal (has a literal value) or external
/// (references an external resource).
#[derive(Debug, Clone)]
pub enum EntityKind {
    /// Internal entity with a literal replacement text.
    Internal(String),
    /// External entity identified by a system URI and optional public ID.
    External {
        /// The SYSTEM identifier (URI).
        system_id: String,
        /// The PUBLIC identifier, if any.
        public_id: Option<String>,
    },
}

/// A notation declaration from `<!NOTATION name ...>`.
///
/// See XML 1.0 section 4.7.
#[derive(Debug, Clone)]
pub struct NotationDecl {
    /// The notation name.
    pub name: String,
    /// The SYSTEM identifier, if any.
    pub system_id: Option<String>,
    /// The PUBLIC identifier, if any.
    pub public_id: Option<String>,
}

impl fmt::Display for ContentModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "EMPTY"),
            Self::Any => write!(f, "ANY"),
            Self::Mixed(names) => {
                if names.is_empty() {
                    write!(f, "(#PCDATA)")
                } else {
                    write!(f, "(#PCDATA|{})*", names.join("|"))
                }
            }
            Self::Children(spec) => write!(f, "{spec}"),
        }
    }
}

impl fmt::Display for ContentSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ContentSpecKind::Name(name) => write!(f, "{name}")?,
            ContentSpecKind::Seq(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, " , ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")?;
            }
            ContentSpecKind::Choice(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")?;
            }
        }
        match self.occurrence {
            Occurrence::Once => {}
            Occurrence::Optional => write!(f, "?")?,
            Occurrence::ZeroOrMore => write!(f, "*")?,
            Occurrence::OneOrMore => write!(f, "+")?,
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// DTD Serializer
// ---------------------------------------------------------------------------

/// Serializes a parsed DTD's declarations into the internal subset format
/// used by libxml2.
///
/// Each declaration appears on its own line. The output does NOT include
/// the surrounding `[` and `]>` — the caller adds those.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn serialize_dtd(dtd: &Dtd) -> String {
    let mut out = String::new();
    let mut last_was_comment = false;

    for decl in &dtd.declarations {
        // Don't add a newline before declarations that immediately follow
        // a comment — the comment text already contains any needed whitespace.
        // libxml2 concatenates the comment closing `-->` and the next
        // declaration on the same line.
        if !last_was_comment {
            out.push('\n');
        }
        match decl {
            DtdDeclaration::Element(e) => {
                out.push_str("<!ELEMENT ");
                out.push_str(&e.name);
                out.push(' ');
                write_content_model(&mut out, &e.content_model);
                out.push('>');
                last_was_comment = false;
            }
            DtdDeclaration::Attlist(a) => {
                out.push_str("<!ATTLIST ");
                out.push_str(&a.element_name);
                out.push(' ');
                out.push_str(&a.attribute_name);
                out.push(' ');
                write_attribute_type(&mut out, &a.attribute_type);
                out.push(' ');
                write_attribute_default(&mut out, &a.default);
                out.push('>');
                last_was_comment = false;
            }
            DtdDeclaration::Entity(e) => {
                out.push_str("<!ENTITY ");
                out.push_str(&e.name);
                match &e.kind {
                    EntityKind::Internal(value) => {
                        out.push(' ');
                        write_entity_value(&mut out, value);
                    }
                    EntityKind::External {
                        system_id,
                        public_id,
                    } => {
                        if let Some(pub_id) = public_id {
                            out.push_str(" PUBLIC \"");
                            out.push_str(pub_id);
                            out.push_str("\" \"");
                            out.push_str(system_id);
                            out.push('"');
                        } else {
                            out.push_str(" SYSTEM \"");
                            out.push_str(system_id);
                            out.push('"');
                        }
                    }
                }
                out.push('>');
                last_was_comment = false;
            }
            DtdDeclaration::Notation(n) => {
                out.push_str("<!NOTATION ");
                out.push_str(&n.name);
                match (&n.public_id, &n.system_id) {
                    (Some(pub_id), Some(sys_id)) => {
                        out.push_str(" PUBLIC \"");
                        out.push_str(pub_id);
                        out.push_str("\" \"");
                        out.push_str(sys_id);
                        out.push('"');
                    }
                    (Some(pub_id), None) => {
                        out.push_str(" PUBLIC \"");
                        out.push_str(pub_id);
                        out.push('"');
                    }
                    (None, Some(sys_id)) => {
                        out.push_str(" SYSTEM \"");
                        out.push_str(sys_id);
                        out.push('"');
                    }
                    (None, None) => {}
                }
                out.push('>');
                last_was_comment = false;
            }
            DtdDeclaration::Comment(text) => {
                out.push_str("<!--");
                out.push_str(text);
                out.push_str("-->");
                last_was_comment = true;
            }
            DtdDeclaration::Pi(target, data) => {
                out.push_str("<?");
                out.push_str(target);
                if let Some(d) = data {
                    out.push(' ');
                    out.push_str(d);
                }
                out.push_str("?>");
                last_was_comment = false;
            }
        }
    }

    // libxml2 adds a newline before ]> unless the last item was a comment.
    if !last_was_comment && !dtd.declarations.is_empty() {
        out.push('\n');
    }

    out
}

/// Writes a content model in libxml2's format.
fn write_content_model(out: &mut String, model: &ContentModel) {
    match model {
        ContentModel::Empty => out.push_str("EMPTY"),
        ContentModel::Any => out.push_str("ANY"),
        ContentModel::Mixed(names) => {
            if names.is_empty() {
                out.push_str("(#PCDATA)");
            } else {
                out.push_str("(#PCDATA");
                for name in names {
                    out.push_str(" | ");
                    out.push_str(name);
                }
                out.push_str(")*");
            }
        }
        ContentModel::Children(spec) => {
            use std::fmt::Write;
            let _ = write!(out, "{spec}");
        }
    }
}

/// Writes an attribute type in libxml2's format.
fn write_attribute_type(out: &mut String, attr_type: &AttributeType) {
    match attr_type {
        AttributeType::CData => out.push_str("CDATA"),
        AttributeType::Id => out.push_str("ID"),
        AttributeType::IdRef => out.push_str("IDREF"),
        AttributeType::IdRefs => out.push_str("IDREFS"),
        AttributeType::Entity => out.push_str("ENTITY"),
        AttributeType::Entities => out.push_str("ENTITIES"),
        AttributeType::NmToken => out.push_str("NMTOKEN"),
        AttributeType::NmTokens => out.push_str("NMTOKENS"),
        AttributeType::Notation(values) | AttributeType::Enumeration(values) => {
            if matches!(attr_type, AttributeType::Notation(_)) {
                out.push_str("NOTATION ");
            }
            out.push('(');
            for (i, v) in values.iter().enumerate() {
                if i > 0 {
                    out.push_str(" | ");
                }
                out.push_str(v);
            }
            out.push(')');
        }
    }
}

/// Writes an attribute default in libxml2's format.
fn write_attribute_default(out: &mut String, default: &AttributeDefault) {
    match default {
        AttributeDefault::Required => out.push_str("#REQUIRED"),
        AttributeDefault::Implied => out.push_str("#IMPLIED"),
        AttributeDefault::Fixed(value) => {
            out.push_str("#FIXED \"");
            out.push_str(value);
            out.push('"');
        }
        AttributeDefault::Default(value) => {
            out.push('"');
            out.push_str(value);
            out.push('"');
        }
    }
}

/// Escapes an entity value for DTD serialization.
///
/// Entity references (`&name;`) and character references (`&#...;`) within
/// the value are preserved as-is (matching libxml2 behavior). Only standalone
/// `&` characters are escaped. The quote character is chosen to minimize
/// escaping: single quotes when the value contains double quotes.
fn write_entity_value(out: &mut String, value: &str) {
    // Choose quote character: use single quotes if value contains double quotes
    // but not single quotes (avoids escaping). Otherwise use double quotes.
    let quote = if value.contains('"') && !value.contains('\'') {
        '\''
    } else {
        '"'
    };
    out.push(quote);

    let bytes = value.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'&' {
            // Check if this is a valid entity or character reference — if so, pass through.
            if let Some(ref_end) = find_reference_end(bytes, i) {
                // Copy the reference as-is
                let ref_str = std::str::from_utf8(&bytes[i..=ref_end]).unwrap_or("&amp;");
                out.push_str(ref_str);
                i = ref_end + 1;
            } else {
                out.push_str("&amp;");
                i += 1;
            }
        } else if bytes[i] == b'%' {
            out.push_str("&#37;");
            i += 1;
        } else if bytes[i] == quote as u8 {
            if quote == '"' {
                out.push_str("&quot;");
            } else {
                out.push_str("&apos;");
            }
            i += 1;
        } else {
            // Push the char (may be multi-byte UTF-8)
            let ch = &value[i..];
            if let Some(c) = ch.chars().next() {
                out.push(c);
                i += c.len_utf8();
            } else {
                i += 1;
            }
        }
    }

    out.push(quote);
}

/// Finds the end position (inclusive, the `;`) of an entity or character
/// reference starting at `start` in `bytes`. Returns `None` if the `&` at
/// `start` is not the beginning of a valid reference.
fn find_reference_end(bytes: &[u8], start: usize) -> Option<usize> {
    if start >= bytes.len() || bytes[start] != b'&' {
        return None;
    }
    let mut i = start + 1;
    if i >= bytes.len() {
        return None;
    }

    if bytes[i] == b'#' {
        // Character reference: &#digits; or &#xhexdigits;
        i += 1;
        if i >= bytes.len() {
            return None;
        }
        if bytes[i] == b'x' {
            i += 1;
            let digit_start = i;
            while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            if i == digit_start || i >= bytes.len() || bytes[i] != b';' {
                return None;
            }
        } else {
            let digit_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == digit_start || i >= bytes.len() || bytes[i] != b';' {
                return None;
            }
        }
        Some(i)
    } else {
        // Named entity reference: &name;
        // Name must start with a name start char (letter or _)
        if !is_name_start_byte(bytes[i]) {
            return None;
        }
        i += 1;
        while i < bytes.len() && is_name_byte(bytes[i]) {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b';' {
            return None;
        }
        Some(i)
    }
}

/// Checks if a byte is valid as the start of an XML name.
fn is_name_start_byte(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b':'
}

/// Checks if a byte is valid within an XML name.
fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b':' || b == b'-' || b == b'.'
}

// ---------------------------------------------------------------------------
// DTD Parser
// ---------------------------------------------------------------------------

/// Parses a DTD internal subset string into a [`Dtd`] data structure.
///
/// The input should be the content from inside `<!DOCTYPE root [ ... ]>`,
/// i.e., just the internal subset without the surrounding brackets.
///
/// # Errors
///
/// Returns a `ParseError` if the DTD content is malformed.
///
/// # Examples
///
/// ```
/// use xmloxide::validation::dtd::parse_dtd;
///
/// let dtd = parse_dtd("<!ELEMENT root (#PCDATA)>").unwrap();
/// assert!(dtd.elements.contains_key("root"));
/// ```
pub fn parse_dtd(input: &str) -> Result<Dtd, ParseError> {
    let mut parser = DtdParser::new(input);
    parser.parse()
}

/// Maximum nesting depth for `<!ELEMENT>` content-model groups.
///
/// Bounds the mutual recursion between [`DtdParser::parse_content_spec_group`]
/// and [`DtdParser::parse_content_particle`] so that a deeply-nested
/// parenthesised content model cannot exhaust the stack (CWE-674). Matches
/// the spirit of the parser's `DEFAULT_MAX_DEPTH`, which bounds element
/// nesting but does not reach the DTD content-model grammar.
const MAX_CONTENT_MODEL_DEPTH: u32 = 256;

/// Internal DTD parser state.
struct DtdParser<'a> {
    input: &'a [u8],
    pos: usize,
    line: u32,
    column: u32,
    dtd: Dtd,
}

impl<'a> DtdParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
            line: 1,
            column: 1,
            dtd: Dtd::default(),
        }
    }

    fn parse(&mut self) -> Result<Dtd, ParseError> {
        loop {
            self.skip_whitespace();
            if self.at_end() {
                break;
            }

            if self.looking_at(b"<!--") {
                self.parse_comment_decl()?;
            } else if self.looking_at(b"<!ELEMENT") {
                self.parse_element_decl()?;
            } else if self.looking_at(b"<!ATTLIST") {
                self.parse_attlist_decl()?;
            } else if self.looking_at(b"<!ENTITY") {
                self.parse_entity_decl()?;
            } else if self.looking_at(b"<!NOTATION") {
                self.parse_notation_decl()?;
            } else if self.looking_at(b"<?") {
                self.parse_pi_decl()?;
            } else if self.peek() == Some(b'%') {
                // Parameter entity reference — skip it since we don't expand
                self.skip_pe_reference()?;
            } else {
                return Err(self.fatal(format!(
                    "unexpected character '{}' in DTD",
                    self.peek().map_or('?', |b| b as char)
                )));
            }
        }

        self.post_validate()?;

        Ok(std::mem::take(&mut self.dtd))
    }

    /// Post-parse validation checks that require the complete entity map.
    ///
    /// Detects entity recursion (WFC: No Recursion), validates that entity
    /// references in attribute defaults refer to internal parsed entities,
    /// and checks for `<` in entity replacement text used in attributes.
    fn post_validate(&self) -> Result<(), ParseError> {
        // Check for entity recursion (WFC: No Recursion, XML 1.0 §4.1).
        // `safe` memoizes entities already proven acyclic so the walk is
        // linear in the total size of the entity declarations rather than
        // exponential in chain depth.
        let mut safe = std::collections::HashSet::new();
        for (name, decl) in &self.dtd.entities {
            if let EntityKind::Internal(ref value) = decl.kind {
                if safe.contains(name.as_str()) {
                    continue;
                }
                let mut visited = std::collections::HashSet::new();
                visited.insert(name.clone());
                self.check_entity_recursion(value, &mut visited, &mut safe)?;
                safe.insert(name.clone());
            }
        }

        // Check for parameter entity recursion (WFC: No Recursion, XML 1.0 §4.1).
        // PE values may contain encoded PE references via &#37; (which is '%').
        // After char ref expansion, if %name; appears in its own value, that's
        // direct or indirect recursion.
        let mut safe = std::collections::HashSet::new();
        for (name, decl) in &self.dtd.param_entities {
            if let EntityKind::Internal(ref value) = decl.kind {
                if safe.contains(name.as_str()) {
                    continue;
                }
                let expanded = expand_char_refs_only(value);
                let mut visited = std::collections::HashSet::new();
                visited.insert(name.clone());
                self.check_pe_recursion(&expanded, &mut visited, &mut safe)?;
                safe.insert(name.clone());
            }
        }

        // Validate entity replacement text after character reference
        // expansion (XML 1.0 §4.5). Character references in entity
        // values are expanded at declaration time. The resulting
        // replacement text must be well-formed when re-parsed.
        for (name, decl) in &self.dtd.entities {
            if let EntityKind::Internal(ref value) = decl.kind {
                self.validate_replacement_text(name, value)?;
            }
        }

        // Validate predefined entity redeclarations (XML 1.0 §4.6).
        // If lt, gt, amp, apos, or quot are declared, their replacement
        // text must be a character reference to the respective character.
        self.validate_predefined_entities()?;

        // Note: content production validation (XML 1.0 §4.3.2) is
        // performed at entity expansion time in the XML parser, not
        // here, because it only applies to entities that are actually
        // referenced in the document.

        // Validate entity references in ATTLIST defaults. `checked`
        // memoizes entities already validated so shared references are
        // walked once (the reference graph is acyclic at this point).
        let mut checked = std::collections::HashSet::new();
        for attrs in self.dtd.attributes.values() {
            for attr in attrs {
                let (AttributeDefault::Default(default_value)
                | AttributeDefault::Fixed(default_value)) = &attr.default
                else {
                    continue;
                };
                self.validate_attr_default_entities(default_value, &mut checked)?;
            }
        }

        Ok(())
    }

    /// Validates that predefined entity redeclarations (lt, gt, amp, apos,
    /// quot) use the correct character reference as replacement text.
    ///
    /// Per XML 1.0 §4.6: "If the entities lt or amp are declared, they MUST
    /// be declared as internal entities whose replacement text is a character
    /// reference to the respective character."
    fn validate_predefined_entities(&self) -> Result<(), ParseError> {
        // Per XML 1.0 §4.6: lt and amp MUST use character references.
        // gt, apos, and quot may use either the literal character or a
        // character reference.
        let expected: &[(&str, &str, &[&str])] = &[
            ("lt", "<", &["&#60;", "&#x3C;", "&#x3c;"]),
            ("gt", ">", &[">", "&#62;", "&#x3E;", "&#x3e;"]),
            ("amp", "&", &["&#38;", "&#x26;"]),
            ("apos", "'", &["'", "&#39;", "&#x27;"]),
            ("quot", "\"", &["\"", "&#34;", "&#x22;"]),
        ];
        for &(name, _char_val, valid_refs) in expected {
            if let Some(decl) = self.dtd.entities.get(name) {
                match &decl.kind {
                    EntityKind::Internal(value) => {
                        // Check if the value is a valid character reference
                        // for this predefined entity.
                        if !valid_refs.iter().any(|r| r == value) {
                            return Err(self.fatal(format!(
                                "predefined entity '{name}' must be declared as \
                                 a character reference (e.g., '{}')",
                                valid_refs[0]
                            )));
                        }
                    }
                    EntityKind::External { .. } => {
                        return Err(self.fatal(format!(
                            "predefined entity '{name}' must be an internal entity"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Validates entity replacement text after character reference
    /// expansion per XML 1.0 §4.5.
    ///
    /// Expands only character references in the entity value (not entity
    /// references), then checks the resulting replacement text for basic
    /// well-formedness: bare `&` characters from `&#38;` expansion that
    /// don't form valid references are rejected.
    fn validate_replacement_text(&self, entity_name: &str, value: &str) -> Result<(), ParseError> {
        // Only check values that contain character references
        if !value.contains("&#") {
            return Ok(());
        }

        // Build replacement text by expanding only character references
        let replacement = Self::expand_char_refs_only(value);

        // Check for bare '&' in the replacement text that don't form
        // valid entity or character references
        let bytes = replacement.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'&' {
                i += 1;
                if i >= bytes.len() {
                    return Err(self.fatal(format!(
                        "entity '{entity_name}' replacement text contains \
                         bare '&' at end of text"
                    )));
                }
                if bytes[i] == b'#' {
                    // Character reference — check it's complete
                    i += 1;
                    let has_digits = if i < bytes.len() && bytes[i] == b'x' {
                        i += 1;
                        let start = i;
                        while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                            i += 1;
                        }
                        i > start
                    } else {
                        let start = i;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            i += 1;
                        }
                        i > start
                    };
                    if !has_digits || i >= bytes.len() || bytes[i] != b';' {
                        return Err(self.fatal(format!(
                            "entity '{entity_name}' replacement text contains \
                             incomplete character reference"
                        )));
                    }
                    i += 1;
                } else if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' || bytes[i] == b':' {
                    // Entity reference — skip name
                    while i < bytes.len() && bytes[i] != b';' {
                        i += 1;
                    }
                    if i >= bytes.len() {
                        return Err(self.fatal(format!(
                            "entity '{entity_name}' replacement text contains \
                             incomplete entity reference"
                        )));
                    }
                    i += 1;
                } else {
                    return Err(self.fatal(format!(
                        "entity '{entity_name}' replacement text contains \
                         bare '&' not followed by a valid reference"
                    )));
                }
            } else {
                i += 1;
            }
        }
        Ok(())
    }

    /// Expands only character references in a string, leaving entity
    /// references as-is. Returns the expanded text.
    fn expand_char_refs_only(value: &str) -> String {
        expand_char_refs_only(value)
    }

    /// Recursively checks for entity reference cycles.
    fn check_entity_recursion(
        &self,
        value: &str,
        visited: &mut std::collections::HashSet<String>,
        safe: &mut std::collections::HashSet<String>,
    ) -> Result<(), ParseError> {
        for ref_name in Self::extract_entity_refs(value) {
            if safe.contains(ref_name) {
                continue;
            }
            if visited.contains(ref_name) {
                return Err(self.fatal(format!("recursive entity reference: '{ref_name}'")));
            }
            if let Some(decl) = self.dtd.entities.get(ref_name) {
                if let EntityKind::Internal(ref inner_value) = decl.kind {
                    visited.insert(ref_name.to_string());
                    self.check_entity_recursion(inner_value, visited, safe)?;
                    visited.remove(ref_name);
                    safe.insert(ref_name.to_string());
                }
            }
        }
        Ok(())
    }

    /// Recursively checks for parameter entity reference cycles.
    ///
    /// Examines the char-ref-expanded replacement text for `%name;` patterns.
    fn check_pe_recursion(
        &self,
        value: &str,
        visited: &mut std::collections::HashSet<String>,
        safe: &mut std::collections::HashSet<String>,
    ) -> Result<(), ParseError> {
        for ref_name in Self::extract_pe_refs(value) {
            if safe.contains(&ref_name) {
                continue;
            }
            if visited.contains(&ref_name) {
                return Err(self.fatal(format!(
                    "recursive parameter entity reference: '%{ref_name}'"
                )));
            }
            if let Some(decl) = self.dtd.param_entities.get(&ref_name) {
                if let EntityKind::Internal(ref inner_value) = decl.kind {
                    let expanded = expand_char_refs_only(inner_value);
                    visited.insert(ref_name.clone());
                    self.check_pe_recursion(&expanded, visited, safe)?;
                    visited.remove(&ref_name);
                    safe.insert(ref_name.clone());
                }
            }
        }
        Ok(())
    }

    /// Extracts parameter entity reference names (`%name;`) from a string.
    fn extract_pe_refs(value: &str) -> Vec<String> {
        let mut refs = Vec::new();
        let bytes = value.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                i += 1;
                if i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
                    let start = i;
                    while i < bytes.len() && bytes[i] != b';' && !bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i < bytes.len() && bytes[i] == b';' && i > start {
                        if let Ok(name) = std::str::from_utf8(&bytes[start..i]) {
                            refs.push(name.to_string());
                        }
                        i += 1;
                    }
                }
            } else {
                i += 1;
            }
        }
        refs
    }

    /// Validates entity references in attribute default values.
    ///
    /// Checks WFC: No External Entity References (§3.1) and
    /// WFC: No `<` in Attribute Values for entity replacement text.
    fn validate_attr_default_entities(
        &self,
        value: &str,
        checked: &mut std::collections::HashSet<String>,
    ) -> Result<(), ParseError> {
        for ref_name in Self::extract_entity_refs(value) {
            // Built-in entities are always fine
            if matches!(ref_name, "amp" | "lt" | "gt" | "apos" | "quot") {
                continue;
            }
            if checked.contains(ref_name) {
                continue;
            }
            match self.dtd.entities.get(ref_name) {
                None => {
                    return Err(self.fatal(format!(
                        "undeclared entity '{ref_name}' referenced in \
                         attribute default value"
                    )));
                }
                Some(decl) => match &decl.kind {
                    EntityKind::External { .. } => {
                        return Err(self.fatal(format!(
                            "attribute default value must not reference \
                             external entity '{ref_name}'"
                        )));
                    }
                    EntityKind::Internal(ref text) => {
                        // Check for '<' in replacement text (WFC: No < in
                        // Attribute Values, XML 1.0 §3.1)
                        if text.contains('<') {
                            return Err(self.fatal(format!(
                                "entity '{ref_name}' contains '<' and cannot \
                                 be used in attribute values"
                            )));
                        }
                        // Recursively check referenced entities
                        checked.insert(ref_name.to_string());
                        self.validate_attr_default_entities(text, checked)?;
                    }
                },
            }
        }
        Ok(())
    }

    /// Extracts entity reference names from a string value.
    ///
    /// Returns an iterator over entity names found in `&name;` patterns,
    /// excluding character references (`&#...;`).
    fn extract_entity_refs(value: &str) -> Vec<&str> {
        let mut refs = Vec::new();
        let bytes = value.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'&' {
                i += 1;
                if i < bytes.len() && bytes[i] == b'#' {
                    // Character reference — skip
                    while i < bytes.len() && bytes[i] != b';' {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += 1;
                    }
                } else {
                    // Entity reference
                    let start = i;
                    while i < bytes.len() && bytes[i] != b';' && bytes[i] != b'&' {
                        i += 1;
                    }
                    if i < bytes.len() && bytes[i] == b';' && i > start {
                        if let Ok(name) = std::str::from_utf8(&bytes[start..i]) {
                            refs.push(name);
                        }
                        i += 1;
                    }
                }
            } else {
                i += 1;
            }
        }
        refs
    }

    // --- ELEMENT declaration ---
    // See XML 1.0 §3.2: [45] elementdecl

    fn parse_element_decl(&mut self) -> Result<(), ParseError> {
        self.expect_str(b"<!ELEMENT")?;
        self.skip_whitespace_required()?;
        let name = self.parse_name()?;
        self.skip_whitespace_required()?;
        let content_model = self.parse_content_model()?;
        self.skip_whitespace();
        self.expect_byte(b'>')?;

        let decl = ElementDecl {
            name: name.clone(),
            content_model,
        };
        self.dtd
            .declarations
            .push(DtdDeclaration::Element(decl.clone()));
        self.dtd.elements.insert(name, decl);
        Ok(())
    }

    fn parse_content_model(&mut self) -> Result<ContentModel, ParseError> {
        if self.looking_at(b"EMPTY") {
            self.expect_str(b"EMPTY")?;
            return Ok(ContentModel::Empty);
        }
        if self.looking_at(b"ANY") {
            self.expect_str(b"ANY")?;
            return Ok(ContentModel::Any);
        }

        // Must be Mixed or Children, both start with '('
        self.expect_byte(b'(')?;
        self.skip_whitespace();

        // Check for mixed content: (#PCDATA ...)
        if self.looking_at(b"#PCDATA") {
            self.expect_str(b"#PCDATA")?;
            self.skip_whitespace();

            let mut names = Vec::new();

            if self.peek() == Some(b')') {
                // (#PCDATA) — text only
                self.advance(1);
                // Optional '*' after (#PCDATA) — some DTDs write (#PCDATA)*
                if self.peek() == Some(b'*') {
                    self.advance(1);
                }
                return Ok(ContentModel::Mixed(names));
            }

            // (#PCDATA|a|b)*
            while self.peek() == Some(b'|') {
                self.advance(1);
                self.skip_whitespace();
                let elem_name = self.parse_name()?;
                names.push(elem_name);
                self.skip_whitespace();
            }

            self.expect_byte(b')')?;
            self.expect_byte(b'*')?;

            return Ok(ContentModel::Mixed(names));
        }

        // Element-only content: parse as a content spec group. The opening
        // '(' has already been consumed, so we are one level deep.
        let spec = self.parse_content_spec_group(1)?;
        Ok(ContentModel::Children(spec))
    }

    /// Parses a content spec starting after the opening '(' has been consumed
    /// and the first item is NOT `#PCDATA`.
    ///
    /// `depth` is the current parenthesis-nesting level (1 for the outermost
    /// group). It bounds the mutual recursion with `parse_content_particle` so
    /// that untrusted input cannot exhaust the stack; see
    /// [`MAX_CONTENT_MODEL_DEPTH`].
    fn parse_content_spec_group(&mut self, depth: u32) -> Result<ContentSpec, ParseError> {
        if depth > MAX_CONTENT_MODEL_DEPTH {
            return Err(self.fatal(format!(
                "content model nesting exceeds maximum depth of {MAX_CONTENT_MODEL_DEPTH}"
            )));
        }
        let mut first = self.parse_content_particle(depth)?;
        self.skip_whitespace();

        // Determine if this is a sequence (,) or choice (|)
        if self.peek() == Some(b',') {
            // Sequence
            let mut items = vec![first];
            while self.peek() == Some(b',') {
                self.advance(1);
                self.skip_whitespace();
                let item = self.parse_content_particle(depth)?;
                items.push(item);
                self.skip_whitespace();
            }
            self.expect_byte(b')')?;
            let occurrence = self.parse_occurrence();
            Ok(ContentSpec {
                kind: ContentSpecKind::Seq(items),
                occurrence,
            })
        } else if self.peek() == Some(b'|') {
            // Choice
            let mut items = vec![first];
            while self.peek() == Some(b'|') {
                self.advance(1);
                self.skip_whitespace();
                let item = self.parse_content_particle(depth)?;
                items.push(item);
                self.skip_whitespace();
            }
            self.expect_byte(b')')?;
            let occurrence = self.parse_occurrence();
            Ok(ContentSpec {
                kind: ContentSpecKind::Choice(items),
                occurrence,
            })
        } else {
            // Single item group: (item)?/*
            self.expect_byte(b')')?;
            let group_occurrence = self.parse_occurrence();

            if group_occurrence != Occurrence::Once {
                // Group has occurrence: (X)+ → wrap in Seq
                Ok(ContentSpec {
                    kind: ContentSpecKind::Seq(vec![first]),
                    occurrence: group_occurrence,
                })
            } else if first.occurrence != Occurrence::Once {
                // Inner particle has occurrence but group doesn't.
                // libxml2 normalizes (X+) → (X)+ by moving occurrence
                // to the outer group.
                let inner_occ = first.occurrence;
                first.occurrence = Occurrence::Once;
                Ok(ContentSpec {
                    kind: ContentSpecKind::Seq(vec![first]),
                    occurrence: inner_occ,
                })
            } else {
                // No occurrence on either — unwrap the group
                Ok(first)
            }
        }
    }

    /// Parses a single content particle: a name with an optional occurrence
    /// indicator, or a nested parenthesised group.
    ///
    /// `depth` is the enclosing parenthesis-nesting level; a nested group
    /// recurses at `depth + 1`. See [`MAX_CONTENT_MODEL_DEPTH`].
    fn parse_content_particle(&mut self, depth: u32) -> Result<ContentSpec, ParseError> {
        if self.peek() == Some(b'(') {
            self.advance(1);
            self.skip_whitespace();
            self.parse_content_spec_group(depth + 1)
        } else {
            let name = self.parse_name()?;
            let occurrence = self.parse_occurrence();
            Ok(ContentSpec {
                kind: ContentSpecKind::Name(name),
                occurrence,
            })
        }
    }

    fn parse_occurrence(&mut self) -> Occurrence {
        match self.peek() {
            Some(b'?') => {
                self.advance(1);
                Occurrence::Optional
            }
            Some(b'*') => {
                self.advance(1);
                Occurrence::ZeroOrMore
            }
            Some(b'+') => {
                self.advance(1);
                Occurrence::OneOrMore
            }
            _ => Occurrence::Once,
        }
    }

    // --- ATTLIST declaration ---
    // See XML 1.0 §3.3: [52] AttlistDecl

    fn parse_attlist_decl(&mut self) -> Result<(), ParseError> {
        self.expect_str(b"<!ATTLIST")?;
        self.skip_whitespace_required()?;
        let element_name = self.parse_name()?;

        loop {
            self.skip_whitespace();
            if self.peek() == Some(b'>') {
                self.advance(1);
                break;
            }

            let attribute_name = self.parse_name()?;
            self.skip_whitespace_required()?;
            let attribute_type = self.parse_attribute_type()?;
            self.skip_whitespace_required()?;
            let default = self.parse_attribute_default()?;

            let decl = AttributeDecl {
                element_name: element_name.clone(),
                attribute_name,
                attribute_type,
                default,
            };

            // Per XML 1.0 §3.3, the first attribute declaration is binding;
            // subsequent declarations for the same attribute are ignored.
            let attrs = self.dtd.attributes.entry(element_name.clone()).or_default();
            if !attrs
                .iter()
                .any(|a| a.attribute_name == decl.attribute_name)
            {
                self.dtd
                    .declarations
                    .push(DtdDeclaration::Attlist(decl.clone()));
                attrs.push(decl);
            }
        }

        Ok(())
    }

    fn parse_attribute_type(&mut self) -> Result<AttributeType, ParseError> {
        if self.looking_at(b"CDATA") {
            self.expect_str(b"CDATA")?;
            Ok(AttributeType::CData)
        } else if self.looking_at(b"IDREFS") {
            self.expect_str(b"IDREFS")?;
            Ok(AttributeType::IdRefs)
        } else if self.looking_at(b"IDREF") {
            self.expect_str(b"IDREF")?;
            Ok(AttributeType::IdRef)
        } else if self.looking_at(b"ID") {
            self.expect_str(b"ID")?;
            Ok(AttributeType::Id)
        } else if self.looking_at(b"ENTITIES") {
            self.expect_str(b"ENTITIES")?;
            Ok(AttributeType::Entities)
        } else if self.looking_at(b"ENTITY") {
            self.expect_str(b"ENTITY")?;
            Ok(AttributeType::Entity)
        } else if self.looking_at(b"NMTOKENS") {
            self.expect_str(b"NMTOKENS")?;
            Ok(AttributeType::NmTokens)
        } else if self.looking_at(b"NMTOKEN") {
            self.expect_str(b"NMTOKEN")?;
            Ok(AttributeType::NmToken)
        } else if self.looking_at(b"NOTATION") {
            self.expect_str(b"NOTATION")?;
            self.skip_whitespace_required()?;
            let values = self.parse_enumerated_values()?;
            Ok(AttributeType::Notation(values))
        } else if self.peek() == Some(b'(') {
            let values = self.parse_enumerated_values()?;
            Ok(AttributeType::Enumeration(values))
        } else {
            Err(self.fatal("expected attribute type"))
        }
    }

    fn parse_enumerated_values(&mut self) -> Result<Vec<String>, ParseError> {
        self.expect_byte(b'(')?;
        self.skip_whitespace();
        let mut values = Vec::new();

        let first = self.parse_nmtoken()?;
        values.push(first);

        loop {
            self.skip_whitespace();
            if self.peek() == Some(b')') {
                self.advance(1);
                break;
            }
            self.expect_byte(b'|')?;
            self.skip_whitespace();
            let val = self.parse_nmtoken()?;
            values.push(val);
        }

        Ok(values)
    }

    fn parse_attribute_default(&mut self) -> Result<AttributeDefault, ParseError> {
        if self.looking_at(b"#REQUIRED") {
            self.expect_str(b"#REQUIRED")?;
            Ok(AttributeDefault::Required)
        } else if self.looking_at(b"#IMPLIED") {
            self.expect_str(b"#IMPLIED")?;
            Ok(AttributeDefault::Implied)
        } else if self.looking_at(b"#FIXED") {
            self.expect_str(b"#FIXED")?;
            self.skip_whitespace_required()?;
            let value = self.parse_quoted_value()?;
            self.validate_default_value(&value)?;
            Ok(AttributeDefault::Fixed(value))
        } else {
            let value = self.parse_quoted_value()?;
            self.validate_default_value(&value)?;
            Ok(AttributeDefault::Default(value))
        }
    }

    // --- ENTITY declaration ---
    // See XML 1.0 §4.2: [70] EntityDecl

    #[allow(clippy::too_many_lines)]
    fn parse_entity_decl(&mut self) -> Result<(), ParseError> {
        self.expect_str(b"<!ENTITY")?;
        self.skip_whitespace_required()?;

        // Parameter entities (% name)
        if self.peek() == Some(b'%') {
            self.advance(1);
            self.skip_whitespace_required()?;
            let pe_name = self.parse_name()?;
            // Namespaces in XML 1.0: entity names must be NCNames (no colons).
            if pe_name.contains(':') {
                return Err(self.fatal(format!("entity name '{pe_name}' must not contain a colon")));
            }
            self.skip_whitespace_required()?;

            let pe_kind = if self.peek() == Some(b'"') || self.peek() == Some(b'\'') {
                // Internal PE — parse and validate the value
                let value = self.parse_quoted_value()?;
                self.validate_entity_value(&value, true)?;
                Some(EntityKind::Internal(value))
            } else if self.looking_at(b"SYSTEM") {
                // External PE — parse external ID
                self.expect_str(b"SYSTEM")?;
                self.skip_whitespace_required()?;
                let system_id = self.parse_quoted_value()?;
                Some(EntityKind::External {
                    system_id,
                    public_id: None,
                })
            } else if self.looking_at(b"PUBLIC") {
                self.expect_str(b"PUBLIC")?;
                self.skip_whitespace_required()?;
                let public_id = self.parse_quoted_value()?;
                self.validate_public_id(&public_id)?;
                self.skip_whitespace_required()?;
                let system_id = self.parse_quoted_value()?;
                Some(EntityKind::External {
                    system_id,
                    public_id: Some(public_id),
                })
            } else {
                return Err(self.fatal("expected entity value or external ID"));
            };

            self.skip_whitespace();
            // Reject NDATA on parameter entities (XML 1.0 §4.2.2)
            if self.looking_at(b"NDATA") {
                return Err(self.fatal("NDATA annotation is not allowed on parameter entities"));
            }
            self.expect_byte(b'>')?;

            // Store PE declaration (first declaration wins per XML 1.0 §4.2)
            if let Some(kind) = pe_kind {
                self.dtd
                    .param_entities
                    .entry(pe_name)
                    .or_insert(EntityDecl {
                        name: String::new(),
                        kind,
                    });
            }
            return Ok(());
        }

        let name = self.parse_name()?;
        // Namespaces in XML 1.0: entity names must be NCNames (no colons).
        if name.contains(':') {
            return Err(self.fatal(format!("entity name '{name}' must not contain a colon")));
        }
        self.skip_whitespace_required()?;

        let is_parameter_entity = false;
        let kind = if self.peek() == Some(b'"') || self.peek() == Some(b'\'') {
            // Internal entity
            let value = self.parse_quoted_value()?;
            self.validate_entity_value(&value, is_parameter_entity)?;
            EntityKind::Internal(value)
        } else if self.looking_at(b"SYSTEM") {
            self.expect_str(b"SYSTEM")?;
            self.skip_whitespace_required()?;
            let system_id = self.parse_quoted_value()?;
            EntityKind::External {
                system_id,
                public_id: None,
            }
        } else if self.looking_at(b"PUBLIC") {
            self.expect_str(b"PUBLIC")?;
            self.skip_whitespace_required()?;
            let public_id = self.parse_quoted_value()?;
            self.validate_public_id(&public_id)?;
            self.skip_whitespace_required()?;
            let system_id = self.parse_quoted_value()?;
            EntityKind::External {
                system_id,
                public_id: Some(public_id),
            }
        } else {
            return Err(self.fatal("expected entity value or external ID"));
        };

        let had_ws = self.skip_whitespace();

        // Handle optional NDATA for unparsed external entities (XML 1.0 §4.2.2)
        if self.looking_at(b"NDATA") {
            // NDATA is only allowed on external entities
            if matches!(kind, EntityKind::Internal(_)) {
                return Err(self.fatal("NDATA annotation is not allowed on internal entities"));
            }
            // Whitespace is required before NDATA (XML 1.0 §4.2.2)
            if !had_ws {
                return Err(self.fatal("whitespace required before NDATA"));
            }
            self.expect_str(b"NDATA")?;
            self.skip_whitespace_required()?;
            let _notation_name = self.parse_name()?;
            self.skip_whitespace();
        }

        self.expect_byte(b'>')?;

        // Per XML 1.0 §4.2, the first entity declaration is binding;
        // subsequent declarations of the same entity are ignored.
        let decl = EntityDecl {
            name: name.clone(),
            kind,
        };
        self.dtd
            .declarations
            .push(DtdDeclaration::Entity(decl.clone()));
        self.dtd.entities.entry(name).or_insert(decl);
        Ok(())
    }

    // --- NOTATION declaration ---
    // See XML 1.0 §4.7: [82] NotationDecl

    fn parse_notation_decl(&mut self) -> Result<(), ParseError> {
        self.expect_str(b"<!NOTATION")?;
        self.skip_whitespace_required()?;
        let name = self.parse_name()?;
        // Namespaces in XML 1.0: notation names must be NCNames (no colons).
        if name.contains(':') {
            return Err(self.fatal(format!("notation name '{name}' must not contain a colon")));
        }
        self.skip_whitespace_required()?;

        let (system_id, public_id) = if self.looking_at(b"SYSTEM") {
            self.expect_str(b"SYSTEM")?;
            self.skip_whitespace_required()?;
            let sid = self.parse_quoted_value()?;
            (Some(sid), None)
        } else if self.looking_at(b"PUBLIC") {
            self.expect_str(b"PUBLIC")?;
            self.skip_whitespace_required()?;
            let pid = self.parse_quoted_value()?;
            self.validate_public_id(&pid)?;
            // System ID is optional for notations with PUBLIC
            self.skip_whitespace();
            let sid = if self.peek() == Some(b'"') || self.peek() == Some(b'\'') {
                Some(self.parse_quoted_value()?)
            } else {
                None
            };
            (sid, Some(pid))
        } else {
            return Err(self.fatal("expected SYSTEM or PUBLIC in NOTATION declaration"));
        };

        self.skip_whitespace();
        self.expect_byte(b'>')?;

        let decl = NotationDecl {
            name: name.clone(),
            system_id,
            public_id,
        };
        self.dtd
            .declarations
            .push(DtdDeclaration::Notation(decl.clone()));
        self.dtd.notations.insert(name, decl);
        Ok(())
    }

    // --- Skip helpers ---

    /// Parses a comment and stores it as a `DtdDeclaration::Comment`.
    fn parse_comment_decl(&mut self) -> Result<(), ParseError> {
        self.expect_str(b"<!--")?;
        let start = self.pos;
        loop {
            if self.at_end() {
                return Err(self.fatal("unexpected end of input in comment"));
            }
            if self.looking_at(b"-->") {
                let text = std::str::from_utf8(&self.input[start..self.pos])
                    .unwrap_or("")
                    .to_string();
                self.advance(3);
                self.dtd.declarations.push(DtdDeclaration::Comment(text));
                return Ok(());
            }
            self.advance(1);
        }
    }

    /// Parses a processing instruction and stores it as a `DtdDeclaration::Pi`.
    fn parse_pi_decl(&mut self) -> Result<(), ParseError> {
        self.expect_str(b"<?")?;

        // Parse and validate the PI target name (XML 1.0 §2.6)
        let target = self.parse_name()?;

        // Reject <?xml ...?> inside DTD (XML 1.0 §2.8)
        if target.eq_ignore_ascii_case("xml") {
            return Err(self.fatal("XML declaration is not allowed inside DTD"));
        }

        // If we're immediately at ?>, no data — that's fine
        if self.looking_at(b"?>") {
            self.advance(2);
            self.dtd.declarations.push(DtdDeclaration::Pi(target, None));
            return Ok(());
        }

        // If there's data, whitespace is required between target and data
        let is_ws = self
            .peek()
            .is_some_and(|b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n');
        if !is_ws {
            return Err(self.fatal("space required between PI target and data"));
        }

        let start = self.pos;
        loop {
            if self.at_end() {
                return Err(self.fatal("unexpected end of input in processing instruction"));
            }
            if self.looking_at(b"?>") {
                let data = std::str::from_utf8(&self.input[start..self.pos])
                    .unwrap_or("")
                    .trim()
                    .to_string();
                self.advance(2);
                let data = if data.is_empty() { None } else { Some(data) };
                self.dtd.declarations.push(DtdDeclaration::Pi(target, data));
                return Ok(());
            }
            self.advance(1);
        }
    }

    fn skip_pe_reference(&mut self) -> Result<(), ParseError> {
        self.expect_byte(b'%')?;
        // Read the name
        let _name = self.parse_name()?;
        self.expect_byte(b';')?;
        Ok(())
    }

    // --- Name / token parsing ---

    fn parse_name(&mut self) -> Result<String, ParseError> {
        if self.pos >= self.input.len() {
            return Err(self.fatal("expected name, found end of input"));
        }

        let start = self.pos;
        let first = self.input[self.pos];

        // ASCII fast path
        if is_ascii_name_start(first) {
            self.pos += 1;
            self.column += 1;
            while self.pos < self.input.len() && is_ascii_name_char(self.input[self.pos]) {
                self.pos += 1;
                self.column += 1;
            }
            if self.pos >= self.input.len() || self.input[self.pos] < 0x80 {
                let name = std::str::from_utf8(&self.input[start..self.pos])
                    .map_err(|_| self.fatal("invalid UTF-8 in name"))?;
                return Ok(name.to_string());
            }
            // Fall through to slow path for non-ASCII continuation
        } else {
            let ch = self
                .peek_char()
                .ok_or_else(|| self.fatal("expected name"))?;
            if !is_name_start_char(ch) {
                return Err(self.fatal(format!("invalid name start character: '{ch}'")));
            }
            self.advance_char(ch);
        }

        while let Some(ch) = self.peek_char() {
            if is_name_char(ch) {
                self.advance_char(ch);
            } else {
                break;
            }
        }

        let name = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| self.fatal("invalid UTF-8 in name"))?;
        Ok(name.to_string())
    }

    fn parse_nmtoken(&mut self) -> Result<String, ParseError> {
        if self.pos >= self.input.len() {
            return Err(self.fatal("expected NMTOKEN, found end of input"));
        }

        let start = self.pos;
        let first = self.input[self.pos];

        // ASCII fast path
        if is_ascii_name_char(first) {
            self.pos += 1;
            self.column += 1;
            while self.pos < self.input.len() && is_ascii_name_char(self.input[self.pos]) {
                self.pos += 1;
                self.column += 1;
            }
            if self.pos >= self.input.len() || self.input[self.pos] < 0x80 {
                let token = std::str::from_utf8(&self.input[start..self.pos])
                    .map_err(|_| self.fatal("invalid UTF-8 in NMTOKEN"))?;
                return Ok(token.to_string());
            }
            // Fall through to slow path
        } else {
            let ch = self
                .peek_char()
                .ok_or_else(|| self.fatal("expected NMTOKEN"))?;
            if !is_name_char(ch) {
                return Err(self.fatal(format!("invalid NMTOKEN character: '{ch}'")));
            }
            self.advance_char(ch);
        }

        while let Some(ch) = self.peek_char() {
            if is_name_char(ch) {
                self.advance_char(ch);
            } else {
                break;
            }
        }

        let token = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| self.fatal("invalid UTF-8 in NMTOKEN"))?;
        Ok(token.to_string())
    }

    /// Validates an entity value per XML 1.0 §4.3.2 `EntityValue` production.
    ///
    /// Checks that `&` is only used in valid entity/character references,
    /// and that `%` is not present in general entity values.
    #[allow(clippy::too_many_lines)]
    fn validate_entity_value(
        &self,
        value: &str,
        is_parameter_entity: bool,
    ) -> Result<(), ParseError> {
        // First validate all characters are valid XML chars.
        for c in value.chars() {
            if !crate::parser::input::is_xml_char(c) {
                return Err(self.fatal(format!(
                    "invalid XML character U+{:04X} in entity value",
                    c as u32
                )));
            }
        }

        // Text declarations (<?xml ...?>) are forbidden in internal
        // entities (XML 1.0 §4.3.1). They may only appear at the start
        // of external parsed entities.
        if value.starts_with("<?xml") {
            let after = value.as_bytes().get(5).copied();
            if after.map_or(true, |b| b == b' ' || b == b'\t' || b == b'?') {
                return Err(self.fatal("text declaration is not allowed in internal entity value"));
            }
        }

        let bytes = value.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'&' => {
                    // Must be a valid reference: &name; or &#N; or &#xH;
                    i += 1;
                    if i >= bytes.len() {
                        return Err(self.fatal("incomplete reference in entity value: '&' at end"));
                    }
                    if bytes[i] == b'#' {
                        // Character reference — parse and validate
                        i += 1;
                        let char_val = if i < bytes.len() && bytes[i] == b'x' {
                            i += 1;
                            let hex_start = i;
                            if i >= bytes.len() || !bytes[i].is_ascii_hexdigit() {
                                return Err(
                                    self.fatal("malformed character reference in entity value")
                                );
                            }
                            while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                                i += 1;
                            }
                            let hex_str = std::str::from_utf8(&bytes[hex_start..i]).unwrap_or("");
                            u32::from_str_radix(hex_str, 16).unwrap_or(0)
                        } else {
                            let dec_start = i;
                            if i >= bytes.len() || !bytes[i].is_ascii_digit() {
                                return Err(
                                    self.fatal("malformed character reference in entity value")
                                );
                            }
                            while i < bytes.len() && bytes[i].is_ascii_digit() {
                                i += 1;
                            }
                            let dec_str = std::str::from_utf8(&bytes[dec_start..i]).unwrap_or("");
                            dec_str.parse::<u32>().unwrap_or(0)
                        };
                        if i >= bytes.len() || bytes[i] != b';' {
                            return Err(
                                self.fatal("incomplete character reference in entity value")
                            );
                        }
                        i += 1;
                        // Validate the referenced character is a valid XML char
                        if let Some(c) = char::from_u32(char_val) {
                            if !crate::parser::input::is_xml_char(c) {
                                return Err(self.fatal(format!(
                                    "character reference &#x{char_val:X}; refers to invalid XML character"
                                )));
                            }
                        } else {
                            return Err(self.fatal(format!(
                                "character reference value {char_val} is not a valid Unicode code point"
                            )));
                        }
                    } else {
                        // Entity reference — must be Name followed by ';'
                        let start = i;
                        while i < bytes.len()
                            && bytes[i] != b';'
                            && bytes[i] != b'&'
                            && !bytes[i].is_ascii_whitespace()
                        {
                            i += 1;
                        }
                        if i == start || i >= bytes.len() || bytes[i] != b';' {
                            return Err(self.fatal("malformed entity reference in entity value"));
                        }
                        // Validate the entity name starts with a NameStartChar
                        let name_str = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                        if let Some(first_char) = name_str.chars().next() {
                            if !is_name_start_char(first_char) {
                                return Err(self.fatal(format!(
                                    "entity reference name must start with a letter or underscore, found '{first_char}'"
                                )));
                            }
                        }
                        i += 1;
                    }
                }
                b'%' if !is_parameter_entity => {
                    // '%' is not allowed in general entity values (XML 1.0 §4.3.2)
                    return Err(self.fatal("'%' not allowed in general entity value"));
                }
                b'%' if is_parameter_entity => {
                    // WFC: PEs in Internal Subset — PE references MUST NOT
                    // occur within markup declarations in the internal subset
                    // (XML 1.0 §2.8).
                    i += 1;
                    if i < bytes.len() {
                        let first = bytes[i];
                        if first.is_ascii_alphabetic() || first == b'_' || first == b':' {
                            return Err(self.fatal(
                                "parameter entity reference not allowed within \
                                 markup declaration in internal subset",
                            ));
                        }
                    }
                }
                _ => {
                    i += 1;
                }
            }
        }
        Ok(())
    }

    /// Validates an attribute default value per XML 1.0 §3.3.2.
    ///
    /// Checks that entity references within the default value refer to
    /// entities that have already been declared (WFC: Entity Declared).
    /// Also rejects `<` in default values (WFC: No `<` in Attribute Values).
    fn validate_default_value(&self, value: &str) -> Result<(), ParseError> {
        let bytes = value.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'<' => {
                    return Err(self.fatal("'<' not allowed in attribute default value"));
                }
                b'&' => {
                    i += 1;
                    if i < bytes.len() && bytes[i] == b'#' {
                        // Character reference — skip over it
                        i += 1;
                        while i < bytes.len() && bytes[i] != b';' {
                            i += 1;
                        }
                        if i < bytes.len() {
                            i += 1;
                        }
                    } else {
                        // Entity reference — extract name and check declaration
                        let start = i;
                        while i < bytes.len() && bytes[i] != b';' {
                            i += 1;
                        }
                        if i > start && i < bytes.len() {
                            let name = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                            // Built-in entities are always available
                            let is_builtin = matches!(name, "amp" | "lt" | "gt" | "apos" | "quot");
                            if !is_builtin && !self.dtd.entities.contains_key(name) {
                                return Err(self.fatal(format!(
                                    "undeclared entity '{name}' in attribute default value"
                                )));
                            }
                        }
                        if i < bytes.len() {
                            i += 1;
                        }
                    }
                }
                _ => {
                    i += 1;
                }
            }
        }
        Ok(())
    }

    /// Validates that a public ID string contains only valid `PubidChar`s
    /// per XML 1.0 §2.3 `[13]`.
    fn validate_public_id(&self, pid: &str) -> Result<(), ParseError> {
        for c in pid.chars() {
            let valid = matches!(c,
                ' ' | '\r' | '\n' |
                'a'..='z' | 'A'..='Z' | '0'..='9' |
                '-' | '\'' | '(' | ')' | '+' | ',' | '.' | '/' | ':' |
                '=' | '?' | ';' | '!' | '*' | '#' | '@' | '$' | '_' | '%'
            );
            if !valid {
                return Err(self.fatal(format!(
                    "invalid character in public ID: U+{:04X}",
                    c as u32
                )));
            }
        }
        Ok(())
    }

    fn parse_quoted_value(&mut self) -> Result<String, ParseError> {
        let quote = self.next_byte()?;
        if quote != b'"' && quote != b'\'' {
            return Err(self.fatal("expected quoted value"));
        }
        let start = self.pos;
        while !self.at_end() && self.peek() != Some(quote) {
            self.advance(1);
        }
        let value = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| self.fatal("invalid UTF-8 in quoted value"))?
            .to_string();
        if self.at_end() {
            return Err(self.fatal("unexpected end of input in quoted value"));
        }
        self.advance(1); // consume closing quote
        Ok(value)
    }

    // --- Low-level input helpers ---

    fn location(&self) -> SourceLocation {
        SourceLocation {
            line: self.line,
            column: self.column,
            byte_offset: self.pos,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn peek_char(&self) -> Option<char> {
        if self.pos >= self.input.len() {
            return None;
        }
        let first = self.input[self.pos];
        // Fast path: ASCII
        if first < 0x80 {
            return Some(first as char);
        }
        // Slow path: multi-byte UTF-8 — decode only the needed bytes
        let len = match first {
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => return None,
        };
        let remaining = &self.input[self.pos..];
        if remaining.len() < len {
            return None;
        }
        std::str::from_utf8(&remaining[..len])
            .ok()
            .and_then(|s| s.chars().next())
    }

    fn advance(&mut self, count: usize) {
        for _ in 0..count {
            if self.pos < self.input.len() {
                if self.input[self.pos] == b'\n' {
                    self.line += 1;
                    self.column = 1;
                } else {
                    self.column += 1;
                }
                self.pos += 1;
            }
        }
    }

    fn advance_char(&mut self, ch: char) {
        let len = ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        self.pos += len;
    }

    fn next_byte(&mut self) -> Result<u8, ParseError> {
        if self.at_end() {
            return Err(self.fatal("unexpected end of input"));
        }
        let b = self.input[self.pos];
        self.advance(1);
        Ok(b)
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), ParseError> {
        let b = self.next_byte()?;
        if b == expected {
            Ok(())
        } else {
            Err(self.fatal(format!(
                "expected '{}', found '{}'",
                expected as char, b as char
            )))
        }
    }

    fn expect_str(&mut self, expected: &[u8]) -> Result<(), ParseError> {
        for &b in expected {
            self.expect_byte(b)?;
        }
        Ok(())
    }

    fn looking_at(&self, s: &[u8]) -> bool {
        self.pos + s.len() <= self.input.len() && self.input[self.pos..].starts_with(s)
    }

    fn skip_whitespace(&mut self) -> bool {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' {
                self.advance(1);
            } else {
                break;
            }
        }
        self.pos > start
    }

    fn skip_whitespace_required(&mut self) -> Result<(), ParseError> {
        if !self.skip_whitespace() {
            return Err(self.fatal("whitespace required"));
        }
        Ok(())
    }

    fn fatal(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            location: self.location(),
            diagnostics: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Entity value helper functions (used by DTD parser and XML parser)
// ---------------------------------------------------------------------------

/// Expands only character references in a string, leaving entity references
/// as-is. Returns the expanded text.
///
/// Used to compute the replacement text of an internal entity per XML 1.0
/// §4.5: character references are expanded at declaration time, while
/// entity references are left for expansion at reference time.
pub(crate) fn expand_char_refs_only(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut result = String::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' && i + 1 < bytes.len() && bytes[i + 1] == b'#' {
            i += 2;
            let char_val = if i < bytes.len() && bytes[i] == b'x' {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let hex = std::str::from_utf8(&bytes[start..i]).unwrap_or("0");
                u32::from_str_radix(hex, 16).unwrap_or(0)
            } else {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let dec = std::str::from_utf8(&bytes[start..i]).unwrap_or("0");
                dec.parse::<u32>().unwrap_or(0)
            };
            if i < bytes.len() && bytes[i] == b';' {
                i += 1;
            }
            if let Some(ch) = char::from_u32(char_val) {
                result.push(ch);
            }
        } else {
            // Copy one complete UTF-8 character
            let ch = value[i..].chars().next().unwrap_or('\u{FFFD}');
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    result
}

/// Replaces entity references (`&name;`) with spaces, leaving character
/// references (`&#...;`) and other text unchanged. Correctly handles
/// multi-byte UTF-8 characters.
///
/// Used to sanitize entity replacement text before fragment parsing so
/// that entity references (which are valid `Reference` productions in
/// content) don't cause undeclared-entity errors.
pub(crate) fn replace_entity_refs(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' && i + 1 < bytes.len() && bytes[i + 1] != b'#' {
            // Possible entity reference: &name;
            let start = i;
            i += 1;
            if i < bytes.len()
                && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' || bytes[i] == b':')
            {
                // Scan to semicolon
                while i < bytes.len() && bytes[i] != b';' {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b';' {
                    // Complete entity reference — replace with space
                    result.push(' ');
                    i += 1;
                } else {
                    // Incomplete — keep original text
                    result.push_str(&text[start..i]);
                }
            } else {
                // Not a valid entity ref start — keep the '&'
                result.push('&');
            }
        } else {
            // Copy one complete UTF-8 character
            let ch = text[i..].chars().next().unwrap_or('\u{FFFD}');
            result.push(ch);
            i += ch.len_utf8();
        }
    }
    result
}

// ---------------------------------------------------------------------------
// XML Name character classes (shared with parser/xml.rs)
// ---------------------------------------------------------------------------

fn is_ascii_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b':'
}

fn is_ascii_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b':' || b == b'-' || b == b'.'
}

fn is_name_start_char(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z' |
        '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}' |
        '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' |
        '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}' |
        '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}' |
        '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' |
        '\u{10000}'..='\u{EFFFF}'
    )
}

fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c,
            '-' | '.' | '0'..='9' | '\u{B7}' |
            '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}'
        )
}

// ---------------------------------------------------------------------------
// DTD Validator
// ---------------------------------------------------------------------------

/// Validates a document against a DTD.
///
/// Checks that the document conforms to the element declarations, attribute
/// declarations, and other constraints in the DTD. Returns a
/// [`ValidationResult`] with any errors and warnings.
///
/// # Checks Performed
///
/// - Root element name matches DOCTYPE declaration
/// - Element content matches declared content models
/// - Required attributes are present
/// - Attribute values match their declared types (ID uniqueness, IDREF targets,
///   enumeration values)
/// - No undeclared elements (when the DTD declares elements)
/// - No undeclared attributes (when the DTD declares attributes for that element)
/// - `#FIXED` attribute values match the declared value
///
/// # Examples
///
/// ```
/// use xmloxide::Document;
/// use xmloxide::validation::dtd::{parse_dtd, validate};
///
/// let dtd = parse_dtd("<!ELEMENT root (#PCDATA)>").unwrap();
/// let mut doc = Document::parse_str("<!DOCTYPE root><root>hello</root>").unwrap();
/// let result = validate(&mut doc, &dtd);
/// assert!(result.is_valid);
/// ```
pub fn validate(doc: &mut Document, dtd: &Dtd) -> ValidationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut id_values: HashSet<String> = HashSet::new();
    let mut idref_values: Vec<String> = Vec::new();

    // Check root element name against DOCTYPE
    check_root_element(doc, dtd, &mut errors);

    // Walk all element nodes and validate
    if let Some(root_elem) = doc.root_element() {
        validate_element_recursive(
            doc,
            dtd,
            root_elem,
            &mut errors,
            &mut warnings,
            &mut id_values,
            &mut idref_values,
        );
    }

    // Check that all IDREF values point to existing IDs
    for idref in &idref_values {
        if !id_values.contains(idref) {
            errors.push(ValidationError {
                message: format!("IDREF '{idref}' does not match any ID in the document"),
                line: None,
                column: None,
            });
        }
    }

    let is_valid = errors.is_empty();
    ValidationResult {
        is_valid,
        errors,
        warnings,
    }
}

/// Checks that the root element name matches the DOCTYPE name.
fn check_root_element(doc: &Document, _dtd: &Dtd, errors: &mut Vec<ValidationError>) {
    // Find the DOCTYPE node to get the declared root name
    let doctype_name = doc.children(doc.root()).find_map(|id| {
        if let NodeKind::DocumentType { ref name, .. } = doc.node(id).kind {
            Some(name.clone())
        } else {
            None
        }
    });

    if let Some(ref expected_name) = doctype_name {
        if let Some(root_elem) = doc.root_element() {
            if let Some(actual_name) = doc.node_name(root_elem) {
                if actual_name != expected_name {
                    errors.push(ValidationError {
                        message: format!(
                            "root element '{actual_name}' does not match \
                             DOCTYPE name '{expected_name}'"
                        ),
                        line: None,
                        column: None,
                    });
                }
            }
        }
    }
}

/// Recursively validates an element and its descendants.
#[allow(clippy::too_many_arguments)]
fn validate_element_recursive(
    doc: &mut Document,
    dtd: &Dtd,
    node_id: NodeId,
    errors: &mut Vec<ValidationError>,
    warnings: &mut Vec<ValidationError>,
    id_values: &mut HashSet<String>,
    idref_values: &mut Vec<String>,
) {
    let elem_name = match doc.node_name(node_id) {
        Some(name) => name.to_string(),
        None => return,
    };

    // Check if element is declared
    let has_element_decls = !dtd.elements.is_empty();
    if has_element_decls && !dtd.elements.contains_key(&elem_name) {
        errors.push(ValidationError {
            message: format!("element '{elem_name}' is not declared in the DTD"),
            line: None,
            column: None,
        });
    }

    // Check content model
    if let Some(elem_decl) = dtd.elements.get(&elem_name) {
        validate_content_model(doc, node_id, &elem_name, &elem_decl.content_model, errors);
    }

    // Check attributes
    validate_attributes(
        doc,
        dtd,
        node_id,
        &elem_name,
        errors,
        warnings,
        id_values,
        idref_values,
    );

    // Collect child element IDs first to avoid borrow conflicts. Entity
    // references are looked through so that entity-supplied elements are
    // validated too (XML 1.0 §4.4.3: included replacement text participates
    // in validation).
    let child_ids: Vec<NodeId> = effective_content_children(doc, node_id)
        .into_iter()
        .filter(|&child_id| matches!(doc.node(child_id).kind, NodeKind::Element { .. }))
        .collect();

    // Recurse into child elements
    for child_id in child_ids {
        validate_element_recursive(
            doc,
            dtd,
            child_id,
            errors,
            warnings,
            id_values,
            idref_values,
        );
    }
}

/// Collects an element's content children in document order, transparently
/// expanding entity-reference nodes into their parsed replacement children.
///
/// Per XML 1.0 §4.4.3, replacement text included for an entity reference is
/// part of the document's content and participates in validation, so the
/// content-model checks must see through `EntityRef` nodes. An entity
/// reference without parsed children (e.g. an undeclared entity preserved in
/// tolerant mode) contributes nothing.
fn effective_content_children(doc: &Document, node_id: NodeId) -> Vec<NodeId> {
    fn collect(doc: &Document, node_id: NodeId, out: &mut Vec<NodeId>) {
        for child in doc.children(node_id) {
            if matches!(doc.node(child).kind, NodeKind::EntityRef { .. }) {
                collect(doc, child, out);
            } else {
                out.push(child);
            }
        }
    }
    let mut out = Vec::new();
    collect(doc, node_id, &mut out);
    out
}

/// Validates that an element's children match its declared content model.
fn validate_content_model(
    doc: &Document,
    node_id: NodeId,
    elem_name: &str,
    model: &ContentModel,
    errors: &mut Vec<ValidationError>,
) {
    // Entity references are looked through: their parsed replacement
    // children are part of the content being validated (XML 1.0 §4.4.3).
    let content_children = effective_content_children(doc, node_id);
    match model {
        ContentModel::Empty => {
            // No children at all
            let has_content = content_children.iter().any(|&child| {
                matches!(
                    doc.node(child).kind,
                    NodeKind::Element { .. } | NodeKind::Text { .. } | NodeKind::CData { .. }
                )
            });
            if has_content {
                errors.push(ValidationError {
                    message: format!(
                        "element '{elem_name}' is declared EMPTY \
                         but has content"
                    ),
                    line: None,
                    column: None,
                });
            }
        }
        ContentModel::Any => {
            // Anything is valid
        }
        ContentModel::Mixed(allowed_names) => {
            // Text is always allowed. Check that element children are in the allowed list.
            for &child_id in &content_children {
                if let NodeKind::Element { ref name, .. } = doc.node(child_id).kind {
                    if !allowed_names.contains(name) {
                        errors.push(ValidationError {
                            message: format!(
                                "element '{name}' is not allowed in mixed content \
                                 of '{elem_name}' (allowed: #PCDATA{})",
                                if allowed_names.is_empty() {
                                    String::new()
                                } else {
                                    format!("|{}", allowed_names.join("|"))
                                }
                            ),
                            line: None,
                            column: None,
                        });
                    }
                }
            }
        }
        ContentModel::Children(spec) => {
            // Collect element child names (ignore text, comments, PIs)
            let child_names: Vec<String> = content_children
                .iter()
                .filter_map(|&child_id| {
                    if let NodeKind::Element { ref name, .. } = doc.node(child_id).kind {
                        Some(name.clone())
                    } else {
                        None
                    }
                })
                .collect();

            // Check for text content in element-only content model
            let has_text = content_children.iter().any(|&child_id| {
                if let NodeKind::Text { ref content } = doc.node(child_id).kind {
                    !content.trim().is_empty()
                } else {
                    matches!(doc.node(child_id).kind, NodeKind::CData { .. })
                }
            });

            if has_text {
                errors.push(ValidationError {
                    message: format!(
                        "element '{elem_name}' has element-only content model \
                         but contains text"
                    ),
                    line: None,
                    column: None,
                });
            }

            // Match the sequence of child element names against the content spec
            let consumed = match_content_spec(spec, &child_names, 0);
            match consumed {
                Some(n) if n == child_names.len() => {
                    // Perfect match
                }
                _ => {
                    errors.push(ValidationError {
                        message: format!(
                            "element '{elem_name}' content does not match \
                             declared content model {model}; \
                             found children: [{}]",
                            child_names.join(", ")
                        ),
                        line: None,
                        column: None,
                    });
                }
            }
        }
    }
}

/// Matches a content spec against a slice of element names starting at `pos`.
///
/// Returns `Some(count)` if the spec matches, consuming `count` names from
/// position `pos`. Returns `None` if no match is possible.
fn match_content_spec(spec: &ContentSpec, names: &[String], pos: usize) -> Option<usize> {
    match &spec.kind {
        ContentSpecKind::Name(expected) => match_with_occurrence(
            |all_names, p| {
                if p < all_names.len() && all_names[p] == *expected {
                    Some(1)
                } else {
                    None
                }
            },
            names,
            pos,
            spec.occurrence,
        ),
        ContentSpecKind::Seq(items) => match_with_occurrence(
            |all_names, p| {
                let mut current = p;
                for item in items {
                    match match_content_spec(item, all_names, current) {
                        Some(consumed) => current += consumed,
                        None => return None,
                    }
                }
                Some(current - p)
            },
            names,
            pos,
            spec.occurrence,
        ),
        ContentSpecKind::Choice(items) => match_with_occurrence(
            |all_names, p| {
                for item in items {
                    if let Some(consumed) = match_content_spec(item, all_names, p) {
                        return Some(consumed);
                    }
                }
                None
            },
            names,
            pos,
            spec.occurrence,
        ),
    }
}

/// Applies occurrence matching around a base matcher function.
///
/// The `base_match` function attempts a single match at a given position,
/// returning `Some(consumed)` on success.
fn match_with_occurrence(
    base_match: impl Fn(&[String], usize) -> Option<usize>,
    names: &[String],
    pos: usize,
    occurrence: Occurrence,
) -> Option<usize> {
    match occurrence {
        Occurrence::Once => base_match(names, pos),
        Occurrence::Optional => {
            // Try matching once; if it fails, succeed consuming 0
            Some(base_match(names, pos).unwrap_or(0))
        }
        Occurrence::ZeroOrMore | Occurrence::OneOrMore => {
            let mut total = 0;
            loop {
                match base_match(names, pos + total) {
                    Some(0) | None => break, // zero-width or no match
                    Some(n) => total += n,
                }
            }
            // OneOrMore requires at least one match
            if occurrence == Occurrence::OneOrMore && total == 0 {
                None
            } else {
                Some(total)
            }
        }
    }
}

/// Validates attributes for an element against DTD attribute declarations.
#[allow(clippy::too_many_arguments)]
fn validate_attributes(
    doc: &mut Document,
    dtd: &Dtd,
    node_id: NodeId,
    elem_name: &str,
    errors: &mut Vec<ValidationError>,
    _warnings: &mut Vec<ValidationError>,
    id_values: &mut HashSet<String>,
    idref_values: &mut Vec<String>,
) {
    let attr_decls = dtd.attributes.get(elem_name);
    let actual_attrs = doc.attributes(node_id).to_vec();

    if let Some(decls) = attr_decls {
        // Check each declared attribute
        for decl in decls {
            let actual = actual_attrs.iter().find(|a| a.name == decl.attribute_name);

            match (&decl.default, actual) {
                (AttributeDefault::Required, None) => {
                    errors.push(ValidationError {
                        message: format!(
                            "required attribute '{}' missing on element '{elem_name}'",
                            decl.attribute_name
                        ),
                        line: None,
                        column: None,
                    });
                }
                (AttributeDefault::Fixed(fixed_val), Some(attr)) if attr.value != *fixed_val => {
                    errors.push(ValidationError {
                        message: format!(
                            "attribute '{}' on element '{elem_name}' must have \
                             fixed value '{fixed_val}', found '{}'",
                            decl.attribute_name, attr.value
                        ),
                        line: None,
                        column: None,
                    });
                }
                _ => {}
            }

            // Type checking for present attributes
            if let Some(attr) = actual {
                validate_attribute_type(
                    doc,
                    node_id,
                    &attr.value,
                    &decl.attribute_type,
                    &decl.attribute_name,
                    elem_name,
                    errors,
                    id_values,
                    idref_values,
                );
            }
        }

        // Check for undeclared attributes (skip xmlns-related attributes)
        for attr in &actual_attrs {
            if attr.name == "xmlns" || attr.prefix.as_deref() == Some("xmlns") {
                continue;
            }
            let is_declared = decls.iter().any(|d| d.attribute_name == attr.name);
            if !is_declared {
                errors.push(ValidationError {
                    message: format!(
                        "attribute '{}' on element '{elem_name}' is not declared in the DTD",
                        attr.name
                    ),
                    line: None,
                    column: None,
                });
            }
        }
    }
}

/// Validates an attribute value against its declared type.
#[allow(clippy::too_many_arguments)]
fn validate_attribute_type(
    doc: &mut Document,
    node_id: NodeId,
    value: &str,
    attr_type: &AttributeType,
    attr_name: &str,
    elem_name: &str,
    errors: &mut Vec<ValidationError>,
    id_values: &mut HashSet<String>,
    idref_values: &mut Vec<String>,
) {
    match attr_type {
        AttributeType::CData => {
            // Any string is valid CDATA
        }
        AttributeType::Id => {
            validate_id_value(doc, node_id, value, attr_name, elem_name, errors, id_values);
        }
        AttributeType::IdRef => {
            validate_idref_value(value, attr_name, elem_name, errors, idref_values);
        }
        AttributeType::IdRefs => {
            validate_idrefs_value(value, attr_name, elem_name, errors, idref_values);
        }
        AttributeType::NmToken => {
            validate_nmtoken_value(value, attr_name, elem_name, errors);
        }
        AttributeType::NmTokens => {
            validate_nmtokens_value(value, attr_name, elem_name, errors);
        }
        AttributeType::Enumeration(values) | AttributeType::Notation(values) => {
            validate_enumeration_value(value, values, attr_name, elem_name, errors);
        }
        AttributeType::Entity | AttributeType::Entities => {
            validate_entity_value(value, attr_type, attr_name, elem_name, errors);
        }
    }
}

/// Validates an ID attribute value: must be a valid XML Name and unique.
///
/// On success, registers the ID in the document's `id_map` so it can be
/// looked up via [`Document::element_by_id`] and the `XPath` `id()` function.
fn validate_id_value(
    doc: &mut Document,
    node_id: NodeId,
    value: &str,
    attr_name: &str,
    elem_name: &str,
    errors: &mut Vec<ValidationError>,
    id_values: &mut HashSet<String>,
) {
    if !is_valid_name(value) {
        errors.push(ValidationError {
            message: format!(
                "attribute '{attr_name}' on element '{elem_name}' \
                 has invalid ID value '{value}' (not a valid XML Name)"
            ),
            line: None,
            column: None,
        });
    } else if !id_values.insert(value.to_string()) {
        errors.push(ValidationError {
            message: format!(
                "duplicate ID value '{value}' on attribute '{attr_name}' \
                 of element '{elem_name}'"
            ),
            line: None,
            column: None,
        });
    } else {
        doc.set_id(value, node_id);
    }
}

/// Validates an IDREF attribute value.
fn validate_idref_value(
    value: &str,
    attr_name: &str,
    elem_name: &str,
    errors: &mut Vec<ValidationError>,
    idref_values: &mut Vec<String>,
) {
    if is_valid_name(value) {
        idref_values.push(value.to_string());
    } else {
        errors.push(ValidationError {
            message: format!(
                "attribute '{attr_name}' on element '{elem_name}' \
                 has invalid IDREF value '{value}'"
            ),
            line: None,
            column: None,
        });
    }
}

/// Validates an IDREFS attribute value (space-separated list).
fn validate_idrefs_value(
    value: &str,
    attr_name: &str,
    elem_name: &str,
    errors: &mut Vec<ValidationError>,
    idref_values: &mut Vec<String>,
) {
    for token in value.split_whitespace() {
        if is_valid_name(token) {
            idref_values.push(token.to_string());
        } else {
            errors.push(ValidationError {
                message: format!(
                    "attribute '{attr_name}' on element '{elem_name}' \
                     has invalid IDREFS token '{token}'"
                ),
                line: None,
                column: None,
            });
        }
    }
}

/// Validates a NMTOKEN attribute value.
fn validate_nmtoken_value(
    value: &str,
    attr_name: &str,
    elem_name: &str,
    errors: &mut Vec<ValidationError>,
) {
    if !is_valid_nmtoken(value) {
        errors.push(ValidationError {
            message: format!(
                "attribute '{attr_name}' on element '{elem_name}' \
                 has invalid NMTOKEN value '{value}'"
            ),
            line: None,
            column: None,
        });
    }
}

/// Validates a NMTOKENS attribute value (space-separated list).
fn validate_nmtokens_value(
    value: &str,
    attr_name: &str,
    elem_name: &str,
    errors: &mut Vec<ValidationError>,
) {
    for token in value.split_whitespace() {
        if !is_valid_nmtoken(token) {
            errors.push(ValidationError {
                message: format!(
                    "attribute '{attr_name}' on element '{elem_name}' \
                     has invalid NMTOKENS token '{token}'"
                ),
                line: None,
                column: None,
            });
        }
    }
}

/// Validates an enumeration or notation attribute value.
fn validate_enumeration_value(
    value: &str,
    allowed: &[String],
    attr_name: &str,
    elem_name: &str,
    errors: &mut Vec<ValidationError>,
) {
    if !allowed.contains(&value.to_string()) {
        errors.push(ValidationError {
            message: format!(
                "attribute '{attr_name}' on element '{elem_name}' \
                 has value '{value}' which is not in the allowed \
                 values ({})",
                allowed.join("|")
            ),
            line: None,
            column: None,
        });
    }
}

/// Validates an ENTITY or ENTITIES attribute value.
fn validate_entity_value(
    value: &str,
    attr_type: &AttributeType,
    attr_name: &str,
    elem_name: &str,
    errors: &mut Vec<ValidationError>,
) {
    // Entity/Entities validation would require checking against
    // declared unparsed entities. For now we just check Name validity.
    let tokens: Vec<&str> = if matches!(attr_type, AttributeType::Entities) {
        value.split_whitespace().collect()
    } else {
        vec![value]
    };
    for token in tokens {
        if !is_valid_name(token) {
            errors.push(ValidationError {
                message: format!(
                    "attribute '{attr_name}' on element '{elem_name}' \
                     has invalid ENTITY/ENTITIES value '{token}'"
                ),
                line: None,
                column: None,
            });
        }
    }
}

/// Checks if a string is a valid XML Name.
fn is_valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) if is_name_start_char(first) => chars.all(is_name_char),
        _ => false,
    }
}

/// Checks if a string is a valid NMTOKEN.
fn is_valid_nmtoken(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_name_char)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // --- DTD Parsing Tests ---

    #[test]
    fn test_parse_element_empty() {
        let dtd = parse_dtd("<!ELEMENT br EMPTY>").unwrap();
        let decl = dtd.elements.get("br").unwrap();
        assert_eq!(decl.content_model, ContentModel::Empty);
    }

    #[test]
    fn test_parse_element_any() {
        let dtd = parse_dtd("<!ELEMENT container ANY>").unwrap();
        let decl = dtd.elements.get("container").unwrap();
        assert_eq!(decl.content_model, ContentModel::Any);
    }

    #[test]
    fn test_parse_element_pcdata() {
        let dtd = parse_dtd("<!ELEMENT title (#PCDATA)>").unwrap();
        let decl = dtd.elements.get("title").unwrap();
        assert_eq!(decl.content_model, ContentModel::Mixed(vec![]));
    }

    #[test]
    fn test_parse_element_mixed_content() {
        let dtd = parse_dtd("<!ELEMENT p (#PCDATA|em|strong)*>").unwrap();
        let decl = dtd.elements.get("p").unwrap();
        assert_eq!(
            decl.content_model,
            ContentModel::Mixed(vec!["em".to_string(), "strong".to_string()])
        );
    }

    #[test]
    fn test_parse_element_sequence() {
        let dtd = parse_dtd("<!ELEMENT book (title,author,year)>").unwrap();
        let decl = dtd.elements.get("book").unwrap();
        match &decl.content_model {
            ContentModel::Children(spec) => {
                assert_eq!(spec.occurrence, Occurrence::Once);
                match &spec.kind {
                    ContentSpecKind::Seq(items) => {
                        assert_eq!(items.len(), 3);
                        assert_eq!(items[0].kind, ContentSpecKind::Name("title".to_string()));
                        assert_eq!(items[1].kind, ContentSpecKind::Name("author".to_string()));
                        assert_eq!(items[2].kind, ContentSpecKind::Name("year".to_string()));
                    }
                    other => panic!("expected Seq, got {other:?}"),
                }
            }
            other => panic!("expected Children, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_element_choice() {
        let dtd = parse_dtd("<!ELEMENT item (a|b|c)>").unwrap();
        let decl = dtd.elements.get("item").unwrap();
        match &decl.content_model {
            ContentModel::Children(spec) => match &spec.kind {
                ContentSpecKind::Choice(items) => {
                    assert_eq!(items.len(), 3);
                }
                other => panic!("expected Choice, got {other:?}"),
            },
            other => panic!("expected Children, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_element_occurrence_indicators() {
        let dtd = parse_dtd("<!ELEMENT doc (head, body?, appendix*)>").unwrap();
        let decl = dtd.elements.get("doc").unwrap();
        match &decl.content_model {
            ContentModel::Children(spec) => match &spec.kind {
                ContentSpecKind::Seq(items) => {
                    assert_eq!(items[0].occurrence, Occurrence::Once);
                    assert_eq!(items[1].occurrence, Occurrence::Optional);
                    assert_eq!(items[2].occurrence, Occurrence::ZeroOrMore);
                }
                other => panic!("expected Seq, got {other:?}"),
            },
            other => panic!("expected Children, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_element_nested_groups() {
        let dtd = parse_dtd("<!ELEMENT article ((title, author), body)>").unwrap();
        let decl = dtd.elements.get("article").unwrap();
        match &decl.content_model {
            ContentModel::Children(spec) => match &spec.kind {
                ContentSpecKind::Seq(items) => {
                    assert_eq!(items.len(), 2);
                    // First item is a nested sequence (title, author)
                    match &items[0].kind {
                        ContentSpecKind::Seq(inner) => {
                            assert_eq!(inner.len(), 2);
                        }
                        other => panic!("expected nested Seq, got {other:?}"),
                    }
                }
                other => panic!("expected Seq, got {other:?}"),
            },
            other => panic!("expected Children, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_attlist_cdata() {
        let dtd = parse_dtd("<!ATTLIST img src CDATA #REQUIRED>").unwrap();
        let decls = dtd.attributes.get("img").unwrap();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].attribute_name, "src");
        assert_eq!(decls[0].attribute_type, AttributeType::CData);
        assert_eq!(decls[0].default, AttributeDefault::Required);
    }

    #[test]
    fn test_parse_attlist_id() {
        let dtd = parse_dtd("<!ATTLIST div id ID #IMPLIED>").unwrap();
        let decls = dtd.attributes.get("div").unwrap();
        assert_eq!(decls[0].attribute_type, AttributeType::Id);
        assert_eq!(decls[0].default, AttributeDefault::Implied);
    }

    #[test]
    fn test_parse_attlist_enumeration() {
        let dtd = parse_dtd("<!ATTLIST input type (text|password|submit) \"text\">").unwrap();
        let decls = dtd.attributes.get("input").unwrap();
        assert_eq!(
            decls[0].attribute_type,
            AttributeType::Enumeration(vec![
                "text".to_string(),
                "password".to_string(),
                "submit".to_string()
            ])
        );
        assert_eq!(
            decls[0].default,
            AttributeDefault::Default("text".to_string())
        );
    }

    #[test]
    fn test_parse_attlist_fixed() {
        let dtd = parse_dtd("<!ATTLIST doc version CDATA #FIXED \"1.0\">").unwrap();
        let decls = dtd.attributes.get("doc").unwrap();
        assert_eq!(decls[0].default, AttributeDefault::Fixed("1.0".to_string()));
    }

    #[test]
    fn test_parse_attlist_multiple_attrs() {
        let dtd =
            parse_dtd("<!ATTLIST person\n  name CDATA #REQUIRED\n  age NMTOKEN #IMPLIED>").unwrap();
        let decls = dtd.attributes.get("person").unwrap();
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].attribute_name, "name");
        assert_eq!(decls[1].attribute_name, "age");
        assert_eq!(decls[1].attribute_type, AttributeType::NmToken);
    }

    #[test]
    fn test_parse_entity_internal() {
        let dtd = parse_dtd("<!ENTITY copy \"&#169;\">").unwrap();
        let ent = dtd.entities.get("copy").unwrap();
        match &ent.kind {
            EntityKind::Internal(value) => assert_eq!(value, "&#169;"),
            EntityKind::External { .. } => panic!("expected Internal, got External"),
        }
    }

    #[test]
    fn test_parse_entity_external() {
        let dtd = parse_dtd("<!ENTITY chapter SYSTEM \"chapter.xml\">").unwrap();
        let ent = dtd.entities.get("chapter").unwrap();
        match &ent.kind {
            EntityKind::External {
                system_id,
                public_id,
            } => {
                assert_eq!(system_id, "chapter.xml");
                assert_eq!(*public_id, None);
            }
            EntityKind::Internal(val) => panic!("expected External, got Internal({val})"),
        }
    }

    #[test]
    fn test_parse_notation() {
        let dtd = parse_dtd("<!NOTATION png SYSTEM \"image/png\">").unwrap();
        let notation = dtd.notations.get("png").unwrap();
        assert_eq!(notation.system_id.as_deref(), Some("image/png"));
    }

    #[test]
    fn test_parse_dtd_with_comments() {
        let dtd = parse_dtd(
            "<!-- element declarations -->\n\
             <!ELEMENT root (#PCDATA)>\n\
             <!-- end -->",
        )
        .unwrap();
        assert!(dtd.elements.contains_key("root"));
    }

    #[test]
    fn test_parse_dtd_complex() {
        let input = "\
            <!ELEMENT doc (head, body)>\n\
            <!ELEMENT head (title)>\n\
            <!ELEMENT title (#PCDATA)>\n\
            <!ELEMENT body (p+)>\n\
            <!ELEMENT p (#PCDATA|em)*>\n\
            <!ELEMENT em (#PCDATA)>\n\
            <!ATTLIST doc version CDATA #FIXED \"1.0\">\n\
            <!ATTLIST p id ID #IMPLIED>\n\
            <!ENTITY copyright \"Copyright 2024\">\n";
        let dtd = parse_dtd(input).unwrap();
        assert_eq!(dtd.elements.len(), 6);
        assert!(dtd.attributes.contains_key("doc"));
        assert!(dtd.attributes.contains_key("p"));
        assert!(dtd.entities.contains_key("copyright"));
    }

    // --- Validation Tests ---

    fn make_doc(xml: &str) -> Document {
        Document::parse_str(xml).unwrap()
    }

    #[test]
    fn test_validate_valid_document() {
        let dtd = parse_dtd("<!ELEMENT root (#PCDATA)>").unwrap();
        let mut doc = make_doc("<!DOCTYPE root><root>hello</root>");
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_root_name_mismatch() {
        let dtd = parse_dtd("<!ELEMENT root (#PCDATA)>").unwrap();
        let mut doc = make_doc("<!DOCTYPE root><other>text</other>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("root element 'other'")
                    && e.message.contains("does not match DOCTYPE name 'root'")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_empty_element() {
        let dtd = parse_dtd("<!ELEMENT br EMPTY>").unwrap();
        let mut doc = make_doc("<!DOCTYPE br><br/>");
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_empty_element_has_content() {
        let dtd = parse_dtd("<!ELEMENT br EMPTY>").unwrap();
        let mut doc = make_doc("<!DOCTYPE br><br>text</br>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("EMPTY") && e.message.contains("has content")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_any_content() {
        let dtd = parse_dtd(
            "<!ELEMENT container ANY>\n\
             <!ELEMENT child (#PCDATA)>",
        )
        .unwrap();
        let mut doc = make_doc("<!DOCTYPE container><container><child>text</child></container>");
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_sequence_correct() {
        let dtd = parse_dtd(
            "<!ELEMENT book (title,author)>\n\
             <!ELEMENT title (#PCDATA)>\n\
             <!ELEMENT author (#PCDATA)>",
        )
        .unwrap();
        let mut doc = make_doc(
            "<!DOCTYPE book>\
             <book><title>XML</title><author>Jon</author></book>",
        );
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_sequence_wrong_order() {
        let dtd = parse_dtd(
            "<!ELEMENT book (title,author)>\n\
             <!ELEMENT title (#PCDATA)>\n\
             <!ELEMENT author (#PCDATA)>",
        )
        .unwrap();
        let mut doc = make_doc(
            "<!DOCTYPE book>\
             <book><author>Jon</author><title>XML</title></book>",
        );
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("content does not match")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_required_attribute_missing() {
        let dtd = parse_dtd(
            "<!ELEMENT img EMPTY>\n\
             <!ATTLIST img src CDATA #REQUIRED>",
        )
        .unwrap();
        let mut doc = make_doc("<!DOCTYPE img><img/>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("required attribute 'src'")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_required_attribute_present() {
        let dtd = parse_dtd(
            "<!ELEMENT img EMPTY>\n\
             <!ATTLIST img src CDATA #REQUIRED>",
        )
        .unwrap();
        let mut doc = make_doc("<!DOCTYPE img><img src=\"photo.jpg\"/>");
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_fixed_attribute_correct() {
        let dtd = parse_dtd(
            "<!ELEMENT doc (#PCDATA)>\n\
             <!ATTLIST doc version CDATA #FIXED \"1.0\">",
        )
        .unwrap();
        let mut doc = make_doc("<!DOCTYPE doc><doc version=\"1.0\">text</doc>");
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_fixed_attribute_wrong_value() {
        let dtd = parse_dtd(
            "<!ELEMENT doc (#PCDATA)>\n\
             <!ATTLIST doc version CDATA #FIXED \"1.0\">",
        )
        .unwrap();
        let mut doc = make_doc("<!DOCTYPE doc><doc version=\"2.0\">text</doc>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("fixed value '1.0'")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_enumeration_valid() {
        let dtd = parse_dtd(
            "<!ELEMENT input EMPTY>\n\
             <!ATTLIST input type (text|password) #REQUIRED>",
        )
        .unwrap();
        let mut doc = make_doc("<!DOCTYPE input><input type=\"text\"/>");
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_enumeration_invalid() {
        let dtd = parse_dtd(
            "<!ELEMENT input EMPTY>\n\
             <!ATTLIST input type (text|password) #REQUIRED>",
        )
        .unwrap();
        let mut doc = make_doc("<!DOCTYPE input><input type=\"checkbox\"/>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("not in the allowed values")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_duplicate_id() {
        let dtd = parse_dtd(
            "<!ELEMENT root (item, item)>\n\
             <!ELEMENT item (#PCDATA)>\n\
             <!ATTLIST item id ID #REQUIRED>",
        )
        .unwrap();
        let mut doc = make_doc(
            "<!DOCTYPE root>\
             <root>\
               <item id=\"a\">first</item>\
               <item id=\"a\">second</item>\
             </root>",
        );
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("duplicate ID value 'a'")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_idref_valid() {
        let dtd = parse_dtd(
            "<!ELEMENT root (item, ref)>\n\
             <!ELEMENT item (#PCDATA)>\n\
             <!ELEMENT ref (#PCDATA)>\n\
             <!ATTLIST item id ID #REQUIRED>\n\
             <!ATTLIST ref target IDREF #REQUIRED>",
        )
        .unwrap();
        let mut doc = make_doc(
            "<!DOCTYPE root>\
             <root>\
               <item id=\"x\">item</item>\
               <ref target=\"x\">ref</ref>\
             </root>",
        );
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_idref_dangling() {
        let dtd = parse_dtd(
            "<!ELEMENT root (ref)>\n\
             <!ELEMENT ref (#PCDATA)>\n\
             <!ATTLIST ref target IDREF #REQUIRED>",
        )
        .unwrap();
        let mut doc = make_doc(
            "<!DOCTYPE root>\
             <root><ref target=\"nonexistent\">ref</ref></root>",
        );
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result.errors.iter().any(|e| e
                .message
                .contains("IDREF 'nonexistent' does not match any ID")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_undeclared_element() {
        let dtd = parse_dtd("<!ELEMENT root (child)>\n<!ELEMENT child (#PCDATA)>").unwrap();
        let mut doc = make_doc("<!DOCTYPE root><root><unknown/></root>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("element 'unknown' is not declared")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_undeclared_attribute() {
        let dtd = parse_dtd(
            "<!ELEMENT root (#PCDATA)>\n\
             <!ATTLIST root id ID #IMPLIED>",
        )
        .unwrap();
        let mut doc = make_doc("<!DOCTYPE root><root id=\"x\" bogus=\"y\">text</root>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("attribute 'bogus'")
                    && e.message.contains("not declared")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_mixed_content_valid() {
        let dtd = parse_dtd(
            "<!ELEMENT p (#PCDATA|em|strong)*>\n\
             <!ELEMENT em (#PCDATA)>\n\
             <!ELEMENT strong (#PCDATA)>",
        )
        .unwrap();
        let mut doc = make_doc(
            "<!DOCTYPE p>\
             <p>Hello <em>world</em> and <strong>friends</strong></p>",
        );
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_mixed_content_invalid_child() {
        let dtd = parse_dtd(
            "<!ELEMENT p (#PCDATA|em)*>\n\
             <!ELEMENT em (#PCDATA)>\n\
             <!ELEMENT b (#PCDATA)>",
        )
        .unwrap();
        let mut doc = make_doc(
            "<!DOCTYPE p>\
             <p>Hello <b>world</b></p>",
        );
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("'b' is not allowed in mixed content")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_choice_correct() {
        let dtd = parse_dtd(
            "<!ELEMENT item (a|b)>\n\
             <!ELEMENT a (#PCDATA)>\n\
             <!ELEMENT b (#PCDATA)>",
        )
        .unwrap();
        let mut doc = make_doc("<!DOCTYPE item><item><b>hello</b></item>");
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_one_or_more() {
        let dtd = parse_dtd("<!ELEMENT list (item+)>\n<!ELEMENT item (#PCDATA)>").unwrap();

        // Valid: one item
        let mut doc = make_doc("<!DOCTYPE list><list><item>a</item></list>");
        assert!(validate(&mut doc, &dtd).is_valid);

        // Valid: multiple items
        let mut doc = make_doc("<!DOCTYPE list><list><item>a</item><item>b</item></list>");
        assert!(validate(&mut doc, &dtd).is_valid);

        // Invalid: zero items
        let mut doc = make_doc("<!DOCTYPE list><list></list>");
        assert!(!validate(&mut doc, &dtd).is_valid);
    }

    #[test]
    fn test_validate_zero_or_more() {
        let dtd = parse_dtd("<!ELEMENT list (item*)>\n<!ELEMENT item (#PCDATA)>").unwrap();

        // Valid: zero items
        let mut doc = make_doc("<!DOCTYPE list><list></list>");
        assert!(validate(&mut doc, &dtd).is_valid);

        // Valid: multiple items
        let mut doc = make_doc("<!DOCTYPE list><list><item>a</item><item>b</item></list>");
        assert!(validate(&mut doc, &dtd).is_valid);
    }

    #[test]
    fn test_validate_optional_element() {
        let dtd = parse_dtd(
            "<!ELEMENT doc (title, subtitle?)>\n\
             <!ELEMENT title (#PCDATA)>\n\
             <!ELEMENT subtitle (#PCDATA)>",
        )
        .unwrap();

        // Valid: with optional
        let mut doc = make_doc(
            "<!DOCTYPE doc>\
             <doc><title>T</title><subtitle>S</subtitle></doc>",
        );
        assert!(validate(&mut doc, &dtd).is_valid);

        // Valid: without optional
        let mut doc = make_doc("<!DOCTYPE doc><doc><title>T</title></doc>");
        assert!(validate(&mut doc, &dtd).is_valid);
    }

    #[test]
    fn test_content_model_display() {
        assert_eq!(ContentModel::Empty.to_string(), "EMPTY");
        assert_eq!(ContentModel::Any.to_string(), "ANY");
        assert_eq!(ContentModel::Mixed(vec![]).to_string(), "(#PCDATA)");
        assert_eq!(
            ContentModel::Mixed(vec!["a".to_string(), "b".to_string()]).to_string(),
            "(#PCDATA|a|b)*"
        );

        let spec = ContentSpec {
            kind: ContentSpecKind::Seq(vec![
                ContentSpec {
                    kind: ContentSpecKind::Name("a".to_string()),
                    occurrence: Occurrence::Once,
                },
                ContentSpec {
                    kind: ContentSpecKind::Name("b".to_string()),
                    occurrence: Occurrence::ZeroOrMore,
                },
            ]),
            occurrence: Occurrence::Once,
        };
        assert_eq!(ContentModel::Children(spec).to_string(), "(a , b*)");
    }

    #[test]
    fn test_parse_attlist_idref_idrefs() {
        let dtd = parse_dtd(
            "<!ATTLIST link target IDREF #REQUIRED>\n\
             <!ATTLIST group members IDREFS #REQUIRED>",
        )
        .unwrap();
        let link_decls = dtd.attributes.get("link").unwrap();
        assert_eq!(link_decls[0].attribute_type, AttributeType::IdRef);
        let group_decls = dtd.attributes.get("group").unwrap();
        assert_eq!(group_decls[0].attribute_type, AttributeType::IdRefs);
    }

    #[test]
    fn test_validate_element_content_with_text() {
        let dtd = parse_dtd("<!ELEMENT book (title)>\n<!ELEMENT title (#PCDATA)>").unwrap();
        let mut doc = make_doc("<!DOCTYPE book><book>stray text<title>T</title></book>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("element-only content model")
                    && e.message.contains("contains text")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_parse_entity_public() {
        let dtd = parse_dtd("<!ENTITY logo PUBLIC \"-//LOGO//\" \"logo.png\">").unwrap();
        let ent = dtd.entities.get("logo").unwrap();
        match &ent.kind {
            EntityKind::External {
                system_id,
                public_id,
            } => {
                assert_eq!(system_id, "logo.png");
                assert_eq!(public_id.as_deref(), Some("-//LOGO//"));
            }
            EntityKind::Internal(val) => panic!("expected External, got Internal({val})"),
        }
    }

    #[test]
    fn test_parse_notation_public() {
        let dtd = parse_dtd("<!NOTATION gif PUBLIC \"-//GIF//\">").unwrap();
        let notation = dtd.notations.get("gif").unwrap();
        assert_eq!(notation.public_id.as_deref(), Some("-//GIF//"));
        assert_eq!(notation.system_id, None);
    }

    #[test]
    fn test_parse_parameter_entity_skipped() {
        // Parameter entities should be skipped without error
        let dtd = parse_dtd(
            "<!ENTITY % common \"(#PCDATA)\">\n\
             <!ELEMENT root (#PCDATA)>",
        )
        .unwrap();
        assert!(dtd.elements.contains_key("root"));
    }

    #[test]
    fn test_validate_nmtoken_attribute() {
        let dtd = parse_dtd(
            "<!ELEMENT root (#PCDATA)>\n\
             <!ATTLIST root token NMTOKEN #REQUIRED>",
        )
        .unwrap();

        // Valid NMTOKEN
        let mut doc = make_doc("<!DOCTYPE root><root token=\"abc-123\">text</root>");
        assert!(validate(&mut doc, &dtd).is_valid);

        // Invalid NMTOKEN (spaces not allowed)
        let mut doc = make_doc("<!DOCTYPE root><root token=\"abc 123\">text</root>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("invalid NMTOKEN")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_populates_id_map() {
        let dtd = parse_dtd(
            "<!ELEMENT root (item*)>\n\
             <!ELEMENT item (#PCDATA)>\n\
             <!ATTLIST item id ID #REQUIRED>",
        )
        .unwrap();
        let mut doc =
            make_doc(r#"<!DOCTYPE root><root><item id="a">A</item><item id="b">B</item></root>"#);
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);

        // The id_map should have been populated
        let item_a = doc.element_by_id("a");
        assert!(item_a.is_some(), "expected to find element with id='a'");
        let item_b = doc.element_by_id("b");
        assert!(item_b.is_some(), "expected to find element with id='b'");
        assert_eq!(doc.element_by_id("c"), None);

        // Verify the nodes are the correct elements
        assert_eq!(doc.node_name(item_a.unwrap()), Some("item"));
        assert_eq!(doc.node_name(item_b.unwrap()), Some("item"));
    }

    #[test]
    fn test_content_model_deep_nesting_is_bounded() {
        // Regression test for CVE-2026-61727 (GHSA-7jmw-29gc-ffx4): a content
        // model with deeply nested '(' must return a normal error instead of
        // recursing until the stack overflows.
        let depth = (MAX_CONTENT_MODEL_DEPTH as usize) + 50;
        let mut src = String::from("<!ELEMENT r ");
        src.push_str(&"(".repeat(depth));
        src.push('a');
        src.push_str(&")".repeat(depth));
        src.push('>');

        let result = parse_dtd(&src);
        let Err(err) = result else {
            panic!("expected deep content model to be rejected, got Ok");
        };
        assert!(
            err.message.contains("maximum depth"),
            "unexpected error message: {}",
            err.message
        );
    }

    #[test]
    fn test_content_model_shallow_nesting_still_parses() {
        // The depth bound must not affect ordinary, shallowly-nested models.
        let dtd = parse_dtd("<!ELEMENT a (b,(c|d),e)>").unwrap();
        assert!(dtd.elements.contains_key("a"));
    }

    // --- Entity recursion checks (WFC: No Recursion, XML 1.0 §4.1) ---

    #[test]
    fn test_entity_recursion_direct_cycle_rejected() {
        let err = parse_dtd("<!ENTITY x \"&x;\">").unwrap_err();
        assert!(
            err.message.contains("recursive entity reference"),
            "unexpected error message: {}",
            err.message
        );
    }

    #[test]
    fn test_entity_recursion_indirect_cycle_rejected() {
        let err =
            parse_dtd("<!ENTITY a \"&b;\"><!ENTITY b \"&c;\"><!ENTITY c \"&a;\">").unwrap_err();
        assert!(
            err.message.contains("recursive entity reference"),
            "unexpected error message: {}",
            err.message
        );
    }

    #[test]
    fn test_entity_recursion_cycle_behind_safe_entity_rejected() {
        // Regression test for the memoized walk (issue #42): `ok` is acyclic
        // and shared by both cycle members. Marking `ok` as proven-safe must
        // not mask the a<->b cycle behind it.
        let err = parse_dtd(
            "<!ENTITY ok \"text\">\
             <!ENTITY a \"&ok;&b;\">\
             <!ENTITY b \"&ok;&a;\">",
        )
        .unwrap_err();
        assert!(
            err.message.contains("recursive entity reference"),
            "unexpected error message: {}",
            err.message
        );
    }

    #[test]
    fn test_entity_recursion_acyclic_diamond_accepted() {
        // d is reachable through two paths (a->b->d and a->c->d) but there
        // is no cycle; the memoized walk must accept this.
        let dtd = parse_dtd(
            "<!ENTITY d \"leaf\">\
             <!ENTITY b \"&d;&d;\">\
             <!ENTITY c \"&d;&d;\">\
             <!ENTITY a \"&b;&c;\">",
        )
        .unwrap();
        assert_eq!(dtd.entities.len(), 4);
    }

    #[test]
    fn test_entity_recursion_deep_chain_is_linear() {
        // Regression test for issue #42: each level references the previous
        // entity 10 times. Without memoization the recursion check walks
        // 10^30 paths and never finishes; with it, this parses instantly.
        use std::fmt::Write as _;
        let mut src = String::from("<!ENTITY a \"AAAA\">");
        for i in 1..=30u32 {
            let prev = if i == 1 {
                "a".to_string()
            } else {
                format!("e{}", i - 1)
            };
            let refs = format!("&{prev};").repeat(10);
            let _ = write!(src, "<!ENTITY e{i} \"{refs}\">");
        }
        let dtd = parse_dtd(&src).unwrap();
        assert_eq!(dtd.entities.len(), 31);
    }

    #[test]
    fn test_pe_recursion_direct_cycle_rejected() {
        // %p; encoded as &#37;p; inside the value (XML 1.0 §4.1).
        let err = parse_dtd("<!ENTITY % p \"&#37;p;\">").unwrap_err();
        assert!(
            err.message.contains("recursive parameter entity reference"),
            "unexpected error message: {}",
            err.message
        );
    }

    #[test]
    fn test_pe_recursion_indirect_cycle_rejected() {
        let err = parse_dtd("<!ENTITY % p \"&#37;q;\"><!ENTITY % q \"&#37;p;\">").unwrap_err();
        assert!(
            err.message.contains("recursive parameter entity reference"),
            "unexpected error message: {}",
            err.message
        );
    }

    // --- Content-model validation through entity references (§4.4.3) ---

    #[test]
    fn test_validate_entity_supplied_child_matches_model() {
        // The required <x> child arrives via an entity reference; the
        // content model must see through the EntityRef node.
        let dtd = parse_dtd("<!ELEMENT d (x)><!ELEMENT x (#PCDATA)>").unwrap();
        let mut doc = make_doc("<!DOCTYPE d [<!ENTITY e \"<x>hi</x>\">]><d>&e;</d>");
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }

    #[test]
    fn test_validate_entity_supplied_child_violates_model() {
        // An entity-supplied element that violates the content model must
        // still be reported.
        let dtd =
            parse_dtd("<!ELEMENT d (x)><!ELEMENT x (#PCDATA)><!ELEMENT y (#PCDATA)>").unwrap();
        let mut doc = make_doc("<!DOCTYPE d [<!ENTITY e \"<y>hi</y>\">]><d>&e;</d>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid, "expected content-model violation");
    }

    #[test]
    fn test_validate_entity_supplied_element_content_checked() {
        // Elements inside entity expansions are themselves validated: <x>
        // is declared EMPTY but the entity gives it text content.
        let dtd = parse_dtd("<!ELEMENT d (x)><!ELEMENT x EMPTY>").unwrap();
        let mut doc = make_doc("<!DOCTYPE d [<!ENTITY e \"<x>hi</x>\">]><d>&e;</d>");
        let result = validate(&mut doc, &dtd);
        assert!(!result.is_valid, "expected EMPTY-content violation");
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("EMPTY") && e.message.contains("has content")),
            "errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_validate_mixed_content_through_entity() {
        let dtd = parse_dtd("<!ELEMENT p (#PCDATA|em)*><!ELEMENT em (#PCDATA)>").unwrap();
        let mut doc =
            make_doc("<!DOCTYPE p [<!ENTITY e \"text <em>emph</em>\">]><p>before &e; after</p>");
        let result = validate(&mut doc, &dtd);
        assert!(result.is_valid, "errors: {:?}", result.errors);
    }
}
