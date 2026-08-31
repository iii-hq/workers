//! XML 1.0 parser.
//!
//! A hand-rolled recursive descent parser conforming to the W3C XML 1.0
//! (Fifth Edition) specification. The parser builds a `Document` tree and
//! supports error recovery mode for processing malformed input.
//!
//! The parser is hand-rolled (not combinator-based) because:
//! 1. libxml2's parser is recursive descent and we need identical behavior
//! 2. Error recovery requires fine-grained control over parse state
//! 3. Push/incremental parsing requires suspendable state
//! 4. Performance — no abstraction overhead

pub(crate) mod input;
pub mod push;
mod xml;

pub use push::PushParser;

use std::sync::Arc;

use crate::error::ParseError;
use crate::tree::Document;

use input::{
    DEFAULT_MAX_ATTRIBUTES, DEFAULT_MAX_ATTRIBUTE_LENGTH, DEFAULT_MAX_DEPTH,
    DEFAULT_MAX_ENTITY_EXPANSIONS, DEFAULT_MAX_NAME_LENGTH, DEFAULT_MAX_TEXT_LENGTH,
};

/// A request to resolve an external entity.
///
/// Passed to the [`EntityResolver`] callback when the parser encounters
/// a reference to an externally-declared entity (SYSTEM or PUBLIC).
#[derive(Debug)]
pub struct ExternalEntityRequest<'a> {
    /// The entity name as declared in the DTD.
    pub name: &'a str,
    /// The SYSTEM identifier (URI) from the entity declaration.
    pub system_id: &'a str,
    /// The PUBLIC identifier from the entity declaration, if any.
    pub public_id: Option<&'a str>,
}

/// A callback for resolving external entities.
///
/// Returns `Some(replacement_text)` to expand the entity, or `None` to
/// reject the reference (which will produce a parse error or, in recovery
/// mode, an empty expansion).
///
/// # Security
///
/// **Warning:** Enabling external entity resolution opens the door to
/// XML External Entity (XXE) attacks. Only use this with trusted input,
/// and consider restricting which URIs the resolver is willing to fetch.
pub type EntityResolver = Arc<dyn Fn(ExternalEntityRequest<'_>) -> Option<String> + Send + Sync>;

/// Parse options controlling parser behavior and security limits.
///
/// Use the builder pattern to configure options:
///
/// ```
/// use xmloxide::parser::ParseOptions;
///
/// let opts = ParseOptions::default()
///     .recover(true)
///     .no_blanks(true)
///     .max_depth(128);
/// ```
pub struct ParseOptions {
    /// If true, attempt to recover from errors and produce a partial tree.
    pub recover: bool,
    /// If true, strip ignorable whitespace-only text nodes.
    pub no_blanks: bool,

    // -- Security limits --
    /// Maximum element nesting depth (default: 256).
    pub max_depth: u32,
    /// Maximum number of attributes on a single element (default: 256).
    pub max_attributes: u32,
    /// Maximum length in bytes of a single attribute value (default: 10 MB).
    pub max_attribute_length: usize,
    /// Maximum length in bytes of a single text node (default: 10 MB).
    pub max_text_length: usize,
    /// Maximum length in bytes of an element or attribute name (default: 50,000).
    pub max_name_length: usize,
    /// Maximum number of entity reference expansions per document (default: 10,000).
    pub max_entity_expansions: u32,
    /// Optional callback for resolving external entities.
    ///
    /// When set, references to externally-declared entities (SYSTEM/PUBLIC)
    /// are passed to this resolver instead of producing an error. See
    /// [`EntityResolver`] for security considerations.
    pub entity_resolver: Option<EntityResolver>,
}

impl Clone for ParseOptions {
    fn clone(&self) -> Self {
        Self {
            recover: self.recover,
            no_blanks: self.no_blanks,
            max_depth: self.max_depth,
            max_attributes: self.max_attributes,
            max_attribute_length: self.max_attribute_length,
            max_text_length: self.max_text_length,
            max_name_length: self.max_name_length,
            max_entity_expansions: self.max_entity_expansions,
            entity_resolver: self.entity_resolver.clone(),
        }
    }
}

impl std::fmt::Debug for ParseOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParseOptions")
            .field("recover", &self.recover)
            .field("no_blanks", &self.no_blanks)
            .field("max_depth", &self.max_depth)
            .field("max_attributes", &self.max_attributes)
            .field("max_attribute_length", &self.max_attribute_length)
            .field("max_text_length", &self.max_text_length)
            .field("max_name_length", &self.max_name_length)
            .field("max_entity_expansions", &self.max_entity_expansions)
            .field(
                "entity_resolver",
                &self.entity_resolver.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            recover: false,
            no_blanks: false,
            max_depth: DEFAULT_MAX_DEPTH,
            max_attributes: DEFAULT_MAX_ATTRIBUTES,
            max_attribute_length: DEFAULT_MAX_ATTRIBUTE_LENGTH,
            max_text_length: DEFAULT_MAX_TEXT_LENGTH,
            max_name_length: DEFAULT_MAX_NAME_LENGTH,
            max_entity_expansions: DEFAULT_MAX_ENTITY_EXPANSIONS,
            entity_resolver: None,
        }
    }
}

impl ParseOptions {
    /// Enables or disables error recovery mode.
    #[must_use]
    pub fn recover(mut self, yes: bool) -> Self {
        self.recover = yes;
        self
    }

    /// Enables or disables stripping of blank text nodes.
    #[must_use]
    pub fn no_blanks(mut self, yes: bool) -> Self {
        self.no_blanks = yes;
        self
    }

    /// Sets the maximum element nesting depth.
    #[must_use]
    pub fn max_depth(mut self, max: u32) -> Self {
        self.max_depth = max;
        self
    }

    /// Sets the maximum number of attributes per element.
    #[must_use]
    pub fn max_attributes(mut self, max: u32) -> Self {
        self.max_attributes = max;
        self
    }

    /// Sets the maximum attribute value length in bytes.
    #[must_use]
    pub fn max_attribute_length(mut self, max: usize) -> Self {
        self.max_attribute_length = max;
        self
    }

    /// Sets the maximum text node length in bytes.
    #[must_use]
    pub fn max_text_length(mut self, max: usize) -> Self {
        self.max_text_length = max;
        self
    }

    /// Sets the maximum element/attribute name length in bytes.
    #[must_use]
    pub fn max_name_length(mut self, max: usize) -> Self {
        self.max_name_length = max;
        self
    }

    /// Sets the maximum number of entity reference expansions.
    #[must_use]
    pub fn max_entity_expansions(mut self, max: u32) -> Self {
        self.max_entity_expansions = max;
        self
    }

    /// Sets the external entity resolver callback.
    ///
    /// When set, references to externally-declared entities (SYSTEM/PUBLIC)
    /// are passed to this callback for resolution. The callback receives an
    /// [`ExternalEntityRequest`] and should return `Some(replacement_text)`
    /// to expand the entity, or `None` to reject the reference.
    ///
    /// # Security
    ///
    /// **Warning:** Enabling external entity resolution opens the door to
    /// XML External Entity (XXE) attacks. Only use this with trusted input,
    /// and consider restricting which URIs the resolver is willing to fetch.
    #[must_use]
    pub fn entity_resolver(
        mut self,
        resolver: impl Fn(ExternalEntityRequest<'_>) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.entity_resolver = Some(Arc::new(resolver));
        self
    }
}

/// Parses an XML string with default options.
///
/// # Errors
///
/// Returns `ParseError` if the input is not well-formed XML.
pub fn parse_str(input: &str) -> Result<Document, ParseError> {
    parse_str_with_options(input, &ParseOptions::default())
}

/// Parses an XML string with the given options.
///
/// # Errors
///
/// Returns `ParseError` if the input is not well-formed XML and recovery
/// mode is not enabled.
pub fn parse_str_with_options(input: &str, options: &ParseOptions) -> Result<Document, ParseError> {
    let mut parser = xml::XmlParser::new(input, options);
    parser.parse()
}
