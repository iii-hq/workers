//! `harness::session-tree` — resolve the durable root-and-descendant session
//! membership recorded by the harness in session metadata.

use std::collections::{BTreeSet, VecDeque};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::clients::session::SessionLink;
use crate::deps::Deps;
use crate::error::HarnessError;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionTreeRequestV1 {
    pub root_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionTreeNodeV1 {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<String>,
    pub depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionTreeResponseV1 {
    pub root_session_id: String,
    pub sessions: Vec<SessionTreeNodeV1>,
    pub complete: bool,
}

pub async fn handle(
    deps: &Deps,
    req: SessionTreeRequestV1,
) -> Result<SessionTreeResponseV1, HarnessError> {
    collect(deps, &req.root_session_id).await
}

pub(crate) async fn collect(
    deps: &Deps,
    root_session_id: &str,
) -> Result<SessionTreeResponseV1, HarnessError> {
    let session = deps.session().await;
    if !session.exists(root_session_id).await? {
        return Ok(SessionTreeResponseV1 {
            root_session_id: root_session_id.to_string(),
            sessions: Vec::new(),
            complete: false,
        });
    }

    let mut builder = TreeBuilder::new(root_session_id);
    while let Some((parent_session_id, depth)) = builder.next_parent() {
        let children = session.children(&parent_session_id).await?;
        builder.add_children(&parent_session_id, depth + 1, children);
    }
    Ok(builder.finish())
}

struct TreeBuilder {
    root_session_id: String,
    sessions: Vec<SessionTreeNodeV1>,
    pending: VecDeque<(String, u32)>,
    seen: BTreeSet<String>,
    complete: bool,
}

impl TreeBuilder {
    fn new(root_session_id: &str) -> Self {
        let root_session_id = root_session_id.to_string();
        Self {
            sessions: vec![SessionTreeNodeV1 {
                session_id: root_session_id.clone(),
                parent_session_id: None,
                parent_turn_id: None,
                depth: 0,
            }],
            pending: VecDeque::from([(root_session_id.clone(), 0)]),
            seen: BTreeSet::from([root_session_id.clone()]),
            root_session_id,
            complete: true,
        }
    }

    fn next_parent(&mut self) -> Option<(String, u32)> {
        self.pending.pop_front()
    }

    fn add_children(&mut self, parent_session_id: &str, depth: u32, children: Vec<SessionLink>) {
        for child in children {
            if child.parent_session_id != parent_session_id
                || !self.seen.insert(child.session_id.clone())
            {
                self.complete = false;
                continue;
            }
            self.pending.push_back((child.session_id.clone(), depth));
            self.sessions.push(SessionTreeNodeV1 {
                session_id: child.session_id,
                parent_session_id: Some(parent_session_id.to_string()),
                parent_turn_id: child.parent_turn_id,
                depth,
            });
        }
    }

    fn finish(mut self) -> SessionTreeResponseV1 {
        self.sessions.sort_by(|left, right| {
            left.depth
                .cmp(&right.depth)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        SessionTreeResponseV1 {
            root_session_id: self.root_session_id,
            sessions: self.sessions,
            complete: self.complete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(session: &str, parent: &str, turn: Option<&str>) -> SessionLink {
        SessionLink {
            session_id: session.into(),
            parent_session_id: parent.into(),
            parent_turn_id: turn.map(str::to_string),
        }
    }

    #[test]
    fn builder_orders_root_then_depth_then_session_id() {
        let mut tree = TreeBuilder::new("root");
        let (root, depth) = tree.next_parent().unwrap();
        tree.add_children(
            &root,
            depth + 1,
            vec![
                link("child-b", "root", None),
                link("child-a", "root", Some("turn-1")),
            ],
        );
        while let Some((parent, parent_depth)) = tree.next_parent() {
            if parent == "child-a" {
                tree.add_children(
                    &parent,
                    parent_depth + 1,
                    vec![link("grandchild", "child-a", Some("turn-2"))],
                );
            }
        }
        let tree = tree.finish();
        assert!(tree.complete);
        assert_eq!(
            tree.sessions
                .iter()
                .map(|node| (node.session_id.as_str(), node.depth))
                .collect::<Vec<_>>(),
            vec![
                ("root", 0),
                ("child-a", 1),
                ("child-b", 1),
                ("grandchild", 2)
            ]
        );
        assert_eq!(tree.sessions[1].parent_turn_id.as_deref(), Some("turn-1"));
    }

    #[test]
    fn duplicate_or_mismatched_links_make_tree_incomplete() {
        let mut tree = TreeBuilder::new("root");
        tree.add_children(
            "root",
            1,
            vec![
                link("child", "root", None),
                link("child", "root", None),
                link("other", "wrong-parent", None),
                link("root", "root", None),
            ],
        );
        let tree = tree.finish();
        assert!(!tree.complete);
        assert_eq!(tree.sessions.len(), 2);
    }
}
