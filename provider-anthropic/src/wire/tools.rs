//! AgentFunction (iii function invocation schemas) → Anthropic `tools` array.
use crate::wire::names::encode_tool_name;
use llm_router::types::model::AgentFunction;
use serde_json::{json, Value};

pub fn functions_to_wire(tools: &[AgentFunction]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "name": encode_tool_name(&t.name),
                "description": t.description,
                "input_schema": t.parameters,
                // Stream tool input incrementally instead of the default
                // server-side buffering, which goes ping-only silent for the
                // whole generation of a large input (measured: ~120s for a
                // ~4k-token input, one leading ~100B delta then a single
                // burst) — long enough to trip the router's 120s post-content
                // idle guard and kill a healthy stream. GA successor of the
                // fine-grained-tool-streaming-2025-05-14 beta header (which
                // some gateways now reject). Trade-off: on an early stop the
                // input may be partial/invalid JSON, which degraded_arguments
                // salvages and the harness refuses to execute.
                "eager_input_streaming": true,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_name_description_and_schema() {
        let tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "Invoke an iii function".into(),
            parameters: json!({ "type": "object", "properties": { "id": { "type": "string" } } }),
            label: None,
            execution_mode: None,
        }];
        let wire = functions_to_wire(&tools);
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["name"], "agent__trigger");
        assert_eq!(wire[0]["description"], "Invoke an iii function");
        assert_eq!(wire[0]["input_schema"]["type"], "object");
        // Incremental tool-input streaming: without it, large inputs are
        // server-buffered into a ping-only silence that trips the router's
        // idle guard.
        assert_eq!(wire[0]["eager_input_streaming"], true);
        assert!(
            wire[0].get("label").is_none(),
            "label/execution_mode are iii-side only"
        );
    }

    #[test]
    fn empty_input_yields_empty_array() {
        assert!(functions_to_wire(&[]).is_empty());
    }
}
