//! Abstract syntax tree types for `XPath` 1.0 expressions.
//!
//! This module defines the AST that results from parsing an `XPath` expression
//! string. The AST closely follows the `XPath` 1.0 grammar from
//! <https://www.w3.org/TR/xpath-10/#section-Basics>.
//!
//! The primary type is [`Expr`], which represents any `XPath` expression.
//! Location paths are composed of [`Step`]s, each having an [`Axis`],
//! a [`NodeTest`], and zero or more predicate expressions.

/// An `XPath` 1.0 expression.
///
/// This enum represents the full range of `XPath` 1.0 expressions, including
/// literals, operators, function calls, and location paths.
///
/// See `XPath` 1.0 section 3.
#[derive(Debug, Clone)]
pub enum Expr {
    /// A numeric literal (e.g., `42`, `3.14`).
    ///
    /// See `XPath` 1.0 section 3.5.
    Number(f64),

    /// A string literal (e.g., `"hello"` or `'world'`).
    ///
    /// See `XPath` 1.0 section 3.5.
    String(String),

    /// A variable reference (e.g., `$foo`).
    ///
    /// The string contains the variable name without the leading `$`.
    ///
    /// See `XPath` 1.0 section 3.1.
    Variable(String),

    /// A binary operation (e.g., `a + b`, `x = y`, `p and q`).
    ///
    /// See `XPath` 1.0 sections 3.3, 3.4, 3.5.
    BinaryOp {
        /// The operator.
        op: BinaryOp,
        /// The left-hand operand.
        left: Box<Expr>,
        /// The right-hand operand.
        right: Box<Expr>,
    },

    /// Unary negation (e.g., `-x`).
    ///
    /// See `XPath` 1.0 section 3.5.
    UnaryNeg(Box<Expr>),

    /// A function call (e.g., `contains(name, 'foo')`).
    ///
    /// See `XPath` 1.0 section 3.2.
    FunctionCall {
        /// The function name.
        name: String,
        /// The argument expressions.
        args: Vec<Expr>,
    },

    /// A relative location path (e.g., `child::p/child::a`).
    ///
    /// See `XPath` 1.0 section 2.
    Path {
        /// The steps in the path, evaluated left to right.
        steps: Vec<Step>,
    },

    /// An absolute location path (e.g., `/html/body`).
    ///
    /// An empty `steps` vector represents the bare `/` (root node).
    ///
    /// See `XPath` 1.0 section 2.
    RootPath {
        /// The steps following the initial `/`.
        steps: Vec<Step>,
    },

    /// A filter expression with predicates (e.g., `$nodes[1]`).
    ///
    /// See `XPath` 1.0 section 3.3.
    Filter {
        /// The primary expression being filtered.
        expr: Box<Expr>,
        /// The predicate expressions.
        predicates: Vec<Expr>,
    },

    /// A filter expression followed by a relative location path
    /// (e.g., `(//a)[1]/@href` or `$nodes//b`).
    ///
    /// See `XPath` 1.0 section 3.3 (`PathExpr`).
    FilterPath {
        /// The filter expression producing the initial node-set.
        expr: Box<Expr>,
        /// The location path steps applied to that node-set.
        steps: Vec<Step>,
    },

    /// A union of two node-sets (e.g., `a | b`).
    ///
    /// See `XPath` 1.0 section 3.3.
    Union(Box<Expr>, Box<Expr>),
}

/// A binary operator in an `XPath` expression.
///
/// Covers arithmetic, comparison, and logical operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    /// Addition (`+`).
    Add,
    /// Subtraction (`-`).
    Sub,
    /// Multiplication (`*`).
    Mul,
    /// Division (`div`).
    Div,
    /// Modulo (`mod`).
    Mod,
    /// Equality (`=`).
    Eq,
    /// Inequality (`!=`).
    Neq,
    /// Less than (`<`).
    Lt,
    /// Less than or equal (`<=`).
    Lte,
    /// Greater than (`>`).
    Gt,
    /// Greater than or equal (`>=`).
    Gte,
    /// Logical and (`and`).
    And,
    /// Logical or (`or`).
    Or,
}

/// A single step in a location path.
///
/// A step consists of an axis, a node test, and zero or more predicates.
/// For example, in `child::p[@class='intro']`, the axis is `Child`,
/// the node test is `Name("p")`, and there is one predicate.
///
/// See `XPath` 1.0 section 2.1.
#[derive(Debug, Clone)]
pub struct Step {
    /// The axis along which to select nodes.
    pub axis: Axis,
    /// The test applied to each candidate node.
    pub node_test: NodeTest,
    /// Predicate expressions that further filter the selected nodes.
    pub predicates: Vec<Expr>,
}

/// An `XPath` axis, specifying the direction of node selection.
///
/// `XPath` 1.0 defines 13 axes. See `XPath` 1.0 section 2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// The `child` axis: direct children.
    Child,
    /// The `descendant` axis: all descendants (children, grandchildren, etc.).
    Descendant,
    /// The `parent` axis: the immediate parent.
    Parent,
    /// The `ancestor` axis: all ancestors up to and including the root.
    Ancestor,
    /// The `following-sibling` axis: siblings that come after this node.
    FollowingSibling,
    /// The `preceding-sibling` axis: siblings that come before this node.
    PrecedingSibling,
    /// The `following` axis: all nodes after this node in document order.
    Following,
    /// The `preceding` axis: all nodes before this node in document order.
    Preceding,
    /// The `attribute` axis: attributes of the context node.
    Attribute,
    /// The `namespace` axis: namespace nodes of the context node.
    Namespace,
    /// The `self` axis: just the context node itself.
    Self_,
    /// The `descendant-or-self` axis: the context node and its descendants.
    DescendantOrSelf,
    /// The `ancestor-or-self` axis: the context node and its ancestors.
    AncestorOrSelf,
}

impl Axis {
    /// Returns the axis name as it appears in `XPath` syntax.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(Axis::Child.as_str(), "child");
    /// assert_eq!(Axis::DescendantOrSelf.as_str(), "descendant-or-self");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Child => "child",
            Self::Descendant => "descendant",
            Self::Parent => "parent",
            Self::Ancestor => "ancestor",
            Self::FollowingSibling => "following-sibling",
            Self::PrecedingSibling => "preceding-sibling",
            Self::Following => "following",
            Self::Preceding => "preceding",
            Self::Attribute => "attribute",
            Self::Namespace => "namespace",
            Self::Self_ => "self",
            Self::DescendantOrSelf => "descendant-or-self",
            Self::AncestorOrSelf => "ancestor-or-self",
        }
    }

    /// Parses an axis name string into an `Axis` variant.
    ///
    /// Returns `None` if the string is not a recognized axis name.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "child" => Some(Self::Child),
            "descendant" => Some(Self::Descendant),
            "parent" => Some(Self::Parent),
            "ancestor" => Some(Self::Ancestor),
            "following-sibling" => Some(Self::FollowingSibling),
            "preceding-sibling" => Some(Self::PrecedingSibling),
            "following" => Some(Self::Following),
            "preceding" => Some(Self::Preceding),
            "attribute" => Some(Self::Attribute),
            "namespace" => Some(Self::Namespace),
            "self" => Some(Self::Self_),
            "descendant-or-self" => Some(Self::DescendantOrSelf),
            "ancestor-or-self" => Some(Self::AncestorOrSelf),
            _ => None,
        }
    }
}

impl std::fmt::Display for Axis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A node test in a location path step.
///
/// Node tests filter candidate nodes by name or kind.
///
/// See `XPath` 1.0 section 2.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeTest {
    /// A name test matching a specific element or attribute name.
    ///
    /// The string may be an `NCName` or a `QName` (with prefix).
    Name(String),

    /// The `*` wildcard, matching any name.
    Wildcard,

    /// A prefixed wildcard like `prefix:*`, matching any local name in
    /// the namespace bound to the given prefix.
    PrefixWildcard(String),

    /// The `node()` node type test, matching any node.
    Node,

    /// The `text()` node type test, matching text nodes.
    Text,

    /// The `comment()` node type test, matching comment nodes.
    Comment,

    /// The `processing-instruction()` node type test.
    ///
    /// When the optional string is `Some`, it matches only PIs with that target
    /// name (e.g., `processing-instruction('xml-stylesheet')`).
    ProcessingInstruction(Option<String>),
}

impl std::fmt::Display for NodeTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name(name) => write!(f, "{name}"),
            Self::Wildcard => f.write_str("*"),
            Self::PrefixWildcard(prefix) => write!(f, "{prefix}:*"),
            Self::Node => f.write_str("node()"),
            Self::Text => f.write_str("text()"),
            Self::Comment => f.write_str("comment()"),
            Self::ProcessingInstruction(None) => f.write_str("processing-instruction()"),
            Self::ProcessingInstruction(Some(name)) => {
                write!(f, "processing-instruction('{name}')")
            }
        }
    }
}

impl std::fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Add => f.write_str("+"),
            Self::Sub => f.write_str("-"),
            Self::Mul => f.write_str("*"),
            Self::Div => f.write_str("div"),
            Self::Mod => f.write_str("mod"),
            Self::Eq => f.write_str("="),
            Self::Neq => f.write_str("!="),
            Self::Lt => f.write_str("<"),
            Self::Lte => f.write_str("<="),
            Self::Gt => f.write_str(">"),
            Self::Gte => f.write_str(">="),
            Self::And => f.write_str("and"),
            Self::Or => f.write_str("or"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_axis_roundtrip() {
        let axes = [
            Axis::Child,
            Axis::Descendant,
            Axis::Parent,
            Axis::Ancestor,
            Axis::FollowingSibling,
            Axis::PrecedingSibling,
            Axis::Following,
            Axis::Preceding,
            Axis::Attribute,
            Axis::Namespace,
            Axis::Self_,
            Axis::DescendantOrSelf,
            Axis::AncestorOrSelf,
        ];
        for axis in axes {
            let name = axis.as_str();
            let parsed = Axis::parse(name);
            assert_eq!(parsed, Some(axis), "roundtrip failed for {name}");
        }
    }

    #[test]
    fn test_axis_from_str_invalid() {
        assert_eq!(Axis::parse("invalid"), None);
        assert_eq!(Axis::parse(""), None);
        assert_eq!(Axis::parse("children"), None);
    }

    #[test]
    fn test_axis_display() {
        assert_eq!(Axis::Child.to_string(), "child");
        assert_eq!(Axis::DescendantOrSelf.to_string(), "descendant-or-self");
        assert_eq!(Axis::AncestorOrSelf.to_string(), "ancestor-or-self");
        assert_eq!(Axis::FollowingSibling.to_string(), "following-sibling");
    }

    #[test]
    fn test_node_test_display() {
        assert_eq!(NodeTest::Name("foo".to_string()).to_string(), "foo");
        assert_eq!(NodeTest::Wildcard.to_string(), "*");
        assert_eq!(
            NodeTest::PrefixWildcard("svg".to_string()).to_string(),
            "svg:*"
        );
        assert_eq!(NodeTest::Node.to_string(), "node()");
        assert_eq!(NodeTest::Text.to_string(), "text()");
        assert_eq!(NodeTest::Comment.to_string(), "comment()");
        assert_eq!(
            NodeTest::ProcessingInstruction(None).to_string(),
            "processing-instruction()"
        );
        assert_eq!(
            NodeTest::ProcessingInstruction(Some("xml-stylesheet".to_string())).to_string(),
            "processing-instruction('xml-stylesheet')"
        );
    }

    #[test]
    fn test_binary_op_display() {
        assert_eq!(BinaryOp::Add.to_string(), "+");
        assert_eq!(BinaryOp::Sub.to_string(), "-");
        assert_eq!(BinaryOp::Mul.to_string(), "*");
        assert_eq!(BinaryOp::Div.to_string(), "div");
        assert_eq!(BinaryOp::Mod.to_string(), "mod");
        assert_eq!(BinaryOp::Eq.to_string(), "=");
        assert_eq!(BinaryOp::Neq.to_string(), "!=");
        assert_eq!(BinaryOp::Lt.to_string(), "<");
        assert_eq!(BinaryOp::Lte.to_string(), "<=");
        assert_eq!(BinaryOp::Gt.to_string(), ">");
        assert_eq!(BinaryOp::Gte.to_string(), ">=");
        assert_eq!(BinaryOp::And.to_string(), "and");
        assert_eq!(BinaryOp::Or.to_string(), "or");
    }

    #[test]
    fn test_expr_clone() {
        let expr = Expr::BinaryOp {
            op: BinaryOp::Add,
            left: Box::new(Expr::Number(1.0)),
            right: Box::new(Expr::Number(2.0)),
        };
        let cloned = expr.clone();
        match cloned {
            Expr::BinaryOp { op, .. } => assert_eq!(op, BinaryOp::Add),
            _ => panic!("unexpected variant after clone"),
        }
    }

    #[test]
    fn test_step_construction() {
        let step = Step {
            axis: Axis::Child,
            node_test: NodeTest::Name("p".to_string()),
            predicates: vec![Expr::Number(1.0)],
        };
        assert_eq!(step.axis, Axis::Child);
        assert_eq!(step.node_test, NodeTest::Name("p".to_string()));
        assert_eq!(step.predicates.len(), 1);
    }
}
