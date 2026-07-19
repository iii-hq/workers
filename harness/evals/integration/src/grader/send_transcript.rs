use serde_json::{json, Map, Value};

use super::helpers::{flag, literal_subset};
use super::{Evidence, GradeOutcome};

pub(super) fn grade_send_flags(params: &Map<String, Value>, evidence: &Evidence) -> GradeOutcome {
    let expected = json!({
        "accepted": true,
        "merged": flag(params, "merged"),
        "queued": flag(params, "queued"),
        "deduplicated": flag(params, "deduplicated"),
    });
    // Absent optional flags normalize to false.
    let actual = match &evidence.send_response {
        Some(response) => json!({
            "accepted": response.get("accepted") == Some(&Value::Bool(true)),
            "merged": response.get("merged") == Some(&Value::Bool(true)),
            "queued": response.get("queued") == Some(&Value::Bool(true)),
            "deduplicated": response.get("deduplicated") == Some(&Value::Bool(true)),
        }),
        None => Value::Null,
    };
    (
        actual == expected,
        expected,
        actual,
        vec!["send-response.json"],
    )
}

pub(super) fn grade_message_counts(
    params: &Map<String, Value>,
    evidence: &Evidence,
) -> GradeOutcome {
    let mut counts = std::collections::BTreeMap::new();
    for role in ["user", "assistant", "function_result"] {
        counts.insert(role.to_string(), 0u64);
    }
    for item in &evidence.transcript {
        if let Some(role) = item
            .get("message")
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
        {
            *counts.entry(role.to_string()).or_insert(0) += 1;
        }
    }
    let expected = Value::Object(params.clone());
    let actual = serde_json::to_value(&counts).expect("counts serialize");
    let passed = params.iter().all(|(role, want)| {
        counts.get(role).copied().unwrap_or(0) == want.as_u64().unwrap_or(u64::MAX)
    });
    (passed, expected, actual, vec!["transcript.json"])
}

pub(super) fn grade_assistant_text(
    params: &Map<String, Value>,
    evidence: &Evidence,
) -> GradeOutcome {
    let expected = Value::Object(params.clone());
    let last_assistant = evidence
        .transcript
        .iter()
        .filter_map(|item| item.get("message"))
        .rfind(|message| message.get("role").and_then(Value::as_str) == Some("assistant"));
    let actual = match last_assistant {
        Some(message) => {
            let text: String = message
                .get("content")
                .and_then(Value::as_array)
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|block| block.get("text").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default();
            json!({
                "text": text,
                "usage": message.get("usage").cloned().unwrap_or(Value::Null)
            })
        }
        None => Value::Null,
    };
    let passed = match &actual {
        Value::Null => false,
        actual => {
            actual.get("text") == params.get("text")
                && params.get("usage").is_none_or(|want| {
                    literal_subset(want, actual.get("usage").unwrap_or(&Value::Null))
                })
        }
    };
    (passed, expected, actual, vec!["transcript.json"])
}

pub(super) fn grade_no_duplicates(evidence: &Evidence) -> GradeOutcome {
    let mut seen = std::collections::BTreeSet::new();
    let mut duplicates = Vec::new();
    for item in &evidence.transcript {
        if let Some(entry_id) = item.get("entry_id").and_then(Value::as_str) {
            if !seen.insert(entry_id.to_string()) {
                duplicates.push(entry_id.to_string());
            }
        }
    }
    let passed = duplicates.is_empty();
    (
        passed,
        json!({ "duplicate_entry_ids": [] }),
        json!({ "duplicate_entry_ids": duplicates }),
        vec!["transcript.json"],
    )
}

/// Every dispatched function call must be closed by a durable result.
pub(super) fn grade_calls_closed(evidence: &Evidence) -> GradeOutcome {
    let mut call_ids = Vec::new();
    let mut result_ids = std::collections::BTreeSet::new();
    for item in &evidence.transcript {
        let Some(message) = item.get("message") else {
            continue;
        };
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                for block in message
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if block.get("type").and_then(Value::as_str) == Some("function_call") {
                        if let Some(id) = block.get("id").and_then(Value::as_str) {
                            call_ids.push(id.to_string());
                        }
                    }
                }
            }
            Some("function_result") => {
                if let Some(id) = message.get("function_call_id").and_then(Value::as_str) {
                    result_ids.insert(id.to_string());
                }
            }
            _ => {}
        }
    }
    let dangling: Vec<&String> = call_ids
        .iter()
        .filter(|id| !result_ids.contains(*id))
        .collect();
    let passed = dangling.is_empty();
    (
        passed,
        json!({ "dangling_function_calls": [] }),
        json!({
            "dangling_function_calls": dangling,
            "calls": call_ids,
            "results": result_ids,
        }),
        vec!["transcript.json"],
    )
}

pub(super) fn grade_function_result(
    params: &Map<String, Value>,
    evidence: &Evidence,
) -> GradeOutcome {
    let expected = Value::Object(params.clone());
    let expected_call_id = params
        .get("function_call_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let results: Vec<&Value> = evidence
        .transcript
        .iter()
        .filter_map(|item| item.get("message"))
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("function_result"))
        .filter(|message| {
            message.get("function_call_id").and_then(Value::as_str) == Some(expected_call_id)
        })
        .collect();
    let actual = json!(results);
    let passed = results.len() == 1 && literal_subset(&expected, results[0]);
    (passed, expected, actual, vec!["transcript.json"])
}
