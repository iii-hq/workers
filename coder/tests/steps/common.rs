//! Cross-cutting steps shared by every feature file:
//!
//! - `Given the iii engine is reachable` — Background marker.
//! - Fixture Givens for files, directories, and binary content.
//! - Generic `When I call coder::<fn> with payload:` dispatcher.
//! - Outcome assertions — top-level success / error code, per-item
//!   batch result success / error code.
//!
//! Every `Then` step short-circuits when `world.iii.is_none()` so
//! `@engine` scenarios soft-skip on hosts without an iii engine.

use cucumber::{given, then, when};

use crate::common::helpers::{
    batch_result, call_coder, docstring_body, last_err, last_ok, mkdir, parse_payload,
    write_fixture, LAST_OK,
};
use crate::common::world::CoderWorld;

#[given("the iii engine is reachable")]
async fn engine_reachable(_world: &mut CoderWorld) {}

#[given(regex = r#"^a file at "([^"]+)" with content:$"#)]
async fn given_file_with_content(
    world: &mut CoderWorld,
    rel: String,
    step: &cucumber::gherkin::Step,
) {
    let body = docstring_body(step);
    write_fixture(world, &rel, &body);
}

#[given(regex = r#"^a file at "([^"]+)" with (\d+) bytes of content$"#)]
async fn given_file_with_bytes(world: &mut CoderWorld, rel: String, size: usize) {
    let body = "a".repeat(size);
    write_fixture(world, &rel, &body);
}

#[given(regex = r#"^a directory at "([^"]+)"$"#)]
async fn given_directory(world: &mut CoderWorld, rel: String) {
    mkdir(world, &rel);
}

#[given(regex = r#"^a binary file at "([^"]+)" containing "([^"]+)"$"#)]
async fn given_binary_file(world: &mut CoderWorld, rel: String, marker: String) {
    // Single NUL splits the content; the search heuristic treats the
    // whole file as binary and skips it.
    let body = format!("{marker}\u{0000}{marker}");
    write_fixture(world, &rel, &body);
}

#[when(regex = r#"^I call (coder::[a-z\-]+) with payload:$"#)]
async fn call_with_payload(
    world: &mut CoderWorld,
    function_id: String,
    step: &cucumber::gherkin::Step,
) {
    let payload = parse_payload(step);
    call_coder(world, &function_id, payload).await;
}

#[then("the call succeeded")]
fn call_succeeded(world: &mut CoderWorld) {
    if world.iii.is_none() {
        return;
    }
    let _ = last_ok(world);
}

#[then(regex = r#"^the call failed with code "([^"]+)"$"#)]
fn call_failed_with_code(world: &mut CoderWorld, code: String) {
    if world.iii.is_none() {
        return;
    }
    let err = last_err(world);
    assert!(
        !err.is_empty(),
        "expected an error containing code {code:?}; got success: {:?}",
        world.stash.get(LAST_OK)
    );
    assert!(
        err.contains(&code),
        "expected error to contain {code:?}; got: {err:?}"
    );
}

#[then(regex = r#"^the result for "([^"]+)" succeeded$"#)]
fn result_succeeded(world: &mut CoderWorld, path: String) {
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
        entry["success"], true,
        "expected success for {path:?}; got: {entry:?}"
    );
}

#[then(regex = r#"^the result for "([^"]+)" failed with code "([^"]+)"$"#)]
fn result_failed_with_code(world: &mut CoderWorld, path: String, code: String) {
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
        entry["success"], false,
        "expected failure for {path:?}; got: {entry:?}"
    );
    // `error` is now a structured object `{"code":"C2xx","message":"..."}`;
    // extract the `code` field for stable programmatic assertions.
    let err_code = entry["error"]["code"].as_str().unwrap_or("");
    assert_eq!(
        err_code, code,
        "expected error code {code:?} for {path:?}; got entry: {entry:?}"
    );
}
