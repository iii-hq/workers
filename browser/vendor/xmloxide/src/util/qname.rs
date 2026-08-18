//! `QName` (qualified name) handling.
//!
//! A `QName` is a name of the form `prefix:localname` or just `localname` (with
//! no prefix). This module provides utilities for splitting and working with
//! qualified names as defined by the Namespaces in XML 1.0 specification.
//!
//! See <https://www.w3.org/TR/xml-names/#NT-QName>

/// Splits a `QName` into its prefix and local name parts.
///
/// Returns `(Some(prefix), localname)` if the name contains a colon,
/// or `(None, localname)` if it does not.
///
/// # Examples
///
/// ```ignore
/// use xmloxide::util::qname::split_qname;
///
/// assert_eq!(split_qname("svg:rect"), (Some("svg"), "rect"));
/// assert_eq!(split_qname("div"), (None, "div"));
/// ```
#[must_use]
pub fn split_qname(qname: &str) -> (Option<&str>, &str) {
    match qname.find(':') {
        Some(pos) => (Some(&qname[..pos]), &qname[pos + 1..]),
        None => (None, qname),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_qname_with_prefix() {
        assert_eq!(split_qname("xml:lang"), (Some("xml"), "lang"));
    }

    #[test]
    fn test_split_qname_without_prefix() {
        assert_eq!(split_qname("div"), (None, "div"));
    }

    #[test]
    fn test_split_qname_empty() {
        assert_eq!(split_qname(""), (None, ""));
    }

    #[test]
    fn test_split_qname_colon_at_start() {
        assert_eq!(split_qname(":local"), (Some(""), "local"));
    }

    #[test]
    fn test_split_qname_colon_at_end() {
        assert_eq!(split_qname("prefix:"), (Some("prefix"), ""));
    }

    #[test]
    fn test_split_qname_multiple_colons() {
        // Only splits on first colon
        assert_eq!(split_qname("a:b:c"), (Some("a"), "b:c"));
    }
}
