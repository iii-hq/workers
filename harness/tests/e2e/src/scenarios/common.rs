use serde_json::{json, Value};

use crate::report::HardGateReport;

use super::CriterionAward;

#[derive(Debug, Clone, PartialEq)]
pub struct ObservedFunctionCall {
    pub function_id: String,
    pub arguments: Value,
}

pub fn assistant_texts(transcript: &Value) -> Vec<String> {
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
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
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
        id,
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

pub fn final_response(output: &[String]) -> &str {
    output
        .iter()
        .rev()
        .find(|text| !text.trim().is_empty())
        .map(String::as_str)
        .unwrap_or("")
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
    fn extracts_only_assistant_text_blocks() {
        let transcript = json!({
            "messages": [
                {"message": {"role": "user", "content": [{"type": "text", "text": "no"}]}},
                {"message": {"role": "assistant", "content": [
                    {"type": "text", "text": "yes"},
                    {"type": "function_call", "function_id": "x", "arguments": {}}
                ]}}
            ]
        });
        assert_eq!(assistant_texts(&transcript), ["yes"]);
    }
}
