//! Schema-based readiness (spec § Readiness): never sleep-based. The probe
//! retries until every surface is present or the deadline passes, then
//! reports **every** missing surface by name (classification `setup_error`).

use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;

#[derive(Debug, Clone)]
pub struct ReadinessSpec {
    /// Exact function ids that must be registered. Internal functions are
    /// visible because the probe passes `include_internal: true`.
    pub functions: Vec<String>,
    /// Trigger types that must be registered (e.g. `harness::turn-completed`).
    pub trigger_types: Vec<String>,
    /// Queue topics that must exist as (name, expected broker type).
    pub queue_topics: Vec<(String, String)>,
    /// `configuration::get` id → expected seeded value (canonical-JSON
    /// byte-compare).
    pub config_entries: Vec<(String, Value)>,
}

impl ReadinessSpec {
    /// The surface required before Arm — everything except the harness,
    /// which is spawned after Arm (see `stack::WORKER_START_ORDER`).
    pub fn pre_harness(config_entries: Vec<(String, Value)>) -> Self {
        let functions = [
            // Session durability.
            "session::messages",
            // Context manager is mandatory and fails closed when absent.
            "context::assemble",
            "context::count-tokens",
            // The scripted router owns the fixed router ids.
            "router::chat",
            "router::abort",
            "router::models::list",
            "router::models::get",
            "router::models::supports",
            "router::system_prompt::get",
            // Recorder's only public engine surface. Configuration,
            // reset, and snapshots stay inside the runner process.
            "integration-recorder::lifecycle",
            // Queue surface consumed by the probe itself.
            "engine::queue::list_topics",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        Self {
            functions,
            trigger_types: Vec::new(),
            queue_topics: Vec::new(),
            config_entries,
        }
    }

    /// The harness surface, probed after `Stack::spawn_harness` and before
    /// Send: public functions, lifecycle trigger types, the provisioned
    /// `harness-turn` topic, and the harness's own seeded config entry.
    pub fn harness_surface(config_entries: Vec<(String, Value)>) -> Self {
        Self {
            functions: vec!["harness::send".to_string(), "harness::status".to_string()],
            trigger_types: vec![
                "harness::turn-started".to_string(),
                "harness::turn-completed".to_string(),
            ],
            queue_topics: vec![("harness-turn".to_string(), "builtin".to_string())],
            config_entries,
        }
    }
}

#[derive(Debug)]
pub struct ReadinessReport {
    /// Empty when ready. Each entry names one missing/mismatched surface.
    pub missing: Vec<String>,
}

/// Probe until ready or deadline. Returns the last report on timeout.
pub async fn probe(
    client: &Client,
    spec: &ReadinessSpec,
    deadline: Deadline,
) -> Result<(), ReadinessReport> {
    loop {
        let report = probe_once(client, spec, deadline).await;
        if report.missing.is_empty() {
            return Ok(());
        }
        if deadline.is_expired() {
            return Err(report);
        }
        let remaining = deadline.remaining();
        tokio::time::sleep(std::time::Duration::from_millis(250).min(remaining)).await;
    }
}

async fn probe_once(client: &Client, spec: &ReadinessSpec, deadline: Deadline) -> ReadinessReport {
    let mut missing = Vec::new();

    // 1. Discovery responds, and every required function id is registered.
    match client
        .call_with_deadline(
            "engine::functions::list",
            json!({ "include_internal": true }),
            deadline,
            DEFAULT_CALL_TIMEOUT_MS,
        )
        .await
    {
        Ok(listed) => missing.extend(missing_functions(spec, &listed)),
        Err(e) => missing.push(format!("engine::functions::list unavailable: {e}")),
    }

    // 2. Trigger types.
    if !spec.trigger_types.is_empty() {
        match client
            .call_with_deadline(
                "engine::triggers::list",
                json!({ "include_internal": true }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
        {
            Ok(listed) => missing.extend(missing_trigger_types(spec, &listed)),
            Err(e) => missing.push(format!("engine::triggers::list unavailable: {e}")),
        }
    }

    // 3. Queue topics with broker type.
    if !spec.queue_topics.is_empty() {
        match client
            .call_with_deadline(
                "engine::queue::list_topics",
                json!({}),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
        {
            Ok(listed) => missing.extend(topic_failures(spec, &listed)),
            Err(e) => missing.push(format!("engine::queue::list_topics unavailable: {e}")),
        }
    }

    // 4. Seeded configuration entries are authoritative. Workers store their
    // RESOLVED config (seed merged with defaults — observed on first boot),
    // so the check is: every seeded key is present with exactly the seeded
    // value. Recorded as a spec correction to the original byte-compare.
    for (id, expected) in &spec.config_entries {
        match client
            .call_with_deadline(
                "configuration::get",
                json!({ "id": id }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
        {
            Ok(resp) => missing.extend(config_failure(id, expected, &resp)),
            Err(e) => missing.push(format!("configuration {id} unavailable: {e}")),
        }
    }

    ReadinessReport { missing }
}

/// Pure check: required function ids against a discovery listing.
pub fn missing_functions(spec: &ReadinessSpec, listed: &Value) -> Vec<String> {
    let ids = collect_ids(listed, &["function_id", "id"]);
    spec.functions
        .iter()
        .filter(|required| !ids.contains(*required))
        .map(|required| format!("function {required}"))
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
                .filter_map(|t| {
                    Some((
                        t.get("name")?.as_str()?.to_string(),
                        t.get("broker_type")
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
fn collect_ids(listed: &Value, keys: &[&str]) -> std::collections::BTreeSet<String> {
    let items: Vec<&Value> = match listed {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map
            .values()
            .find_map(|v| v.as_array())
            .map(|a| a.iter().collect())
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
                .find_map(|k| item.get(k).and_then(Value::as_str))
                .map(String::from)
        })
        .collect()
}
