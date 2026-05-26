//! `coder::update-file`-specific assertions. The bottom-up application,
//! atomicity, and per-file partial success all surface through the
//! generic `result for "<path>"` Thens; this module covers `applied`
//! and `new_line_count` on each per-file result.

use cucumber::then;

use crate::common::helpers::{batch_result, last_ok};
use crate::common::world::CoderWorld;

#[then(regex = r#"^the result for "([^"]+)" applied (\d+) ops$"#)]
fn update_applied(world: &mut CoderWorld, path: String, expected: u64) {
    if world.iii.is_none() {
        return;
    }
    let entry = batch_result(world, &path).unwrap_or_else(|| {
        panic!(
            "no result entry for path {path:?}; got: {:?}",
            last_ok(world)
        )
    });
    let got = entry["applied"].as_u64().unwrap_or_default();
    assert_eq!(
        got, expected,
        "expected applied {expected} for {path:?}; got: {entry:?}"
    );
}

#[then(regex = r#"^the result for "([^"]+)" has line count (\d+)$"#)]
fn update_line_count(world: &mut CoderWorld, path: String, expected: u64) {
    if world.iii.is_none() {
        return;
    }
    let entry = batch_result(world, &path).unwrap_or_else(|| {
        panic!(
            "no result entry for path {path:?}; got: {:?}",
            last_ok(world)
        )
    });
    let got = entry["new_line_count"].as_u64().unwrap_or_default();
    assert_eq!(
        got, expected,
        "expected new_line_count {expected} for {path:?}; got: {entry:?}"
    );
}
