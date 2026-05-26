//! `coder::create-file`-specific assertions. Most create scenarios rely
//! on the generic `result for "<path>" succeeded / failed with code "..."`
//! steps in [`common`](super::common); this module adds the response
//! fields that are unique to create-file (`bytes_written`) plus a
//! filesystem-side check that the file actually landed.

use std::path::PathBuf;

use cucumber::then;

use crate::common::helpers::{batch_result, last_ok};
use crate::common::world::CoderWorld;

#[then(regex = r#"^the result for "([^"]+)" wrote (\d+) bytes$"#)]
fn create_wrote_bytes(world: &mut CoderWorld, path: String, expected: u64) {
    if world.iii.is_none() {
        return;
    }
    let entry = batch_result(world, &path).unwrap_or_else(|| {
        panic!(
            "no result entry for path {path:?}; got: {:?}",
            last_ok(world)
        )
    });
    let got = entry["bytes_written"].as_u64().unwrap_or_default();
    assert_eq!(
        got, expected,
        "expected bytes_written {expected} for {path:?}; got: {entry:?}"
    );
}

#[then(regex = r#"^the file "([^"]+)" exists on disk$"#)]
fn file_exists_on_disk(world: &mut CoderWorld, rel: String) {
    if world.iii.is_none() {
        return;
    }
    let Some(base) = world.base_path.as_ref() else {
        return;
    };
    let path: PathBuf = base.join(&rel);
    assert!(path.exists(), "expected {path:?} to exist on disk");
}

#[then(regex = r#"^the file "([^"]+)" does not exist on disk$"#)]
fn file_missing_on_disk(world: &mut CoderWorld, rel: String) {
    if world.iii.is_none() {
        return;
    }
    let Some(base) = world.base_path.as_ref() else {
        return;
    };
    let path: PathBuf = base.join(&rel);
    assert!(!path.exists(), "expected {path:?} to be absent");
}

#[then(regex = r#"^the file "([^"]+)" on disk contains "([^"]+)"$"#)]
fn file_contains(world: &mut CoderWorld, rel: String, needle: String) {
    if world.iii.is_none() {
        return;
    }
    let Some(base) = world.base_path.as_ref() else {
        return;
    };
    let path = base.join(&rel);
    let body = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert!(
        body.contains(&needle),
        "expected {path:?} to contain {needle:?}; got: {body:?}"
    );
}
