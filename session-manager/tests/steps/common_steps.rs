//! Generic call / response / error / clock steps shared by every @pure
//! feature, plus the JSON helpers other step modules reuse.

use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use serde_json::Value;

use crate::common::world::SessionWorld;

/// Navigate a dotted path (`meta.status`, `messages.0.entry_id`) into a
/// JSON value. Array segments are numeric indices.
pub fn lookup_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Parse the right-hand side of an `is <value>` assertion: JSON when it
/// parses, raw string otherwise (so `is "idle"`, `is 3`, `is null`,
/// `is true` all behave naturally).
pub fn parse_expected(raw: &str) -> Value {
    serde_json::from_str(raw.trim()).unwrap_or_else(|_| Value::String(raw.trim().to_string()))
}

pub fn docstring_payload(world: &SessionWorld, step: &Step) -> Value {
    let text = world.substitute(step.docstring.as_deref().unwrap_or("{}"));
    serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("payload docstring must be valid JSON: {e}\n---\n{text}\n---"))
}

pub fn response_or_panic(world: &SessionWorld) -> &Value {
    if let Some(err) = &world.last_error {
        panic!("expected a response but the last call failed: {err}");
    }
    world
        .last_response
        .as_ref()
        .expect("no call has been made yet")
}

/// True when an engine-dependent step already soft-skipped this
/// scenario — generic assertions then pass vacuously.
fn skipped(world: &SessionWorld) -> bool {
    world.soft_skipped
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

#[given(regex = r#"^I call "([^"]+)" with:$"#)]
#[when(regex = r#"^I call "([^"]+)" with:$"#)]
async fn call_with(world: &mut SessionWorld, function: String, step: &Step) {
    let payload = docstring_payload(world, step);
    world.call_pure(&function, payload).await;
}

#[given(regex = r#"^I call "([^"]+)" with "\{\}"$"#)]
#[when(regex = r#"^I call "([^"]+)" with "\{\}"$"#)]
async fn call_empty(world: &mut SessionWorld, function: String) {
    world.call_pure(&function, serde_json::json!({})).await;
}

// ---------------------------------------------------------------------------
// Outcome assertions
// ---------------------------------------------------------------------------

#[then("the call succeeds")]
async fn call_succeeds(world: &mut SessionWorld) {
    if skipped(world) {
        return;
    }
    if let Some(err) = &world.last_error {
        panic!("expected success but the call failed: {err}");
    }
    assert!(world.last_response.is_some(), "no call has been made yet");
}

#[then(regex = r#"^the call fails with code "([^"]+)"$"#)]
async fn call_fails_with(world: &mut SessionWorld, code: String) {
    if skipped(world) {
        return;
    }
    let err = world
        .last_error
        .as_ref()
        .unwrap_or_else(|| panic!("expected failure with {code} but the call succeeded"));
    assert!(
        err.contains(&code),
        "expected error containing `{code}`, got `{err}`"
    );
}

#[then("the response is null")]
async fn response_is_null(world: &mut SessionWorld) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    assert!(response.is_null(), "expected null response, got {response}");
}

#[then(regex = r#"^the response field "([^"]+)" is (.+)$"#)]
async fn response_field_is(world: &mut SessionWorld, path: String, raw: String) {
    if skipped(world) {
        return;
    }
    let expected = parse_expected(&world.substitute(&raw));
    let response = response_or_panic(world);
    let actual = lookup_path(response, &path)
        .unwrap_or_else(|| panic!("response has no field `{path}`: {response}"));
    assert_eq!(
        actual, &expected,
        "response field `{path}` is {actual}, expected {expected}"
    );
}

#[then(regex = r#"^the response has no field "([^"]+)"$"#)]
async fn response_field_absent(world: &mut SessionWorld, path: String) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    assert!(
        lookup_path(response, &path).is_none(),
        "expected `{path}` to be absent, got {:?}",
        lookup_path(response, &path)
    );
}

#[then(regex = r#"^the response field "([^"]+)" has length (\d+)$"#)]
async fn response_field_length(world: &mut SessionWorld, path: String, expected: usize) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    let value = lookup_path(response, &path)
        .unwrap_or_else(|| panic!("response has no field `{path}`: {response}"));
    let len = value
        .as_array()
        .unwrap_or_else(|| panic!("field `{path}` is not an array: {value}"))
        .len();
    assert_eq!(
        len, expected,
        "field `{path}` has length {len}, expected {expected}"
    );
}

// ---------------------------------------------------------------------------
// Aliases & clock
// ---------------------------------------------------------------------------

#[given(regex = r#"^I alias the response field "([^"]+)" as "([^"]+)"$"#)]
#[when(regex = r#"^I alias the response field "([^"]+)" as "([^"]+)"$"#)]
#[then(regex = r#"^I alias the response field "([^"]+)" as "([^"]+)"$"#)]
async fn alias_response_field(world: &mut SessionWorld, path: String, alias: String) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    let value = lookup_path(response, &path)
        .unwrap_or_else(|| panic!("response has no field `{path}`: {response}"));
    let raw = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    world.aliases.insert(alias, raw);
}

#[given(regex = r#"^a fresh unique marker as "([^"]+)"$"#)]
async fn fresh_marker(world: &mut SessionWorld, alias: String) {
    world
        .aliases
        .insert(alias, uuid::Uuid::new_v4().simple().to_string());
}

#[given(regex = r"^the clock is at (\d+)$")]
async fn clock_is_at(world: &mut SessionWorld, ms: i64) {
    world.clock.set(ms);
}

#[given(regex = r"^the clock advances by (\d+) ms$")]
#[when(regex = r"^the clock advances by (\d+) ms$")]
async fn clock_advances(world: &mut SessionWorld, ms: i64) {
    world.clock.advance(ms);
}
