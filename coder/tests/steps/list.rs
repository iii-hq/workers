//! `coder::list-folder`-specific assertions. The list response carries
//! its entries plus pagination metadata (`total`, `page`, `page_size`,
//! `has_more`) and a `non_accessible` flag per entry — each Then below
//! pokes one of those fields.

use cucumber::then;
use serde_json::Value;

use crate::common::helpers::last_ok;
use crate::common::world::CoderWorld;

fn entries(v: &Value) -> &[Value] {
    v["entries"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn find_entry<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    entries(v).iter().find(|e| e["name"].as_str() == Some(name))
}

#[then(regex = r#"^the listing has an entry named "([^"]+)"$"#)]
fn listing_has_entry(world: &mut CoderWorld, name: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert!(
        find_entry(v, &name).is_some(),
        "expected entry named {name:?}; got: {:?}",
        entries(v)
    );
}

#[then(regex = r#"^the listing has no entry named "([^"]+)"$"#)]
fn listing_lacks_entry(world: &mut CoderWorld, name: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    assert!(
        find_entry(v, &name).is_none(),
        "entry named {name:?} unexpectedly present; got: {:?}",
        entries(v)
    );
}

#[then(regex = r#"^the listing entry "([^"]+)" has kind "([^"]+)"$"#)]
fn listing_entry_kind(world: &mut CoderWorld, name: String, kind: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let entry = find_entry(v, &name)
        .unwrap_or_else(|| panic!("missing entry {name:?}; got: {:?}", entries(v)));
    assert_eq!(
        entry["kind"].as_str().unwrap_or(""),
        kind,
        "kind mismatch for {name:?}: {entry:?}"
    );
}

#[then(regex = r#"^the listing entry "([^"]+)" is non_accessible$"#)]
fn listing_entry_non_accessible(world: &mut CoderWorld, name: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let entry = find_entry(v, &name)
        .unwrap_or_else(|| panic!("missing entry {name:?}; got: {:?}", entries(v)));
    assert_eq!(
        entry["non_accessible"], true,
        "expected non_accessible: true for {name:?}; got: {entry:?}"
    );
}

#[then(regex = r#"^the listing entry "([^"]+)" is accessible$"#)]
fn listing_entry_accessible(world: &mut CoderWorld, name: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let entry = find_entry(v, &name)
        .unwrap_or_else(|| panic!("missing entry {name:?}; got: {:?}", entries(v)));
    assert_eq!(
        entry["non_accessible"], false,
        "expected non_accessible: false for {name:?}; got: {entry:?}"
    );
}

#[then(regex = r#"^the listing total equals (\d+)$"#)]
fn listing_total(world: &mut CoderWorld, expected: u64) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = v["total"].as_u64().unwrap_or_default();
    assert_eq!(got, expected, "total mismatch; got: {v}");
}

#[then(regex = r#"^the listing page equals (\d+)$"#)]
fn listing_page(world: &mut CoderWorld, expected: u64) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = v["page"].as_u64().unwrap_or_default();
    assert_eq!(got, expected, "page mismatch; got: {v}");
}

#[then(regex = r#"^the listing page_size equals (\d+)$"#)]
fn listing_page_size(world: &mut CoderWorld, expected: u64) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = v["page_size"].as_u64().unwrap_or_default();
    assert_eq!(got, expected, "page_size mismatch; got: {v}");
}

#[then(regex = r#"^the listing has_more is (true|false)$"#)]
fn listing_has_more(world: &mut CoderWorld, expected: String) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = v["has_more"].as_bool().unwrap_or_default();
    let expected_bool = expected == "true";
    assert_eq!(got, expected_bool, "has_more mismatch; got: {v}");
}

#[then(regex = r#"^the listing has (\d+) entries$"#)]
fn listing_entry_count(world: &mut CoderWorld, expected: usize) {
    if world.iii.is_none() {
        return;
    }
    let v = last_ok(world);
    let got = entries(v).len();
    assert_eq!(got, expected, "entry-count mismatch; got: {v}");
}
