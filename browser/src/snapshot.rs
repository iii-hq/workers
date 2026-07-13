//! Accessibility-tree snapshot: the model's default perception. Serializes
//! the AX tree as an indented text outline with stable element refs
//! (`e1`, `e2`, …) that `browser::act` / `browser::evaluate` resolve back to
//! CDP backend node ids. Text is what the model reads; refs are how it acts.

use std::collections::HashMap;

use chromiumoxide::cdp::browser_protocol::accessibility::AxNode;

/// Roles that carry no signal on their own and are skipped unless named —
/// pure structure the model doesn't need spelled out.
const SKIP_UNNAMED_ROLES: &[&str] = &[
    "none",
    "generic",
    "InlineTextBox",
    "LineBreak",
    "StaticText",
    "paragraph",
];

pub struct SnapshotResult {
    pub tree: String,
    pub refs: HashMap<String, i64>,
    pub truncated: bool,
}

fn ax_value_string(
    v: &Option<chromiumoxide::cdp::browser_protocol::accessibility::AxValue>,
) -> String {
    v.as_ref()
        .and_then(|val| val.value.as_ref())
        .map(|val| match val {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

/// Serialize the flat `Accessibility.getFullAXTree` node list into an
/// indented outline, assigning refs to nodes that are backed by a DOM node.
pub fn serialize(nodes: &[AxNode], max_nodes: usize) -> SnapshotResult {
    let by_id: HashMap<&str, &AxNode> = nodes
        .iter()
        .map(|n| (n.node_id.inner().as_str(), n))
        .collect();

    let root = nodes.first();
    let mut out = String::new();
    let mut refs = HashMap::new();
    let mut ref_counter = 0u64;
    let mut emitted = 0usize;
    let mut truncated = false;

    // Iterative DFS: (node id, depth). The AX tree can be deep; no recursion.
    let mut stack: Vec<(&str, usize)> = root
        .map(|r| vec![(r.node_id.inner().as_str(), 0)])
        .unwrap_or_default();

    while let Some((id, depth)) = stack.pop() {
        let Some(node) = by_id.get(id) else { continue };

        if emitted >= max_nodes {
            truncated = true;
            break;
        }

        let role = ax_value_string(&node.role);
        let name = ax_value_string(&node.name);
        let value = ax_value_string(&node.value);
        let skip_line =
            node.ignored || (name.is_empty() && SKIP_UNNAMED_ROLES.contains(&role.as_str()));

        let child_depth = if skip_line { depth } else { depth + 1 };
        if let Some(children) = &node.child_ids {
            for child in children.iter().rev() {
                stack.push((child.inner().as_str(), child_depth));
            }
        }

        if skip_line {
            continue;
        }

        let mut line = format!("{}- {role}", "  ".repeat(depth));
        if !name.is_empty() {
            line.push_str(&format!(" \"{}\"", crate::session::truncate(&name, 120)));
        }
        if !value.is_empty() && value != name {
            line.push_str(&format!(" = {}", crate::session::truncate(&value, 120)));
        }
        if let Some(backend_id) = &node.backend_dom_node_id {
            ref_counter += 1;
            let r = format!("e{ref_counter}");
            refs.insert(r.clone(), *backend_id.inner());
            line.push_str(&format!(" [ref={r}]"));
        }
        out.push_str(&line);
        out.push('\n');
        emitted += 1;
    }

    if truncated {
        out.push_str("… [snapshot truncated: raise max_snapshot_nodes or snapshot a subtree]\n");
    }

    SnapshotResult {
        tree: out,
        refs,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chromiumoxide::cdp::browser_protocol::accessibility::{AxNodeId, AxValue, AxValueType};
    use chromiumoxide::cdp::browser_protocol::dom::BackendNodeId;

    fn value(s: &str) -> AxValue {
        AxValue {
            r#type: AxValueType::String,
            value: Some(serde_json::Value::String(s.to_string())),
            related_nodes: None,
            sources: None,
        }
    }

    fn node(id: &str, role: &str, name: &str, children: &[&str], backend: Option<i64>) -> AxNode {
        AxNode {
            node_id: AxNodeId::new(id),
            ignored: false,
            ignored_reasons: None,
            role: Some(value(role)),
            chrome_role: None,
            name: Some(value(name)),
            description: None,
            value: None,
            properties: None,
            parent_id: None,
            child_ids: Some(children.iter().map(|c| AxNodeId::new(*c)).collect()),
            backend_dom_node_id: backend.map(BackendNodeId::new),
            frame_id: None,
        }
    }

    #[test]
    fn serializes_named_nodes_with_refs() {
        let nodes = vec![
            node("1", "RootWebArea", "Checkout", &["2", "3"], Some(10)),
            node("2", "button", "Place order", &[], Some(11)),
            node("3", "generic", "", &[], Some(12)),
        ];
        let result = serialize(&nodes, 100);
        assert!(result.tree.contains("RootWebArea \"Checkout\" [ref=e1]"));
        assert!(result.tree.contains("  - button \"Place order\" [ref=e2]"));
        assert!(!result.tree.contains("generic"));
        assert_eq!(result.refs.get("e2"), Some(&11));
        assert!(!result.truncated);
    }

    #[test]
    fn unnamed_skip_roles_collapse_but_children_survive() {
        let nodes = vec![
            node("1", "RootWebArea", "App", &["2"], None),
            node("2", "generic", "", &["3"], Some(20)),
            node("3", "link", "Docs", &[], Some(21)),
        ];
        let result = serialize(&nodes, 100);
        assert!(result.tree.contains("- link \"Docs\""));
        // link is nested under the root, not under the skipped generic
        assert!(result.tree.contains("\n  - link"));
    }

    #[test]
    fn truncates_at_cap() {
        let mut nodes = vec![node("1", "RootWebArea", "App", &["2", "3", "4"], None)];
        for i in 2..=4 {
            nodes.push(node(
                &i.to_string(),
                "button",
                &format!("b{i}"),
                &[],
                Some(i),
            ));
        }
        let result = serialize(&nodes, 2);
        assert!(result.truncated);
        assert!(result.tree.contains("[snapshot truncated"));
    }
}
