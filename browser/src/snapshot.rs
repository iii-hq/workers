//! Accessibility-tree snapshot: the model's default perception. Serializes
//! the AX tree as an indented text outline with stable element refs
//! (`e1`, `e2`, …) that `browser::act` / `browser::evaluate` resolve back to
//! CDP backend node ids. Text is what the model reads; refs are how it acts.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// One entry per emitted line, without the `[ref=eN]` suffix: the
    /// ref-stable identity used as the diff-mode baseline (ref names are
    /// session-monotonic, so the same node gets a different name in every
    /// snapshot and raw lines would always differ).
    pub keys: Vec<String>,
    /// The emitted lines with refs, parallel to `keys`; diff mode reports
    /// added lines in this form so they are directly actionable.
    pub lines: Vec<String>,
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
/// `ref_counter` is the session's monotonic counter: names continue from
/// where the previous snapshot stopped, so refs from different snapshots of
/// one document never collide.
pub fn serialize(nodes: &[AxNode], max_nodes: usize, ref_counter: &AtomicU64) -> SnapshotResult {
    let by_id: HashMap<&str, &AxNode> = nodes
        .iter()
        .map(|n| (n.node_id.inner().as_str(), n))
        .collect();

    let root = nodes.first();
    let mut out = String::new();
    let mut refs = HashMap::new();
    let mut keys = Vec::new();
    let mut lines = Vec::new();
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
        keys.push(line.clone());
        if let Some(backend_id) = &node.backend_dom_node_id {
            let r = format!("e{}", ref_counter.fetch_add(1, Ordering::Relaxed) + 1);
            refs.insert(r.clone(), *backend_id.inner());
            line.push_str(&format!(" [ref={r}]"));
        }
        lines.push(line.clone());
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
        keys,
        lines,
    }
}

/// Diff of the current snapshot against the previous one, over ref-less key
/// lines. Multiset semantics: a line appearing twice before and once now
/// counts one removal. `added` holds indices into the current snapshot's
/// lines so the caller can report the ref-bearing form.
pub struct SnapshotDiffResult {
    pub added: Vec<usize>,
    pub removed: Vec<String>,
    pub unchanged: u64,
}

pub fn diff_keys(previous: &[String], current: &[String]) -> SnapshotDiffResult {
    let mut old_counts: HashMap<&str, i64> = HashMap::new();
    for k in previous {
        *old_counts.entry(k.as_str()).or_default() += 1;
    }
    let mut added = Vec::new();
    let mut unchanged = 0u64;
    for (i, k) in current.iter().enumerate() {
        match old_counts.get_mut(k.as_str()) {
            Some(n) if *n > 0 => {
                *n -= 1;
                unchanged += 1;
            }
            _ => added.push(i),
        }
    }
    let mut removed = Vec::new();
    for k in previous {
        if let Some(n) = old_counts.get_mut(k.as_str()) {
            if *n > 0 {
                *n -= 1;
                removed.push(k.clone());
            }
        }
    }
    SnapshotDiffResult {
        added,
        removed,
        unchanged,
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
        let result = serialize(&nodes, 100, &AtomicU64::new(0));
        assert!(result.tree.contains("RootWebArea \"Checkout\" [ref=e1]"));
        assert!(result.tree.contains("  - button \"Place order\" [ref=e2]"));
        assert!(!result.tree.contains("generic"));
        assert_eq!(result.refs.get("e2"), Some(&11));
        assert!(!result.truncated);
        assert_eq!(result.keys.len(), result.lines.len());
        assert!(result.keys[1].ends_with("\"Place order\""));
        assert!(result.lines[1].ends_with("[ref=e2]"));
    }

    #[test]
    fn ref_names_continue_across_snapshots() {
        let nodes = vec![node("1", "RootWebArea", "App", &[], Some(10))];
        let counter = AtomicU64::new(0);
        let first = serialize(&nodes, 100, &counter);
        let second = serialize(&nodes, 100, &counter);
        assert!(first.refs.contains_key("e1"));
        assert!(second.refs.contains_key("e2"));
        assert!(!second.refs.contains_key("e1"));
    }

    #[test]
    fn unnamed_skip_roles_collapse_but_children_survive() {
        let nodes = vec![
            node("1", "RootWebArea", "App", &["2"], None),
            node("2", "generic", "", &["3"], Some(20)),
            node("3", "link", "Docs", &[], Some(21)),
        ];
        let result = serialize(&nodes, 100, &AtomicU64::new(0));
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
        let result = serialize(&nodes, 2, &AtomicU64::new(0));
        assert!(result.truncated);
        assert!(result.tree.contains("[snapshot truncated"));
    }

    #[test]
    fn diff_reports_added_removed_unchanged() {
        let previous = vec!["- a".to_string(), "- b".to_string(), "- b".to_string()];
        let current = vec!["- a".to_string(), "- b".to_string(), "- c".to_string()];
        let diff = diff_keys(&previous, &current);
        assert_eq!(diff.added, vec![2]);
        assert_eq!(diff.removed, vec!["- b".to_string()]);
        assert_eq!(diff.unchanged, 2);
    }

    #[test]
    fn diff_of_identical_snapshots_is_empty() {
        let keys = vec!["- a".to_string(), "- b".to_string()];
        let diff = diff_keys(&keys, &keys);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.unchanged, 2);
    }
}
