//! `coder::tree`-specific assertions. The response is a single nested
//! `root` node; navigation through the tree uses a slash-separated path
//! ("a/b/c") starting from the root's children. Pointer ".” / empty
//! string addresses the root itself.

use cucumber::then;
use serde_json::Value;

use crate::common::helpers::last_ok;
use crate::common::world::CoderWorld;

/// Walk the tree from `root` to a node whose path-from-root matches the
/// slash-separated `rel`. `"."` or `""` returns the root. Returns
/// `None` when any segment is missing.
fn find_node<'a>(root: &'a Value, rel: &str) -> Option<&'a Value> {
    let trimmed = rel.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Some(root);
    }
    let mut current = root;
    for segment in trimmed.split('/') {
        let kids = current.get("children")?.as_array()?;
        current = kids.iter().find(|c| c["name"].as_str() == Some(segment))?;
    }
    Some(current)
}

fn root_of(world: &CoderWorld) -> &Value {
    &last_ok(world)["root"]
}

#[then(regex = r#"^the tree has a node at "([^"]+)"$"#)]
fn tree_has_node(world: &mut CoderWorld, rel: String) {
    if world.iii.is_none() {
        return;
    }
    let root = root_of(world);
    assert!(
        find_node(root, &rel).is_some(),
        "missing node {rel:?}; tree: {root}"
    );
}

#[then(regex = r#"^the tree has no node at "([^"]+)"$"#)]
fn tree_lacks_node(world: &mut CoderWorld, rel: String) {
    if world.iii.is_none() {
        return;
    }
    let root = root_of(world);
    assert!(
        find_node(root, &rel).is_none(),
        "node {rel:?} unexpectedly present; tree: {root}"
    );
}

#[then(regex = r#"^the tree node at "([^"]+)" has kind "([^"]+)"$"#)]
fn tree_node_kind(world: &mut CoderWorld, rel: String, kind: String) {
    if world.iii.is_none() {
        return;
    }
    let root = root_of(world);
    let node =
        find_node(root, &rel).unwrap_or_else(|| panic!("missing node {rel:?}; tree: {root}"));
    assert_eq!(
        node["kind"].as_str().unwrap_or(""),
        kind,
        "kind mismatch at {rel:?}: {node}"
    );
}

#[then(regex = r#"^the tree node at "([^"]+)" is non_accessible$"#)]
fn tree_node_non_accessible(world: &mut CoderWorld, rel: String) {
    if world.iii.is_none() {
        return;
    }
    let root = root_of(world);
    let node =
        find_node(root, &rel).unwrap_or_else(|| panic!("missing node {rel:?}; tree: {root}"));
    assert_eq!(
        node["non_accessible"], true,
        "expected non_accessible: true at {rel:?}; got: {node}"
    );
}

#[then(regex = r#"^the tree node at "([^"]+)" is truncated with reason "([^"]+)"$"#)]
fn tree_node_truncated(world: &mut CoderWorld, rel: String, reason: String) {
    if world.iii.is_none() {
        return;
    }
    let root = root_of(world);
    let node =
        find_node(root, &rel).unwrap_or_else(|| panic!("missing node {rel:?}; tree: {root}"));
    let trunc = &node["truncated"];
    assert!(
        !trunc.is_null(),
        "expected truncated info at {rel:?}; got: {node}"
    );
    assert_eq!(
        trunc["reason"].as_str().unwrap_or(""),
        reason,
        "reason mismatch at {rel:?}: {trunc}"
    );
}

#[then(regex = r#"^the tree node at "([^"]+)" hint mentions "([^"]+)"$"#)]
fn tree_node_hint_mentions(world: &mut CoderWorld, rel: String, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let root = root_of(world);
    let node =
        find_node(root, &rel).unwrap_or_else(|| panic!("missing node {rel:?}; tree: {root}"));
    let hint = node["truncated"]["hint"].as_str().unwrap_or("");
    assert!(
        hint.contains(&needle),
        "expected hint to contain {needle:?}; got: {hint:?}"
    );
}

#[then(regex = r#"^the tree node at "([^"]+)" has (\d+) children$"#)]
fn tree_node_children_count(world: &mut CoderWorld, rel: String, expected: usize) {
    if world.iii.is_none() {
        return;
    }
    let root = root_of(world);
    let node =
        find_node(root, &rel).unwrap_or_else(|| panic!("missing node {rel:?}; tree: {root}"));
    let got = node["children"].as_array().map(Vec::len).unwrap_or(0);
    assert_eq!(got, expected, "child-count mismatch at {rel:?}: {node}");
}
