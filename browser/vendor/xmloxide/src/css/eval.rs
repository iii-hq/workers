//! CSS selector evaluation against a [`Document`] tree.

use crate::tree::{Document, NodeId, NodeKind};

use super::types::{
    AttrOp, AttrSelector, Combinator, CompoundSelector, NthExpr, PseudoClass, Selector,
    SelectorGroup,
};

/// Evaluate a parsed selector group against the document, starting from `scope`.
///
/// Returns all descendant nodes of `scope` that match any selector in the group.
pub fn select(doc: &Document, scope: NodeId, group: &SelectorGroup) -> Vec<NodeId> {
    // Fast path: if every selector in the group is a simple `#id` selector,
    // use element_by_id for O(1) lookup instead of walking the tree.
    if let Some(results) = try_fast_id_select(doc, scope, group) {
        return results;
    }

    let mut results = Vec::new();
    collect_descendants(doc, scope, group, &mut results);
    results
}

/// Attempts to use the fast `id_map` for pure `#id` selectors.
/// Returns `None` if any selector is not a simple ID selector.
fn try_fast_id_select(doc: &Document, scope: NodeId, group: &SelectorGroup) -> Option<Vec<NodeId>> {
    let mut results = Vec::new();
    for sel in &group.selectors {
        // Must be a single compound with only an ID
        if sel.compounds.len() != 1 {
            return None;
        }
        let compound = &sel.compounds[0].compound;
        let id = compound.id.as_ref()?;
        if compound.tag.is_some()
            || !compound.classes.is_empty()
            || !compound.attrs.is_empty()
            || !compound.pseudos.is_empty()
        {
            return None;
        }

        // Look up via id_map
        if let Some(node) = doc.element_by_id(id) {
            // Verify the node is a descendant of scope
            if is_descendant_of(doc, node, scope) && !results.contains(&node) {
                results.push(node);
            }
        }
    }
    Some(results)
}

/// Returns true if `node` is a descendant of `ancestor`.
fn is_descendant_of(doc: &Document, node: NodeId, ancestor: NodeId) -> bool {
    let mut current = doc.parent(node);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = doc.parent(id);
    }
    false
}

/// Recursively collect matching descendants.
fn collect_descendants(
    doc: &Document,
    node: NodeId,
    group: &SelectorGroup,
    results: &mut Vec<NodeId>,
) {
    for child in doc.children(node) {
        if matches!(doc.node(child).kind, NodeKind::Element { .. }) {
            if group
                .selectors
                .iter()
                .any(|sel| matches_selector(doc, child, sel))
            {
                results.push(child);
            }
            collect_descendants(doc, child, group, results);
        }
    }
}

/// Check if a node matches a complete selector (chain of compounds with combinators).
fn matches_selector(doc: &Document, node: NodeId, selector: &Selector) -> bool {
    // Walk the compound chain backwards from the rightmost (subject) compound
    let compounds = &selector.compounds;
    if compounds.is_empty() {
        return false;
    }

    // The last compound must match the node itself
    let last = compounds.len() - 1;
    if !matches_compound(doc, node, &compounds[last].compound) {
        return false;
    }

    // Walk backwards through the chain
    let mut current = node;
    for i in (0..last).rev() {
        let entry = &compounds[i];
        let next_combinator = compounds[i + 1].combinator;
        match next_combinator {
            Combinator::None => {}
            Combinator::Descendant => {
                // Find an ancestor that matches
                let mut found = false;
                let mut ancestor = doc.parent(current);
                while let Some(anc) = ancestor {
                    if matches!(doc.node(anc).kind, NodeKind::Element { .. })
                        && matches_compound(doc, anc, &entry.compound)
                    {
                        current = anc;
                        found = true;
                        break;
                    }
                    ancestor = doc.parent(anc);
                }
                if !found {
                    return false;
                }
            }
            Combinator::Child => {
                // Parent must match
                if let Some(parent) = doc.parent(current) {
                    if matches!(doc.node(parent).kind, NodeKind::Element { .. })
                        && matches_compound(doc, parent, &entry.compound)
                    {
                        current = parent;
                    } else {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            Combinator::NextSibling => {
                // Previous sibling element must match
                if let Some(prev) = prev_element_sibling(doc, current) {
                    if matches_compound(doc, prev, &entry.compound) {
                        current = prev;
                    } else {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            Combinator::SubsequentSibling => {
                // Any preceding sibling element must match
                let mut found = false;
                let mut prev = prev_element_sibling(doc, current);
                while let Some(p) = prev {
                    if matches_compound(doc, p, &entry.compound) {
                        current = p;
                        found = true;
                        break;
                    }
                    prev = prev_element_sibling(doc, p);
                }
                if !found {
                    return false;
                }
            }
        }
    }

    true
}

/// Check if a node matches a compound selector (all simple selectors must match).
fn matches_compound(doc: &Document, node: NodeId, compound: &CompoundSelector) -> bool {
    // Tag name
    if let Some(ref tag) = compound.tag {
        let name = doc.node_name(node).unwrap_or("");
        if !name.eq_ignore_ascii_case(tag) {
            return false;
        }
    }

    // ID — use element_by_id for O(1) lookup when the id_map is populated,
    // falling back to attribute scan when it's not.
    if let Some(ref id) = compound.id {
        if let Some(target) = doc.element_by_id(id) {
            if target != node {
                return false;
            }
        } else {
            // id_map doesn't have this ID — either the element doesn't exist
            // or the id_map wasn't populated. Fall back to attribute scan.
            let node_id_attr = doc.attribute(node, "id").unwrap_or("");
            if node_id_attr != id {
                return false;
            }
        }
    }

    // Classes
    for class in &compound.classes {
        let class_attr = doc.attribute(node, "class").unwrap_or("");
        if !class_attr.split_ascii_whitespace().any(|c| c == class) {
            return false;
        }
    }

    // Attribute selectors
    for attr in &compound.attrs {
        if !matches_attr(doc, node, attr) {
            return false;
        }
    }

    // Pseudo-classes
    for pseudo in &compound.pseudos {
        if !matches_pseudo(doc, node, pseudo) {
            return false;
        }
    }

    true
}

/// Check if a node matches an attribute selector.
fn matches_attr(doc: &Document, node: NodeId, sel: &AttrSelector) -> bool {
    let Some(value) = doc.attribute(node, &sel.name) else {
        return false;
    };

    let Some(matcher) = &sel.matcher else {
        return true; // existence check only
    };

    let (val, expected) = if matcher.case_insensitive {
        (
            value.to_ascii_lowercase(),
            matcher.value.to_ascii_lowercase(),
        )
    } else {
        (value.to_string(), matcher.value.clone())
    };

    match matcher.op {
        AttrOp::Exact => val == expected,
        AttrOp::Word => val.split_ascii_whitespace().any(|w| w == expected),
        AttrOp::DashPrefix => val == expected || val.starts_with(&format!("{expected}-")),
        AttrOp::Prefix => val.starts_with(&expected),
        AttrOp::Suffix => val.ends_with(&expected),
        AttrOp::Substring => val.contains(&expected),
    }
}

/// Check if a node matches a pseudo-class.
fn matches_pseudo(doc: &Document, node: NodeId, pseudo: &PseudoClass) -> bool {
    match pseudo {
        PseudoClass::FirstChild => {
            // Node is the first element child of its parent
            doc.parent(node)
                .and_then(|p| first_element_child(doc, p))
                .is_some_and(|first| first == node)
        }
        PseudoClass::LastChild => doc
            .parent(node)
            .and_then(|p| last_element_child(doc, p))
            .is_some_and(|last| last == node),
        PseudoClass::OnlyChild => {
            if let Some(parent) = doc.parent(node) {
                let element_children: Vec<_> = doc
                    .children(parent)
                    .filter(|&c| matches!(doc.node(c).kind, NodeKind::Element { .. }))
                    .collect();
                element_children.len() == 1 && element_children[0] == node
            } else {
                false
            }
        }
        PseudoClass::Empty => {
            // No child elements or text nodes
            !doc.children(node).any(|c| {
                matches!(
                    doc.node(c).kind,
                    NodeKind::Element { .. } | NodeKind::Text { .. } | NodeKind::CData { .. }
                )
            })
        }
        PseudoClass::Not(inner) => !matches_compound(doc, node, inner),
        PseudoClass::NthChild(expr) => nth_child_matches(doc, node, *expr, false),
        PseudoClass::NthLastChild(expr) => nth_child_matches(doc, node, *expr, true),
    }
}

/// Check if a node's position among sibling elements matches an `An+B` expression.
fn nth_child_matches(doc: &Document, node: NodeId, expr: NthExpr, from_end: bool) -> bool {
    let Some(parent) = doc.parent(node) else {
        return false;
    };

    let element_children: Vec<_> = doc
        .children(parent)
        .filter(|&c| matches!(doc.node(c).kind, NodeKind::Element { .. }))
        .collect();

    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let pos = if from_end {
        element_children
            .iter()
            .rev()
            .position(|&c| c == node)
            .map(|p| p as i32 + 1)
    } else {
        element_children
            .iter()
            .position(|&c| c == node)
            .map(|p| p as i32 + 1)
    };

    pos.is_some_and(|p| expr.matches(p))
}

/// Find the previous element sibling of a node.
fn prev_element_sibling(doc: &Document, node: NodeId) -> Option<NodeId> {
    let mut prev = doc.prev_sibling(node);
    while let Some(p) = prev {
        if matches!(doc.node(p).kind, NodeKind::Element { .. }) {
            return Some(p);
        }
        prev = doc.prev_sibling(p);
    }
    None
}

/// Find the first element child.
fn first_element_child(doc: &Document, parent: NodeId) -> Option<NodeId> {
    doc.children(parent)
        .find(|&c| matches!(doc.node(c).kind, NodeKind::Element { .. }))
}

/// Find the last element child.
fn last_element_child(doc: &Document, parent: NodeId) -> Option<NodeId> {
    doc.children(parent)
        .filter(|&c| matches!(doc.node(c).kind, NodeKind::Element { .. }))
        .last()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::css::parser::parse_selector;
    use crate::tree::Document;

    /// Helper: parse a selector string and evaluate it against the document.
    fn eval(doc: &Document, scope: NodeId, css: &str) -> Vec<NodeId> {
        let group = parse_selector(css).unwrap();
        select(doc, scope, &group)
    }

    /// Shared test document covering common structures.
    fn test_doc() -> Document {
        Document::parse_str(
            r#"<root>
                <div id="main" class="container wide">
                    <h1>Title</h1>
                    <p class="intro">Hello</p>
                    <p class="body">World</p>
                    <ul>
                        <li class="active">One</li>
                        <li>Two</li>
                        <li class="last">Three</li>
                    </ul>
                    <a href="https://example.com" data-type="external">Link</a>
                    <img src="photo.png"/>
                    <span lang="en-US">English</span>
                    <span lang="en">Plain English</span>
                    <span lang="fr">French</span>
                    <div class="empty-div"/>
                </div>
                <div id="sidebar" class="sidebar">
                    <p class="intro">Side</p>
                </div>
            </root>"#,
        )
        .unwrap()
    }

    // ---------------------------------------------------------------
    // 1. Basic element matching by tag name
    // ---------------------------------------------------------------

    #[test]
    fn test_tag_name_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "p");
        assert_eq!(result.len(), 3); // 2 in main + 1 in sidebar
        for &node in &result {
            assert_eq!(doc.node_name(node), Some("p"));
        }
    }

    #[test]
    fn test_tag_name_case_insensitive() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // CSS tag matching should be case-insensitive
        let result = eval(&doc, root, "P");
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_tag_name_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "table");
        assert!(result.is_empty());
    }

    #[test]
    fn test_tag_name_h1() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "h1");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "Title");
    }

    // ---------------------------------------------------------------
    // 2. Class matching
    // ---------------------------------------------------------------

    #[test]
    fn test_class_single() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, ".intro");
        assert_eq!(result.len(), 2); // main p.intro + sidebar p.intro
    }

    #[test]
    fn test_class_multiple_on_element() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // The main div has class="container wide" — match on either individually
        let result_container = eval(&doc, root, ".container");
        assert_eq!(result_container.len(), 1);
        let result_wide = eval(&doc, root, ".wide");
        assert_eq!(result_wide.len(), 1);
        assert_eq!(result_container[0], result_wide[0]);
    }

    #[test]
    fn test_class_compound_both_required() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // Require both classes on the same element
        let result = eval(&doc, root, ".container.wide");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("div"));
    }

    #[test]
    fn test_class_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, ".nonexistent");
        assert!(result.is_empty());
    }

    #[test]
    fn test_class_with_tag() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "p.intro");
        assert_eq!(result.len(), 2);
    }

    // ---------------------------------------------------------------
    // 3. ID matching
    // ---------------------------------------------------------------

    #[test]
    fn test_id_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "#main");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("div"));
    }

    #[test]
    fn test_id_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "#nonexistent");
        assert!(result.is_empty());
    }

    #[test]
    fn test_id_with_tag() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "div#sidebar");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.attribute(result[0], "class"), Some("sidebar"));
    }

    #[test]
    fn test_id_multiple_ids_in_doc() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let main = eval(&doc, root, "#main");
        let sidebar = eval(&doc, root, "#sidebar");
        assert_eq!(main.len(), 1);
        assert_eq!(sidebar.len(), 1);
        assert_ne!(main[0], sidebar[0]);
    }

    // ---------------------------------------------------------------
    // 4. Attribute matching
    // ---------------------------------------------------------------

    #[test]
    fn test_attr_existence() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[href]");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("a"));
    }

    #[test]
    fn test_attr_existence_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[title]");
        assert!(result.is_empty());
    }

    #[test]
    fn test_attr_exact_value() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[data-type=\"external\"]");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("a"));
    }

    #[test]
    fn test_attr_exact_value_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[data-type=\"internal\"]");
        assert!(result.is_empty());
    }

    #[test]
    fn test_attr_prefix() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[href^=\"https\"]");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("a"));
    }

    #[test]
    fn test_attr_prefix_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[href^=\"ftp\"]");
        assert!(result.is_empty());
    }

    #[test]
    fn test_attr_suffix() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[src$=\".png\"]");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("img"));
    }

    #[test]
    fn test_attr_suffix_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[src$=\".jpg\"]");
        assert!(result.is_empty());
    }

    #[test]
    fn test_attr_substring() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[href*=\"example\"]");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("a"));
    }

    #[test]
    fn test_attr_substring_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[href*=\"missing\"]");
        assert!(result.is_empty());
    }

    #[test]
    fn test_attr_word() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // class="container wide" — match the word "container"
        let result = eval(&doc, root, "[class~=\"container\"]");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.attribute(result[0], "id"), Some("main"));
    }

    #[test]
    fn test_attr_dash_prefix_exact() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // lang="en" exactly matches [lang|="en"]
        let result = eval(&doc, root, "[lang|=\"en\"]");
        // Should match both lang="en-US" and lang="en", but NOT lang="fr"
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_attr_dash_prefix_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "[lang|=\"de\"]");
        assert!(result.is_empty());
    }

    // ---------------------------------------------------------------
    // 5. Pseudo-class matching
    // ---------------------------------------------------------------

    #[test]
    fn test_pseudo_first_child() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "li:first-child");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "One");
    }

    #[test]
    fn test_pseudo_last_child() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "li:last-child");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "Three");
    }

    #[test]
    fn test_pseudo_first_child_div() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // The first div child of root is #main
        let result = eval(&doc, root, "div:first-child");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.attribute(result[0], "id"), Some("main"));
    }

    #[test]
    fn test_pseudo_empty() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, ":empty");
        // img and empty-div should be empty
        let names: Vec<_> = result.iter().map(|&n| doc.node_name(n).unwrap()).collect();
        assert!(names.contains(&"img"));
        assert!(names.contains(&"div")); // empty-div
    }

    #[test]
    fn test_pseudo_empty_excludes_non_empty() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, ":empty");
        // h1 has text content, should not match :empty
        assert!(!result.iter().any(|&n| doc.node_name(n) == Some("h1")));
    }

    #[test]
    fn test_pseudo_not_class() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "li:not(.active)");
        assert_eq!(result.len(), 2);
        // Should be "Two" and "Three"
        let texts: Vec<_> = result.iter().map(|&n| doc.text_content(n)).collect();
        assert!(texts.contains(&"Two".to_string()));
        assert!(texts.contains(&"Three".to_string()));
    }

    #[test]
    fn test_pseudo_not_tag() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // All children of #main that are not <p>
        let result = eval(&doc, root, "#main > :not(p)");
        assert!(!result.iter().any(|&n| doc.node_name(n) == Some("p")));
        assert!(result.len() >= 4); // h1, ul, a, img, span, span, span, div
    }

    #[test]
    fn test_pseudo_only_child() {
        let doc = Document::parse_str(
            r"<root><wrapper><only>Only child</only></wrapper><multi><a/><b/></multi></root>",
        )
        .unwrap();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, ":only-child");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.node_name(result[0]), Some("only"));
    }

    #[test]
    fn test_pseudo_nth_child_specific() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // Second li
        let result = eval(&doc, root, "li:nth-child(2)");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "Two");
    }

    #[test]
    fn test_pseudo_nth_child_odd() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "li:nth-child(odd)");
        assert_eq!(result.len(), 2); // 1st and 3rd
        assert_eq!(doc.text_content(result[0]), "One");
        assert_eq!(doc.text_content(result[1]), "Three");
    }

    #[test]
    fn test_pseudo_nth_child_even() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "li:nth-child(even)");
        assert_eq!(result.len(), 1); // 2nd only
        assert_eq!(doc.text_content(result[0]), "Two");
    }

    #[test]
    fn test_pseudo_nth_last_child() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // :nth-last-child(1) is last child
        let result = eval(&doc, root, "li:nth-last-child(1)");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "Three");
    }

    // ---------------------------------------------------------------
    // 6. Combinator matching
    // ---------------------------------------------------------------

    #[test]
    fn test_combinator_descendant() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // All p descendants of div (any depth)
        let result = eval(&doc, root, "div p");
        assert_eq!(result.len(), 3); // 2 in #main + 1 in #sidebar
    }

    #[test]
    fn test_combinator_descendant_deep() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // li is nested inside root > div > ul > li
        let result = eval(&doc, root, "div li");
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_combinator_child() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // Only direct children of #main that are <p>
        let result = eval(&doc, root, "#main > p");
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_combinator_child_excludes_deeper() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // li is NOT a direct child of div — it's a child of ul
        let result = eval(&doc, root, "div > li");
        assert!(result.is_empty());
    }

    #[test]
    fn test_combinator_adjacent_sibling() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // p immediately after h1
        let result = eval(&doc, root, "h1 + p");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "Hello");
    }

    #[test]
    fn test_combinator_adjacent_sibling_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // h1 is not immediately preceded by a <p>
        let result = eval(&doc, root, "p + h1");
        assert!(result.is_empty());
    }

    #[test]
    fn test_combinator_general_sibling() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // All p elements that come after an h1 in the same parent
        let result = eval(&doc, root, "h1 ~ p");
        assert_eq!(result.len(), 2); // both p's in #main
    }

    #[test]
    fn test_combinator_general_sibling_no_match() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // h1 has no preceding sibling <a>
        let result = eval(&doc, root, "a ~ h1");
        assert!(result.is_empty());
    }

    #[test]
    fn test_combinator_chain() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // Chain: div with class container > ul, then descendant li with class active
        let result = eval(&doc, root, "div.container > ul li.active");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "One");
    }

    #[test]
    fn test_combinator_three_levels() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // root > div > ul > li
        let result = eval(&doc, root, "div > ul > li");
        assert_eq!(result.len(), 3);
    }

    // ---------------------------------------------------------------
    // 7. Universal selector matching
    // ---------------------------------------------------------------

    #[test]
    fn test_universal_all_elements() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "*");
        // Should match every element descendant of root
        assert!(result.len() >= 14); // div, h1, p, p, ul, li, li, li, a, img, span, span, span, div, div, p
    }

    #[test]
    fn test_universal_direct_children() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // Direct children of #main
        let result = eval(&doc, root, "#main > *");
        // h1, p, p, ul, a, img, span, span, span, empty-div
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_universal_with_class() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // Universal + class is equivalent to just .intro
        let result_star = eval(&doc, root, "*.intro");
        let result_class = eval(&doc, root, ".intro");
        assert_eq!(result_star.len(), result_class.len());
        assert_eq!(result_star, result_class);
    }

    #[test]
    fn test_universal_with_pseudo() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "*:first-child");
        // First element child of each parent
        assert!(result.len() >= 2);
        // All returned nodes should be first element children of their parents
        for &node in &result {
            let parent = doc.parent(node).unwrap();
            let first = doc
                .children(parent)
                .find(|&c| matches!(doc.node(c).kind, NodeKind::Element { .. }))
                .unwrap();
            assert_eq!(first, node);
        }
    }

    // ---------------------------------------------------------------
    // Selector group (comma-separated)
    // ---------------------------------------------------------------

    #[test]
    fn test_selector_group() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "h1, img");
        assert_eq!(result.len(), 2);
        let names: Vec<_> = result.iter().map(|&n| doc.node_name(n).unwrap()).collect();
        assert!(names.contains(&"h1"));
        assert!(names.contains(&"img"));
    }

    // ---------------------------------------------------------------
    // Edge cases
    // ---------------------------------------------------------------

    #[test]
    fn test_empty_selector_group() {
        let group = SelectorGroup {
            selectors: vec![Selector {
                compounds: Vec::new(),
            }],
        };
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = select(&doc, root, &group);
        assert!(result.is_empty());
    }

    #[test]
    fn test_scope_limits_results() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // Get the sidebar div, then scope the search to it
        let sidebar_nodes = eval(&doc, root, "#sidebar");
        assert_eq!(sidebar_nodes.len(), 1);
        let sidebar = sidebar_nodes[0];
        // Only 1 <p> inside sidebar
        let result = eval(&doc, sidebar, "p");
        assert_eq!(result.len(), 1);
        assert_eq!(doc.text_content(result[0]), "Side");
    }

    #[test]
    fn test_no_elements_in_scope() {
        let doc = Document::parse_str("<root/>").unwrap();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "div");
        assert!(result.is_empty());
    }

    #[test]
    fn test_fast_id_path_descendant_check() {
        // The fast #id path should verify the node is a descendant of scope
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        // Get #sidebar, then search for #main from within it — should not find it
        let sidebar_nodes = eval(&doc, root, "#sidebar");
        let sidebar = sidebar_nodes[0];
        let result = eval(&doc, sidebar, "#main");
        assert!(result.is_empty());
    }

    #[test]
    fn test_document_order_preserved() {
        let doc = test_doc();
        let root = doc.root_element().unwrap();
        let result = eval(&doc, root, "li");
        assert_eq!(result.len(), 3);
        assert_eq!(doc.text_content(result[0]), "One");
        assert_eq!(doc.text_content(result[1]), "Two");
        assert_eq!(doc.text_content(result[2]), "Three");
    }
}
