//! AgentFunction (iii function invocation schemas) → OpenAI `tools` array.
use crate::wire::names::encode_tool_name;
use llm_router::types::model::AgentFunction;
use serde_json::{json, Value};

pub fn functions_to_wire(tools: &[AgentFunction]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": encode_tool_name(&t.name),
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })
        })
        .collect()
}

/// Responses uses the same function definition fields without the nested
/// `function` envelope used by Chat Completions.
pub fn functions_to_responses_wire(tools: &[AgentFunction]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": encode_tool_name(&tool.name),
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_name_description_and_schema_under_function_envelope() {
        let tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "Invoke an iii function".into(),
            parameters: json!({ "type": "object", "properties": { "id": { "type": "string" } } }),
            label: None,
            execution_mode: None,
        }];
        let wire = functions_to_wire(&tools);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["type"], "function");
        assert_eq!(wire[0]["function"]["name"], "agent__trigger");
        assert_eq!(wire[0]["function"]["description"], "Invoke an iii function");
        assert_eq!(wire[0]["function"]["parameters"]["type"], "object");
        assert!(
            wire[0]["function"].get("label").is_none(),
            "label/execution_mode are iii-side only"
        );
    }

    #[test]
    fn empty_input_yields_empty_array() {
        assert!(functions_to_wire(&[]).is_empty());
        assert!(functions_to_responses_wire(&[]).is_empty());
    }

    #[test]
    fn responses_shape_has_flat_function_fields() {
        let tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "Invoke an iii function".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let wire = functions_to_responses_wire(&tools);
        assert_eq!(wire[0]["type"], "function");
        assert_eq!(wire[0]["name"], "agent__trigger");
        assert!(wire[0].get("function").is_none());
    }
}
