//! `XPath` 1.0 expression evaluator.
//!
//! This module implements the core evaluation engine for `XPath` 1.0 expressions
//! as specified in <https://www.w3.org/TR/xpath-10/>. It walks an [`Expr`] AST
//! (produced by [`super::parser::parse`]) and evaluates it against a
//! [`Document`] tree, producing an [`XPathValue`].
//!
//! # Evaluation Context
//!
//! Per `XPath` 1.0 section 1, every expression is evaluated with respect to a
//! **context** consisting of a context node, context position, context size,
//! variable bindings, and a function library. The [`XPathContext`] struct holds
//! all of these.
//!
//! # Location Paths
//!
//! The evaluator supports all 13 `XPath` axes, node test filtering, and
//! predicate evaluation. Results are maintained in document order.
//!
//! # Functions
//!
//! All 27 core `XPath` 1.0 functions are implemented (node-set, string,
//! boolean, and number function groups).

use std::collections::{HashMap, HashSet};

use super::ast::{Axis, BinaryOp, Expr, NodeTest, Step};
use super::types::{XPathError, XPathNode};
use crate::tree::{Attribute, Document, NodeId, NodeKind};

/// Evaluation context for an `XPath` 1.0 expression.
///
/// Holds the document reference, the context node, context position and size,
/// and variable bindings. Created via [`XPathContext::new`] and then used to
/// evaluate parsed `XPath` expressions via [`XPathContext::evaluate`].
///
/// # Examples
///
/// ```ignore
/// use xmloxide::xpath::eval::XPathContext;
/// use xmloxide::xpath::parser::parse;
/// use xmloxide::Document;
///
/// let doc = Document::parse_str("<root><a/><b/></root>").unwrap();
/// let root = doc.root_element().unwrap();
/// let expr = parse("count(*)").unwrap();
/// let ctx = XPathContext::new(&doc, root);
/// let result = ctx.evaluate(&expr).unwrap();
/// ```
pub struct XPathContext<'a> {
    /// The document being queried.
    doc: &'a Document,
    /// The context node for expression evaluation (may be an attribute node
    /// when evaluating predicates over attribute-axis results).
    context_node: XPathNode,
    /// 1-based position of the context node within its context node-set.
    context_position: usize,
    /// The size of the context node-set.
    context_size: usize,
    /// Variable bindings available during evaluation.
    variables: HashMap<String, XPathValue>,
    /// Namespace prefix → URI bindings for resolving prefixed name tests.
    ///
    /// When set (e.g., via Schematron `<sch:ns>` bindings), prefixed names
    /// like `inv:invoice` in `XPath` expressions are matched by resolving
    /// the prefix to a URI and comparing against the element's namespace URI
    /// and local name.
    namespaces: HashMap<String, String>,
}

/// An `XPath` 1.0 value.
///
/// Re-exported locally to keep the evaluator self-contained. This mirrors
/// [`super::types::XPathValue`] but uses `String` payloads computed within
/// the evaluator for node-set string-value access.
pub use super::types::XPathValue;

impl<'a> XPathContext<'a> {
    /// Creates a new evaluation context rooted at `context_node`.
    ///
    /// The context position and size are both set to 1 (as if the context
    /// node is the only member of a singleton node-set).
    #[must_use]
    pub fn new(doc: &'a Document, context_node: NodeId) -> Self {
        Self::new_at(doc, XPathNode::Node(context_node))
    }

    /// Creates a new evaluation context at an arbitrary `XPath` node,
    /// including an attribute node.
    ///
    /// Used when the context node comes from a previous `XPath` evaluation
    /// (e.g., a Schematron rule whose context expression selects
    /// attributes).
    #[must_use]
    pub fn new_at(doc: &'a Document, context_node: XPathNode) -> Self {
        Self {
            doc,
            context_node,
            context_position: 1,
            context_size: 1,
            variables: HashMap::new(),
            namespaces: HashMap::new(),
        }
    }

    /// Looks up an attribute node's underlying [`Attribute`] data.
    fn attr(&self, owner: NodeId, index: u32) -> Option<&Attribute> {
        self.doc.attributes(owner).get(index as usize)
    }

    /// Registers a namespace prefix → URI binding for name resolution.
    ///
    /// When evaluating `XPath` expressions containing prefixed name tests
    /// (e.g., `//inv:invoice`), the prefix is resolved to a URI using these
    /// bindings, and the element's namespace URI and local name are compared
    /// instead of the raw `QName` string.
    ///
    /// This is used by Schematron validation to pass `<sch:ns>` bindings
    /// to the `XPath` evaluator.
    pub fn set_namespace(&mut self, prefix: &str, uri: &str) {
        self.namespaces.insert(prefix.to_owned(), uri.to_owned());
    }

    /// Binds a variable name to a value in this context.
    ///
    /// Variable references in expressions (e.g., `$x`) will resolve to the
    /// value set here.
    pub fn set_variable(&mut self, name: &str, value: XPathValue) {
        self.variables.insert(name.to_owned(), value);
    }

    /// Evaluates an `XPath` expression AST against this context.
    ///
    /// # Errors
    ///
    /// Returns [`XPathError`] if evaluation fails (e.g., undefined variable,
    /// unknown function, type mismatch).
    pub fn evaluate(&self, expr: &Expr) -> Result<XPathValue, XPathError> {
        self.eval_expr(expr)
    }

    // -----------------------------------------------------------------------
    // Internal expression dispatch
    // -----------------------------------------------------------------------

    fn eval_expr(&self, expr: &Expr) -> Result<XPathValue, XPathError> {
        match expr {
            Expr::Number(n) => Ok(XPathValue::Number(*n)),
            Expr::String(s) => Ok(XPathValue::String(s.clone())),
            Expr::Variable(name) => self.eval_variable(name),
            Expr::BinaryOp { op, left, right } => self.eval_binary_op(*op, left, right),
            Expr::UnaryNeg(inner) => self.eval_unary_neg(inner),
            Expr::FunctionCall { name, args } => self.eval_function(name, args),
            Expr::Path { steps } => self.eval_relative_path(steps),
            Expr::RootPath { steps } => self.eval_root_path(steps),
            Expr::Filter { expr, predicates } => self.eval_filter(expr, predicates),
            Expr::FilterPath { expr, steps } => self.eval_filter_path(expr, steps),
            Expr::Union(left, right) => self.eval_union(left, right),
        }
    }

    // -----------------------------------------------------------------------
    // Variable lookup
    // -----------------------------------------------------------------------

    fn eval_variable(&self, name: &str) -> Result<XPathValue, XPathError> {
        self.variables
            .get(name)
            .cloned()
            .ok_or_else(|| XPathError::UndefinedVariable {
                name: name.to_owned(),
            })
    }

    // -----------------------------------------------------------------------
    // Binary operations
    // -----------------------------------------------------------------------

    fn eval_binary_op(
        &self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<XPathValue, XPathError> {
        match op {
            BinaryOp::And => {
                let lv = self.eval_expr(left)?;
                if !self.value_to_boolean(&lv) {
                    return Ok(XPathValue::Boolean(false));
                }
                let rv = self.eval_expr(right)?;
                Ok(XPathValue::Boolean(self.value_to_boolean(&rv)))
            }
            BinaryOp::Or => {
                let lv = self.eval_expr(left)?;
                if self.value_to_boolean(&lv) {
                    return Ok(XPathValue::Boolean(true));
                }
                let rv = self.eval_expr(right)?;
                Ok(XPathValue::Boolean(self.value_to_boolean(&rv)))
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                let lv = self.eval_expr(left)?;
                let rv = self.eval_expr(right)?;
                let ln = self.value_to_number(&lv);
                let rn = self.value_to_number(&rv);
                let result = match op {
                    BinaryOp::Add => ln + rn,
                    BinaryOp::Sub => ln - rn,
                    BinaryOp::Mul => ln * rn,
                    BinaryOp::Div => ln / rn,
                    BinaryOp::Mod => ln % rn,
                    _ => unreachable!(),
                };
                Ok(XPathValue::Number(result))
            }
            BinaryOp::Eq | BinaryOp::Neq => {
                let lv = self.eval_expr(left)?;
                let rv = self.eval_expr(right)?;
                Ok(XPathValue::Boolean(self.compare_equality(op, &lv, &rv)))
            }
            BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
                let lv = self.eval_expr(left)?;
                let rv = self.eval_expr(right)?;
                let result = self.compare_relational(op, &lv, &rv);
                Ok(XPathValue::Boolean(result))
            }
        }
    }

    // -----------------------------------------------------------------------
    // Unary negation
    // -----------------------------------------------------------------------

    fn eval_unary_neg(&self, inner: &Expr) -> Result<XPathValue, XPathError> {
        let val = self.eval_expr(inner)?;
        Ok(XPathValue::Number(-self.value_to_number(&val)))
    }

    // -----------------------------------------------------------------------
    // Location paths
    // -----------------------------------------------------------------------

    fn eval_relative_path(&self, steps: &[Step]) -> Result<XPathValue, XPathError> {
        self.eval_steps_from(vec![self.context_node], steps)
    }

    /// Applies location path `steps` to an initial node-set.
    ///
    /// Shared by relative paths, absolute paths, and filter-path expressions
    /// (e.g., `(//a)[1]/@href`). Handles the
    /// `descendant-or-self::node()/child::X` fusion.
    fn eval_steps_from(
        &self,
        mut nodes: Vec<XPathNode>,
        steps: &[Step],
    ) -> Result<XPathValue, XPathError> {
        let mut i = 0;
        while i < steps.len() {
            let step = &steps[i];
            // Optimization: fuse descendant-or-self::node()/child::X into
            // descendant::X — avoids materializing the huge intermediate
            // node-set that `//` produces.
            if let Some(fused) = Self::try_fuse_descendant_child(steps, i) {
                nodes = self.apply_step(&nodes, &fused)?;
                i += 2; // skip both steps
            } else {
                nodes = self.apply_step(&nodes, step)?;
                i += 1;
            }
        }
        Ok(XPathValue::NodeSet(nodes))
    }

    /// Evaluates a filter expression followed by a relative location path,
    /// e.g., `(//a)[1]/@href`: the filter yields a node-set, and the steps
    /// are applied to it like any other location path continuation.
    fn eval_filter_path(&self, expr: &Expr, steps: &[Step]) -> Result<XPathValue, XPathError> {
        let val = self.eval_expr(expr)?;
        let nodes = match val {
            XPathValue::NodeSet(ns) => ns,
            other => {
                return Err(XPathError::TypeError {
                    expected: "node-set".to_owned(),
                    found: other.type_name().to_owned(),
                });
            }
        };
        self.eval_steps_from(nodes, steps)
    }

    /// Tries to fuse `descendant-or-self::node()` + `child::X` at position `i`
    /// into a single `descendant::X` step when the child step has no
    /// predicates. Predicates must retain their per-parent proximity context,
    /// so fusing `//*[1]` would be observably wrong.
    fn try_fuse_descendant_child(steps: &[Step], i: usize) -> Option<Step> {
        if i + 1 >= steps.len() {
            return None;
        }
        let first = &steps[i];
        let second = &steps[i + 1];
        // Pattern: descendant-or-self::node() with no predicates,
        // followed by child::X (with or without predicates).
        if first.axis == Axis::DescendantOrSelf
            && first.node_test == NodeTest::Node
            && first.predicates.is_empty()
            && second.axis == Axis::Child
            && second.predicates.is_empty()
        {
            Some(Step {
                axis: Axis::Descendant,
                node_test: second.node_test.clone(),
                predicates: second.predicates.clone(),
            })
        } else {
            None
        }
    }

    fn eval_root_path(&self, steps: &[Step]) -> Result<XPathValue, XPathError> {
        self.eval_steps_from(vec![XPathNode::Node(self.doc.root())], steps)
    }

    /// Applies a single step to every node in `input`, producing a new node
    /// set in document order with duplicates removed.
    fn apply_step(&self, input: &[XPathNode], step: &Step) -> Result<Vec<XPathNode>, XPathError> {
        let mut result: Vec<XPathNode> = Vec::new();

        if step.predicates.is_empty() {
            if input.len() == 1 {
                // Fast path: single input node — no dedup needed.
                self.expand_axis_filtered(input[0], step.axis, &step.node_test, &mut result);
            } else {
                // Multiple input nodes — need dedup via HashSet.
                let mut seen = HashSet::new();
                for &node in input {
                    self.expand_axis_filtered_dedup(
                        node,
                        step.axis,
                        &step.node_test,
                        &mut result,
                        &mut seen,
                    );
                }
            }
        } else {
            // Predicates filter the node-set generated for EACH context node
            // separately, with proximity positions relative to that
            // per-context set (XPath 1.0 §2.4/§3.3): //a/b[1] selects the
            // first b of every a. The per-context results are then merged
            // with duplicates removed.
            let mut seen: HashSet<XPathNode> = HashSet::new();
            for &node in input {
                let mut per_node = Vec::new();
                self.expand_axis_filtered(node, step.axis, &step.node_test, &mut per_node);
                for pred in &step.predicates {
                    per_node = self.apply_predicate(&per_node, pred)?;
                }
                if input.len() == 1 {
                    result = per_node;
                    break;
                }
                for n in per_node {
                    if seen.insert(n) {
                        result.push(n);
                    }
                }
            }
        }

        // Each context node contributes a separately evaluated axis set. Their
        // concatenation is not necessarily document ordered (notably the
        // child step after `//`), while libxml returns node-sets in document
        // order. Reverse-axis proximity was already applied above.
        sort_document_order(&mut result);

        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Fused axis expansion + node test filtering
    // -----------------------------------------------------------------------

    /// Expands axis from `node`, filters by `test`, and pushes matching nodes
    /// directly into `result`. No intermediate Vec allocation.
    #[allow(clippy::too_many_lines)]
    fn expand_axis_filtered(
        &self,
        node: XPathNode,
        axis: Axis,
        test: &NodeTest,
        result: &mut Vec<XPathNode>,
    ) {
        let node = match node {
            XPathNode::Attribute { owner, index } => {
                self.expand_axis_from_attribute(owner, index, axis, test, result);
                return;
            }
            XPathNode::Node(id) => id,
        };
        if axis == Axis::Attribute {
            self.push_matching_attributes(node, test, result);
            return;
        }
        if axis == Axis::Namespace {
            result.extend(
                self.apply_namespace_node_test(node, test)
                    .into_iter()
                    .map(XPathNode::Node),
            );
            return;
        }

        // Fast path for Name test on element-scanning axes: inline the name
        // check to avoid per-node function call overhead through
        // node_matches_test. This is the hottest path for queries like
        // //entry/title or /root/child.
        //
        // Only use the fast path when no namespace bindings are registered,
        // because namespace-aware matching needs the full matches_element_name
        // logic.
        if let NodeTest::Name(name) = test {
            if self.namespaces.is_empty() {
                match axis {
                    Axis::Child => {
                        for child in self.doc.children(node) {
                            if let NodeKind::Element {
                                name: elem_name, ..
                            } = &self.doc.node(child).kind
                            {
                                if elem_name == name {
                                    result.push(XPathNode::Node(child));
                                }
                            }
                        }
                        return;
                    }
                    Axis::Descendant => {
                        for desc in self.doc.descendants(node) {
                            if let NodeKind::Element {
                                name: elem_name, ..
                            } = &self.doc.node(desc).kind
                            {
                                if elem_name == name {
                                    result.push(XPathNode::Node(desc));
                                }
                            }
                        }
                        return;
                    }
                    Axis::DescendantOrSelf => {
                        if let NodeKind::Element {
                            name: elem_name, ..
                        } = &self.doc.node(node).kind
                        {
                            if elem_name == name {
                                result.push(XPathNode::Node(node));
                            }
                        }
                        for desc in self.doc.descendants(node) {
                            if let NodeKind::Element {
                                name: elem_name, ..
                            } = &self.doc.node(desc).kind
                            {
                                if elem_name == name {
                                    result.push(XPathNode::Node(desc));
                                }
                            }
                        }
                        return;
                    }
                    _ => {} // fall through to generic path
                }
            }
        }

        match axis {
            Axis::Child => {
                for child in self.doc.children(node) {
                    if self.node_matches_test(child, test, axis) {
                        result.push(XPathNode::Node(child));
                    }
                }
            }
            Axis::Descendant => {
                for desc in self.doc.descendants(node) {
                    if self.node_matches_test(desc, test, axis) {
                        result.push(XPathNode::Node(desc));
                    }
                }
            }
            Axis::DescendantOrSelf => {
                if self.node_matches_test(node, test, axis) {
                    result.push(XPathNode::Node(node));
                }
                for desc in self.doc.descendants(node) {
                    if self.node_matches_test(desc, test, axis) {
                        result.push(XPathNode::Node(desc));
                    }
                }
            }
            Axis::Self_ => {
                if self.node_matches_test(node, test, axis) {
                    result.push(XPathNode::Node(node));
                }
            }
            Axis::Parent => {
                if let Some(p) = self.doc.parent(node) {
                    if self.node_matches_test(p, test, axis) {
                        result.push(XPathNode::Node(p));
                    }
                }
            }
            _ => {
                // For less common axes, fall back to expand + filter
                let axis_nodes = self.expand_axis(node, axis);
                for id in axis_nodes {
                    if self.node_matches_test(id, test, axis) {
                        result.push(XPathNode::Node(id));
                    }
                }
            }
        }
    }

    /// Like `expand_axis_filtered` but with deduplication via `seen` `HashSet`.
    fn expand_axis_filtered_dedup(
        &self,
        node: XPathNode,
        axis: Axis,
        test: &NodeTest,
        result: &mut Vec<XPathNode>,
        seen: &mut HashSet<XPathNode>,
    ) {
        // Expand into a scratch vec, then dedup-push. Axis expansion from a
        // single node never yields duplicates, so dedup is only needed
        // across input nodes.
        let mut scratch = Vec::new();
        self.expand_axis_filtered(node, axis, test, &mut scratch);
        for n in scratch {
            if seen.insert(n) {
                result.push(n);
            }
        }
    }

    /// Expands an axis from an attribute node (`XPath` 1.0 section 2.2):
    /// the owner element is the attribute's parent, and attributes have no
    /// children and are not siblings of any node.
    fn expand_axis_from_attribute(
        &self,
        owner: NodeId,
        index: u32,
        axis: Axis,
        test: &NodeTest,
        result: &mut Vec<XPathNode>,
    ) {
        match axis {
            Axis::Parent if self.node_matches_test(owner, test, axis) => {
                result.push(XPathNode::Node(owner));
            }
            Axis::Ancestor => {
                let mut current = Some(owner);
                while let Some(n) = current {
                    if self.node_matches_test(n, test, axis) {
                        result.push(XPathNode::Node(n));
                    }
                    current = self.doc.parent(n);
                }
            }
            Axis::Self_ | Axis::AncestorOrSelf | Axis::DescendantOrSelf => {
                // The principal node type of these axes is element, so only
                // node() matches the attribute itself (XPath 1.0 §2.3).
                if *test == NodeTest::Node {
                    result.push(XPathNode::Attribute { owner, index });
                }
                if axis == Axis::AncestorOrSelf {
                    let mut current = Some(owner);
                    while let Some(n) = current {
                        if self.node_matches_test(n, test, axis) {
                            result.push(XPathNode::Node(n));
                        }
                        current = self.doc.parent(n);
                    }
                }
            }
            Axis::Following => {
                // libxml anchors an attribute's following axis after its
                // owner element, excluding that owner's descendants.
                for id in self.following_nodes(owner) {
                    if self.node_matches_test(id, test, axis) {
                        result.push(XPathNode::Node(id));
                    }
                }
            }
            Axis::Preceding => {
                for id in self.preceding_nodes(owner) {
                    if self.node_matches_test(id, test, axis) {
                        result.push(XPathNode::Node(id));
                    }
                }
            }
            // Child, Descendant, Attribute, Namespace, FollowingSibling,
            // PrecedingSibling: empty for attribute nodes.
            _ => {}
        }
    }

    /// Expands the attribute axis of an element: one entry per matching
    /// attribute, in attribute-list order.
    ///
    /// Namespace declarations (`xmlns`, `xmlns:*`) are not attribute nodes
    /// in the `XPath` 1.0 data model (section 5.3) and are skipped.
    fn push_matching_attributes(
        &self,
        element: NodeId,
        test: &NodeTest,
        result: &mut Vec<XPathNode>,
    ) {
        for (i, attr) in self.doc.attributes(element).iter().enumerate() {
            if attr.prefix.as_deref() == Some("xmlns")
                || (attr.prefix.is_none() && attr.name == "xmlns")
            {
                continue;
            }
            let matches = match test {
                NodeTest::Name(name) => self.attr_name_matches(attr, name),
                NodeTest::Wildcard | NodeTest::Node => true,
                NodeTest::PrefixWildcard(prefix) => {
                    if let Some(uri) = self.namespaces.get(prefix.as_str()) {
                        attr.namespace.as_deref() == Some(uri.as_str())
                    } else {
                        attr.prefix.as_deref() == Some(prefix.as_str())
                    }
                }
                _ => false,
            };
            if matches {
                #[allow(clippy::cast_possible_truncation)]
                result.push(XPathNode::Attribute {
                    owner: element,
                    index: i as u32,
                });
            }
        }
    }

    /// Tests an attribute against a name test.
    ///
    /// A `prefix:local` test matches namespace-aware when the prefix is
    /// registered via [`set_namespace`](Self::set_namespace), and by literal
    /// prefix otherwise. An unprefixed test matches on the local name
    /// (matching the evaluator's historical behavior for element names).
    fn attr_name_matches(&self, attr: &Attribute, name: &str) -> bool {
        if let Some(colon) = name.find(':') {
            let prefix = &name[..colon];
            let local = &name[colon + 1..];
            if let Some(uri) = self.namespaces.get(prefix) {
                return attr.namespace.as_deref() == Some(uri.as_str()) && attr.name == local;
            }
            return attr.prefix.as_deref() == Some(prefix) && attr.name == local;
        }
        attr.name == name
    }

    // -----------------------------------------------------------------------
    // Axis expansion (for uncommon axes)
    // -----------------------------------------------------------------------

    /// Returns all candidate nodes along the given axis from `node`.
    fn expand_axis(&self, node: NodeId, axis: Axis) -> Vec<NodeId> {
        match axis {
            Axis::Child => self.doc.children(node).collect(),
            Axis::Descendant => self.doc.descendants(node).collect(),
            Axis::Parent => self.doc.parent(node).into_iter().collect(),
            Axis::Ancestor => {
                let mut result = Vec::new();
                let mut current = self.doc.parent(node);
                while let Some(p) = current {
                    result.push(p);
                    current = self.doc.parent(p);
                }
                result
            }
            Axis::FollowingSibling => {
                let mut result = Vec::new();
                let mut current = self.doc.next_sibling(node);
                while let Some(s) = current {
                    result.push(s);
                    current = self.doc.next_sibling(s);
                }
                result
            }
            Axis::PrecedingSibling => {
                let mut result = Vec::new();
                let mut current = self.doc.prev_sibling(node);
                while let Some(s) = current {
                    result.push(s);
                    current = self.doc.prev_sibling(s);
                }
                result
            }
            Axis::Following => self.following_nodes(node),
            Axis::Preceding => self.preceding_nodes(node),
            Axis::Attribute => {
                // Attributes are not tree nodes in our arena. We return an
                // empty vec here; attribute axis handling is done via
                // `apply_node_test` which inspects the element's attributes
                // directly.
                Vec::new()
            }
            Axis::Namespace => {
                // Namespace axis handled via apply_namespace_node_test
                // in the fused paths. Fallback here for completeness.
                Vec::new()
            }
            Axis::Self_ => vec![node],
            Axis::DescendantOrSelf => {
                let mut result = vec![node];
                result.extend(self.doc.descendants(node));
                result
            }
            Axis::AncestorOrSelf => {
                let mut result = vec![node];
                let mut current = self.doc.parent(node);
                while let Some(p) = current {
                    result.push(p);
                    current = self.doc.parent(p);
                }
                result
            }
        }
    }

    /// Returns all nodes after `node` in document order, excluding descendants.
    fn following_nodes(&self, node: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        // First, try following siblings and their subtrees
        let mut current = self.doc.next_sibling(node);
        while let Some(s) = current {
            result.push(s);
            result.extend(self.doc.descendants(s));
            current = self.doc.next_sibling(s);
        }
        // Then walk up ancestors and collect their following siblings
        let mut ancestor = self.doc.parent(node);
        while let Some(anc) = ancestor {
            let mut sib = self.doc.next_sibling(anc);
            while let Some(s) = sib {
                result.push(s);
                result.extend(self.doc.descendants(s));
                sib = self.doc.next_sibling(s);
            }
            ancestor = self.doc.parent(anc);
        }
        result
    }

    /// Returns all nodes before `node` in document order, excluding ancestors.
    fn preceding_nodes(&self, node: NodeId) -> Vec<NodeId> {
        let mut result = Vec::new();
        // Preceding siblings and their subtrees (in reverse document order)
        let mut current = self.doc.prev_sibling(node);
        while let Some(s) = current {
            // Add descendants first (they come after `s` in document order)
            let descs: Vec<NodeId> = self.doc.descendants(s).collect();
            for &d in descs.iter().rev() {
                result.push(d);
            }
            result.push(s);
            current = self.doc.prev_sibling(s);
        }
        // Walk up ancestors and collect their preceding siblings
        let mut ancestor = self.doc.parent(node);
        while let Some(anc) = ancestor {
            let mut sib = self.doc.prev_sibling(anc);
            while let Some(s) = sib {
                let descs: Vec<NodeId> = self.doc.descendants(s).collect();
                for &d in descs.iter().rev() {
                    result.push(d);
                }
                result.push(s);
                sib = self.doc.prev_sibling(s);
            }
            ancestor = self.doc.parent(anc);
        }
        result
    }

    // -----------------------------------------------------------------------
    // Node test filtering
    // -----------------------------------------------------------------------

    /// Tests whether a name test matches an element, accounting for namespace
    /// prefix bindings when available.
    ///
    /// When the name contains a colon and namespace bindings are registered,
    /// splits the name into prefix + local, resolves the prefix to a URI,
    /// and compares against the element's namespace URI and local name.
    /// Otherwise, falls back to direct string comparison.
    #[inline]
    fn matches_element_name(&self, name: &str, elem: &NodeKind) -> bool {
        if let NodeKind::Element {
            name: elem_name,
            namespace,
            ..
        } = elem
        {
            // If the XPath name has a prefix and we have namespace bindings,
            // do namespace-aware matching.
            if let Some(colon_pos) = name.find(':') {
                if !self.namespaces.is_empty() {
                    let prefix = &name[..colon_pos];
                    let local = &name[colon_pos + 1..];
                    if let Some(uri) = self.namespaces.get(prefix) {
                        return namespace.as_deref() == Some(uri.as_str()) && elem_name == local;
                    }
                }
            }
            // Default: direct string comparison (matches existing behavior)
            elem_name == name
        } else {
            false
        }
    }

    /// Checks whether a single node matches a node test for a given axis.
    #[inline]
    fn node_matches_test(&self, id: NodeId, test: &NodeTest, axis: Axis) -> bool {
        let node = self.doc.node(id);
        match test {
            NodeTest::Name(name) => match &node.kind {
                kind @ NodeKind::Element { .. } => self.matches_element_name(name, kind),
                NodeKind::ProcessingInstruction { target, .. }
                    if axis == Axis::Child
                        || axis == Axis::Descendant
                        || axis == Axis::DescendantOrSelf =>
                {
                    target == name
                }
                _ => false,
            },
            NodeTest::Wildcard => {
                // Wildcard matches any element on most axes; on the attribute
                // axis it matches any attribute (handled separately).
                matches!(node.kind, NodeKind::Element { .. })
            }
            NodeTest::PrefixWildcard(prefix) => match &node.kind {
                NodeKind::Element {
                    namespace,
                    prefix: elem_prefix,
                    ..
                } => {
                    // If we have namespace bindings, resolve the prefix to a URI
                    // and match against the element's namespace.
                    if let Some(uri) = self.namespaces.get(prefix.as_str()) {
                        namespace.as_deref() == Some(uri.as_str())
                    } else {
                        elem_prefix.as_deref() == Some(prefix.as_str())
                    }
                }
                _ => false,
            },
            NodeTest::Node => true,
            NodeTest::Text => matches!(node.kind, NodeKind::Text { .. } | NodeKind::CData { .. }),
            NodeTest::Comment => matches!(node.kind, NodeKind::Comment { .. }),
            NodeTest::ProcessingInstruction(opt_name) => match &node.kind {
                NodeKind::ProcessingInstruction { target, .. } => opt_name
                    .as_ref()
                    .map_or(true, |expected| target == expected),
                _ => false,
            },
        }
    }

    /// Handles the namespace axis: returns the element's `NodeId` if the
    /// element has any in-scope namespace bindings that match `test`.
    ///
    /// Namespace nodes in `XPath` 1.0 (section 5.4) represent namespace
    /// bindings in scope on an element. Each element has one namespace node
    /// for every namespace prefix in scope, plus the implicit `xml` prefix.
    ///
    /// Since namespace nodes are not tree nodes in our arena, we follow the
    /// same pattern as the attribute axis: return the element's own `NodeId`
    /// when a match is found.
    fn apply_namespace_node_test(&self, element: NodeId, test: &NodeTest) -> Vec<NodeId> {
        if !matches!(self.doc.node(element).kind, NodeKind::Element { .. }) {
            return Vec::new();
        }

        // Collect in-scope namespace prefixes by walking from the element up
        // to the root, respecting the closest declaration for each prefix.
        let mut ns_map: Vec<(Option<&str>, &str)> = Vec::new();
        let mut seen_prefixes: HashSet<Option<&str>> = HashSet::new();

        let mut current = Some(element);
        while let Some(id) = current {
            for attr in self.doc.attributes(id) {
                if attr.name == "xmlns" && attr.prefix.is_none() {
                    // Default namespace declaration: xmlns="..."
                    if seen_prefixes.insert(None) && !attr.value.is_empty() {
                        ns_map.push((None, &attr.value));
                    }
                } else if attr.prefix.as_deref() == Some("xmlns") {
                    // Prefixed namespace declaration: xmlns:prefix="..."
                    // The local name (attr.name) is the namespace prefix.
                    if seen_prefixes.insert(Some(attr.name.as_str())) {
                        ns_map.push((Some(attr.name.as_str()), &attr.value));
                    }
                }
            }
            current = self.doc.parent(id);
        }

        // The xml prefix is always implicitly in scope.
        if seen_prefixes.insert(Some("xml")) {
            ns_map.push((Some("xml"), "http://www.w3.org/XML/1998/namespace"));
        }

        match test {
            NodeTest::Name(name) => {
                // Match a specific namespace prefix
                let target: Option<&str> = if name.is_empty() {
                    None
                } else {
                    Some(name.as_str())
                };
                if ns_map.iter().any(|(prefix, _)| *prefix == target) {
                    vec![element]
                } else {
                    Vec::new()
                }
            }
            NodeTest::Wildcard | NodeTest::Node => {
                // Return the element once if any namespace bindings exist.
                if ns_map.is_empty() {
                    Vec::new()
                } else {
                    vec![element]
                }
            }
            _ => Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Predicate evaluation
    // -----------------------------------------------------------------------

    /// Applies a predicate to a node set, filtering down to nodes where the
    /// predicate is true.
    ///
    /// Per `XPath` 1.0 section 2.4, if the predicate evaluates to a number,
    /// it is compared to the context position (positional predicate). Otherwise
    /// it is converted to boolean.
    fn apply_predicate(
        &self,
        nodes: &[XPathNode],
        predicate: &Expr,
    ) -> Result<Vec<XPathNode>, XPathError> {
        let size = nodes.len();
        let mut result = Vec::new();

        // Fast path for positional predicates like [1], [last()], etc.
        // If the predicate is a numeric literal, we can skip evaluating
        // it for every node.
        if let Expr::Number(n) = predicate {
            // Convert float position to 1-based integer index.
            // XPath positions are 1-based, so [1] means the first node.
            let pos = *n;
            if pos >= 1.0 && pos.fract() == 0.0 {
                // Safe: we checked pos is a non-negative integer that fits in usize range.
                // Node sets can never exceed u32::MAX nodes (arena limit), so no precision loss.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let idx = pos as usize - 1;
                if let Some(&node) = nodes.get(idx) {
                    result.push(node);
                }
            }
            return Ok(result);
        }

        for (i, &node) in nodes.iter().enumerate() {
            let ctx = XPathContext {
                doc: self.doc,
                context_node: node,
                context_position: i + 1, // 1-based
                context_size: size,
                // Share the variables reference — avoid cloning the HashMap.
                // HashMap::new() doesn't allocate, so this is free when empty.
                variables: if self.variables.is_empty() {
                    HashMap::new()
                } else {
                    self.variables.clone()
                },
                namespaces: self.namespaces.clone(),
            };
            let val = ctx.eval_expr(predicate)?;
            let keep = match &val {
                XPathValue::Number(n) => {
                    // Positional predicate: [1] means first node
                    #[allow(clippy::float_cmp, clippy::cast_precision_loss)]
                    let pos_match = *n == (i + 1) as f64;
                    pos_match
                }
                _ => self.value_to_boolean(&val),
            };
            if keep {
                result.push(node);
            }
        }

        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Filter expressions
    // -----------------------------------------------------------------------

    fn eval_filter(&self, expr: &Expr, predicates: &[Expr]) -> Result<XPathValue, XPathError> {
        let val = self.eval_expr(expr)?;
        let mut nodes = match val {
            XPathValue::NodeSet(ns) => ns,
            other => {
                return Err(XPathError::TypeError {
                    expected: "node-set".to_owned(),
                    found: other.type_name().to_owned(),
                });
            }
        };

        for pred in predicates {
            nodes = self.apply_predicate(&nodes, pred)?;
        }

        Ok(XPathValue::NodeSet(nodes))
    }

    // -----------------------------------------------------------------------
    // Union
    // -----------------------------------------------------------------------

    fn eval_union(&self, left: &Expr, right: &Expr) -> Result<XPathValue, XPathError> {
        let lv = self.eval_expr(left)?;
        let rv = self.eval_expr(right)?;

        let mut lnodes = match lv {
            XPathValue::NodeSet(ns) => ns,
            other => {
                return Err(XPathError::TypeError {
                    expected: "node-set".to_owned(),
                    found: other.type_name().to_owned(),
                });
            }
        };
        let rnodes = match rv {
            XPathValue::NodeSet(ns) => ns,
            other => {
                return Err(XPathError::TypeError {
                    expected: "node-set".to_owned(),
                    found: other.type_name().to_owned(),
                });
            }
        };

        // Merge, dedup, sort
        let seen: HashSet<XPathNode> = lnodes.iter().copied().collect();
        for id in rnodes {
            if !seen.contains(&id) {
                lnodes.push(id);
            }
        }
        sort_document_order(&mut lnodes);

        Ok(XPathValue::NodeSet(lnodes))
    }

    // -----------------------------------------------------------------------
    // Function dispatch
    // -----------------------------------------------------------------------

    fn eval_function(&self, name: &str, args: &[Expr]) -> Result<XPathValue, XPathError> {
        match name {
            // Node-set functions
            "last" => self.fn_last(args),
            "position" => self.fn_position(args),
            "count" => self.fn_count(args),
            "local-name" => self.fn_local_name(args),
            "namespace-uri" => self.fn_namespace_uri(args),
            "name" => self.fn_name(args),

            // String functions
            "string" => self.fn_string(args),
            "concat" => self.fn_concat(args),
            "starts-with" => self.fn_starts_with(args),
            "contains" => self.fn_contains(args),
            "substring-before" => self.fn_substring_before(args),
            "substring-after" => self.fn_substring_after(args),
            "substring" => self.fn_substring(args),
            "string-length" => self.fn_string_length(args),
            "normalize-space" => self.fn_normalize_space(args),
            "translate" => self.fn_translate(args),

            // Boolean functions
            "boolean" => self.fn_boolean(args),
            "not" => self.fn_not(args),
            "true" => self.fn_true(args),
            "false" => self.fn_false(args),
            "lang" => self.fn_lang(args),

            // Number functions
            "number" => self.fn_number(args),
            "sum" => self.fn_sum(args),
            "floor" => self.fn_floor(args),
            "ceiling" => self.fn_ceiling(args),
            "round" => self.fn_round(args),

            // id() - stub
            "id" => self.fn_id(args),

            // XPath 2.0 / XSD functions commonly used in Schematron
            "matches" => self.fn_matches(args),
            "replace" => self.fn_replace(args),
            "tokenize" => self.fn_tokenize(args),

            // XPath 2.0 string functions
            "upper-case" => self.fn_upper_case(args),
            "lower-case" => self.fn_lower_case(args),
            "ends-with" => self.fn_ends_with(args),
            "string-join" => self.fn_string_join(args),

            // XPath 2.0 sequence/numeric functions
            "empty" => self.fn_empty(args),
            "exists" => self.fn_exists(args),
            "abs" => self.fn_abs(args),
            "min" => self.fn_min(args),
            "max" => self.fn_max(args),
            "reverse" => self.fn_reverse(args),

            _ => Err(XPathError::UndefinedFunction {
                name: name.to_owned(),
            }),
        }
    }

    // -- Node-set functions -------------------------------------------------

    #[allow(clippy::cast_precision_loss)]
    fn fn_last(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("last", args, 0)?;
        Ok(XPathValue::Number(self.context_size as f64))
    }

    #[allow(clippy::cast_precision_loss)]
    fn fn_position(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("position", args, 0)?;
        Ok(XPathValue::Number(self.context_position as f64))
    }

    #[allow(clippy::cast_precision_loss)]
    fn fn_count(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("count", args, 1)?;
        let val = self.eval_expr(&args[0])?;
        match &val {
            XPathValue::NodeSet(ns) => Ok(XPathValue::Number(ns.len() as f64)),
            other => Err(XPathError::TypeError {
                expected: "node-set".to_owned(),
                found: other.type_name().to_owned(),
            }),
        }
    }

    fn fn_local_name(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() > 1 {
            return Err(XPathError::InvalidArgCount {
                function: "local-name".to_owned(),
                expected: 1,
                found: args.len(),
            });
        }
        let node = if args.is_empty() {
            self.context_node
        } else {
            let val = self.eval_expr(&args[0])?;
            match &val {
                XPathValue::NodeSet(ns) if !ns.is_empty() => ns[0],
                XPathValue::NodeSet(_) => return Ok(XPathValue::String(String::new())),
                other => {
                    return Err(XPathError::TypeError {
                        expected: "node-set".to_owned(),
                        found: other.type_name().to_owned(),
                    });
                }
            }
        };
        if let XPathNode::Attribute { owner, index } = node {
            let local = self.attr(owner, index).map(|a| a.name.clone());
            return Ok(XPathValue::String(local.unwrap_or_default()));
        }
        let name = self.doc.node_name(node.anchor()).unwrap_or("");
        // Strip prefix if present (local-name returns the part after ':')
        let local = name.split(':').next_back().unwrap_or(name);
        Ok(XPathValue::String(local.to_owned()))
    }

    fn fn_namespace_uri(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() > 1 {
            return Err(XPathError::InvalidArgCount {
                function: "namespace-uri".to_owned(),
                expected: 1,
                found: args.len(),
            });
        }
        let node = if args.is_empty() {
            self.context_node
        } else {
            let val = self.eval_expr(&args[0])?;
            match &val {
                XPathValue::NodeSet(ns) if !ns.is_empty() => ns[0],
                XPathValue::NodeSet(_) => return Ok(XPathValue::String(String::new())),
                other => {
                    return Err(XPathError::TypeError {
                        expected: "node-set".to_owned(),
                        found: other.type_name().to_owned(),
                    });
                }
            }
        };
        if let XPathNode::Attribute { owner, index } = node {
            let uri = self.attr(owner, index).and_then(|a| a.namespace.clone());
            return Ok(XPathValue::String(uri.unwrap_or_default()));
        }
        let uri = self.doc.node_namespace(node.anchor()).unwrap_or("");
        Ok(XPathValue::String(uri.to_owned()))
    }

    fn fn_name(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() > 1 {
            return Err(XPathError::InvalidArgCount {
                function: "name".to_owned(),
                expected: 1,
                found: args.len(),
            });
        }
        let node = if args.is_empty() {
            self.context_node
        } else {
            let val = self.eval_expr(&args[0])?;
            match &val {
                XPathValue::NodeSet(ns) if !ns.is_empty() => ns[0],
                XPathValue::NodeSet(_) => return Ok(XPathValue::String(String::new())),
                other => {
                    return Err(XPathError::TypeError {
                        expected: "node-set".to_owned(),
                        found: other.type_name().to_owned(),
                    });
                }
            }
        };
        if let XPathNode::Attribute { owner, index } = node {
            let qname = self.attr(owner, index).map(|a| match &a.prefix {
                Some(prefix) => format!("{prefix}:{}", a.name),
                None => a.name.clone(),
            });
            return Ok(XPathValue::String(qname.unwrap_or_default()));
        }
        let name = self.doc.node_name(node.anchor()).unwrap_or("");
        Ok(XPathValue::String(name.to_owned()))
    }

    // -- String functions ---------------------------------------------------

    fn fn_string(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() > 1 {
            return Err(XPathError::InvalidArgCount {
                function: "string".to_owned(),
                expected: 1,
                found: args.len(),
            });
        }
        if args.is_empty() {
            let sv = self.string_value(self.context_node);
            return Ok(XPathValue::String(sv));
        }
        let val = self.eval_expr(&args[0])?;
        Ok(XPathValue::String(self.value_to_string(&val)))
    }

    fn fn_concat(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() < 2 {
            return Err(XPathError::InvalidArgCount {
                function: "concat".to_owned(),
                expected: 2,
                found: args.len(),
            });
        }
        let mut result = String::new();
        for arg in args {
            let val = self.eval_expr(arg)?;
            result.push_str(&self.value_to_string(&val));
        }
        Ok(XPathValue::String(result))
    }

    fn fn_starts_with(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("starts-with", args, 2)?;
        let s = self.value_to_string(&self.eval_expr(&args[0])?);
        let prefix = self.value_to_string(&self.eval_expr(&args[1])?);
        Ok(XPathValue::Boolean(s.starts_with(prefix.as_str())))
    }

    fn fn_contains(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("contains", args, 2)?;
        let s = self.value_to_string(&self.eval_expr(&args[0])?);
        let sub = self.value_to_string(&self.eval_expr(&args[1])?);
        Ok(XPathValue::Boolean(s.contains(sub.as_str())))
    }

    fn fn_substring_before(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("substring-before", args, 2)?;
        let s = self.value_to_string(&self.eval_expr(&args[0])?);
        let sub = self.value_to_string(&self.eval_expr(&args[1])?);
        let result = s
            .find(sub.as_str())
            .map_or_else(String::new, |pos| s[..pos].to_owned());
        Ok(XPathValue::String(result))
    }

    fn fn_substring_after(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("substring-after", args, 2)?;
        let s = self.value_to_string(&self.eval_expr(&args[0])?);
        let sub = self.value_to_string(&self.eval_expr(&args[1])?);
        let result = s
            .find(sub.as_str())
            .map_or_else(String::new, |pos| s[(pos + sub.len())..].to_owned());
        Ok(XPathValue::String(result))
    }

    /// `XPath` `substring(string, number, number?)` per section 4.2.
    ///
    /// Uses `XPath` rounding: `round()` each numeric argument, 1-based indexing.
    /// The substring starts at position `round(arg2)` and has length
    /// `round(arg3)` if provided.
    fn fn_substring(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(XPathError::InvalidArgCount {
                function: "substring".to_owned(),
                expected: 2,
                found: args.len(),
            });
        }
        let s = self.value_to_string(&self.eval_expr(&args[0])?);
        let pos = self.value_to_number(&self.eval_expr(&args[1])?);
        let len = if args.len() == 3 {
            Some(self.value_to_number(&self.eval_expr(&args[2])?))
        } else {
            None
        };

        // XPath substring is 1-based with special rounding rules
        let rounded_pos = xpath_round(pos);
        let chars: Vec<char> = s.chars().collect();
        #[allow(clippy::cast_precision_loss)]
        let str_len = chars.len() as f64;

        // Compute start and end indices (1-based, may be NaN/Inf)
        let start = rounded_pos;
        let end = len.map_or(str_len + 1.0, |l| rounded_pos + xpath_round(l));

        // Handle NaN: if start or end is NaN, return empty string
        if start.is_nan() || end.is_nan() {
            return Ok(XPathValue::String(String::new()));
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let actual_start = (start - 1.0).max(0.0) as usize;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let actual_end = (end - 1.0).min(str_len).max(0.0) as usize;

        if actual_start >= actual_end || actual_start >= chars.len() {
            return Ok(XPathValue::String(String::new()));
        }

        let result: String = chars[actual_start..actual_end].iter().collect();
        Ok(XPathValue::String(result))
    }

    fn fn_string_length(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() > 1 {
            return Err(XPathError::InvalidArgCount {
                function: "string-length".to_owned(),
                expected: 1,
                found: args.len(),
            });
        }
        let s = if args.is_empty() {
            self.string_value(self.context_node)
        } else {
            self.value_to_string(&self.eval_expr(&args[0])?)
        };
        #[allow(clippy::cast_precision_loss)]
        let len = s.chars().count() as f64;
        Ok(XPathValue::Number(len))
    }

    /// `normalize-space(string?)` per `XPath` 1.0 section 4.2.
    ///
    /// Strips leading and trailing whitespace and collapses sequences of
    /// whitespace characters to a single space.
    fn fn_normalize_space(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() > 1 {
            return Err(XPathError::InvalidArgCount {
                function: "normalize-space".to_owned(),
                expected: 1,
                found: args.len(),
            });
        }
        let s = if args.is_empty() {
            self.string_value(self.context_node)
        } else {
            self.value_to_string(&self.eval_expr(&args[0])?)
        };
        let normalized: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
        Ok(XPathValue::String(normalized))
    }

    /// `translate(string, string, string)` per `XPath` 1.0 section 4.2.
    ///
    /// Replaces characters in the first string that appear in the second string
    /// with the corresponding character in the third string. Characters with no
    /// corresponding replacement are removed.
    fn fn_translate(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("translate", args, 3)?;
        let s = self.value_to_string(&self.eval_expr(&args[0])?);
        let from = self.value_to_string(&self.eval_expr(&args[1])?);
        let to = self.value_to_string(&self.eval_expr(&args[2])?);

        let from_chars: Vec<char> = from.chars().collect();
        let to_chars: Vec<char> = to.chars().collect();

        let result: String = s
            .chars()
            .filter_map(|c| {
                if let Some(pos) = from_chars.iter().position(|&fc| fc == c) {
                    to_chars.get(pos).copied()
                } else {
                    Some(c)
                }
            })
            .collect();
        Ok(XPathValue::String(result))
    }

    // -- Boolean functions --------------------------------------------------

    fn fn_boolean(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("boolean", args, 1)?;
        let val = self.eval_expr(&args[0])?;
        Ok(XPathValue::Boolean(self.value_to_boolean(&val)))
    }

    fn fn_not(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("not", args, 1)?;
        let val = self.eval_expr(&args[0])?;
        Ok(XPathValue::Boolean(!self.value_to_boolean(&val)))
    }

    #[allow(clippy::unused_self)]
    fn fn_true(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("true", args, 0)?;
        Ok(XPathValue::Boolean(true))
    }

    #[allow(clippy::unused_self)]
    fn fn_false(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("false", args, 0)?;
        Ok(XPathValue::Boolean(false))
    }

    fn fn_lang(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("lang", args, 1)?;
        let target = self.value_to_string(&self.eval_expr(&args[0])?);
        let target_lower = target.to_lowercase();

        // Walk ancestors looking for xml:lang attribute
        let mut node = Some(self.context_node.anchor());
        while let Some(n) = node {
            if let Some(lang) = self.doc.attribute(n, "xml:lang") {
                let lang_lower = lang.to_lowercase();
                if lang_lower == target_lower || lang_lower.starts_with(&format!("{target_lower}-"))
                {
                    return Ok(XPathValue::Boolean(true));
                }
                return Ok(XPathValue::Boolean(false));
            }
            node = self.doc.parent(n);
        }
        Ok(XPathValue::Boolean(false))
    }

    // -- Number functions ---------------------------------------------------

    fn fn_number(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() > 1 {
            return Err(XPathError::InvalidArgCount {
                function: "number".to_owned(),
                expected: 1,
                found: args.len(),
            });
        }
        if args.is_empty() {
            let sv = self.string_value(self.context_node);
            return Ok(XPathValue::Number(parse_xpath_number(&sv)));
        }
        let val = self.eval_expr(&args[0])?;
        Ok(XPathValue::Number(self.value_to_number(&val)))
    }

    fn fn_sum(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("sum", args, 1)?;
        let val = self.eval_expr(&args[0])?;
        match &val {
            XPathValue::NodeSet(ns) => {
                let total: f64 = ns
                    .iter()
                    .map(|&n| parse_xpath_number(&self.string_value(n)))
                    .sum();
                Ok(XPathValue::Number(total))
            }
            other => Err(XPathError::TypeError {
                expected: "node-set".to_owned(),
                found: other.type_name().to_owned(),
            }),
        }
    }

    fn fn_floor(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("floor", args, 1)?;
        let n = self.value_to_number(&self.eval_expr(&args[0])?);
        Ok(XPathValue::Number(n.floor()))
    }

    fn fn_ceiling(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("ceiling", args, 1)?;
        let n = self.value_to_number(&self.eval_expr(&args[0])?);
        Ok(XPathValue::Number(n.ceil()))
    }

    /// `round(number)` per `XPath` 1.0 section 4.4.
    ///
    /// Rounds to the nearest integer, with halfway cases going to positive
    /// infinity (e.g., `round(0.5)` = 1, `round(-0.5)` = 0).
    fn fn_round(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("round", args, 1)?;
        let n = self.value_to_number(&self.eval_expr(&args[0])?);
        Ok(XPathValue::Number(xpath_round(n)))
    }

    /// `id(object)` — selects elements by their ID attribute value.
    ///
    /// When the argument is a node-set, the string-value of each node is used
    /// as a whitespace-separated list of IDs. When the argument is any other
    /// type, it is converted to a string and treated as a whitespace-separated
    /// list of IDs. Returns the elements whose ID matches, in document order.
    ///
    /// See `XPath` 1.0 section 4.1.
    fn fn_id(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("id", args, 1)?;
        let val = self.eval_expr(&args[0])?;
        let mut result = Vec::new();
        let mut seen = HashSet::new();

        let id_strings: Vec<String> = match &val {
            XPathValue::NodeSet(nodes) => nodes.iter().map(|&n| self.string_value(n)).collect(),
            other => vec![self.value_to_string(other)],
        };

        for id_str in &id_strings {
            for token in id_str.split_whitespace() {
                if let Some(node) = self.doc.element_by_id(token) {
                    if seen.insert(node) {
                        result.push(XPathNode::Node(node));
                    }
                }
            }
        }

        sort_document_order(&mut result);
        Ok(XPathValue::NodeSet(result))
    }

    // -- XPath 2.0 functions (commonly used in Schematron) ------------------

    /// `matches(string, pattern)` or `matches(string, pattern, flags)`.
    ///
    /// Returns true if the string matches the regular expression pattern.
    /// See `XPath` 2.0 Functions and Operators section 7.6.2.
    fn fn_matches(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() < 2 || args.len() > 3 {
            return Err(XPathError::InvalidArgCount {
                function: "matches".to_owned(),
                expected: 2,
                found: args.len(),
            });
        }
        let input = self.value_to_string(&self.eval_expr(&args[0])?);
        let pattern = self.value_to_string(&self.eval_expr(&args[1])?);
        let flags = if args.len() == 3 {
            self.value_to_string(&self.eval_expr(&args[2])?)
        } else {
            String::new()
        };
        match super::regex::xpath_matches(&input, &pattern, &flags) {
            Ok(result) => Ok(XPathValue::Boolean(result)),
            Err(e) => Err(XPathError::InternalError {
                message: format!("regex error in matches(): {e}"),
            }),
        }
    }

    /// `replace(string, pattern, replacement)` or with flags.
    ///
    /// Replaces all matches of `pattern` in `string` with `replacement`.
    /// See `XPath` 2.0 F&O section 7.6.3.
    fn fn_replace(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.len() < 3 || args.len() > 4 {
            return Err(XPathError::InvalidArgCount {
                function: "replace".to_owned(),
                expected: 3,
                found: args.len(),
            });
        }
        let input = self.value_to_string(&self.eval_expr(&args[0])?);
        let pattern = self.value_to_string(&self.eval_expr(&args[1])?);
        let replacement = self.value_to_string(&self.eval_expr(&args[2])?);
        let flags = if args.len() == 4 {
            self.value_to_string(&self.eval_expr(&args[3])?)
        } else {
            String::new()
        };
        match super::regex::xpath_replace(&input, &pattern, &replacement, &flags) {
            Ok(result) => Ok(XPathValue::String(result)),
            Err(e) => Err(XPathError::InternalError {
                message: format!("regex error in replace(): {e}"),
            }),
        }
    }

    /// `tokenize(string, pattern)` or with flags.
    ///
    /// Splits `string` on occurrences of `pattern`.
    /// See `XPath` 2.0 F&O section 7.6.4.
    fn fn_tokenize(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.is_empty() || args.len() > 3 {
            return Err(XPathError::InvalidArgCount {
                function: "tokenize".to_owned(),
                expected: 2,
                found: args.len(),
            });
        }
        let input = self.value_to_string(&self.eval_expr(&args[0])?);
        let pattern = if args.len() >= 2 {
            self.value_to_string(&self.eval_expr(&args[1])?)
        } else {
            r"\s+".to_string()
        };
        let flags = if args.len() == 3 {
            self.value_to_string(&self.eval_expr(&args[2])?)
        } else {
            String::new()
        };
        match super::regex::xpath_tokenize(&input, &pattern, &flags) {
            Ok(tokens) => {
                // Return as a NodeSet-like structure. Since tokenize returns
                // strings not nodes, return as a String (joining with space)
                // for XPath 1.0 compatibility. In a full XPath 2.0 impl this
                // would return a sequence of strings.
                Ok(XPathValue::String(tokens.join(" ")))
            }
            Err(e) => Err(XPathError::InternalError {
                message: format!("regex error in tokenize(): {e}"),
            }),
        }
    }

    // -- XPath 2.0 string functions -------------------------------------------

    /// `upper-case(string)` — converts to uppercase.
    fn fn_upper_case(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("upper-case", args, 1)?;
        let s = self.value_to_string(&self.eval_expr(&args[0])?);
        Ok(XPathValue::String(s.to_uppercase()))
    }

    /// `lower-case(string)` — converts to lowercase.
    fn fn_lower_case(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("lower-case", args, 1)?;
        let s = self.value_to_string(&self.eval_expr(&args[0])?);
        Ok(XPathValue::String(s.to_lowercase()))
    }

    /// `ends-with(string, suffix)` — tests if string ends with suffix.
    fn fn_ends_with(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("ends-with", args, 2)?;
        let s = self.value_to_string(&self.eval_expr(&args[0])?);
        let suffix = self.value_to_string(&self.eval_expr(&args[1])?);
        Ok(XPathValue::Boolean(s.ends_with(&suffix)))
    }

    /// `string-join(sequence, separator)` — joins strings with a separator.
    ///
    /// In our `XPath` 1.0-based model, operates on a node-set by joining
    /// each node's string value.
    fn fn_string_join(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        if args.is_empty() || args.len() > 2 {
            return Err(XPathError::InvalidArgCount {
                function: "string-join".to_owned(),
                expected: 2,
                found: args.len(),
            });
        }
        let val = self.eval_expr(&args[0])?;
        let sep = if args.len() == 2 {
            self.value_to_string(&self.eval_expr(&args[1])?)
        } else {
            String::new()
        };
        match &val {
            XPathValue::NodeSet(nodes) => {
                let strings: Vec<String> = nodes.iter().map(|&n| self.string_value(n)).collect();
                Ok(XPathValue::String(strings.join(&sep)))
            }
            XPathValue::String(s) => Ok(XPathValue::String(s.clone())),
            other => Ok(XPathValue::String(other.to_xpath_string())),
        }
    }

    // -- XPath 2.0 sequence/boolean functions ---------------------------------

    /// `empty(sequence)` — returns true if the node-set is empty.
    fn fn_empty(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("empty", args, 1)?;
        let val = self.eval_expr(&args[0])?;
        match &val {
            XPathValue::NodeSet(ns) => Ok(XPathValue::Boolean(ns.is_empty())),
            XPathValue::String(s) => Ok(XPathValue::Boolean(s.is_empty())),
            _ => Ok(XPathValue::Boolean(false)),
        }
    }

    /// `exists(sequence)` — returns true if the node-set is non-empty.
    fn fn_exists(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("exists", args, 1)?;
        let val = self.eval_expr(&args[0])?;
        match &val {
            XPathValue::NodeSet(ns) => Ok(XPathValue::Boolean(!ns.is_empty())),
            XPathValue::String(s) => Ok(XPathValue::Boolean(!s.is_empty())),
            _ => Ok(XPathValue::Boolean(true)),
        }
    }

    /// `abs(number)` — absolute value.
    fn fn_abs(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("abs", args, 1)?;
        let n = self.value_to_number(&self.eval_expr(&args[0])?);
        Ok(XPathValue::Number(n.abs()))
    }

    /// `min(node-set)` — minimum numeric value of node string values.
    fn fn_min(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("min", args, 1)?;
        let val = self.eval_expr(&args[0])?;
        match &val {
            XPathValue::NodeSet(nodes) if nodes.is_empty() => Ok(XPathValue::Number(f64::NAN)),
            XPathValue::NodeSet(nodes) => {
                let mut min = f64::INFINITY;
                for &n in nodes {
                    let v = parse_xpath_number(&self.string_value(n));
                    if v.is_nan() {
                        return Ok(XPathValue::Number(f64::NAN));
                    }
                    if v < min {
                        min = v;
                    }
                }
                Ok(XPathValue::Number(min))
            }
            other => Ok(XPathValue::Number(self.value_to_number(other))),
        }
    }

    /// `max(node-set)` — maximum numeric value of node string values.
    fn fn_max(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("max", args, 1)?;
        let val = self.eval_expr(&args[0])?;
        match &val {
            XPathValue::NodeSet(nodes) if nodes.is_empty() => Ok(XPathValue::Number(f64::NAN)),
            XPathValue::NodeSet(nodes) => {
                let mut max = f64::NEG_INFINITY;
                for &n in nodes {
                    let v = parse_xpath_number(&self.string_value(n));
                    if v.is_nan() {
                        return Ok(XPathValue::Number(f64::NAN));
                    }
                    if v > max {
                        max = v;
                    }
                }
                Ok(XPathValue::Number(max))
            }
            other => Ok(XPathValue::Number(self.value_to_number(other))),
        }
    }

    /// `reverse(node-set)` — returns the node-set in reverse document order.
    fn fn_reverse(&self, args: &[Expr]) -> Result<XPathValue, XPathError> {
        check_arg_count("reverse", args, 1)?;
        let val = self.eval_expr(&args[0])?;
        match val {
            XPathValue::NodeSet(mut nodes) => {
                nodes.reverse();
                Ok(XPathValue::NodeSet(nodes))
            }
            other => Ok(other),
        }
    }

    // -----------------------------------------------------------------------
    // Type conversion helpers (with document access)
    // -----------------------------------------------------------------------

    /// Converts a value to boolean per `XPath` 1.0 section 4.3.
    #[allow(clippy::unused_self)]
    fn value_to_boolean(&self, val: &XPathValue) -> bool {
        val.to_boolean()
    }

    /// Converts a value to number per `XPath` 1.0 section 4.4.
    ///
    /// For node-sets, computes the string-value of the first node first.
    fn value_to_number(&self, val: &XPathValue) -> f64 {
        match val {
            XPathValue::NodeSet(ns) => {
                if ns.is_empty() {
                    f64::NAN
                } else {
                    let sv = self.string_value(ns[0]);
                    parse_xpath_number(&sv)
                }
            }
            _ => val.to_number(),
        }
    }

    /// Converts a value to string per `XPath` 1.0 section 4.2.
    ///
    /// For node-sets, computes the string-value of the first node in document
    /// order.
    fn value_to_string(&self, val: &XPathValue) -> String {
        match val {
            XPathValue::NodeSet(ns) => {
                if ns.is_empty() {
                    String::new()
                } else {
                    self.string_value(ns[0])
                }
            }
            _ => val.to_xpath_string(),
        }
    }

    /// Computes the string-value of a node per `XPath` 1.0 section 5.
    ///
    /// - Root / Element: concatenation of all descendant text nodes
    /// - Attribute: the attribute value
    /// - Text / CDATA: the text content
    /// - Comment: the comment text
    /// - PI: the PI data
    fn string_value(&self, node: XPathNode) -> String {
        let node = match node {
            XPathNode::Attribute { owner, index } => {
                return self
                    .attr(owner, index)
                    .map(|a| a.value.clone())
                    .unwrap_or_default();
            }
            XPathNode::Node(id) => id,
        };
        let kind = &self.doc.node(node).kind;
        match kind {
            // EntityRef: the expansion lives in the node's children, so the
            // string-value is the same concatenation of descendant text.
            NodeKind::Document | NodeKind::Element { .. } | NodeKind::EntityRef { .. } => {
                self.doc.text_content(node)
            }
            NodeKind::Text { content }
            | NodeKind::CData { content }
            | NodeKind::Comment { content } => content.clone(),
            NodeKind::ProcessingInstruction { data, .. } => {
                data.as_deref().unwrap_or("").to_owned()
            }
            NodeKind::DocumentType { .. } => String::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Comparison helpers
    // -----------------------------------------------------------------------

    /// Compares two values with `=` or `!=` per `XPath` 1.0 section 3.4.
    ///
    /// When either operand is a node-set (and the other is not a boolean),
    /// both operators are existential: the result is true iff *some* node
    /// satisfies the predicate. `!=` is therefore **not** the negation of
    /// `=` — an empty node-set compares false under both operators, and a
    /// multi-node set can satisfy both at once. A node-set compared against
    /// a boolean is converted to a boolean first, and scalar operands are
    /// compared directly.
    #[allow(clippy::float_cmp)]
    fn compare_equality(&self, op: BinaryOp, lhs: &XPathValue, rhs: &XPathValue) -> bool {
        let ne = op == BinaryOp::Neq;
        let str_cmp = |a: &str, b: &str| (a == b) != ne;
        let num_cmp = |a: f64, b: f64| (a == b) != ne;
        let bool_cmp = |a: bool, b: bool| (a == b) != ne;
        match (lhs, rhs) {
            // node-set = node-set
            (XPathValue::NodeSet(lns), XPathValue::NodeSet(rns)) => {
                for &ln in lns {
                    let lsv = self.string_value(ln);
                    for &rn in rns {
                        let rsv = self.string_value(rn);
                        if str_cmp(&lsv, &rsv) {
                            return true;
                        }
                    }
                }
                false
            }
            // node-set = boolean
            (XPathValue::NodeSet(ns), XPathValue::Boolean(b))
            | (XPathValue::Boolean(b), XPathValue::NodeSet(ns)) => bool_cmp(!ns.is_empty(), *b),
            // node-set = number
            (XPathValue::NodeSet(ns), XPathValue::Number(n))
            | (XPathValue::Number(n), XPathValue::NodeSet(ns)) => ns
                .iter()
                .any(|&node| num_cmp(parse_xpath_number(&self.string_value(node)), *n)),
            // node-set = string
            (XPathValue::NodeSet(ns), XPathValue::String(s))
            | (XPathValue::String(s), XPathValue::NodeSet(ns)) => {
                ns.iter().any(|&node| str_cmp(&self.string_value(node), s))
            }
            // Both booleans
            (XPathValue::Boolean(a), XPathValue::Boolean(b)) => bool_cmp(*a, *b),
            // If either is boolean, convert both to boolean
            (XPathValue::Boolean(_), _) | (_, XPathValue::Boolean(_)) => {
                bool_cmp(lhs.to_boolean(), rhs.to_boolean())
            }
            // If either is number, convert both to number
            (XPathValue::Number(a), XPathValue::Number(b)) => num_cmp(*a, *b),
            (XPathValue::Number(_), _) | (_, XPathValue::Number(_)) => {
                num_cmp(self.value_to_number(lhs), self.value_to_number(rhs))
            }
            // Otherwise compare as strings
            _ => str_cmp(&self.value_to_string(lhs), &self.value_to_string(rhs)),
        }
    }

    /// Compares two values relationally per `XPath` 1.0 section 3.4.
    fn compare_relational(&self, op: BinaryOp, lhs: &XPathValue, rhs: &XPathValue) -> bool {
        let cmp = |a: f64, b: f64| -> bool {
            match op {
                BinaryOp::Lt => a < b,
                BinaryOp::Lte => a <= b,
                BinaryOp::Gt => a > b,
                BinaryOp::Gte => a >= b,
                _ => false,
            }
        };

        match (lhs, rhs) {
            // node-set <op> node-set
            (XPathValue::NodeSet(lns), XPathValue::NodeSet(rns)) => {
                for &ln in lns {
                    let lv = parse_xpath_number(&self.string_value(ln));
                    for &rn in rns {
                        let rv = parse_xpath_number(&self.string_value(rn));
                        if cmp(lv, rv) {
                            return true;
                        }
                    }
                }
                false
            }
            // node-set <op> number
            (XPathValue::NodeSet(ns), XPathValue::Number(n)) => ns
                .iter()
                .any(|&node| cmp(parse_xpath_number(&self.string_value(node)), *n)),
            (XPathValue::Number(n), XPathValue::NodeSet(ns)) => ns
                .iter()
                .any(|&node| cmp(*n, parse_xpath_number(&self.string_value(node)))),
            // node-set <op> string
            (XPathValue::NodeSet(ns), XPathValue::String(s)) => {
                let rn = parse_xpath_number(s);
                ns.iter()
                    .any(|&node| cmp(parse_xpath_number(&self.string_value(node)), rn))
            }
            (XPathValue::String(s), XPathValue::NodeSet(ns)) => {
                let ln = parse_xpath_number(s);
                ns.iter()
                    .any(|&node| cmp(ln, parse_xpath_number(&self.string_value(node))))
            }
            // node-set <op> boolean  -- convert node-set to boolean, then to number
            (XPathValue::NodeSet(ns), _) => {
                let lv = if ns.is_empty() { 0.0 } else { 1.0 };
                let rv = self.value_to_number(rhs);
                cmp(lv, rv)
            }
            (_, XPathValue::NodeSet(ns)) => {
                let lv = self.value_to_number(lhs);
                let rv = if ns.is_empty() { 0.0 } else { 1.0 };
                cmp(lv, rv)
            }
            // Otherwise compare as numbers
            _ => cmp(self.value_to_number(lhs), self.value_to_number(rhs)),
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Sorts a node-set into document order using the arena index as a proxy.
///
/// Since nodes are allocated in document order during parsing, the arena
/// index (encoded in `NodeId`) directly reflects document order; attribute
/// nodes sort just after their owner element, by attribute index.
fn sort_document_order(nodes: &mut [XPathNode]) {
    nodes.sort_unstable();
}

/// Rounds a number using `XPath` rounding rules: round half toward positive
/// infinity.
///
/// Per `XPath` 1.0 section 4.4: `round(-0.5)` = 0, `round(0.5)` = 1.
fn xpath_round(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() {
        return n;
    }
    // XPath round: round half toward positive infinity
    // This is equivalent to floor(n + 0.5)
    (n + 0.5).floor()
}

/// Parses a string into an `XPath` number per section 4.4.
fn parse_xpath_number(s: &str) -> f64 {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return f64::NAN;
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

/// Checks that a function was called with the expected number of arguments.
fn check_arg_count(name: &str, args: &[Expr], expected: usize) -> Result<(), XPathError> {
    if args.len() != expected {
        return Err(XPathError::InvalidArgCount {
            function: name.to_owned(),
            expected,
            found: args.len(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::xpath::parser::parse;

    /// Helper: parse XML, get root element, evaluate `XPath` expression.
    fn eval_xpath(xml: &str, xpath: &str) -> XPathValue {
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root_element().unwrap();
        let expr = parse(xpath).unwrap();
        let ctx = XPathContext::new(&doc, root);
        ctx.evaluate(&expr).unwrap()
    }

    /// Helper: evaluate `XPath` from the document root (not the root element).
    fn eval_xpath_from_doc_root(xml: &str, xpath: &str) -> XPathValue {
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root();
        let expr = parse(xpath).unwrap();
        let ctx = XPathContext::new(&doc, root);
        ctx.evaluate(&expr).unwrap()
    }

    /// Helper: evaluate and return the count of nodes in a node-set.
    fn eval_count(xml: &str, xpath: &str) -> usize {
        match eval_xpath(xml, xpath) {
            XPathValue::NodeSet(ns) => ns.len(),
            other => panic!("expected node-set, got {other:?}"),
        }
    }

    // -- Arithmetic ---------------------------------------------------------

    #[test]
    fn test_arithmetic_add() {
        let result = eval_xpath("<r/>", "1 + 2");
        assert_eq!(result, XPathValue::Number(3.0));
    }

    #[test]
    fn test_arithmetic_multiply() {
        let result = eval_xpath("<r/>", "3 * 4");
        assert_eq!(result, XPathValue::Number(12.0));
    }

    #[test]
    fn test_arithmetic_div() {
        let result = eval_xpath("<r/>", "10 div 3");
        match result {
            XPathValue::Number(n) => assert!((n - 10.0 / 3.0).abs() < 1e-10),
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_arithmetic_mod() {
        let result = eval_xpath("<r/>", "10 mod 3");
        assert_eq!(result, XPathValue::Number(1.0));
    }

    #[test]
    fn test_arithmetic_sub() {
        let result = eval_xpath("<r/>", "5 - 3");
        assert_eq!(result, XPathValue::Number(2.0));
    }

    // -- Comparisons --------------------------------------------------------

    #[test]
    fn test_comparison_eq() {
        let result = eval_xpath("<r/>", "1 = 1");
        assert_eq!(result, XPathValue::Boolean(true));

        let result = eval_xpath("<r/>", "1 = 2");
        assert_eq!(result, XPathValue::Boolean(false));
    }

    #[test]
    fn test_comparison_neq() {
        let result = eval_xpath("<r/>", "'a' != 'b'");
        assert_eq!(result, XPathValue::Boolean(true));

        let result = eval_xpath("<r/>", "'a' != 'a'");
        assert_eq!(result, XPathValue::Boolean(false));
    }

    // Regression tests for issue #44: with a node-set operand, `!=` is its
    // own existential test (XPath 1.0 §3.4), not the negation of `=`.
    const NEQ_DOC: &str =
        "<r><a x='1'>one</a><a x='2'>two</a><g><b>p</b><b>q</b></g><h><b>p</b><b>p</b></h></r>";

    #[test]
    fn test_neq_empty_nodeset_is_false() {
        // An empty node-set satisfies neither `=` nor `!=`.
        assert_eq!(
            eval_xpath(NEQ_DOC, "//nothere != 'x'"),
            XPathValue::Boolean(false)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//nothere != 1"),
            XPathValue::Boolean(false)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//nothere != //a"),
            XPathValue::Boolean(false)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//nothere != //nothere"),
            XPathValue::Boolean(false)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//nothere = 'x'"),
            XPathValue::Boolean(false)
        );
    }

    #[test]
    fn test_neq_nodeset_string_existential() {
        // The node "one" makes `//a != 'two'` true, while the node "two"
        // makes `//a = 'two'` true as well: both hold simultaneously.
        assert_eq!(
            eval_xpath(NEQ_DOC, "//a != 'two'"),
            XPathValue::Boolean(true)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//a = 'two'"),
            XPathValue::Boolean(true)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//a = 'two' and //a != 'two'"),
            XPathValue::Boolean(true)
        );
        // Single-node sets behave like scalars.
        assert_eq!(
            eval_xpath(NEQ_DOC, "//g/b[1] != 'p'"),
            XPathValue::Boolean(false)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//g/b[2] != 'p'"),
            XPathValue::Boolean(true)
        );
    }

    #[test]
    fn test_neq_attribute_nodeset_number() {
        assert_eq!(
            eval_xpath(NEQ_DOC, "//a/@x != 1"),
            XPathValue::Boolean(true)
        );
        assert_eq!(eval_xpath(NEQ_DOC, "//a/@x = 1"), XPathValue::Boolean(true));
    }

    #[test]
    fn test_neq_absent_attribute_is_false() {
        // An attribute step matching nothing is an EMPTY node-set, so both
        // `=` and `!=` are false — not a String("") sentinel that would
        // make `!=` true through the scalar branch.
        assert_eq!(
            eval_xpath(NEQ_DOC, "//g/@missing != 'v'"),
            XPathValue::Boolean(false)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//g/@missing = 'v'"),
            XPathValue::Boolean(false)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//g/@missing != 5"),
            XPathValue::Boolean(false)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//nothere/@x != 'v'"),
            XPathValue::Boolean(false)
        );
        // A predicate on a missing attribute selects nothing.
        assert_eq!(eval_count(NEQ_DOC, "//*[@missing != 'v']"), 0);
        // string() of an absent attribute is still the empty string.
        assert_eq!(
            eval_xpath(NEQ_DOC, "string(//g/@missing)"),
            XPathValue::String(String::new())
        );
    }

    // -- First-class attribute nodes (issue #47) ----------------------------

    #[test]
    fn test_attr_vs_attr_comparison_not_clobbered() {
        // Two attribute node-sets in one comparison must each keep their own
        // values: x = {1,2}, y = {3,3} has an unequal pair, so != is true.
        let xml = "<r><a x='1' y='3'/><a x='2' y='3'/></r>";
        assert_eq!(
            eval_xpath(xml, "//a/@x != //a/@y"),
            XPathValue::Boolean(true)
        );
        // x = {1,3}, y = {2,4} share no value, so = is false.
        let xml2 = "<r><a x='1' y='2'/><a x='3' y='4'/></r>";
        assert_eq!(
            eval_xpath(xml2, "//a/@x = //a/@y"),
            XPathValue::Boolean(false)
        );
        // And a genuinely shared value makes = true.
        let xml3 = "<r><a x='1' y='2'/><a x='3' y='1'/></r>";
        assert_eq!(
            eval_xpath(xml3, "//a/@x = //a/@y"),
            XPathValue::Boolean(true)
        );
    }

    #[test]
    fn test_count_single_attribute_filter_path() {
        // A single-match attribute step is a node-set of one attribute node,
        // usable in node-set contexts like count().
        let xml = "<r><a href='u1'/><a/></r>";
        assert_eq!(
            eval_xpath(xml, "count((//a)[1]/@href)"),
            XPathValue::Number(1.0)
        );
        assert_eq!(eval_xpath(xml, "count(//a/@href)"), XPathValue::Number(1.0));
    }

    #[test]
    fn test_attribute_wildcard_counts_all_attributes() {
        // @* yields one node per attribute — not one per element.
        let xml = "<r><a x='1' y='2' z='3'/><b w='4'/></r>";
        assert_eq!(eval_xpath(xml, "count(//@*)"), XPathValue::Number(4.0));
        assert_eq!(eval_count(xml, "//a/@*"), 3);
        // sum() over the wildcard sees every value.
        assert_eq!(eval_xpath(xml, "sum(//@*)"), XPathValue::Number(10.0));
    }

    #[test]
    fn test_attribute_wildcard_excludes_namespace_declarations() {
        // xmlns/xmlns:* are namespace declarations, not attribute nodes
        // (XPath 1.0 §5.3).
        let xml = "<r xmlns:p='urn:p' id='1'><p:a p:x='2'/></r>";
        assert_eq!(eval_count(xml, "//@*"), 2);
    }

    #[test]
    fn test_attribute_node_name_functions() {
        let xml = "<r xmlns:p='urn:p'><a p:href='u1'/></r>";
        assert_eq!(
            eval_xpath(xml, "name(//a/@*)"),
            XPathValue::String("p:href".to_owned())
        );
        assert_eq!(
            eval_xpath(xml, "local-name(//a/@*)"),
            XPathValue::String("href".to_owned())
        );
        assert_eq!(
            eval_xpath(xml, "namespace-uri(//a/@*)"),
            XPathValue::String("urn:p".to_owned())
        );
    }

    #[test]
    fn test_attribute_parent_axis() {
        let xml = "<r><a x='1'/></r>";
        assert_eq!(
            eval_xpath(xml, "name(//a/@x/..)"),
            XPathValue::String("a".to_owned())
        );
        assert_eq!(eval_count(xml, "//a/@x/ancestor::*"), 2);
    }

    #[test]
    fn test_attribute_predicate_on_attribute_step() {
        // Predicates over attribute node-sets see the attribute's own
        // string-value as the context node.
        let xml = "<r><a x='1'/><a x='2'/><a x='3'/></r>";
        assert_eq!(eval_count(xml, "//a/@x[. > 1]"), 2);
        assert_eq!(
            eval_xpath(xml, "string(//a/@x[. = '2'])"),
            XPathValue::String("2".to_owned())
        );
    }

    #[test]
    fn test_union_of_elements_and_attributes() {
        // Mixed node-sets sort in document order with an attribute directly
        // after its owner element.
        let xml = "<r><a x='1'>t</a></r>";
        let XPathValue::NodeSet(ns) = eval_xpath(xml, "//a | //a/@x") else {
            panic!("expected node-set");
        };
        assert_eq!(ns.len(), 2);
        assert!(!ns[0].is_attribute());
        assert!(ns[1].is_attribute());
        assert_eq!(ns[1].anchor(), ns[0].anchor());
    }

    #[test]
    fn test_mixed_attribute_input_document_order() {
        // Axis expansions from inputs that mix elements and attributes must
        // come out in document order.
        let xml = "<root><e a='1'><c1/><c2/></e></root>";
        let XPathValue::NodeSet(ns) = eval_xpath(xml, "(//e | //@a)/descendant-or-self::node()")
        else {
            panic!("expected node-set");
        };
        // Document order: e, @a, c1, c2.
        assert_eq!(ns.len(), 4);
        assert!(!ns[0].is_attribute());
        assert!(ns[1].is_attribute());
        assert_eq!(
            eval_xpath(xml, "string(((//e | //@a)/descendant-or-self::node())[2])"),
            XPathValue::String("1".to_owned())
        );

        let xml2 = "<root><e a='1'><child/></e><after/></root>";
        assert_eq!(
            eval_xpath(xml2, "name(((//e | //@a)/following::*)[1])"),
            XPathValue::String("after".to_owned())
        );
    }

    #[test]
    fn test_step_predicate_position_is_per_context_node() {
        // XPath 1.0 §2.4: a step predicate filters the node-set generated
        // for each context node separately — //a/b[1] selects the FIRST b
        // of EVERY a, not the first of the merged set.
        let xml = "<root><a><b>1</b><b>2</b></a><a><b>3</b><b>4</b></a></root>";
        assert_eq!(eval_count(xml, "//a/b[1]"), 2);
        assert_eq!(eval_count(xml, "//a/b[last()]"), 2);
        assert_eq!(eval_count(xml, "//a/b[2]"), 2);
        // The parenthesized form applies the position globally.
        assert_eq!(eval_count(xml, "(//a/b)[1]"), 1);

        // Same semantics over attribute steps.
        let xml2 = "<root><a p='1' q='2'/><a p='3' q='4'/></root>";
        assert_eq!(eval_count(xml2, "//a/@*[1]"), 2);
        assert_eq!(eval_count(xml2, "//a/@*[last()]"), 2);
        assert_eq!(eval_xpath(xml2, "sum(//a/@*[1])"), XPathValue::Number(4.0));

        // Predicates on the step after `//` also keep the proximity context
        // of each descendant-or-self node.
        assert_eq!(eval_count(xml, "//*[1]"), 4);
        assert_eq!(eval_count(xml, "//a[last()]"), 1);
    }

    #[test]
    fn test_prefixed_attribute_name_test() {
        // A prefixed attribute name test matches by literal prefix without
        // bindings, and namespace-aware with bindings.
        let xml = "<r xmlns:xl='http://www.w3.org/1999/xlink'><a xl:href='u1'/></r>";
        assert_eq!(
            eval_xpath(xml, "string(//a/@xl:href)"),
            XPathValue::String("u1".to_owned())
        );
    }

    #[test]
    fn test_count_of_absent_attribute_filter_path() {
        // count() over a filter-path attribute step with no matches must
        // return 0, not a type error.
        let xml = "<r><a href='u1'/><a/></r>";
        assert_eq!(
            eval_xpath(xml, "count((//a)/@missing)"),
            XPathValue::Number(0.0)
        );
    }

    #[test]
    fn test_neq_nodeset_nodeset() {
        // g/b = {p, q}, h/b = {p, p}: the pair (q, p) is unequal.
        assert_eq!(
            eval_xpath(NEQ_DOC, "//g/b != //h/b"),
            XPathValue::Boolean(true)
        );
        // h/b vs itself: every pair is (p, p), so no unequal pair exists.
        assert_eq!(
            eval_xpath(NEQ_DOC, "//h/b != //h/b"),
            XPathValue::Boolean(false)
        );
    }

    #[test]
    fn test_neq_in_predicate() {
        // Only <g> has a b child whose string-value differs from 'p'.
        assert_eq!(eval_count(NEQ_DOC, "//*[b != 'p']"), 1);
    }

    #[test]
    fn test_neq_nodeset_boolean_stays_negation() {
        // Node-set vs boolean converts the node-set to a boolean first
        // (XPath 1.0 §3.4), so `!=` IS plain negation there.
        assert_eq!(
            eval_xpath(NEQ_DOC, "//nothere != true()"),
            XPathValue::Boolean(true)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//nothere != false()"),
            XPathValue::Boolean(false)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//a != true()"),
            XPathValue::Boolean(false)
        );
        assert_eq!(
            eval_xpath(NEQ_DOC, "//a != false()"),
            XPathValue::Boolean(true)
        );
    }

    // -- Filter-path expressions (issue #20) --------------------------------

    #[test]
    fn test_filter_path_attribute_value() {
        // Regression test for issue #20: `string((//span/a)[1]/@href)` must
        // return the attribute value, not the anchor's text content.
        let xml =
            "<r><span class='titleline'><a href='https://x.example/'>Link Text</a></span></r>";
        let result = eval_xpath(xml, "string((//span[@class='titleline']/a)[1]/@href)");
        assert_eq!(result, XPathValue::String("https://x.example/".to_owned()));
    }

    #[test]
    fn test_filter_path_element_step() {
        // `(//a)/b` navigates to the b children; `(//a)[b]` filters the a
        // elements by the existence of a b child. They must differ.
        let xml = "<r><a><b>1</b><b>2</b></a><a/></r>";
        assert_eq!(eval_count(xml, "(//a)/b"), 2);
        assert_eq!(eval_count(xml, "(//a)[b]"), 1);
    }

    #[test]
    fn test_filter_path_double_slash() {
        let xml = "<r><a><b><c>deep</c></b></a><a><c>direct</c></a></r>";
        assert_eq!(eval_count(xml, "(//a)[1]//c"), 1);
        assert_eq!(eval_count(xml, "(//a)//c"), 2);
    }

    #[test]
    fn test_filter_path_parent_step() {
        let xml = "<r><a><b/></a></r>";
        let result = eval_xpath(xml, "name((//b)[1]/..)");
        assert_eq!(result, XPathValue::String("a".to_owned()));
    }

    #[test]
    fn test_filter_path_non_nodeset_is_type_error() {
        // A location-path continuation on a non-node-set is a type error.
        let doc = Document::parse_str("<r/>").unwrap();
        let root = doc.root_element().unwrap();
        let expr = parse("('str')/a").unwrap();
        let ctx = XPathContext::new(&doc, root);
        let err = ctx.evaluate(&expr).unwrap_err();
        assert!(
            matches!(err, XPathError::TypeError { .. }),
            "expected TypeError, got: {err:?}"
        );
    }

    #[test]
    fn test_neq_scalar_nan() {
        // NaN is unequal to everything, including itself.
        assert_eq!(
            eval_xpath("<r/>", "number('foo') != number('foo')"),
            XPathValue::Boolean(true)
        );
        assert_eq!(
            eval_xpath("<r/>", "number('foo') = number('foo')"),
            XPathValue::Boolean(false)
        );
    }

    #[test]
    fn test_comparison_lt_gt() {
        let result = eval_xpath("<r/>", "1 < 2");
        assert_eq!(result, XPathValue::Boolean(true));

        let result = eval_xpath("<r/>", "2 > 1");
        assert_eq!(result, XPathValue::Boolean(true));

        let result = eval_xpath("<r/>", "2 < 1");
        assert_eq!(result, XPathValue::Boolean(false));
    }

    // -- Boolean operators --------------------------------------------------

    #[test]
    fn test_boolean_and() {
        let result = eval_xpath("<r/>", "true() and false()");
        assert_eq!(result, XPathValue::Boolean(false));

        let result = eval_xpath("<r/>", "true() and true()");
        assert_eq!(result, XPathValue::Boolean(true));
    }

    #[test]
    fn test_boolean_or() {
        let result = eval_xpath("<r/>", "true() or false()");
        assert_eq!(result, XPathValue::Boolean(true));

        let result = eval_xpath("<r/>", "false() or false()");
        assert_eq!(result, XPathValue::Boolean(false));
    }

    // -- String functions ---------------------------------------------------

    #[test]
    fn test_concat() {
        let result = eval_xpath("<r/>", "concat('a', 'b')");
        assert_eq!(result, XPathValue::String("ab".to_owned()));

        let result = eval_xpath("<r/>", "concat('a', 'b', 'c')");
        assert_eq!(result, XPathValue::String("abc".to_owned()));
    }

    #[test]
    fn test_string_length() {
        let result = eval_xpath("<r/>", "string-length('hello')");
        assert_eq!(result, XPathValue::Number(5.0));
    }

    #[test]
    fn test_contains() {
        let result = eval_xpath("<r/>", "contains('hello', 'ell')");
        assert_eq!(result, XPathValue::Boolean(true));

        let result = eval_xpath("<r/>", "contains('hello', 'xyz')");
        assert_eq!(result, XPathValue::Boolean(false));
    }

    #[test]
    fn test_starts_with() {
        let result = eval_xpath("<r/>", "starts-with('hello', 'hel')");
        assert_eq!(result, XPathValue::Boolean(true));

        let result = eval_xpath("<r/>", "starts-with('hello', 'xyz')");
        assert_eq!(result, XPathValue::Boolean(false));
    }

    #[test]
    fn test_substring() {
        // substring('12345', 2, 3) = '234'
        let result = eval_xpath("<r/>", "substring('12345', 2, 3)");
        assert_eq!(result, XPathValue::String("234".to_owned()));

        // substring('12345', 2) = '2345'
        let result = eval_xpath("<r/>", "substring('12345', 2)");
        assert_eq!(result, XPathValue::String("2345".to_owned()));
    }

    #[test]
    fn test_normalize_space() {
        let result = eval_xpath("<r/>", "normalize-space('  hello   world  ')");
        assert_eq!(result, XPathValue::String("hello world".to_owned()));
    }

    #[test]
    fn test_translate() {
        let result = eval_xpath("<r/>", "translate('bar', 'abc', 'ABC')");
        assert_eq!(result, XPathValue::String("BAr".to_owned()));
    }

    #[test]
    fn test_substring_before_after() {
        let result = eval_xpath("<r/>", "substring-before('1999/04/01', '/')");
        assert_eq!(result, XPathValue::String("1999".to_owned()));

        let result = eval_xpath("<r/>", "substring-after('1999/04/01', '/')");
        assert_eq!(result, XPathValue::String("04/01".to_owned()));
    }

    // -- Number functions ---------------------------------------------------

    #[test]
    fn test_floor() {
        let result = eval_xpath("<r/>", "floor(1.5)");
        assert_eq!(result, XPathValue::Number(1.0));

        let result = eval_xpath("<r/>", "floor(-1.5)");
        assert_eq!(result, XPathValue::Number(-2.0));
    }

    #[test]
    fn test_ceiling() {
        let result = eval_xpath("<r/>", "ceiling(1.5)");
        assert_eq!(result, XPathValue::Number(2.0));

        let result = eval_xpath("<r/>", "ceiling(-1.5)");
        assert_eq!(result, XPathValue::Number(-1.0));
    }

    #[test]
    fn test_round() {
        let result = eval_xpath("<r/>", "round(1.5)");
        assert_eq!(result, XPathValue::Number(2.0));

        let result = eval_xpath("<r/>", "round(-0.5)");
        assert_eq!(result, XPathValue::Number(0.0));

        let result = eval_xpath("<r/>", "round(2.5)");
        assert_eq!(result, XPathValue::Number(3.0));
    }

    // -- count(), position(), last() ----------------------------------------

    #[test]
    fn test_count() {
        let result = eval_xpath("<r><a/><b/><c/></r>", "count(*)");
        assert_eq!(result, XPathValue::Number(3.0));
    }

    #[test]
    fn test_position_and_last() {
        // position() and last() in the default singleton context
        let result = eval_xpath("<r/>", "position()");
        assert_eq!(result, XPathValue::Number(1.0));

        let result = eval_xpath("<r/>", "last()");
        assert_eq!(result, XPathValue::Number(1.0));
    }

    // -- Path evaluation ----------------------------------------------------

    #[test]
    fn test_simple_child_path() {
        let count = eval_count("<r><a/><b/><c/></r>", "a");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_root_path() {
        let xml = "<root><child>text</child></root>";
        let result = eval_xpath_from_doc_root(xml, "/root/child");
        match &result {
            XPathValue::NodeSet(ns) => assert_eq!(ns.len(), 1),
            _ => panic!("expected node-set"),
        }
    }

    #[test]
    fn test_descendant_axis() {
        // //child matches anywhere in the tree
        let xml = "<r><a><b/></a></r>";
        let result = eval_xpath_from_doc_root(xml, "//b");
        match &result {
            XPathValue::NodeSet(ns) => assert_eq!(ns.len(), 1),
            _ => panic!("expected node-set"),
        }
    }

    #[test]
    fn test_parent_axis() {
        // child/.. should go back to the parent
        let xml = "<r><a><b/></a></r>";
        let doc = Document::parse_str(xml).unwrap();
        let root_elem = doc.root_element().unwrap();
        // Navigate to <a>
        let a = doc.children(root_elem).next().unwrap();
        // Navigate to <b>
        let b = doc.children(a).next().unwrap();

        let expr = parse("..").unwrap();
        let ctx = XPathContext::new(&doc, b);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert_eq!(ns.len(), 1);
                assert_eq!(ns[0], XPathNode::Node(a));
            }
            _ => panic!("expected node-set"),
        }
    }

    #[test]
    fn test_self_axis() {
        let xml = "<r><a/></r>";
        let doc = Document::parse_str(xml).unwrap();
        let root_elem = doc.root_element().unwrap();

        let expr = parse(".").unwrap();
        let ctx = XPathContext::new(&doc, root_elem);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert_eq!(ns.len(), 1);
                assert_eq!(ns[0], XPathNode::Node(root_elem));
            }
            _ => panic!("expected node-set"),
        }
    }

    // -- Predicates ---------------------------------------------------------

    #[test]
    fn test_positional_predicate() {
        let xml = "<r><a/><b/><c/></r>";
        let count = eval_count(xml, "*[1]");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_last_predicate() {
        let xml = "<r><a/><b/><c/></r>";
        let count = eval_count(xml, "*[last()]");
        assert_eq!(count, 1);
    }

    #[test]
    fn test_attribute_predicate() {
        let xml = r#"<r><a id="x"/><a id="y"/></r>"#;
        let doc = Document::parse_str(xml).unwrap();
        let root_elem = doc.root_element().unwrap();
        let expr = parse("a[@id='x']").unwrap();
        let ctx = XPathContext::new(&doc, root_elem);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert_eq!(ns.len(), 1);
                // Check it's the right element
                assert_eq!(doc.attribute(ns[0].anchor(), "id"), Some("x"));
            }
            _ => panic!("expected node-set"),
        }
    }

    // -- Variables -----------------------------------------------------------

    #[test]
    fn test_variable() {
        let xml = "<r/>";
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root_element().unwrap();
        let expr = parse("$x + 1").unwrap();
        let mut ctx = XPathContext::new(&doc, root);
        ctx.set_variable("x", XPathValue::Number(41.0));
        let result = ctx.evaluate(&expr).unwrap();
        assert_eq!(result, XPathValue::Number(42.0));
    }

    #[test]
    fn test_undefined_variable_error() {
        let xml = "<r/>";
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root_element().unwrap();
        let expr = parse("$undefined").unwrap();
        let ctx = XPathContext::new(&doc, root);
        let result = ctx.evaluate(&expr);
        assert!(result.is_err());
    }

    // -- Union ---------------------------------------------------------------

    #[test]
    fn test_union() {
        let xml = "<r><a/><b/></r>";
        let count = eval_count(xml, "a | b");
        assert_eq!(count, 2);
    }

    // -- Complex expression --------------------------------------------------

    #[test]
    fn test_complex_book_price() {
        let xml = r"<store>
            <book><title>A</title><price>30</price></book>
            <book><title>B</title><price>40</price></book>
        </store>";
        let doc = Document::parse_str(xml).unwrap();
        let root_elem = doc.root_element().unwrap();
        // Count books with price > 35
        let expr = parse("count(book[price > 35])").unwrap();
        let ctx = XPathContext::new(&doc, root_elem);
        let result = ctx.evaluate(&expr).unwrap();
        assert_eq!(result, XPathValue::Number(1.0));
    }

    // -- String value of nodes -----------------------------------------------

    #[test]
    fn test_string_function_on_context() {
        let xml = "<r>hello</r>";
        let result = eval_xpath(xml, "string()");
        assert_eq!(result, XPathValue::String("hello".to_owned()));
    }

    // -- Deep path traversal -------------------------------------------------

    #[test]
    fn test_deep_path_traversal() {
        let xml = "<a><b><c><d>deep</d></c></b></a>";
        let result = eval_xpath_from_doc_root(xml, "/a/b/c/d");
        match &result {
            XPathValue::NodeSet(ns) => {
                assert_eq!(ns.len(), 1);
                let doc = Document::parse_str(xml).unwrap();
                assert_eq!(doc.text_content(ns[0].anchor()), "deep");
            }
            _ => panic!("expected node-set"),
        }
    }

    // -- Unary negation ------------------------------------------------------

    #[test]
    fn test_unary_neg() {
        let result = eval_xpath("<r/>", "-(3)");
        assert_eq!(result, XPathValue::Number(-3.0));
    }

    // -- Not function --------------------------------------------------------

    #[test]
    fn test_not() {
        let result = eval_xpath("<r/>", "not(true())");
        assert_eq!(result, XPathValue::Boolean(false));

        let result = eval_xpath("<r/>", "not(false())");
        assert_eq!(result, XPathValue::Boolean(true));
    }

    // -- Name functions ------------------------------------------------------

    #[test]
    fn test_name_function() {
        let xml = "<root/>";
        let result = eval_xpath(xml, "name()");
        assert_eq!(result, XPathValue::String("root".to_owned()));
    }

    #[test]
    fn test_local_name_function() {
        let xml = "<root/>";
        let result = eval_xpath(xml, "local-name()");
        assert_eq!(result, XPathValue::String("root".to_owned()));
    }

    // -- Sum function --------------------------------------------------------

    #[test]
    fn test_sum() {
        let xml = "<r><n>1</n><n>2</n><n>3</n></r>";
        let result = eval_xpath(xml, "sum(n)");
        assert_eq!(result, XPathValue::Number(6.0));
    }

    // -- Following/Preceding sibling axes ------------------------------------

    #[test]
    fn test_following_sibling() {
        let xml = "<r><a/><b/><c/></r>";
        let doc = Document::parse_str(xml).unwrap();
        let root_elem = doc.root_element().unwrap();
        let a = doc.children(root_elem).next().unwrap();

        let expr = parse("following-sibling::*").unwrap();
        let ctx = XPathContext::new(&doc, a);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => assert_eq!(ns.len(), 2), // b and c
            _ => panic!("expected node-set"),
        }
    }

    #[test]
    fn test_preceding_sibling() {
        let xml = "<r><a/><b/><c/></r>";
        let doc = Document::parse_str(xml).unwrap();
        let root_elem = doc.root_element().unwrap();
        let children: Vec<_> = doc.children(root_elem).collect();
        let c = children[2]; // <c/>

        let expr = parse("preceding-sibling::*").unwrap();
        let ctx = XPathContext::new(&doc, c);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => assert_eq!(ns.len(), 2), // a and b
            _ => panic!("expected node-set"),
        }
    }

    // -- id() function -------------------------------------------------------

    #[test]
    fn test_xpath_id_single() {
        let mut doc = Document::parse_str(
            r#"<root><item id="x">Hello</item><item id="y">World</item></root>"#,
        )
        .unwrap();
        // Populate the id_map
        let root = doc.root_element().unwrap();
        let children: Vec<_> = doc.children(root).collect();
        doc.set_id("x", children[0]);
        doc.set_id("y", children[1]);

        let expr = parse("id('x')").unwrap();
        let ctx = XPathContext::new(&doc, root);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert_eq!(ns.len(), 1);
                assert_eq!(doc.node_name(ns[0].anchor()), Some("item"));
                assert_eq!(doc.text_content(ns[0].anchor()), "Hello");
            }
            other => panic!("expected node-set, got {other:?}"),
        }
    }

    #[test]
    fn test_xpath_id_multiple_space_separated() {
        let mut doc =
            Document::parse_str(r#"<root><a id="p">1</a><b id="q">2</b><c id="r">3</c></root>"#)
                .unwrap();
        let root = doc.root_element().unwrap();
        let children: Vec<_> = doc.children(root).collect();
        doc.set_id("p", children[0]);
        doc.set_id("q", children[1]);
        doc.set_id("r", children[2]);

        let expr = parse("id('p r')").unwrap();
        let ctx = XPathContext::new(&doc, root);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert_eq!(ns.len(), 2);
                assert_eq!(doc.node_name(ns[0].anchor()), Some("a"));
                assert_eq!(doc.node_name(ns[1].anchor()), Some("c"));
            }
            other => panic!("expected node-set, got {other:?}"),
        }
    }

    #[test]
    fn test_xpath_id_unknown_returns_empty() {
        let doc = Document::parse_str("<root/>").unwrap();
        let root = doc.root_element().unwrap();

        let expr = parse("id('nonexistent')").unwrap();
        let ctx = XPathContext::new(&doc, root);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => assert!(ns.is_empty()),
            other => panic!("expected empty node-set, got {other:?}"),
        }
    }

    // -- Namespace axis -------------------------------------------------------

    #[test]
    fn test_namespace_axis_with_declaration() {
        let xml = r#"<root xmlns:ns="http://example.com"><child/></root>"#;
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root_element().unwrap();

        // namespace::ns should match because ns is in scope
        let expr = parse("namespace::ns").unwrap();
        let ctx = XPathContext::new(&doc, root);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => assert!(!ns.is_empty(), "expected namespace::ns to match"),
            other => panic!("expected node-set, got {other:?}"),
        }
    }

    #[test]
    fn test_namespace_axis_inherited() {
        let xml = r#"<root xmlns:ns="http://example.com"><child/></root>"#;
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root_element().unwrap();
        let child = doc.first_child(root).unwrap();

        // namespace::ns should be in scope on child (inherited from root)
        let expr = parse("namespace::ns").unwrap();
        let ctx = XPathContext::new(&doc, child);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert!(!ns.is_empty(), "expected namespace::ns to be inherited");
            }
            other => panic!("expected node-set, got {other:?}"),
        }
    }

    #[test]
    fn test_namespace_axis_xml_always_in_scope() {
        let xml = "<root/>";
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root_element().unwrap();

        // namespace::xml should always be in scope
        let expr = parse("namespace::xml").unwrap();
        let ctx = XPathContext::new(&doc, root);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert!(
                    !ns.is_empty(),
                    "expected namespace::xml to always be in scope"
                );
            }
            other => panic!("expected node-set, got {other:?}"),
        }
    }

    #[test]
    fn test_namespace_axis_wildcard() {
        let xml = r#"<root xmlns:a="http://a" xmlns:b="http://b"><child/></root>"#;
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root_element().unwrap();

        // namespace::* should return non-empty (a, b, xml are all in scope)
        let expr = parse("namespace::*").unwrap();
        let ctx = XPathContext::new(&doc, root);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert!(!ns.is_empty(), "expected namespace::* to match");
            }
            other => panic!("expected node-set, got {other:?}"),
        }
    }

    #[test]
    fn test_namespace_axis_nonexistent_prefix() {
        let xml = "<root/>";
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root_element().unwrap();

        // namespace::nonexistent should return empty
        let expr = parse("namespace::nonexistent").unwrap();
        let ctx = XPathContext::new(&doc, root);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert!(ns.is_empty(), "expected no match for nonexistent prefix");
            }
            other => panic!("expected node-set, got {other:?}"),
        }
    }

    #[test]
    fn test_namespace_axis_on_text_node() {
        let xml = "<root>text</root>";
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root_element().unwrap();
        let text = doc.first_child(root).unwrap();

        // namespace axis on a text node should return empty
        let expr = parse("namespace::*").unwrap();
        let ctx = XPathContext::new(&doc, text);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert!(ns.is_empty(), "namespace axis on text node should be empty");
            }
            other => panic!("expected node-set, got {other:?}"),
        }
    }

    #[test]
    fn test_namespace_axis_default_namespace() {
        let xml = r#"<root xmlns="http://default"><child/></root>"#;
        let doc = Document::parse_str(xml).unwrap();
        let root = doc.root_element().unwrap();

        // namespace::* should include the default namespace
        let expr = parse("namespace::*").unwrap();
        let ctx = XPathContext::new(&doc, root);
        let result = ctx.evaluate(&expr).unwrap();
        match &result {
            XPathValue::NodeSet(ns) => {
                assert!(!ns.is_empty(), "expected default namespace to be in scope");
            }
            other => panic!("expected node-set, got {other:?}"),
        }
    }
}
