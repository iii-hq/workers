//! AgentFunction (iii function invocation schemas) → OpenAI Responses `tools`
//! array. Unlike Chat Completions (`{type:function, function:{...}}`), the
//! Responses API takes the function fields flat under the tool entry.
use crate::wire::names::encode_tool_name;
use llm_router::types::model::AgentFunction;
use serde_json::{json, Value};

pub fn functions_to_wire(tools: &[AgentFunction]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            // The V4A patch tool goes out as a FREEFORM custom tool named
            // exactly `apply_patch`: its input is the raw patch text codex
            // models are trained to emit — no JSON escaping of the patch.
            if t.name == crate::wire::names::APPLY_PATCH_FN {
                return json!({
                    "type": "custom",
                    "name": crate::wire::names::APPLY_PATCH_WIRE,
                    "description": t.description,
                });
            }
            json!({
                "type": "function",
                "name": encode_tool_name(&t.name),
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_flat_function_fields_with_encoded_name() {
        let tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "Invoke an iii function".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let wire = functions_to_wire(&tools);
        assert_eq!(wire[0]["type"], "function");
        assert_eq!(wire[0]["name"], "agent__trigger");
        assert_eq!(wire[0]["description"], "Invoke an iii function");
        assert_eq!(wire[0]["parameters"]["type"], "object");
        assert!(
            wire[0].get("function").is_none(),
            "Responses is flat, not nested"
        );
    }

    #[test]
    fn empty_input_yields_empty_array() {
        assert!(functions_to_wire(&[]).is_empty());
    }

    #[test]
    fn apply_patch_goes_out_as_freeform_custom_tool() {
        let tools = vec![AgentFunction {
            name: "coder::apply-patch".into(),
            description: "Apply a V4A patch".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let wire = functions_to_wire(&tools);
        assert_eq!(wire[0]["type"], "custom");
        assert_eq!(wire[0]["name"], "apply_patch");
        assert!(
            wire[0].get("parameters").is_none(),
            "custom tools carry no JSON schema"
        );
    }
}
