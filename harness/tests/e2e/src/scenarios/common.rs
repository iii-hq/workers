use serde_json::{json, Value};

use crate::context::E2eContext;
use crate::report::HardGateReport;

use super::{CriterionAward, EvaluationFuture, ObjectiveEvaluation, ScenarioObservation};

#[derive(Debug, Clone, PartialEq)]
pub struct ObservedFunctionCall {
    pub function_id: String,
    pub arguments: Value,
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
        .filter_map(normalize_call)
        .collect()
}

fn normalize_call(block: &Value) -> Option<ObservedFunctionCall> {
    let function_id = block.get("function_id")?.as_str()?;
    let arguments = block.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if function_id == "agent_trigger" {
        return Some(ObservedFunctionCall {
            function_id: arguments.get("function")?.as_str()?.to_string(),
            arguments: arguments
                .get("payload")
                .cloned()
                .unwrap_or_else(|| json!({})),
        });
    }
    Some(ObservedFunctionCall {
        function_id: function_id.to_string(),
        arguments,
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
                            "function_id": "agent_trigger",
                            "arguments": {
                                "function": "state::set",
                                "payload": { "scope": "s", "key": "k", "value": 1 }
                            }
                        },
                        {
                            "type": "function_call",
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
