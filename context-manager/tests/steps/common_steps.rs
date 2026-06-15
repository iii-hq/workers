//! Generic call / response / error steps shared by every feature, plus
//! the JSON helpers other step modules reuse.

use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use serde_json::Value;

use crate::common::world::ContextWorld;

/// Navigate a dotted path (`applied.summary`, `messages.0.role`) into a
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
/// parses, raw string otherwise (so `is "ok"`, `is 3`, `is null`,
/// `is true` all behave naturally).
pub fn parse_expected(raw: &str) -> Value {
    serde_json::from_str(raw.trim()).unwrap_or_else(|_| Value::String(raw.trim().to_string()))
}

pub fn docstring_payload(world: &ContextWorld, step: &Step) -> Value {
    let text = world.substitute(step.docstring.as_deref().unwrap_or("{}"));
    serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("payload docstring must be valid JSON: {e}\n---\n{text}\n---"))
}

pub fn response_or_panic(world: &ContextWorld) -> &Value {
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
pub fn skipped(world: &ContextWorld) -> bool {
    world.soft_skipped
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

#[given(regex = r#"^I call "([^"]+)" with:$"#)]
#[when(regex = r#"^I call "([^"]+)" with:$"#)]
async fn call_with(world: &mut ContextWorld, function: String, step: &Step) {
    let payload = docstring_payload(world, step);
    world.call_pure(&function, payload).await;
}

#[given(regex = r#"^I call "([^"]+)" with "\{\}"$"#)]
#[when(regex = r#"^I call "([^"]+)" with "\{\}"$"#)]
async fn call_empty(world: &mut ContextWorld, function: String) {
    world.call_pure(&function, serde_json::json!({})).await;
}

// ---------------------------------------------------------------------------
// Outcome assertions
// ---------------------------------------------------------------------------

#[then("the call succeeds")]
async fn call_succeeds(world: &mut ContextWorld) {
    if skipped(world) {
        return;
    }
    if let Some(err) = &world.last_error {
        panic!("expected success but the call failed: {err}");
    }
    assert!(world.last_response.is_some(), "no call has been made yet");
}

#[then(regex = r#"^the call fails with code "([^"]+)"$"#)]
async fn call_fails_with(world: &mut ContextWorld, code: String) {
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

#[then(regex = r#"^the error contains "([^"]+)"$"#)]
async fn error_contains(world: &mut ContextWorld, needle: String) {
    if skipped(world) {
        return;
    }
    let err = world
        .last_error
        .as_ref()
        .expect("expected a failure but the call succeeded");
    assert!(
        err.contains(&needle),
        "expected error containing `{needle}`, got `{err}`"
    );
}

#[then(regex = r#"^the response field "([^"]+)" is (.+)$"#)]
async fn field_is(world: &mut ContextWorld, path: String, raw: String) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    let actual = lookup_path(response, &path)
        .unwrap_or_else(|| panic!("path `{path}` not found in {response}"));
    let expected = parse_expected(&raw);
    assert_eq!(
        actual, &expected,
        "field `{path}`: expected {expected}, got {actual}"
    );
}

#[then(regex = r#"^the response field "([^"]+)" contains "([^"]+)"$"#)]
async fn field_contains(world: &mut ContextWorld, path: String, needle: String) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    let actual = lookup_path(response, &path)
        .unwrap_or_else(|| panic!("path `{path}` not found in {response}"))
        .as_str()
        .unwrap_or_else(|| panic!("field `{path}` is not a string"));
    assert!(
        actual.contains(&needle),
        "field `{path}`: expected to contain `{needle}`, got `{actual}`"
    );
}

#[then(regex = r#"^the response field "([^"]+)" does not contain "([^"]+)"$"#)]
async fn field_not_contains(world: &mut ContextWorld, path: String, needle: String) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    let actual = lookup_path(response, &path)
        .unwrap_or_else(|| panic!("path `{path}` not found in {response}"))
        .as_str()
        .unwrap_or_else(|| panic!("field `{path}` is not a string"));
    assert!(
        !actual.contains(&needle),
        "field `{path}`: expected NOT to contain `{needle}`, got `{actual}`"
    );
}

// NOTE: comparison/absence steps deliberately avoid the word "is" so
// they can never be ambiguous with the generic `is <value>` regex
// above (the regex crate has no lookahead to disambiguate).

#[then(regex = r#"^the response field "([^"]+)" exceeds (\d+)$"#)]
async fn field_greater(world: &mut ContextWorld, path: String, floor: u64) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    let actual = lookup_path(response, &path)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("path `{path}` is not a number in {response}"));
    assert!(
        actual > floor,
        "field `{path}`: expected > {floor}, got {actual}"
    );
}

#[then(regex = r#"^the response field "([^"]+)" does not exceed (\d+)$"#)]
async fn field_at_most(world: &mut ContextWorld, path: String, ceiling: u64) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    let actual = lookup_path(response, &path)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("path `{path}` is not a number in {response}"));
    assert!(
        actual <= ceiling,
        "field `{path}`: expected <= {ceiling}, got {actual}"
    );
}

#[then(regex = r#"^the response has no field "([^"]+)"$"#)]
async fn field_absent(world: &mut ContextWorld, path: String) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    assert!(
        lookup_path(response, &path).is_none(),
        "expected `{path}` to be absent, but it is {:?}",
        lookup_path(response, &path)
    );
}

#[then(regex = r#"^the response field "([^"]+)" has (\d+) items$"#)]
async fn field_len(world: &mut ContextWorld, path: String, expected: usize) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    let actual = lookup_path(response, &path)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("path `{path}` is not an array in {response}"));
    assert_eq!(
        actual.len(),
        expected,
        "field `{path}`: expected {expected} items, got {}",
        actual.len()
    );
}

#[then(regex = r#"^the response field "([^"]+)" equals the sum of the "([^"]+)" fields$"#)]
async fn field_equals_object_sum(world: &mut ContextWorld, total: String, parts: String) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    let total_value = lookup_path(response, &total)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("`{total}` is not a number"));
    let parts_value = lookup_path(response, &parts)
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("`{parts}` is not an object"));
    let parts_sum: u64 = parts_value
        .iter()
        .map(|(key, value)| {
            value
                .as_u64()
                .unwrap_or_else(|| panic!("`{parts}.{key}` is not a non-negative integer"))
        })
        .sum();
    assert_eq!(
        total_value, parts_sum,
        "`{total}` did not match sum of `{parts}`"
    );
}

// ---------------------------------------------------------------------------
// Bookkeeping
// ---------------------------------------------------------------------------

#[given(regex = r#"^I remember the response field "([^"]+)" as "([^"]+)"$"#)]
#[when(regex = r#"^I remember the response field "([^"]+)" as "([^"]+)"$"#)]
#[then(regex = r#"^I remember the response field "([^"]+)" as "([^"]+)"$"#)]
async fn remember_field(world: &mut ContextWorld, path: String, alias: String) {
    if skipped(world) {
        return;
    }
    let response = response_or_panic(world);
    let value = lookup_path(response, &path)
        .unwrap_or_else(|| panic!("path `{path}` not found in {response}"));
    let rendered = match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    world.aliases.insert(alias, rendered);
}
