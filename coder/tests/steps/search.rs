//! `coder::search`-specific assertions. Search returns two arrays
//! (`content_matches[]` and `path_matches[]`) plus a top-level
//! `truncated` flag.

use cucumber::then;
use serde_json::Value;

use crate::common::helpers::last_ok;
use crate::common::world::CoderWorld;

fn content_matches(v: &Value) -> &[Value] {
    v["content_matches"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn path_matches(v: &Value) -> &[Value] {
    v["path_matches"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

#[then(regex = r#"^the search has a content match for "([^"]+)" at line (\d+)$"#)]
fn search_has_content_match(world: &mut CoderWorld, path: String, line: u64) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = content_matches(v);
    let found = arr
        .iter()
        .any(|m| m["path"].as_str() == Some(path.as_str()) && m["line"].as_u64() == Some(line));
    assert!(
        found,
        "expected content match for {path:?} at line {line}; got: {arr:?}"
    );
}

#[then(regex = r#"^the search has a content match for "([^"]+)"$"#)]
fn search_has_content_match_any_line(world: &mut CoderWorld, path: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = content_matches(v);
    let found = arr
        .iter()
        .any(|m| m["path"].as_str() == Some(path.as_str()));
    assert!(
        found,
        "expected at least one content match for {path:?}; got: {arr:?}"
    );
}

#[then(regex = r#"^the search has no content match for "([^"]+)"$"#)]
fn search_lacks_content_match(world: &mut CoderWorld, path: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = content_matches(v);
    let found = arr
        .iter()
        .any(|m| m["path"].as_str() == Some(path.as_str()));
    assert!(
        !found,
        "unexpected content match for {path:?}; got: {arr:?}"
    );
}

#[then(regex = r#"^the search has a path match for "([^"]+)"$"#)]
fn search_has_path_match(world: &mut CoderWorld, path: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = path_matches(v);
    let found = arr
        .iter()
        .any(|m| m["path"].as_str() == Some(path.as_str()));
    assert!(found, "expected path match for {path:?}; got: {arr:?}");
}

#[then(regex = r#"^the search has no path match for "([^"]+)"$"#)]
fn search_lacks_path_match(world: &mut CoderWorld, path: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = path_matches(v);
    let found = arr
        .iter()
        .any(|m| m["path"].as_str() == Some(path.as_str()));
    assert!(!found, "unexpected path match for {path:?}; got: {arr:?}");
}

#[then(regex = r#"^the search truncated is (true|false)$"#)]
fn search_truncated(world: &mut CoderWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = v["truncated"].as_bool().unwrap_or(false);
    let expected_bool = expected == "true";
    assert_eq!(got, expected_bool, "truncated mismatch; got: {v}");
}

#[then("the search has no content matches")]
fn search_no_content_matches(world: &mut CoderWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = content_matches(v);
    assert!(
        arr.is_empty(),
        "expected zero content matches; got: {arr:?}"
    );
}

#[then("the search has no path matches")]
fn search_no_path_matches(world: &mut CoderWorld) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let arr = path_matches(v);
    assert!(arr.is_empty(), "expected zero path matches; got: {arr:?}");
}
