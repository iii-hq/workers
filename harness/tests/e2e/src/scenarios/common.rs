use serde_json::{json, Value};

use crate::context::E2eContext;
use crate::report::HardGateReport;

use super::{CriterionAward, EvaluationFuture, ObjectiveEvaluation, ScenarioObservation};

#[derive(Debug, Clone, PartialEq)]
pub struct ObservedFunctionCall {
    pub function_id: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObservedFunctionInvocation {
    pub call_id: Option<String>,
    pub call: ObservedFunctionCall,
}

pub fn final_response(transcript: &Value) -> String {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .filter_map(|entry| entry.get("message"))
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .map(|message| {
            message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .find(|response| !response.trim().is_empty())
        .unwrap_or_default()
}

pub fn function_calls(transcript: &Value) -> Vec<ObservedFunctionCall> {
    function_invocations(transcript)
        .into_iter()
        .map(|invocation| invocation.call)
        .collect()
}

pub fn function_invocations(transcript: &Value) -> Vec<ObservedFunctionInvocation> {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("message"))
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .flat_map(|message| {
            message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("function_call"))
        .filter_map(normalize_invocation)
        .collect()
}

fn normalize_invocation(block: &Value) -> Option<ObservedFunctionInvocation> {
    let call_id = block.get("id").and_then(Value::as_str).map(str::to_owned);
    let function_id = block.get("function_id")?.as_str()?;
    let arguments = block.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if function_id == "agent_trigger" {
        return Some(ObservedFunctionInvocation {
            call_id,
            call: ObservedFunctionCall {
                function_id: arguments.get("function")?.as_str()?.to_string(),
                arguments: arguments
                    .get("payload")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            },
        });
    }
    Some(ObservedFunctionInvocation {
        call_id,
        call: ObservedFunctionCall {
            function_id: function_id.to_string(),
            arguments,
        },
    })
}

pub fn function_result<'a>(
    transcript: &'a Value,
    invocation: &ObservedFunctionInvocation,
) -> Option<&'a Value> {
    let call_id = invocation.call_id.as_deref()?;
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("message"))
        .find(|message| {
            message.get("role").and_then(Value::as_str) == Some("function_result")
                && message.get("function_call_id").and_then(Value::as_str) == Some(call_id)
                && message.get("function_id").and_then(Value::as_str)
                    == Some(invocation.call.function_id.as_str())
                && message.get("is_error").and_then(Value::as_bool) == Some(false)
        })
}

pub fn gate(id: &str, passed: bool, reason: impl Into<String>) -> HardGateReport {
    HardGateReport {
        id: id.to_string(),
        passed,
        reason: reason.into(),
    }
}

pub fn award(id: &'static str, awarded: u8, reason: impl Into<String>) -> CriterionAward {
    CriterionAward {
        id: id.to_string(),
        awarded,
        reason: reason.into(),
    }
}

pub fn state_value(response: Value) -> Value {
    match response {
        Value::Object(mut object)
            if object.get("ok").and_then(Value::as_bool) == Some(true)
                && object.contains_key("value") =>
        {
            object.remove("value").unwrap_or(Value::Null)
        }
        response => response,
    }
}

pub fn requested_once(arguments: &Value) -> bool {
    arguments.get("once").and_then(Value::as_bool) == Some(true)
        || arguments
            .pointer("/lifecycle/once")
            .and_then(Value::as_bool)
            == Some(true)
}

pub fn is_wake_registration(arguments: &Value) -> bool {
    arguments.get("function_id").is_none_or(Value::is_null)
        && arguments.get("target").is_none_or(|target| {
            target.is_null()
                || target.get("function_id").and_then(Value::as_str) == Some("harness::send")
        })
}

/// Whether ANY text block in the transcript contains `needle` — for spotting
/// error results and machine messages regardless of which entry carried them.
pub fn transcript_contains(transcript: &Value, needle: &str) -> bool {
    fn walk(value: &Value, needle: &str) -> bool {
        match value {
            Value::String(text) => text.contains(needle),
            Value::Array(items) => items.iter().any(|item| walk(item, needle)),
            Value::Object(map) => map.values().any(|item| walk(item, needle)),
            _ => false,
        }
    }
    walk(transcript, needle)
}

/// Validation nudges the harness appended to this transcript — the
/// re-prompts a `harness::hook::post-turn` validator (or the output
/// contract) produced. Recognized by the durable entry id
/// (`e_<turn>_nudge_<n>`) or the `validation` origin flag.
pub fn validation_nudges(transcript: &Value) -> usize {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry
                .get("entry_id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.contains("_nudge_"))
                || entry
                    .pointer("/origin/validation")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
        })
        .count()
}

pub fn trigger_fired_records(transcript: &Value) -> Vec<&Value> {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("custom"))
        .filter(|custom| custom.get("custom_type").and_then(Value::as_str) == Some("trigger_fired"))
        .filter_map(|custom| custom.get("data"))
        .collect()
}

pub async fn active_binding_count(context: &E2eContext, session_id: &str) -> anyhow::Result<usize> {
    Ok(context
        .trigger_value(
            "harness::triggers::list",
            json!({ "session_id": session_id }),
        )
        .await?
        .get("subscriptions")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(usize::MAX))
}

pub fn evaluate_text_response<'a>(
    _context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    _run_id: &'a str,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let calls = function_calls(&observation.transcript);
        let response = observation.response.as_str();
        Ok(ObjectiveEvaluation {
            hard_gates: vec![
                gate(
                    "response_present",
                    !response.trim().is_empty(),
                    if response.trim().is_empty() {
                        "the assistant returned no text"
                    } else {
                        "the assistant returned a textual response"
                    },
                ),
                gate(
                    "no_function_calls",
                    calls.is_empty() && observation.metrics.totals.function_calls == 0,
                    format!("observed {} function call(s)", calls.len()),
                ),
                gate(
                    "single_turn",
                    observation.metrics.totals.turns == 1,
                    format!(
                        "observed {} turn(s), expected exactly one",
                        observation.metrics.totals.turns
                    ),
                ),
            ],
            awards: Vec::new(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_agent_trigger_and_native_function_calls() {
        let transcript = json!({
            "messages": [{
                "message": {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "function_call",
                            "id": "call-state",
                            "function_id": "agent_trigger",
                            "arguments": {
                                "function": "state::set",
                                "payload": { "scope": "s", "key": "k", "value": 1 }
                            }
                        },
                        {
                            "type": "function_call",
                            "id": "call-native",
                            "function_id": "native::call",
                            "arguments": { "value": 2 }
                        }
                    ]
                }
            }]
        });
        let calls = function_calls(&transcript);
        assert_eq!(calls[0].function_id, "state::set");
        assert_eq!(calls[0].arguments["key"], "k");
        assert_eq!(calls[1].function_id, "native::call");
    }

    #[test]
    fn correlates_a_function_result_with_its_call_id() {
        let transcript = json!({
            "messages": [
                {"message": {"role": "assistant", "content": [{
                    "type": "function_call",
                    "id": "call-match",
                    "function_id": "agent_trigger",
                    "arguments": {
                        "function": "state::set",
                        "payload": { "scope": "s", "key": "k", "value": 1 }
                    }
                }]}},
                {"message": {
                    "role": "function_result",
                    "function_call_id": "call-other",
                    "function_id": "state::set",
                    "is_error": false,
                    "details": { "ok": false }
                }},
                {"message": {
                    "role": "function_result",
                    "function_call_id": "call-match",
                    "function_id": "state::set",
                    "is_error": false,
                    "details": { "ok": true }
                }}
            ]
        });
        let invocations = function_invocations(&transcript);
        let result = function_result(&transcript, &invocations[0]).expect("matching result");

        assert_eq!(invocations[0].call_id.as_deref(), Some("call-match"));
        assert_eq!(
            result.pointer("/details/ok").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn extracts_the_last_nonempty_assistant_response() {
        let transcript = json!({
            "messages": [
                {"message": {"role": "user", "content": [{"type": "text", "text": "no"}]}},
                {"message": {"role": "assistant", "content": [
                    {"type": "text", "text": "yes "},
                    {"type": "text", "text": "indeed"},
                    {"type": "function_call", "function_id": "x", "arguments": {}}
                ]}},
                {"message": {"role": "assistant", "content": [
                    {"type": "function_call", "function_id": "x", "arguments": {}}
                ]}},
            ]
        });
        assert_eq!(final_response(&transcript), "yes indeed");
    }
}
