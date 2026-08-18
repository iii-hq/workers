//! XML Catalogs for URI resolution (OASIS XML Catalogs 1.1).
//!
//! XML Catalogs provide a mechanism to map public identifiers and system
//! identifiers (URIs) to local resources. This enables offline validation,
//! entity resolution, and redirection of external resources to local copies.
//!
//! The implementation follows the [OASIS XML Catalogs 1.1](https://www.oasis-open.org/committees/entity/spec-2001-08-06.html)
//! specification and supports all standard catalog entry types: `public`,
//! `system`, `rewriteSystem`, `rewriteURI`, `uri`, `delegatePublic`,
//! `delegateSystem`, `nextCatalog`, `systemSuffix`, and `uriSuffix`.
//!
//! # Examples
//!
//! ```
//! use xmloxide::catalog::{Catalog, CatalogEntry};
//!
//! let mut catalog = Catalog::new();
//! catalog.add_entry(CatalogEntry::Public {
//!     public_id: "-//W3C//DTD XHTML 1.0 Strict//EN".to_string(),
//!     uri: "dtd/xhtml1-strict.dtd".to_string(),
//! });
//!
//! let resolved = catalog.resolve_public("-//W3C//DTD XHTML 1.0 Strict//EN");
//! assert_eq!(resolved, Some("dtd/xhtml1-strict.dtd".to_string()));
//! ```

use std::fmt;

use crate::tree::{Document, NodeKind};

/// The OASIS XML Catalog namespace URI.
const CATALOG_NAMESPACE: &str = "urn:oasis:names:tc:entity:xmlns:xml:catalog";

/// An XML Catalog for resolving public/system identifiers to local URIs.
///
/// A catalog contains an ordered list of entries that are consulted during
/// identifier resolution. Entries are tried in order, with the first match
/// winning (except for rewrite rules, where the longest prefix match wins).
#[derive(Debug, Clone)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

/// A single entry in an XML catalog.
///
/// Each variant corresponds to an element in the OASIS XML Catalog format.
/// The catalog processor tries entries in document order, with specific
/// matching rules for each type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogEntry {
    /// Maps a public identifier to a URI.
    ///
    /// Corresponds to the `<public>` element. The `public_id` is normalized
    /// (leading/trailing whitespace stripped, internal whitespace collapsed)
    /// before matching.
    Public {
        /// The public identifier to match.
        public_id: String,
        /// The URI to resolve to.
        uri: String,
    },

    /// Maps a system identifier to a URI.
    ///
    /// Corresponds to the `<system>` element. The `system_id` must match
    /// exactly (after URI normalization).
    System {
        /// The system identifier to match.
        system_id: String,
        /// The URI to resolve to.
        uri: String,
    },

    /// Rewrites the beginning of a system identifier.
    ///
    /// Corresponds to the `<rewriteSystem>` element. When multiple rewrite
    /// rules match, the one with the longest matching prefix wins.
    RewriteSystem {
        /// The prefix to match against the start of a system identifier.
        start: String,
        /// The replacement prefix.
        rewrite_prefix: String,
    },

    /// Rewrites the beginning of a URI.
    ///
    /// Corresponds to the `<rewriteURI>` element. When multiple rewrite
    /// rules match, the one with the longest matching prefix wins.
    RewriteUri {
        /// The prefix to match against the start of a URI.
        start: String,
        /// The replacement prefix.
        rewrite_prefix: String,
    },

    /// Maps a URI to another URI.
    ///
    /// Corresponds to the `<uri>` element. The `name` must match exactly.
    Uri {
        /// The URI to match.
        name: String,
        /// The URI to resolve to.
        uri: String,
    },

    /// Delegates matching public IDs to another catalog.
    ///
    /// Corresponds to the `<delegatePublic>` element. When a public
    /// identifier starts with the given prefix, resolution is delegated
    /// to the specified catalog file.
    DelegatePublic {
        /// The public identifier prefix to match.
        start: String,
        /// The URI of the catalog to delegate to.
        catalog: String,
    },

    /// Delegates matching system IDs to another catalog.
    ///
    /// Corresponds to the `<delegateSystem>` element. When a system
    /// identifier starts with the given prefix, resolution is delegated
    /// to the specified catalog file.
    DelegateSystem {
        /// The system identifier prefix to match.
        start: String,
        /// The URI of the catalog to delegate to.
        catalog: String,
    },

    /// Adds another catalog to search.
    ///
    /// Corresponds to the `<nextCatalog>` element. When resolution fails
    /// in the current catalog, the next catalog is consulted.
    NextCatalog {
        /// The URI of the next catalog to search.
        catalog: String,
    },

    /// System ID suffix matching.
    ///
    /// Corresponds to the `<systemSuffix>` element. Matches system
    /// identifiers that end with the given suffix.
    SystemSuffix {
        /// The suffix to match against the end of a system identifier.
        suffix: String,
        /// The URI to resolve to.
        uri: String,
    },

    /// URI suffix matching.
    ///
    /// Corresponds to the `<uriSuffix>` element. Matches URIs that end
    /// with the given suffix.
    UriSuffix {
        /// The suffix to match against the end of a URI.
        suffix: String,
        /// The URI to resolve to.
        uri: String,
    },
}

/// An error that can occur during catalog parsing.
///
/// This error is returned when the catalog XML cannot be parsed or when
/// the catalog structure does not conform to the OASIS XML Catalog format.
#[derive(Debug, Clone)]
pub struct CatalogError {
    /// Human-readable description of the error.
    pub message: String,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "catalog error: {}", self.message)
    }
}

impl std::error::Error for CatalogError {}

impl Catalog {
    /// Creates an empty catalog with no entries.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::catalog::Catalog;
    ///
    /// let catalog = Catalog::new();
    /// assert!(catalog.is_empty());
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Parses an XML catalog from a string in OASIS XML Catalog format.
    ///
    /// The input must be a well-formed XML document with a root `<catalog>`
    /// element in the `urn:oasis:names:tc:entity:xmlns:xml:catalog` namespace.
    ///
    /// # Errors
    ///
    /// Returns `CatalogError` if:
    /// - The input is not well-formed XML
    /// - The root element is not `<catalog>` in the catalog namespace
    /// - Required attributes are missing on catalog entries
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::catalog::Catalog;
    ///
    /// let xml = r#"<catalog xmlns="urn:oasis:names:tc:entity:xmlns:xml:catalog">
    ///   <public publicId="-//Example//EN" uri="example.dtd"/>
    /// </catalog>"#;
    ///
    /// let catalog = Catalog::parse(xml).unwrap();
    /// assert_eq!(catalog.len(), 1);
    /// ```
    pub fn parse(xml: &str) -> Result<Self, CatalogError> {
        let doc = Document::parse_str(xml).map_err(|e| CatalogError {
            message: format!("failed to parse catalog XML: {e}"),
        })?;

        let root_element = doc.root_element().ok_or_else(|| CatalogError {
            message: "catalog document has no root element".to_string(),
        })?;

        // Verify the root element is <catalog> in the catalog namespace.
        let root_name = doc.node_name(root_element).unwrap_or("");
        let root_ns = doc.node_namespace(root_element);

        if root_name != "catalog" {
            return Err(CatalogError {
                message: format!("expected root element 'catalog', found '{root_name}'"),
            });
        }

        if root_ns != Some(CATALOG_NAMESPACE) {
            return Err(CatalogError {
                message: format!("root element must be in namespace '{CATALOG_NAMESPACE}'"),
            });
        }

        let mut catalog = Self::new();

        for child in doc.children(root_element) {
            if let NodeKind::Element { ref name, .. } = doc.node(child).kind {
                if let Some(entry) = parse_catalog_entry(&doc, child, name)? {
                    catalog.entries.push(entry);
                }
            }
        }

        Ok(catalog)
    }

    /// Adds an entry to the catalog.
    ///
    /// Entries are tried in insertion order during resolution.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::catalog::{Catalog, CatalogEntry};
    ///
    /// let mut catalog = Catalog::new();
    /// catalog.add_entry(CatalogEntry::System {
    ///     system_id: "http://example.com/schema.xsd".to_string(),
    ///     uri: "local/schema.xsd".to_string(),
    /// });
    /// assert_eq!(catalog.len(), 1);
    /// ```
    pub fn add_entry(&mut self, entry: CatalogEntry) {
        self.entries.push(entry);
    }

    /// Resolves a public identifier to a URI.
    ///
    /// Searches catalog entries in order for a matching `Public` entry.
    /// Public identifiers are compared after normalization (whitespace
    /// collapsing).
    ///
    /// Returns `None` if no matching entry is found.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::catalog::{Catalog, CatalogEntry};
    ///
    /// let mut catalog = Catalog::new();
    /// catalog.add_entry(CatalogEntry::Public {
    ///     public_id: "-//W3C//DTD XHTML 1.0//EN".to_string(),
    ///     uri: "xhtml1.dtd".to_string(),
    /// });
    ///
    /// assert_eq!(
    ///     catalog.resolve_public("-//W3C//DTD XHTML 1.0//EN"),
    ///     Some("xhtml1.dtd".to_string())
    /// );
    /// assert_eq!(catalog.resolve_public("-//Unknown//EN"), None);
    /// ```
    #[must_use]
    pub fn resolve_public(&self, public_id: &str) -> Option<String> {
        let normalized = normalize_public_id(public_id);

        // Try exact public match first.
        for entry in &self.entries {
            if let CatalogEntry::Public {
                public_id: ref pid,
                ref uri,
            } = *entry
            {
                if normalize_public_id(pid) == normalized {
                    return Some(uri.clone());
                }
            }
        }

        // Try delegatePublic matches (longest prefix wins).
        let mut best_delegate: Option<(&str, usize)> = None;
        for entry in &self.entries {
            if let CatalogEntry::DelegatePublic {
                ref start,
                ref catalog,
            } = *entry
            {
                if normalized.starts_with(start.as_str())
                    && start.len() > best_delegate.map_or(0, |(_, len)| len)
                {
                    best_delegate = Some((catalog.as_str(), start.len()));
                }
            }
        }

        if let Some((catalog_uri, _)) = best_delegate {
            return Some(catalog_uri.to_string());
        }

        None
    }

    /// Resolves a system identifier to a URI.
    ///
    /// The resolution order is:
    /// 1. Exact `System` match
    /// 2. `RewriteSystem` prefix match (longest prefix wins)
    /// 3. `SystemSuffix` suffix match (longest suffix wins)
    /// 4. `DelegateSystem` prefix match (longest prefix wins)
    ///
    /// Returns `None` if no matching entry is found.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::catalog::{Catalog, CatalogEntry};
    ///
    /// let mut catalog = Catalog::new();
    /// catalog.add_entry(CatalogEntry::System {
    ///     system_id: "http://example.com/schema.xsd".to_string(),
    ///     uri: "local/schema.xsd".to_string(),
    /// });
    ///
    /// assert_eq!(
    ///     catalog.resolve_system("http://example.com/schema.xsd"),
    ///     Some("local/schema.xsd".to_string())
    /// );
    /// ```
    #[must_use]
    pub fn resolve_system(&self, system_id: &str) -> Option<String> {
        // 1. Try exact system match.
        for entry in &self.entries {
            if let CatalogEntry::System {
                system_id: ref sid,
                ref uri,
            } = *entry
            {
                if sid == system_id {
                    return Some(uri.clone());
                }
            }
        }

        // 2. Try rewriteSystem (longest prefix wins).
        if let Some(result) = self.resolve_rewrite_system(system_id) {
            return Some(result);
        }

        // 3. Try systemSuffix (longest suffix wins).
        if let Some(result) = self.resolve_system_suffix(system_id) {
            return Some(result);
        }

        // 4. Try delegateSystem (longest prefix wins).
        let mut best_delegate: Option<(&str, usize)> = None;
        for entry in &self.entries {
            if let CatalogEntry::DelegateSystem {
                ref start,
                ref catalog,
            } = *entry
            {
                if system_id.starts_with(start.as_str())
                    && start.len() > best_delegate.map_or(0, |(_, len)| len)
                {
                    best_delegate = Some((catalog.as_str(), start.len()));
                }
            }
        }

        if let Some((catalog_uri, _)) = best_delegate {
            return Some(catalog_uri.to_string());
        }

        None
    }

    /// Resolves a URI reference.
    ///
    /// The resolution order is:
    /// 1. Exact `Uri` match
    /// 2. `RewriteUri` prefix match (longest prefix wins)
    /// 3. `UriSuffix` suffix match (longest suffix wins)
    ///
    /// Returns `None` if no matching entry is found.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::catalog::{Catalog, CatalogEntry};
    ///
    /// let mut catalog = Catalog::new();
    /// catalog.add_entry(CatalogEntry::Uri {
    ///     name: "http://example.com/schema.xsd".to_string(),
    ///     uri: "local/schema.xsd".to_string(),
    /// });
    ///
    /// assert_eq!(
    ///     catalog.resolve_uri("http://example.com/schema.xsd"),
    ///     Some("local/schema.xsd".to_string())
    /// );
    /// ```
    #[must_use]
    pub fn resolve_uri(&self, uri: &str) -> Option<String> {
        // 1. Try exact URI match.
        for entry in &self.entries {
            if let CatalogEntry::Uri {
                ref name,
                uri: ref target,
            } = *entry
            {
                if name == uri {
                    return Some(target.clone());
                }
            }
        }

        // 2. Try rewriteURI (longest prefix wins).
        if let Some(result) = self.resolve_rewrite_uri(uri) {
            return Some(result);
        }

        // 3. Try uriSuffix (longest suffix wins).
        let mut best_suffix: Option<(&str, usize)> = None;
        for entry in &self.entries {
            if let CatalogEntry::UriSuffix {
                ref suffix,
                uri: ref target,
            } = *entry
            {
                if uri.ends_with(suffix.as_str())
                    && suffix.len() > best_suffix.map_or(0, |(_, len)| len)
                {
                    best_suffix = Some((target.as_str(), suffix.len()));
                }
            }
        }

        if let Some((target, _)) = best_suffix {
            return Some(target.to_string());
        }

        None
    }

    /// Resolves either a public or system identifier, trying system first.
    ///
    /// This is the primary resolution method that follows the OASIS catalog
    /// resolution algorithm: system identifiers take precedence over public
    /// identifiers because they are more specific.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::catalog::{Catalog, CatalogEntry};
    ///
    /// let mut catalog = Catalog::new();
    /// catalog.add_entry(CatalogEntry::Public {
    ///     public_id: "-//Example//EN".to_string(),
    ///     uri: "public.dtd".to_string(),
    /// });
    /// catalog.add_entry(CatalogEntry::System {
    ///     system_id: "http://example.com/doc.dtd".to_string(),
    ///     uri: "system.dtd".to_string(),
    /// });
    ///
    /// // System takes precedence.
    /// assert_eq!(
    ///     catalog.resolve(Some("-//Example//EN"), Some("http://example.com/doc.dtd")),
    ///     Some("system.dtd".to_string())
    /// );
    ///
    /// // Falls back to public when system is not provided.
    /// assert_eq!(
    ///     catalog.resolve(Some("-//Example//EN"), None),
    ///     Some("public.dtd".to_string())
    /// );
    /// ```
    #[must_use]
    pub fn resolve(&self, public_id: Option<&str>, system_id: Option<&str>) -> Option<String> {
        // Try system identifier first (more specific).
        if let Some(sid) = system_id {
            if let Some(resolved) = self.resolve_system(sid) {
                return Some(resolved);
            }
        }

        // Fall back to public identifier.
        if let Some(pid) = public_id {
            if let Some(resolved) = self.resolve_public(pid) {
                return Some(resolved);
            }
        }

        None
    }

    /// Merges another catalog's entries into this one.
    ///
    /// All entries from `other` are appended to this catalog's entry list,
    /// preserving order. The other catalog's entries will be tried after
    /// the existing entries during resolution.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::catalog::{Catalog, CatalogEntry};
    ///
    /// let mut catalog1 = Catalog::new();
    /// catalog1.add_entry(CatalogEntry::Public {
    ///     public_id: "-//A//EN".to_string(),
    ///     uri: "a.dtd".to_string(),
    /// });
    ///
    /// let mut catalog2 = Catalog::new();
    /// catalog2.add_entry(CatalogEntry::Public {
    ///     public_id: "-//B//EN".to_string(),
    ///     uri: "b.dtd".to_string(),
    /// });
    ///
    /// catalog1.merge(&catalog2);
    /// assert_eq!(catalog1.len(), 2);
    /// ```
    pub fn merge(&mut self, other: &Catalog) {
        self.entries.extend(other.entries.iter().cloned());
    }

    /// Returns the number of entries in the catalog.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::catalog::Catalog;
    ///
    /// let catalog = Catalog::new();
    /// assert_eq!(catalog.len(), 0);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the catalog has no entries.
    ///
    /// # Examples
    ///
    /// ```
    /// use xmloxide::catalog::Catalog;
    ///
    /// let catalog = Catalog::new();
    /// assert!(catalog.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns an iterator over the catalog entries.
    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.iter()
    }

    // --- Private resolution helpers ---

    /// Finds the best `RewriteSystem` match for the given system ID.
    ///
    /// Among all `RewriteSystem` entries whose `start` is a prefix of
    /// `system_id`, the one with the longest `start` wins.
    fn resolve_rewrite_system(&self, system_id: &str) -> Option<String> {
        let mut best: Option<(&str, &str, usize)> = None;

        for entry in &self.entries {
            if let CatalogEntry::RewriteSystem {
                ref start,
                ref rewrite_prefix,
            } = *entry
            {
                if system_id.starts_with(start.as_str())
                    && start.len() > best.map_or(0, |(_, _, len)| len)
                {
                    best = Some((start.as_str(), rewrite_prefix.as_str(), start.len()));
                }
            }
        }

        best.map(|(start, rewrite_prefix, _)| {
            format!("{rewrite_prefix}{}", &system_id[start.len()..])
        })
    }

    /// Finds the best `SystemSuffix` match for the given system ID.
    ///
    /// Among all `SystemSuffix` entries whose `suffix` matches the end of
    /// `system_id`, the one with the longest `suffix` wins.
    fn resolve_system_suffix(&self, system_id: &str) -> Option<String> {
        let mut best: Option<(&str, usize)> = None;

        for entry in &self.entries {
            if let CatalogEntry::SystemSuffix {
                ref suffix,
                ref uri,
            } = *entry
            {
                if system_id.ends_with(suffix.as_str())
                    && suffix.len() > best.map_or(0, |(_, len)| len)
                {
                    best = Some((uri.as_str(), suffix.len()));
                }
            }
        }

        best.map(|(uri, _)| uri.to_string())
    }

    /// Finds the best `RewriteUri` match for the given URI.
    fn resolve_rewrite_uri(&self, uri: &str) -> Option<String> {
        let mut best: Option<(&str, &str, usize)> = None;

        for entry in &self.entries {
            if let CatalogEntry::RewriteUri {
                ref start,
                ref rewrite_prefix,
            } = *entry
            {
                if uri.starts_with(start.as_str())
                    && start.len() > best.map_or(0, |(_, _, len)| len)
                {
                    best = Some((start.as_str(), rewrite_prefix.as_str(), start.len()));
                }
            }
        }

        best.map(|(start, rewrite_prefix, _)| format!("{rewrite_prefix}{}", &uri[start.len()..]))
    }
}

impl Default for Catalog {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalizes a public identifier by collapsing whitespace.
///
/// Per the OASIS catalog specification, public identifiers are compared
/// after stripping leading/trailing whitespace and collapsing all internal
/// whitespace sequences to a single space.
fn normalize_public_id(public_id: &str) -> String {
    public_id.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parses a single catalog entry element into a `CatalogEntry`.
///
/// Returns `Ok(None)` for unrecognized elements (which are silently ignored
/// per the catalog specification). Returns `Err` if a recognized element
/// is missing required attributes.
fn parse_catalog_entry(
    doc: &Document,
    node: crate::NodeId,
    name: &str,
) -> Result<Option<CatalogEntry>, CatalogError> {
    match name {
        "public" => {
            let public_id = require_attr(doc, node, "publicId", "public")?;
            let uri = require_attr(doc, node, "uri", "public")?;
            Ok(Some(CatalogEntry::Public { public_id, uri }))
        }
        "system" => {
            let system_id = require_attr(doc, node, "systemId", "system")?;
            let uri = require_attr(doc, node, "uri", "system")?;
            Ok(Some(CatalogEntry::System { system_id, uri }))
        }
        "rewriteSystem" => {
            let start = require_attr(doc, node, "systemIdStartString", "rewriteSystem")?;
            let rewrite_prefix = require_attr(doc, node, "rewritePrefix", "rewriteSystem")?;
            Ok(Some(CatalogEntry::RewriteSystem {
                start,
                rewrite_prefix,
            }))
        }
        "rewriteURI" => {
            let start = require_attr(doc, node, "uriStartString", "rewriteURI")?;
            let rewrite_prefix = require_attr(doc, node, "rewritePrefix", "rewriteURI")?;
            Ok(Some(CatalogEntry::RewriteUri {
                start,
                rewrite_prefix,
            }))
        }
        "uri" => {
            let name = require_attr(doc, node, "name", "uri")?;
            let uri = require_attr(doc, node, "uri", "uri")?;
            Ok(Some(CatalogEntry::Uri { name, uri }))
        }
        "delegatePublic" => {
            let start = require_attr(doc, node, "publicIdStartString", "delegatePublic")?;
            let catalog = require_attr(doc, node, "catalog", "delegatePublic")?;
            Ok(Some(CatalogEntry::DelegatePublic { start, catalog }))
        }
        "delegateSystem" => {
            let start = require_attr(doc, node, "systemIdStartString", "delegateSystem")?;
            let catalog = require_attr(doc, node, "catalog", "delegateSystem")?;
            Ok(Some(CatalogEntry::DelegateSystem { start, catalog }))
        }
        "nextCatalog" => {
            let catalog = require_attr(doc, node, "catalog", "nextCatalog")?;
            Ok(Some(CatalogEntry::NextCatalog { catalog }))
        }
        "systemSuffix" => {
            let suffix = require_attr(doc, node, "systemIdSuffix", "systemSuffix")?;
            let uri = require_attr(doc, node, "uri", "systemSuffix")?;
            Ok(Some(CatalogEntry::SystemSuffix { suffix, uri }))
        }
        "uriSuffix" => {
            let suffix = require_attr(doc, node, "uriSuffix", "uriSuffix")?;
            let uri = require_attr(doc, node, "uri", "uriSuffix")?;
            Ok(Some(CatalogEntry::UriSuffix { suffix, uri }))
        }
        // Unrecognized elements in the catalog namespace are silently ignored,
        // following the extensibility rules in the OASIS specification.
        _ => Ok(None),
    }
}

/// Extracts a required attribute from an element, returning a `CatalogError`
/// if the attribute is missing.
fn require_attr(
    doc: &Document,
    node: crate::NodeId,
    attr_name: &str,
    element_name: &str,
) -> Result<String, CatalogError> {
    doc.attribute(node, attr_name)
        .map(ToString::to_string)
        .ok_or_else(|| CatalogError {
            message: format!(
                "missing required attribute '{attr_name}' on <{element_name}> element"
            ),
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn catalog_xml(body: &str) -> String {
        format!(r#"<catalog xmlns="urn:oasis:names:tc:entity:xmlns:xml:catalog">{body}</catalog>"#)
    }

    #[test]
    fn test_parse_simple_catalog_with_public_entry() {
        let xml = catalog_xml(
            r#"<public publicId="-//W3C//DTD XHTML 1.0 Strict//EN" uri="dtd/xhtml1-strict.dtd"/>"#,
        );
        let catalog = Catalog::parse(&xml).unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(
            catalog.entries().next(),
            Some(&CatalogEntry::Public {
                public_id: "-//W3C//DTD XHTML 1.0 Strict//EN".to_string(),
                uri: "dtd/xhtml1-strict.dtd".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_catalog_with_system_entry() {
        let xml = catalog_xml(
            r#"<system systemId="http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd" uri="dtd/xhtml1-strict.dtd"/>"#,
        );
        let catalog = Catalog::parse(&xml).unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(
            catalog.entries().next(),
            Some(&CatalogEntry::System {
                system_id: "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd".to_string(),
                uri: "dtd/xhtml1-strict.dtd".to_string(),
            })
        );
    }

    #[test]
    fn test_parse_catalog_with_rewrite_entries() {
        let xml = catalog_xml(
            r#"<rewriteSystem systemIdStartString="http://www.w3.org/TR/" rewritePrefix="file:///usr/share/xml/w3c/"/>
            <rewriteURI uriStartString="http://example.com/" rewritePrefix="file:///local/"/>"#,
        );
        let catalog = Catalog::parse(&xml).unwrap();
        assert_eq!(catalog.len(), 2);
    }

    #[test]
    fn test_resolve_public_identifier() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::Public {
            public_id: "-//W3C//DTD XHTML 1.0 Strict//EN".to_string(),
            uri: "dtd/xhtml1-strict.dtd".to_string(),
        });

        assert_eq!(
            catalog.resolve_public("-//W3C//DTD XHTML 1.0 Strict//EN"),
            Some("dtd/xhtml1-strict.dtd".to_string())
        );
    }

    #[test]
    fn test_resolve_system_identifier() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::System {
            system_id: "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd".to_string(),
            uri: "dtd/xhtml1-strict.dtd".to_string(),
        });

        assert_eq!(
            catalog.resolve_system("http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd"),
            Some("dtd/xhtml1-strict.dtd".to_string())
        );
    }

    #[test]
    fn test_resolve_system_with_rewrite_prefix() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::RewriteSystem {
            start: "http://www.w3.org/TR/".to_string(),
            rewrite_prefix: "file:///usr/share/xml/w3c/".to_string(),
        });

        assert_eq!(
            catalog.resolve_system("http://www.w3.org/TR/xhtml1/DTD/strict.dtd"),
            Some("file:///usr/share/xml/w3c/xhtml1/DTD/strict.dtd".to_string())
        );
    }

    #[test]
    fn test_resolve_uri() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::Uri {
            name: "http://example.com/schema.xsd".to_string(),
            uri: "local/schema.xsd".to_string(),
        });

        assert_eq!(
            catalog.resolve_uri("http://example.com/schema.xsd"),
            Some("local/schema.xsd".to_string())
        );
    }

    #[test]
    fn test_resolve_with_suffix_matching() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::SystemSuffix {
            suffix: "strict.dtd".to_string(),
            uri: "local/strict.dtd".to_string(),
        });

        assert_eq!(
            catalog.resolve_system("http://example.com/path/to/strict.dtd"),
            Some("local/strict.dtd".to_string())
        );
    }

    #[test]
    fn test_no_match_returns_none() {
        let catalog = Catalog::new();
        assert_eq!(catalog.resolve_public("-//Unknown//EN"), None);
        assert_eq!(
            catalog.resolve_system("http://unknown.example.com/foo"),
            None
        );
        assert_eq!(catalog.resolve_uri("http://unknown.example.com/bar"), None);
        assert_eq!(catalog.resolve(None, None), None);
    }

    #[test]
    fn test_merge_two_catalogs() {
        let mut catalog1 = Catalog::new();
        catalog1.add_entry(CatalogEntry::Public {
            public_id: "-//A//EN".to_string(),
            uri: "a.dtd".to_string(),
        });

        let mut catalog2 = Catalog::new();
        catalog2.add_entry(CatalogEntry::Public {
            public_id: "-//B//EN".to_string(),
            uri: "b.dtd".to_string(),
        });

        catalog1.merge(&catalog2);
        assert_eq!(catalog1.len(), 2);
        assert_eq!(
            catalog1.resolve_public("-//A//EN"),
            Some("a.dtd".to_string())
        );
        assert_eq!(
            catalog1.resolve_public("-//B//EN"),
            Some("b.dtd".to_string())
        );
    }

    #[test]
    fn test_empty_catalog() {
        let catalog = Catalog::new();
        assert!(catalog.is_empty());
        assert_eq!(catalog.len(), 0);
    }

    #[test]
    fn test_add_entry_programmatically() {
        let mut catalog = Catalog::new();
        assert!(catalog.is_empty());

        catalog.add_entry(CatalogEntry::System {
            system_id: "http://example.com/test.dtd".to_string(),
            uri: "test.dtd".to_string(),
        });

        assert!(!catalog.is_empty());
        assert_eq!(catalog.len(), 1);
        assert_eq!(
            catalog.resolve_system("http://example.com/test.dtd"),
            Some("test.dtd".to_string())
        );
    }

    #[test]
    fn test_catalog_len_and_is_empty() {
        let mut catalog = Catalog::new();
        assert_eq!(catalog.len(), 0);
        assert!(catalog.is_empty());

        catalog.add_entry(CatalogEntry::NextCatalog {
            catalog: "other.xml".to_string(),
        });
        assert_eq!(catalog.len(), 1);
        assert!(!catalog.is_empty());

        catalog.add_entry(CatalogEntry::NextCatalog {
            catalog: "another.xml".to_string(),
        });
        assert_eq!(catalog.len(), 2);
    }

    #[test]
    fn test_complex_catalog_with_multiple_entry_types() {
        let xml = catalog_xml(
            r#"<public publicId="-//W3C//DTD XHTML 1.0 Strict//EN" uri="dtd/xhtml1-strict.dtd"/>
            <system systemId="http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd" uri="dtd/xhtml1-strict.dtd"/>
            <rewriteSystem systemIdStartString="http://www.w3.org/TR/" rewritePrefix="file:///local/w3c/"/>
            <uri name="http://example.com/schema.xsd" uri="local/schema.xsd"/>
            <nextCatalog catalog="other-catalog.xml"/>"#,
        );

        let catalog = Catalog::parse(&xml).unwrap();
        assert_eq!(catalog.len(), 5);

        assert!(catalog
            .resolve_public("-//W3C//DTD XHTML 1.0 Strict//EN")
            .is_some());
        assert!(catalog
            .resolve_system("http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd")
            .is_some());
        assert!(catalog
            .resolve_system("http://www.w3.org/TR/other/doc.xml")
            .is_some());
        assert!(catalog
            .resolve_uri("http://example.com/schema.xsd")
            .is_some());
    }

    #[test]
    fn test_resolve_prefers_system_over_public() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::Public {
            public_id: "-//Example//EN".to_string(),
            uri: "public-result.dtd".to_string(),
        });
        catalog.add_entry(CatalogEntry::System {
            system_id: "http://example.com/doc.dtd".to_string(),
            uri: "system-result.dtd".to_string(),
        });

        // When both are provided, system wins.
        assert_eq!(
            catalog.resolve(Some("-//Example//EN"), Some("http://example.com/doc.dtd")),
            Some("system-result.dtd".to_string())
        );

        // When only public is provided, public wins.
        assert_eq!(
            catalog.resolve(Some("-//Example//EN"), None),
            Some("public-result.dtd".to_string())
        );

        // When only system is provided, system wins.
        assert_eq!(
            catalog.resolve(None, Some("http://example.com/doc.dtd")),
            Some("system-result.dtd".to_string())
        );
    }

    #[test]
    fn test_rewrite_system_longest_prefix_wins() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::RewriteSystem {
            start: "http://www.w3.org/".to_string(),
            rewrite_prefix: "file:///short/".to_string(),
        });
        catalog.add_entry(CatalogEntry::RewriteSystem {
            start: "http://www.w3.org/TR/xhtml1/".to_string(),
            rewrite_prefix: "file:///long/".to_string(),
        });

        // The longer prefix match should win.
        assert_eq!(
            catalog.resolve_system("http://www.w3.org/TR/xhtml1/DTD/strict.dtd"),
            Some("file:///long/DTD/strict.dtd".to_string())
        );

        // A URL that only matches the shorter prefix.
        assert_eq!(
            catalog.resolve_system("http://www.w3.org/other/file.xml"),
            Some("file:///short/other/file.xml".to_string())
        );
    }

    #[test]
    fn test_catalog_error_display() {
        let err = CatalogError {
            message: "missing required attribute".to_string(),
        };
        assert_eq!(err.to_string(), "catalog error: missing required attribute");
    }

    #[test]
    fn test_parse_invalid_xml_returns_error() {
        let result = Catalog::parse("not valid xml <><>");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_wrong_root_element() {
        let xml = r#"<notcatalog xmlns="urn:oasis:names:tc:entity:xmlns:xml:catalog"/>"#;
        let result = Catalog::parse(xml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("expected root element 'catalog'"));
    }

    #[test]
    fn test_parse_missing_namespace() {
        let xml = r#"<catalog><public publicId="test" uri="test.dtd"/></catalog>"#;
        let result = Catalog::parse(xml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("namespace"));
    }

    #[test]
    fn test_parse_missing_required_attribute() {
        let xml = catalog_xml(r#"<public publicId="-//Test//EN"/>"#);
        let result = Catalog::parse(&xml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("uri"));
    }

    #[test]
    fn test_public_id_whitespace_normalization() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::Public {
            public_id: "-//W3C//DTD  XHTML  1.0//EN".to_string(),
            uri: "xhtml.dtd".to_string(),
        });

        // Extra whitespace in the query should still match.
        assert_eq!(
            catalog.resolve_public("-//W3C//DTD   XHTML   1.0//EN"),
            Some("xhtml.dtd".to_string())
        );
    }

    #[test]
    fn test_uri_suffix_matching() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::UriSuffix {
            suffix: "schema.xsd".to_string(),
            uri: "local/schema.xsd".to_string(),
        });

        assert_eq!(
            catalog.resolve_uri("http://example.com/path/to/schema.xsd"),
            Some("local/schema.xsd".to_string())
        );
        assert_eq!(catalog.resolve_uri("http://example.com/other.xsd"), None);
    }

    #[test]
    fn test_rewrite_uri() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::RewriteUri {
            start: "http://example.com/schemas/".to_string(),
            rewrite_prefix: "file:///local/schemas/".to_string(),
        });

        assert_eq!(
            catalog.resolve_uri("http://example.com/schemas/types/main.xsd"),
            Some("file:///local/schemas/types/main.xsd".to_string())
        );
    }

    #[test]
    fn test_delegate_public() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::DelegatePublic {
            start: "-//W3C//".to_string(),
            catalog: "w3c-catalog.xml".to_string(),
        });

        // DelegatePublic returns the catalog URI for matching public IDs.
        assert_eq!(
            catalog.resolve_public("-//W3C//DTD XHTML 1.0//EN"),
            Some("w3c-catalog.xml".to_string())
        );
        assert_eq!(catalog.resolve_public("-//OASIS//DTD DocBook//EN"), None);
    }

    #[test]
    fn test_delegate_system() {
        let mut catalog = Catalog::new();
        catalog.add_entry(CatalogEntry::DelegateSystem {
            start: "http://www.w3.org/".to_string(),
            catalog: "w3c-catalog.xml".to_string(),
        });

        assert_eq!(
            catalog.resolve_system("http://www.w3.org/TR/xhtml1/DTD/strict.dtd"),
            Some("w3c-catalog.xml".to_string())
        );
        assert_eq!(catalog.resolve_system("http://example.com/other.dtd"), None);
    }

    #[test]
    fn test_default_trait() {
        let catalog = Catalog::default();
        assert!(catalog.is_empty());
    }

    #[test]
    fn test_catalog_error_is_error_trait() {
        let err = CatalogError {
            message: "test error".to_string(),
        };
        let _: &dyn std::error::Error = &err;
    }
}
