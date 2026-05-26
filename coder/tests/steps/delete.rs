//! `coder::delete-file`-specific assertions. The `removed` flag
//! distinguishes idempotent "already absent" calls (`success: true`,
//! `removed: false`) from real deletions (`success: true`, `removed:
//! true`).

use cucumber::then;

use crate::common::helpers::{batch_result, last_ok};
use crate::common::world::CoderWorld;

#[then(regex = r#"^the result for "([^"]+)" was removed$"#)]
fn delete_was_removed(world: &mut CoderWorld, path: String) {
    if world.iii.is_none() {
        return;
    }
    let entry = batch_result(world, &path).unwrap_or_else(|| {
        panic!(
            "no result entry for path {path:?}; got: {:?}",
            last_ok(world)
        )
    });
    assert_eq!(
        entry["removed"], true,
        "expected removed: true for {path:?}; got: {entry:?}"
    );
}

#[then(regex = r#"^the result for "([^"]+)" was not removed$"#)]
fn delete_was_not_removed(world: &mut CoderWorld, path: String) {
    if world.iii.is_none() {
        return;
    }
    let entry = batch_result(world, &path).unwrap_or_else(|| {
        panic!(
            "no result entry for path {path:?}; got: {:?}",
            last_ok(world)
        )
    });
    assert_eq!(
        entry["removed"], false,
        "expected removed: false for {path:?}; got: {entry:?}"
    );
}
