//! `coder::read-file`-specific assertions. Read returns a flat record so
//! every Then maps to a top-level field; we keep them parameterised so
//! one step def covers `content`, `size`, `is_utf8`, and `path` echo.

use cucumber::then;

use crate::common::helpers::{docstring_body, last_ok};
use crate::common::world::CoderWorld;

#[then(regex = r#"^the read content equals "([^"]*)"$"#)]
fn read_content_equals(world: &mut CoderWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = v["content"].as_str().unwrap_or("");
    assert_eq!(got, expected, "content mismatch; got: {v}");
}

#[then("the read content equals:")]
fn read_content_equals_docstring(world: &mut CoderWorld, step: &cucumber::gherkin::Step) {
    if world.iii.is_none() {
        return;
    }
    let expected = docstring_body(step);
    let v = last_ok(world);
    let got = v["content"].as_str().unwrap_or("");
    assert_eq!(got, expected, "content mismatch; got: {v}");
}

#[then(regex = r#"^the read size equals (\d+)$"#)]
fn read_size_equals(world: &mut CoderWorld, expected: u64) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = v["size"].as_u64().unwrap_or_default();
    assert_eq!(got, expected, "size mismatch; got: {v}");
}

#[then(regex = r#"^the read is_utf8 is (true|false)$"#)]
fn read_is_utf8(world: &mut CoderWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = v["is_utf8"].as_bool().unwrap_or(false);
    let expected_bool = expected == "true";
    assert_eq!(got, expected_bool, "is_utf8 mismatch; got: {v}");
}

#[then(regex = r#"^the read path equals "([^"]+)"$"#)]
fn read_path_equals(world: &mut CoderWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    // Responses carry canonical absolute paths; features speak
    // base-relative, so anchor the expectation at the scenario base.
    let expected = world
        .base_path
        .as_ref()
        .expect("base_path set")
        .join(&expected)
        .display()
        .to_string();
    let v = last_ok(world);
    let got = v["path"].as_str().unwrap_or("");
    assert_eq!(got, expected, "path echo mismatch; got: {v}");
}
