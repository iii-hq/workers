use std::collections::BTreeSet;

use serde_json::Value;

use super::{ExpectedTriggerBinding, ReadinessSpec};

/// Pure check: required function ids against a discovery listing.
pub fn missing_functions(spec: &ReadinessSpec, listed: &Value) -> Vec<String> {
    let ids = collect_ids(listed, &["function_id", "id"]);
    spec.functions
        .iter()
        .filter(|required| !ids.contains(&required.function_id))
        .map(|required| format!("function {}", required.function_id))
        .collect()
}

/// Structured discovery checks reused by Arm polling. These deliberately
/// inspect descriptor ids rather than searching a serialized JSON blob.
pub fn has_function(listed: &Value, function_id: &str) -> bool {
    collect_ids(listed, &["function_id", "id"]).contains(function_id)
}

pub fn has_registered_trigger(listed: &Value, function_id: &str) -> bool {
    collect_ids(listed, &["function_id", "id"]).contains(function_id)
}

pub fn registered_trigger_failures(
    expected: &[ExpectedTriggerBinding],
    listed: &Value,
) -> Vec<String> {
    let rows = listed
        .get("registered_triggers")
        .and_then(Value::as_array)
        .or_else(|| listed.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut failures = Vec::new();

    for binding in expected {
        let for_function = rows
            .iter()
            .filter(|row| {
                row.get("function_id").and_then(Value::as_str) == Some(binding.function_id.as_str())
            })
            .collect::<Vec<_>>();
        let exact = for_function
            .iter()
            .filter(|row| {
                row.get("trigger_type").and_then(Value::as_str)
                    == Some(binding.trigger_type.as_str())
                    && row.get("config") == Some(&binding.config)
            })
            .count();
        if exact != 1 {
            failures.push(format!(
                "trigger binding {} -> {} with config {}: expected exactly one, got {exact}",
                binding.trigger_type, binding.function_id, binding.config
            ));
        }

        let expected_for_function = expected
            .iter()
            .filter(|candidate| candidate.function_id == binding.function_id)
            .count();
        if for_function.len() != expected_for_function {
            failures.push(format!(
                "trigger binding target {}: expected {expected_for_function} total registration(s), got {}",
                binding.function_id,
                for_function.len()
            ));
        }
    }
    failures.sort();
    failures.dedup();
    failures
}

pub fn registered_trigger_count(listed: &Value, expected: &ExpectedTriggerBinding) -> usize {
    listed
        .get("registered_triggers")
        .and_then(Value::as_array)
        .or_else(|| listed.as_array())
        .into_iter()
        .flatten()
        .filter(|row| {
            row.get("function_id").and_then(Value::as_str) == Some(expected.function_id.as_str())
                && row.get("trigger_type").and_then(Value::as_str)
                    == Some(expected.trigger_type.as_str())
                && row.get("config") == Some(&expected.config)
        })
        .count()
}

/// Pure check: required trigger types against a trigger-type listing.
pub fn missing_trigger_types(spec: &ReadinessSpec, listed: &Value) -> Vec<String> {
    let ids = collect_ids(listed, &["trigger_type", "id", "name", "type"]);
    spec.trigger_types
        .iter()
        .filter(|required| !ids.contains(*required))
        .map(|required| format!("trigger type {required}"))
        .collect()
}

/// Pure check: required queue topics (name + broker type) against
/// `engine::queue::list_topics` output.
pub fn topic_failures(spec: &ReadinessSpec, listed: &Value) -> Vec<String> {
    let topics: Vec<(String, String)> = listed
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|topic| {
                    Some((
                        topic.get("name")?.as_str()?.to_string(),
                        topic
                            .get("broker_type")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut failures = Vec::new();
    for (topic, broker) in &spec.queue_topics {
        match topics.iter().find(|(name, _)| name == topic) {
            None => failures.push(format!("queue topic {topic}")),
            Some((_, actual)) if actual != broker => failures.push(format!(
                "queue topic {topic} broker type: expected {broker}, got {actual}"
            )),
            Some(_) => {}
        }
    }
    failures
}

/// Pure check: one seeded configuration entry against a
/// `configuration::get` response.
pub fn config_failure(id: &str, expected: &Value, resp: &Value) -> Option<String> {
    match resp.get("value") {
        Some(value) => crate::matcher::subset_of(expected, value)
            .map(|detail| format!("configuration {id}: seed not authoritative: {detail}")),
        None => Some(format!("configuration {id}: no value")),
    }
}

/// Collect id strings from a list response of unknown exact shape: an array
/// of descriptors (or `{functions: [...]}`/`{items: [...]}`), each carrying
/// the id under one of `keys`.
fn collect_ids(listed: &Value, keys: &[&str]) -> BTreeSet<String> {
    let items: Vec<&Value> = match listed {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map
            .values()
            .find_map(|value| value.as_array())
            .map(|items| items.iter().collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(text) = item.as_str() {
                return Some(text.to_string());
            }
            keys.iter()
                .find_map(|key| item.get(key).and_then(Value::as_str))
                .map(String::from)
        })
        .collect()
}

pub(super) fn listed_ids(listed: &Value) -> BTreeSet<String> {
    collect_ids(listed, &["function_id", "id"])
}
