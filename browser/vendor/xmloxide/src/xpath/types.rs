//! `XPath` 1.0 value type system.
//!
//! This module implements the four core data types defined in the `XPath` 1.0
//! specification (<https://www.w3.org/TR/xpath-10/#section-Data-Model>):
//! boolean, number, string, and node-set.
//!
//! It also provides type conversion methods per `XPath` 1.0 sections 4.1
//! through 4.4, comparison helpers per section 3.4, and number formatting
//! rules per the spec's string conversion requirements.

use crate::tree::NodeId;
use std::fmt;

// ---------------------------------------------------------------------------
// XPathNode
// ---------------------------------------------------------------------------

/// A node in an `XPath` node-set.
///
/// The `XPath` 1.0 data model (section 5) includes attribute nodes, but the
/// tree arena stores attributes inline on their owner element rather than as
/// tree nodes. This enum lets node-sets carry both kinds: ordinary tree
/// nodes by [`NodeId`], and attributes by owner element plus position in the
/// element's attribute list.
///
/// Ordering follows document order: an attribute sorts immediately after its
/// owner element (and before the element's children — sufficient for the
/// stable, consistent ordering `XPath` 1.0 section 5.3 requires), and
/// attributes of the same element sort by attribute index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum XPathNode {
    /// A tree node (element, text, comment, PI, or the document root).
    Node(NodeId),
    /// An attribute node, identified by its owner element and its index in
    /// the element's attribute list.
    Attribute {
        /// The element that carries the attribute.
        owner: NodeId,
        /// The index of the attribute in the owner's attribute list.
        index: u32,
    },
}

impl XPathNode {
    /// Returns the tree node this entry anchors to: the node itself, or the
    /// owner element for an attribute.
    #[must_use]
    pub fn anchor(self) -> NodeId {
        match self {
            Self::Node(id) => id,
            Self::Attribute { owner, .. } => owner,
        }
    }

    /// Returns the inner [`NodeId`] if this is a tree node, or `None` for
    /// an attribute.
    #[must_use]
    pub fn as_tree_node(self) -> Option<NodeId> {
        match self {
            Self::Node(id) => Some(id),
            Self::Attribute { .. } => None,
        }
    }

    /// Returns `true` if this entry is an attribute node.
    #[must_use]
    pub fn is_attribute(self) -> bool {
        matches!(self, Self::Attribute { .. })
    }

    /// Document-order sort key: `(anchor, is-attribute, attribute-index)`.
    ///
    /// Relies on the arena property that `NodeId`s of a parsed document are
    /// allocated in document order.
    fn sort_key(self) -> (NodeId, u8, u32) {
        match self {
            Self::Node(id) => (id, 0, 0),
            Self::Attribute { owner, index } => (owner, 1, index),
        }
    }
}

impl Ord for XPathNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl PartialOrd for XPathNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl From<NodeId> for XPathNode {
    fn from(id: NodeId) -> Self {
        Self::Node(id)
    }
}

// ---------------------------------------------------------------------------
// XPathValue
// ---------------------------------------------------------------------------

/// An `XPath` 1.0 value.
///
/// The `XPath` specification defines exactly four data types. Every expression
/// evaluates to one of these variants.
/// See <https://www.w3.org/TR/xpath-10/#section-Data-Model>.
#[derive(Debug, Clone)]
pub enum XPathValue {
    /// A boolean value (`true` or `false`).
    ///
    /// See `XPath` 1.0 section 4.3.
    Boolean(bool),

    /// A number (IEEE 754 double-precision floating-point).
    ///
    /// Numbers follow IEEE 754 semantics including NaN, positive and
    /// negative infinity, and negative zero. See `XPath` 1.0 section 4.4.
    Number(f64),

    /// A string (a sequence of UCS characters).
    ///
    /// See `XPath` 1.0 section 4.2.
    String(String),

    /// An ordered set of nodes.
    ///
    /// Node-sets are ordered in document order and may contain both tree
    /// nodes and attribute nodes (see [`XPathNode`]). Duplicates should not
    /// appear (the caller is responsible for deduplication). See `XPath` 1.0
    /// section 3.3.
    NodeSet(Vec<XPathNode>),
}

impl XPathValue {
    // -- Type conversion methods (`XPath` 1.0 sections 4.1-4.4) -------------

    /// Converts this value to a boolean.
    ///
    /// Conversion rules per `XPath` 1.0 section 4.3:
    /// - **boolean**: identity
    /// - **number**: `false` if the number is zero or NaN, `true` otherwise
    /// - **string**: `false` if the string is empty, `true` otherwise
    /// - **node-set**: `false` if the node-set is empty, `true` otherwise
    #[must_use]
    pub fn to_boolean(&self) -> bool {
        match self {
            Self::Boolean(b) => *b,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::String(s) => !s.is_empty(),
            Self::NodeSet(nodes) => !nodes.is_empty(),
        }
    }

    /// Converts this value to a number.
    ///
    /// Conversion rules per `XPath` 1.0 section 4.4:
    /// - **number**: identity
    /// - **boolean**: `true` becomes `1.0`, `false` becomes `0.0`
    /// - **string**: parsed as an IEEE 754 number; unparseable strings become NaN
    /// - **node-set**: the string-value of the first node in document order is
    ///   converted as a string; an empty node-set converts to NaN
    ///
    /// Note: the node-set conversion requires a `Document` to compute
    /// string-values. Without one, we return NaN as a fallback. For full
    /// node-set conversion, use
    /// [`to_number_with_string_value`](Self::to_number_with_string_value).
    #[must_use]
    pub fn to_number(&self) -> f64 {
        match self {
            Self::Number(n) => *n,
            Self::Boolean(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::String(s) => parse_xpath_number(s),
            Self::NodeSet(_) => {
                // Without a Document reference we cannot compute the
                // string-value of the first node. Return NaN as a safe
                // fallback; callers that hold a Document should use
                // `to_number_with_string_value` instead.
                f64::NAN
            }
        }
    }

    /// Converts this value to a number, using a pre-computed string-value
    /// for the first node in a node-set.
    ///
    /// This is the correct conversion for node-sets when the caller has access
    /// to the `Document` and can supply the string-value of the first node.
    /// For non-node-set values, the `first_node_string_value` parameter is
    /// ignored.
    #[must_use]
    pub fn to_number_with_string_value(&self, first_node_string_value: Option<&str>) -> f64 {
        match self {
            Self::NodeSet(_) => first_node_string_value.map_or(f64::NAN, parse_xpath_number),
            _ => self.to_number(),
        }
    }

    /// Converts this value to a string per the `XPath` `string()` function.
    ///
    /// Conversion rules per `XPath` 1.0 section 4.2:
    /// - **string**: identity
    /// - **boolean**: `"true"` or `"false"`
    /// - **number**: formatted per `XPath` number-to-string rules (see
    ///   [`format_xpath_number`])
    /// - **node-set**: the string-value of the first node in document order;
    ///   an empty node-set yields the empty string
    ///
    /// Note: the node-set conversion requires a `Document` to compute
    /// string-values. Without one, we return the empty string for node-sets.
    /// For full conversion, use
    /// [`to_string_with_string_value`](Self::to_string_with_string_value).
    #[must_use]
    pub fn to_xpath_string(&self) -> String {
        match self {
            Self::String(s) => s.clone(),
            Self::Boolean(b) => {
                if *b {
                    "true".to_owned()
                } else {
                    "false".to_owned()
                }
            }
            Self::Number(n) => format_xpath_number(*n),
            Self::NodeSet(_) => {
                // Without a Document we cannot compute string-values.
                String::new()
            }
        }
    }

    /// Converts this value to a string, using a pre-computed string-value
    /// for the first node in a node-set.
    ///
    /// For non-node-set values the `first_node_string_value` parameter is
    /// ignored.
    #[must_use]
    pub fn to_string_with_string_value(&self, first_node_string_value: Option<&str>) -> String {
        match self {
            Self::NodeSet(_) => first_node_string_value.map_or_else(String::new, str::to_owned),
            _ => self.to_xpath_string(),
        }
    }

    /// Returns a reference to the inner node-set if this value is a
    /// `NodeSet`, or `None` otherwise.
    #[must_use]
    pub fn as_node_set(&self) -> Option<&Vec<XPathNode>> {
        match self {
            Self::NodeSet(nodes) => Some(nodes),
            _ => None,
        }
    }

    /// Returns a human-readable name for the type of this value.
    ///
    /// Useful for error messages.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Boolean(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::NodeSet(_) => "node-set",
        }
    }
}

impl fmt::Display for XPathValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean(b) => {
                if *b {
                    write!(f, "true")
                } else {
                    write!(f, "false")
                }
            }
            Self::Number(n) => write!(f, "{}", format_xpath_number(*n)),
            Self::String(s) => write!(f, "{s}"),
            Self::NodeSet(nodes) => write!(f, "<node-set of {} nodes>", nodes.len()),
        }
    }
}

impl PartialEq for XPathValue {
    #[allow(clippy::float_cmp)]
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Boolean(a), Self::Boolean(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => {
                // NaN != NaN per IEEE 754 and XPath spec
                a == b
            }
            (Self::String(a), Self::String(b)) => a == b,
            (Self::NodeSet(a), Self::NodeSet(b)) => a == b,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Number formatting (XPath 1.0 section 4.2, number-to-string conversion)
// ---------------------------------------------------------------------------

/// Formats an `f64` as a string per the `XPath` number-to-string rules.
///
/// Formatting rules per `XPath` 1.0 section 4.2 (the `string()` function
/// applied to numbers):
/// - NaN produces `"NaN"`
/// - Positive infinity produces `"Infinity"`
/// - Negative infinity produces `"-Infinity"`
/// - Negative zero produces `"0"` (not `"-0"`)
/// - If the number is an integer (no fractional part), it is formatted
///   without a decimal point (e.g., `1.0` becomes `"1"`)
/// - Otherwise, standard decimal notation is used with no trailing zeros
#[must_use]
pub fn format_xpath_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_owned();
    }
    if n.is_infinite() {
        return if n.is_sign_positive() {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        };
    }
    // Negative zero: XPath requires "0", not "-0".
    if n == 0.0 {
        return "0".to_owned();
    }
    // If the number is a mathematical integer, format without decimal point.
    // We check with fract() == 0.0 AND that the number is within the safe
    // integer range for f64 so that very large floats don't produce overly
    // long strings.
    #[allow(clippy::float_cmp, clippy::cast_possible_truncation)]
    if n.fract() == 0.0 && n.abs() < 1e18 {
        // Format as integer (no decimal point).
        return format!("{}", n as i64);
    }
    // General decimal formatting. Rust's default f64 Display uses enough
    // digits to round-trip, which matches XPath's requirement of producing
    // a string that converts back to the same number.
    format!("{n}")
}

// ---------------------------------------------------------------------------
// Number parsing (for string-to-number conversion)
// ---------------------------------------------------------------------------

/// Parses a string into an `XPath` number.
///
/// `XPath` 1.0 section 4.4 defines the `number()` function for strings:
/// the string is trimmed of leading/trailing whitespace and parsed as
/// an optional sign, digits, optional decimal point, and optional
/// fractional digits. Anything that does not match produces NaN.
///
/// Note: the `XPath` string-to-number conversion is stricter than Rust's
/// `f64::parse` in some respects (no hex, no exponent notation), but
/// more lenient in others (leading/trailing whitespace is allowed). We use
/// `f64::parse` on the trimmed string as a reasonable approximation.
fn parse_xpath_number(s: &str) -> f64 {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return f64::NAN;
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

// ---------------------------------------------------------------------------
// XPathError
// ---------------------------------------------------------------------------

/// An error that can occur during `XPath` expression parsing or evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XPathError {
    /// A type mismatch occurred (e.g., expected a node-set but got a string).
    TypeError {
        /// The type that was expected.
        expected: String,
        /// The type that was actually found.
        found: String,
    },

    /// A variable reference used a name that has no binding in the current
    /// context.
    UndefinedVariable {
        /// The name of the undefined variable (without the `$` prefix).
        name: String,
    },

    /// A function call used a name that is not a core `XPath` function and has
    /// no extension binding.
    UndefinedFunction {
        /// The name of the undefined function.
        name: String,
    },

    /// A function was called with the wrong number of arguments.
    InvalidArgCount {
        /// The name of the function.
        function: String,
        /// The number of arguments expected.
        expected: usize,
        /// The number of arguments that were actually provided.
        found: usize,
    },

    /// The `XPath` expression could not be parsed.
    InvalidExpression {
        /// A description of the parse error.
        message: String,
    },

    /// An unexpected internal error occurred during evaluation.
    InternalError {
        /// A description of the internal error.
        message: String,
    },
}

impl fmt::Display for XPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeError { expected, found } => {
                write!(f, "type error: expected {expected}, found {found}")
            }
            Self::UndefinedVariable { name } => {
                write!(f, "undefined variable: ${name}")
            }
            Self::UndefinedFunction { name } => {
                write!(f, "undefined function: {name}()")
            }
            Self::InvalidArgCount {
                function,
                expected,
                found,
            } => {
                write!(
                    f,
                    "invalid argument count for {function}(): \
                     expected {expected}, found {found}"
                )
            }
            Self::InvalidExpression { message } => {
                write!(f, "invalid XPath expression: {message}")
            }
            Self::InternalError { message } => {
                write!(f, "internal XPath error: {message}")
            }
        }
    }
}

impl std::error::Error for XPathError {}

impl From<super::lexer::XPathError> for XPathError {
    fn from(err: super::lexer::XPathError) -> Self {
        Self::InvalidExpression {
            message: err.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Comparison helpers (XPath 1.0 section 3.4)
// ---------------------------------------------------------------------------

/// Compares two values for equality using `XPath` 1.0 section 3.4 rules.
///
/// The comparison semantics depend on the types of the operands:
/// - If either operand is a boolean, the other is converted to boolean.
/// - If either operand is a number, the other is converted to a number.
/// - Otherwise, both are converted to strings.
///
/// Node-set comparisons are handled specially: when comparing a node-set to
/// a non-node-set value, the comparison is true if any node's string-value
/// satisfies the comparison against the other value.
///
/// This function does not handle node-set comparisons (which require access
/// to the document for string-value computation). For those, use
/// [`compare_values_with_string_values`].
///
/// Returns `None` if the comparison involves a node-set (since we cannot
/// compute string-values without a `Document`).
#[must_use]
pub fn compare_values_eq(lhs: &XPathValue, rhs: &XPathValue) -> Option<bool> {
    // Node-set comparisons require document access for string-values
    if matches!(lhs, XPathValue::NodeSet(_)) || matches!(rhs, XPathValue::NodeSet(_)) {
        return None;
    }
    Some(compare_non_nodeset_eq(lhs, rhs))
}

/// Compares two non-node-set values for equality.
///
/// Per `XPath` 1.0 section 3.4:
/// - If either is a boolean, compare as booleans.
/// - Else if either is a number, compare as numbers.
/// - Else compare as strings.
#[allow(clippy::float_cmp)]
fn compare_non_nodeset_eq(lhs: &XPathValue, rhs: &XPathValue) -> bool {
    // If either is boolean, compare as booleans.
    if matches!(lhs, XPathValue::Boolean(_)) || matches!(rhs, XPathValue::Boolean(_)) {
        return lhs.to_boolean() == rhs.to_boolean();
    }
    // If either is a number, compare as numbers.
    if matches!(lhs, XPathValue::Number(_)) || matches!(rhs, XPathValue::Number(_)) {
        let ln = lhs.to_number();
        let rn = rhs.to_number();
        return ln == rn;
    }
    // Otherwise compare as strings.
    lhs.to_xpath_string() == rhs.to_xpath_string()
}

/// Compares two values for equality, using pre-computed string-values
/// for nodes in node-sets.
///
/// `lhs_strings` and `rhs_strings` supply the string-values for each node
/// in the respective node-set (in the same order as the nodes appear in the
/// `NodeSet` vector). For non-node-set operands, the corresponding strings
/// parameter is ignored.
///
/// This implements the full `XPath` 1.0 section 3.4 equality semantics,
/// including node-set-to-node-set and node-set-to-scalar comparisons.
#[must_use]
#[allow(clippy::float_cmp)]
pub fn compare_values_with_string_values(
    lhs: &XPathValue,
    rhs: &XPathValue,
    lhs_strings: &[String],
    rhs_strings: &[String],
) -> bool {
    match (lhs, rhs) {
        // node-set = node-set: true if any pair of string-values are equal
        (XPathValue::NodeSet(_), XPathValue::NodeSet(_)) => {
            for ls in lhs_strings {
                for rs in rhs_strings {
                    if ls == rs {
                        return true;
                    }
                }
            }
            false
        }

        // node-set = boolean: convert node-set to boolean first
        (XPathValue::NodeSet(nodes), XPathValue::Boolean(b))
        | (XPathValue::Boolean(b), XPathValue::NodeSet(nodes)) => nodes.is_empty() != *b,

        // node-set = number: convert each node's string-value to a number
        (XPathValue::NodeSet(_), XPathValue::Number(n)) => {
            lhs_strings.iter().any(|sv| parse_xpath_number(sv) == *n)
        }
        (XPathValue::Number(n), XPathValue::NodeSet(_)) => {
            rhs_strings.iter().any(|sv| parse_xpath_number(sv) == *n)
        }

        // node-set = string: compare each node's string-value to the string
        (XPathValue::NodeSet(_), XPathValue::String(s)) => lhs_strings.iter().any(|sv| sv == s),
        (XPathValue::String(s), XPathValue::NodeSet(_)) => rhs_strings.iter().any(|sv| sv == s),

        // non-node-set comparisons
        _ => compare_non_nodeset_eq(lhs, rhs),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::tree::{Document, NodeKind};

    /// Helper to create a `NodeId` for testing purposes by allocating a real
    /// node in a document arena.
    fn make_node_id_in_doc(doc: &mut Document) -> NodeId {
        doc.create_node(NodeKind::Text {
            content: String::new(),
        })
    }

    // -- Boolean conversion -------------------------------------------------

    #[test]
    fn test_to_boolean_from_boolean() {
        assert!(XPathValue::Boolean(true).to_boolean());
        assert!(!XPathValue::Boolean(false).to_boolean());
    }

    #[test]
    fn test_to_boolean_from_number() {
        assert!(XPathValue::Number(1.0).to_boolean());
        assert!(XPathValue::Number(-1.0).to_boolean());
        assert!(XPathValue::Number(0.5).to_boolean());
        // Zero and NaN are false
        assert!(!XPathValue::Number(0.0).to_boolean());
        assert!(!XPathValue::Number(-0.0).to_boolean());
        assert!(!XPathValue::Number(f64::NAN).to_boolean());
    }

    #[test]
    fn test_to_boolean_from_string() {
        assert!(XPathValue::String("hello".to_owned()).to_boolean());
        assert!(XPathValue::String(" ".to_owned()).to_boolean());
        assert!(!XPathValue::String(String::new()).to_boolean());
    }

    #[test]
    fn test_to_boolean_from_nodeset() {
        let mut doc = Document::new();
        let id = make_node_id_in_doc(&mut doc);
        assert!(XPathValue::NodeSet(vec![id.into()]).to_boolean());
        assert!(!XPathValue::NodeSet(vec![]).to_boolean());
    }

    // -- Number conversion --------------------------------------------------

    #[test]
    fn test_to_number_from_number() {
        assert_eq!(XPathValue::Number(42.0).to_number(), 42.0);
        assert!(XPathValue::Number(f64::NAN).to_number().is_nan());
    }

    #[test]
    fn test_to_number_from_boolean() {
        assert_eq!(XPathValue::Boolean(true).to_number(), 1.0);
        assert_eq!(XPathValue::Boolean(false).to_number(), 0.0);
    }

    #[test]
    fn test_to_number_from_string() {
        assert_eq!(XPathValue::String("42".to_owned()).to_number(), 42.0);
        assert_eq!(XPathValue::String("  3.5  ".to_owned()).to_number(), 3.5);
        assert_eq!(XPathValue::String("-7".to_owned()).to_number(), -7.0);
        assert!(XPathValue::String("not a number".to_owned())
            .to_number()
            .is_nan());
        assert!(XPathValue::String(String::new()).to_number().is_nan());
    }

    #[test]
    fn test_to_number_from_nodeset_without_doc() {
        let mut doc = Document::new();
        let id = make_node_id_in_doc(&mut doc);
        assert!(XPathValue::NodeSet(vec![id.into()]).to_number().is_nan());
    }

    #[test]
    fn test_to_number_with_string_value() {
        let mut doc = Document::new();
        let id = make_node_id_in_doc(&mut doc);
        let val = XPathValue::NodeSet(vec![id.into()]);
        assert_eq!(val.to_number_with_string_value(Some("42")), 42.0);
        assert!(val.to_number_with_string_value(Some("abc")).is_nan());
        assert!(val.to_number_with_string_value(None).is_nan());
    }

    // -- String conversion --------------------------------------------------

    #[test]
    fn test_to_xpath_string_from_string() {
        assert_eq!(
            XPathValue::String("hello".to_owned()).to_xpath_string(),
            "hello"
        );
    }

    #[test]
    fn test_to_xpath_string_from_boolean() {
        assert_eq!(XPathValue::Boolean(true).to_xpath_string(), "true");
        assert_eq!(XPathValue::Boolean(false).to_xpath_string(), "false");
    }

    #[test]
    fn test_to_xpath_string_from_number() {
        assert_eq!(XPathValue::Number(1.0).to_xpath_string(), "1");
        assert_eq!(XPathValue::Number(-1.0).to_xpath_string(), "-1");
        assert_eq!(XPathValue::Number(0.0).to_xpath_string(), "0");
        assert_eq!(XPathValue::Number(1.5).to_xpath_string(), "1.5");
        assert_eq!(XPathValue::Number(f64::NAN).to_xpath_string(), "NaN");
        assert_eq!(
            XPathValue::Number(f64::INFINITY).to_xpath_string(),
            "Infinity"
        );
        assert_eq!(
            XPathValue::Number(f64::NEG_INFINITY).to_xpath_string(),
            "-Infinity"
        );
    }

    // -- Number formatting edge cases ---------------------------------------

    #[test]
    fn test_format_xpath_number_negative_zero() {
        // XPath requires that -0 formats as "0", not "-0"
        assert_eq!(format_xpath_number(-0.0), "0");
    }

    #[test]
    fn test_format_xpath_number_integers() {
        assert_eq!(format_xpath_number(0.0), "0");
        assert_eq!(format_xpath_number(1.0), "1");
        assert_eq!(format_xpath_number(-1.0), "-1");
        assert_eq!(format_xpath_number(100.0), "100");
        assert_eq!(format_xpath_number(999_999.0), "999999");
    }

    #[test]
    fn test_format_xpath_number_fractional() {
        assert_eq!(format_xpath_number(1.5), "1.5");
        assert_eq!(format_xpath_number(0.1), "0.1");
        assert_eq!(format_xpath_number(-2.75), "-2.75");
    }

    #[test]
    fn test_format_xpath_number_special_values() {
        assert_eq!(format_xpath_number(f64::NAN), "NaN");
        assert_eq!(format_xpath_number(f64::INFINITY), "Infinity");
        assert_eq!(format_xpath_number(f64::NEG_INFINITY), "-Infinity");
    }

    // -- as_node_set --------------------------------------------------------

    #[test]
    fn test_as_node_set() {
        let mut doc = Document::new();
        let id = make_node_id_in_doc(&mut doc);
        let ns = XPathValue::NodeSet(vec![id.into()]);
        assert!(ns.as_node_set().is_some());
        assert_eq!(ns.as_node_set().unwrap().len(), 1);

        assert!(XPathValue::Boolean(true).as_node_set().is_none());
        assert!(XPathValue::Number(1.0).as_node_set().is_none());
        assert!(XPathValue::String("x".to_owned()).as_node_set().is_none());
    }

    // -- type_name ----------------------------------------------------------

    #[test]
    fn test_type_name() {
        assert_eq!(XPathValue::Boolean(true).type_name(), "boolean");
        assert_eq!(XPathValue::Number(0.0).type_name(), "number");
        assert_eq!(XPathValue::String(String::new()).type_name(), "string");
        assert_eq!(XPathValue::NodeSet(vec![]).type_name(), "node-set");
    }

    // -- Display ------------------------------------------------------------

    #[test]
    fn test_display() {
        let mut doc = Document::new();
        let id1 = make_node_id_in_doc(&mut doc);
        let id2 = make_node_id_in_doc(&mut doc);

        assert_eq!(XPathValue::Boolean(true).to_string(), "true");
        assert_eq!(XPathValue::Boolean(false).to_string(), "false");
        assert_eq!(XPathValue::Number(42.0).to_string(), "42");
        assert_eq!(XPathValue::String("hi".to_owned()).to_string(), "hi");
        assert_eq!(
            XPathValue::NodeSet(vec![id1.into(), id2.into()]).to_string(),
            "<node-set of 2 nodes>"
        );
    }

    // -- PartialEq ----------------------------------------------------------

    #[test]
    fn test_partial_eq() {
        assert_eq!(XPathValue::Boolean(true), XPathValue::Boolean(true));
        assert_ne!(XPathValue::Boolean(true), XPathValue::Boolean(false));
        assert_eq!(XPathValue::Number(1.0), XPathValue::Number(1.0));
        // NaN != NaN
        assert_ne!(XPathValue::Number(f64::NAN), XPathValue::Number(f64::NAN));
        assert_eq!(
            XPathValue::String("a".to_owned()),
            XPathValue::String("a".to_owned())
        );
        // Different variants are never equal via PartialEq
        assert_ne!(XPathValue::Boolean(true), XPathValue::Number(1.0));
    }

    // -- Comparison helpers -------------------------------------------------

    #[test]
    fn test_compare_values_eq_booleans() {
        let t = XPathValue::Boolean(true);
        let f = XPathValue::Boolean(false);
        assert_eq!(compare_values_eq(&t, &t), Some(true));
        assert_eq!(compare_values_eq(&t, &f), Some(false));
    }

    #[test]
    fn test_compare_values_eq_boolean_coercion() {
        // When one operand is boolean, the other is converted to boolean
        let t = XPathValue::Boolean(true);
        let num = XPathValue::Number(42.0); // to_boolean() -> true
        assert_eq!(compare_values_eq(&t, &num), Some(true));

        let f = XPathValue::Boolean(false);
        let zero = XPathValue::Number(0.0); // to_boolean() -> false
        assert_eq!(compare_values_eq(&f, &zero), Some(true));
    }

    #[test]
    fn test_compare_values_eq_number_and_string() {
        // When one is number and other is string, convert string to number
        let num = XPathValue::Number(42.0);
        let s = XPathValue::String("42".to_owned());
        assert_eq!(compare_values_eq(&num, &s), Some(true));

        let bad = XPathValue::String("abc".to_owned());
        assert_eq!(compare_values_eq(&num, &bad), Some(false));
    }

    #[test]
    fn test_compare_values_eq_strings() {
        let a = XPathValue::String("hello".to_owned());
        let b = XPathValue::String("hello".to_owned());
        let c = XPathValue::String("world".to_owned());
        assert_eq!(compare_values_eq(&a, &b), Some(true));
        assert_eq!(compare_values_eq(&a, &c), Some(false));
    }

    #[test]
    fn test_compare_values_eq_with_nodeset_returns_none() {
        let mut doc = Document::new();
        let id = make_node_id_in_doc(&mut doc);
        let ns = XPathValue::NodeSet(vec![id.into()]);
        let s = XPathValue::String("x".to_owned());
        assert_eq!(compare_values_eq(&ns, &s), None);
        assert_eq!(compare_values_eq(&s, &ns), None);
    }

    #[test]
    fn test_compare_with_string_values_nodeset_to_string() {
        let mut doc = Document::new();
        let id1 = make_node_id_in_doc(&mut doc);
        let id2 = make_node_id_in_doc(&mut doc);
        let ns = XPathValue::NodeSet(vec![id1.into(), id2.into()]);
        let s = XPathValue::String("hello".to_owned());
        let node_strings = vec!["world".to_owned(), "hello".to_owned()];

        assert!(compare_values_with_string_values(
            &ns,
            &s,
            &node_strings,
            &[]
        ));
    }

    #[test]
    fn test_compare_with_string_values_nodeset_to_number() {
        let mut doc = Document::new();
        let id1 = make_node_id_in_doc(&mut doc);
        let id2 = make_node_id_in_doc(&mut doc);
        let ns = XPathValue::NodeSet(vec![id1.into(), id2.into()]);
        let n = XPathValue::Number(42.0);
        let node_strings = vec!["10".to_owned(), "42".to_owned()];

        assert!(compare_values_with_string_values(
            &ns,
            &n,
            &node_strings,
            &[]
        ));
    }

    #[test]
    fn test_compare_with_string_values_nodeset_to_boolean() {
        let mut doc = Document::new();
        let id = make_node_id_in_doc(&mut doc);
        let ns = XPathValue::NodeSet(vec![id.into()]);
        let b = XPathValue::Boolean(true);

        assert!(compare_values_with_string_values(&ns, &b, &[], &[]));

        let empty_ns = XPathValue::NodeSet(vec![]);
        assert!(!compare_values_with_string_values(&empty_ns, &b, &[], &[]));
    }

    #[test]
    fn test_compare_with_string_values_nodeset_to_nodeset() {
        let mut doc = Document::new();
        let id1 = make_node_id_in_doc(&mut doc);
        let id2 = make_node_id_in_doc(&mut doc);
        let ns1 = XPathValue::NodeSet(vec![id1.into()]);
        let ns2 = XPathValue::NodeSet(vec![id2.into()]);

        let strings1 = vec!["hello".to_owned()];
        let strings2 = vec!["hello".to_owned()];

        assert!(compare_values_with_string_values(
            &ns1, &ns2, &strings1, &strings2,
        ));

        let strings2_diff = vec!["world".to_owned()];
        assert!(!compare_values_with_string_values(
            &ns1,
            &ns2,
            &strings1,
            &strings2_diff,
        ));
    }

    // -- XPathError Display -------------------------------------------------

    #[test]
    fn test_xpath_error_display_type_error() {
        let err = XPathError::TypeError {
            expected: "node-set".to_owned(),
            found: "string".to_owned(),
        };
        assert_eq!(
            err.to_string(),
            "type error: expected node-set, found string"
        );
    }

    #[test]
    fn test_xpath_error_display_undefined_variable() {
        let err = XPathError::UndefinedVariable {
            name: "foo".to_owned(),
        };
        assert_eq!(err.to_string(), "undefined variable: $foo");
    }

    #[test]
    fn test_xpath_error_display_undefined_function() {
        let err = XPathError::UndefinedFunction {
            name: "my-func".to_owned(),
        };
        assert_eq!(err.to_string(), "undefined function: my-func()");
    }

    #[test]
    fn test_xpath_error_display_invalid_arg_count() {
        let err = XPathError::InvalidArgCount {
            function: "substring".to_owned(),
            expected: 2,
            found: 5,
        };
        assert_eq!(
            err.to_string(),
            "invalid argument count for substring(): expected 2, found 5"
        );
    }

    #[test]
    fn test_xpath_error_display_invalid_expression() {
        let err = XPathError::InvalidExpression {
            message: "unexpected token ')'".to_owned(),
        };
        assert_eq!(
            err.to_string(),
            "invalid XPath expression: unexpected token ')'"
        );
    }

    #[test]
    fn test_xpath_error_display_internal_error() {
        let err = XPathError::InternalError {
            message: "stack overflow".to_owned(),
        };
        assert_eq!(err.to_string(), "internal XPath error: stack overflow");
    }

    #[test]
    fn test_xpath_error_is_error_trait() {
        let err = XPathError::TypeError {
            expected: "boolean".to_owned(),
            found: "number".to_owned(),
        };
        let _: &dyn std::error::Error = &err;
    }

    // -- to_string_with_string_value ----------------------------------------

    #[test]
    fn test_to_string_with_string_value_nodeset() {
        let mut doc = Document::new();
        let id = make_node_id_in_doc(&mut doc);
        let val = XPathValue::NodeSet(vec![id.into()]);
        assert_eq!(val.to_string_with_string_value(Some("hello")), "hello");
        assert_eq!(val.to_string_with_string_value(None), "");
    }

    #[test]
    fn test_to_string_with_string_value_non_nodeset() {
        let val = XPathValue::Number(42.0);
        // The string value parameter is ignored for non-node-set values
        assert_eq!(val.to_string_with_string_value(Some("ignored")), "42");
    }
}
