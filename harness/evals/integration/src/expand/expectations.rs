use std::collections::BTreeMap;

use anyhow::Context;
use serde_json::{json, Map, Value};

use crate::types::scenario::{IntegrationScenarioV1, InvariantSpecV1, TerminalStatusV1};

use super::{CompiledFunctionCall, SYNTHETIC_FUNCTION_ALIAS};

pub(super) fn compile_expectations(
    authored: &IntegrationScenarioV1,
    function_ids: &BTreeMap<String, String>,
    calls: &[CompiledFunctionCall],
    generation_count: usize,
) -> anyhow::Result<Vec<InvariantSpecV1>> {
    let expect = &authored.expect;
    let mut invariants = vec![invariant(
        "send.flags",
        json!({
            "merged": expect.send_flags.merged,
            "queued": expect.send_flags.queued,
            "deduplicated": expect.send_flags.deduplicated
        }),
    )];

    if let Some(counts) = expect.message_counts {
        invariants.push(invariant(
            "transcript.message_counts",
            serde_json::to_value(counts)?,
        ));
    }
    if let Some(text) = &expect.assistant_text {
        invariants.push(invariant(
            "transcript.assistant_text",
            json!({ "text": text }),
        ));
    }
    for result in &expect.function_results {
        let call = calls.iter().find(|call| call.id == result.function_call_id);
        if call.is_none() {
            anyhow::bail!(
                "function result expectation references unknown function call {:?}",
                result.function_call_id
            );
        }
        if let (Some(call), Some(expected_function)) = (call, &result.function) {
            if call.function != *expected_function {
                anyhow::bail!(
                    "function result expectation for {:?} names function {:?}, but the call targets {:?}",
                    result.function_call_id,
                    expected_function,
                    call.function
                );
            }
        }
        let mut parameters = Map::new();
        parameters.insert(
            "function_call_id".to_string(),
            json!(result.function_call_id),
        );
        if let Some(alias) = &result.function {
            parameters.insert(
                "function_id".to_string(),
                json!(resolve_alias(function_ids, alias, "function result")?),
            );
        }
        if let Some(content) = &result.content {
            parameters.insert("content".to_string(), json!(content));
        }
        if let Some(is_error) = result.is_error {
            parameters.insert("is_error".to_string(), json!(is_error));
        }
        invariants.push(InvariantSpecV1 {
            id: "transcript.function_result".to_string(),
            parameters,
        });
    }
    if expect.calls_closed {
        invariants.push(invariant("transcript.calls_closed", json!({})));
    }
    if expect.no_duplicates {
        invariants.push(invariant("transcript.no_duplicates", json!({})));
    }
    invariants.push(invariant(
        "status.terminal",
        json!({
            "status": terminal_status(expect.terminal.status),
            "pending_calls": expect.terminal.pending_calls,
            "queued_messages": expect.terminal.queued_messages
        }),
    ));
    invariants.push(invariant(
        "lifecycle.completed_once",
        json!({
            "allow_identical_duplicates": expect.lifecycle.allow_identical_duplicates
        }),
    ));
    invariants.push(invariant(
        "router.generations_consumed",
        json!({
            "count": generation_count as u64
        }),
    ));

    for call in &expect.calls {
        let mut parameters = Map::new();
        parameters.insert(
            "function_id".to_string(),
            json!(resolve_alias(
                function_ids,
                &call.function,
                "call expectation"
            )?),
        );
        parameters.insert("count".to_string(), json!(call.count));
        if let Some(payload) = &call.payload {
            parameters.insert("payload".to_string(), payload.clone());
        }
        if let Some(payload_subset) = &call.payload_subset {
            parameters.insert("payload_subset".to_string(), payload_subset.clone());
        }
        invariants.push(InvariantSpecV1 {
            id: "target.calls".to_string(),
            parameters,
        });
    }
    if authored.functions.is_empty() {
        invariants.push(invariant(
            "target.calls",
            json!({
                "function_id": format!("{{{{run_id}}}}::{SYNTHETIC_FUNCTION_ALIAS}"),
                "count": 0
            }),
        ));
    }
    Ok(invariants)
}

fn invariant(id: &str, parameters: Value) -> InvariantSpecV1 {
    InvariantSpecV1 {
        id: id.to_string(),
        parameters: parameters
            .as_object()
            .cloned()
            .expect("invariant parameters are objects"),
    }
}

fn resolve_alias<'a>(
    function_ids: &'a BTreeMap<String, String>,
    alias: &str,
    context: &str,
) -> anyhow::Result<&'a str> {
    function_ids
        .get(alias)
        .map(String::as_str)
        .with_context(|| format!("{context} references unknown function alias {alias:?}"))
}

fn terminal_status(status: TerminalStatusV1) -> &'static str {
    match status {
        TerminalStatusV1::Completed => "completed",
        TerminalStatusV1::Failed => "failed",
        TerminalStatusV1::Cancelled => "cancelled",
    }
}
