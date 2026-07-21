use std::collections::BTreeMap;

use anyhow::Context;

use crate::types::scenario::{
    AuthoredScenarioV1, FunctionResultInvariantV1, InvariantSpecV1, TargetCallsInvariantV1,
};

use super::{CompiledFunctionCall, SYNTHETIC_FUNCTION_ALIAS};

pub(super) fn compile_expectations(
    authored: &AuthoredScenarioV1,
    function_ids: &BTreeMap<String, String>,
    calls: &[CompiledFunctionCall],
    generation_count: usize,
) -> anyhow::Result<Vec<InvariantSpecV1>> {
    let expect = &authored.expect;
    if !expect.turn_completes || !expect.script_fully_consumed {
        anyhow::bail!(
            "every scenario must assert turn_completes() and script_fully_consumed(); \
             without them a run can pass while nothing happened"
        );
    }
    let mut invariants = vec![InvariantSpecV1::send_flags(expect.send_flags)];

    if let Some(counts) = expect.message_counts {
        invariants.push(InvariantSpecV1::transcript_message_counts(counts));
    }
    if let Some(text) = &expect.assistant_text {
        invariants.push(InvariantSpecV1::transcript_assistant_text(text.clone()));
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
        invariants.push(InvariantSpecV1::transcript_function_result(
            FunctionResultInvariantV1 {
                function_call_id: result.function_call_id.clone(),
                function_id: result
                    .function
                    .as_deref()
                    .map(|alias| resolve_alias(function_ids, alias, "function result"))
                    .transpose()?
                    .map(str::to_string),
                content: result.content.clone(),
                is_error: result.is_error,
            },
        ));
    }
    if expect.calls_closed {
        invariants.push(InvariantSpecV1::transcript_calls_closed());
    }
    if expect.no_duplicates {
        invariants.push(InvariantSpecV1::transcript_no_duplicates());
    }
    invariants.push(InvariantSpecV1::status_terminal(expect.terminal));
    invariants.push(InvariantSpecV1::lifecycle_completed_once(
        expect.lifecycle,
        expect.terminal.status,
    ));
    invariants.push(InvariantSpecV1::router_generations_consumed(
        generation_count as u64,
    ));

    for call in &expect.calls {
        invariants.push(InvariantSpecV1::target_calls(TargetCallsInvariantV1 {
            function_id: resolve_alias(function_ids, &call.function, "call expectation")?
                .to_string(),
            count: call.count,
            payload: call.payload.clone(),
            payload_subset: call.payload_subset.clone(),
        }));
    }
    if authored.functions.is_empty() {
        invariants.push(InvariantSpecV1::target_calls(TargetCallsInvariantV1 {
            function_id: format!("{{{{run_id}}}}::{SYNTHETIC_FUNCTION_ALIAS}"),
            count: 0,
            payload: None,
            payload_subset: None,
        }));
    }
    Ok(invariants)
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
