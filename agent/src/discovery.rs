use iii_sdk::{FunctionInfo, III};
use serde_json::json;

use crate::llm::ToolDef;

// Infrastructure namespaces the agent should never call as tools (it would
// recurse into itself, leak engine internals, or invoke routing primitives
// directly). Everything else is exposed — new worker namespaces are
// auto-discovered without a code change. Override with
// `discovery_excluded_prefixes` in config when a deployment needs a tighter
// boundary.
pub const DEFAULT_EXCLUDED_PREFIXES: &[&str] = &[
    "agent::",
    "engine::",
    "state::",
    "stream::",
    "iii.",
];

pub async fn discover_tools(iii: &III) -> Vec<ToolDef> {
    discover_tools_with(iii, DEFAULT_EXCLUDED_PREFIXES).await
}

pub async fn discover_tools_with(iii: &III, excluded: &[&str]) -> Vec<ToolDef> {
    let functions = match iii.list_functions().await {
        Ok(fns) => fns,
        Err(e) => {
            tracing::warn!(error = %e, "failed to discover functions");
            return Vec::new();
        }
    };

    let tools: Vec<ToolDef> = functions
        .into_iter()
        .filter(|f| !f.function_id.is_empty())
        .filter(|f| !is_excluded(&f.function_id, excluded))
        .filter(|f| has_valid_schema(f))
        .map(|f| function_to_tool(&f))
        .filter(|t| !t.name.is_empty())
        .collect();

    tracing::info!(count = tools.len(), "discovered tools");
    tools
}

pub fn function_to_tool(f: &FunctionInfo) -> ToolDef {
    ToolDef {
        name: sanitize_tool_name(&f.function_id),
        description: f.description.clone().unwrap_or_default(),
        input_schema: f
            .request_format
            .clone()
            .unwrap_or(json!({"type": "object", "properties": {}})),
    }
}

pub fn tool_name_to_function_id(tool_name: &str) -> String {
    tool_name.replace("__", "::")
}

pub fn sanitize_tool_name(function_id: &str) -> String {
    let sanitized: String = function_id
        .replace("::", "__")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if sanitized.len() > 128 {
        sanitized[..128].to_string()
    } else {
        sanitized
    }
}

pub fn functions_to_tools(functions: &[FunctionInfo]) -> Vec<ToolDef> {
    functions
        .iter()
        .filter(|f| !is_excluded(&f.function_id, DEFAULT_EXCLUDED_PREFIXES))
        .map(|f| function_to_tool(f))
        .collect()
}

pub fn build_capabilities_summary(tools: &[ToolDef]) -> String {
    if tools.is_empty() {
        return "No external functions are currently available.".to_string();
    }

    let mut summary = String::from("Available functions:\n");
    for tool in tools {
        summary.push_str(&format!("- {}: {}\n", tool.name, tool.description));
    }
    summary
}

// Build the capabilities summary keyed by engine `function_id` (eval::metrics)
// rather than the Anthropic-sanitized tool name (eval__metrics). Use this
// when the model is asked to echo a function_id back — e.g., the planner
// fills `steps[*].function_id`, which downstream executors invoke as-is.
pub async fn build_planner_capabilities(iii: &III) -> String {
    let functions = match iii.list_functions().await {
        Ok(fns) => fns,
        Err(_) => return "No external functions are currently available.".to_string(),
    };

    let eligible: Vec<FunctionInfo> = functions
        .into_iter()
        .filter(|f| !f.function_id.is_empty())
        .filter(|f| !is_excluded(&f.function_id, DEFAULT_EXCLUDED_PREFIXES))
        .filter(has_valid_schema)
        .collect();

    if eligible.is_empty() {
        return "No external functions are currently available.".to_string();
    }

    let mut summary = String::from("Available functions (call these exact ids):\n");
    for f in eligible {
        let desc = f.description.as_deref().unwrap_or("");
        summary.push_str(&format!("- {}: {}\n", f.function_id, desc));
    }
    summary
}

fn is_excluded(function_id: &str, excluded: &[&str]) -> bool {
    excluded.iter().any(|prefix| function_id.starts_with(prefix))
}

fn has_valid_schema(f: &FunctionInfo) -> bool {
    match &f.request_format {
        Some(schema) => {
            schema.get("type").and_then(|t| t.as_str()) == Some("object")
        }
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_colons() {
        assert_eq!(sanitize_tool_name("eval::metrics"), "eval__metrics");
    }

    #[test]
    fn test_sanitize_dots() {
        assert_eq!(sanitize_tool_name("iii.on_functions.abc"), "iii_on_functions_abc");
    }

    #[test]
    fn test_sanitize_uuid() {
        let result = sanitize_tool_name("iii.callback.a1b2c3d4-e5f6");
        assert!(result.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    }

    #[test]
    fn test_sanitize_truncate() {
        let long = "a".repeat(200);
        assert_eq!(sanitize_tool_name(&long).len(), 128);
    }

    #[test]
    fn test_tool_name_to_function_id_roundtrip() {
        assert_eq!(tool_name_to_function_id("eval__metrics"), "eval::metrics");
    }

    #[test]
    fn test_worker_prefixes_not_excluded() {
        assert!(!is_excluded("eval::metrics", DEFAULT_EXCLUDED_PREFIXES));
        assert!(!is_excluded("introspect::topology", DEFAULT_EXCLUDED_PREFIXES));
        assert!(!is_excluded("sensor::scan", DEFAULT_EXCLUDED_PREFIXES));
        assert!(!is_excluded("guardrails::check_input", DEFAULT_EXCLUDED_PREFIXES));
        assert!(!is_excluded("coding::scaffold", DEFAULT_EXCLUDED_PREFIXES));
        assert!(!is_excluded("experiment::create", DEFAULT_EXCLUDED_PREFIXES));
        assert!(!is_excluded("publish", DEFAULT_EXCLUDED_PREFIXES));
    }

    #[test]
    fn test_infrastructure_prefixes_excluded() {
        assert!(is_excluded("state::get", DEFAULT_EXCLUDED_PREFIXES));
        assert!(is_excluded("engine::health", DEFAULT_EXCLUDED_PREFIXES));
        assert!(is_excluded("stream::set", DEFAULT_EXCLUDED_PREFIXES));
        assert!(is_excluded("agent::chat", DEFAULT_EXCLUDED_PREFIXES));
        assert!(is_excluded("iii.on_functions_available.abc", DEFAULT_EXCLUDED_PREFIXES));
    }

    #[test]
    fn test_has_valid_schema_with_object() {
        let f = FunctionInfo {
            function_id: "test".into(),
            description: None,
            request_format: Some(json!({"type": "object", "properties": {}})),
            response_format: None,
            metadata: None,
        };
        assert!(has_valid_schema(&f));
    }

    #[test]
    fn test_has_valid_schema_none() {
        let f = FunctionInfo {
            function_id: "test".into(),
            description: None,
            request_format: None,
            response_format: None,
            metadata: None,
        };
        assert!(has_valid_schema(&f));
    }

    #[test]
    fn test_has_invalid_schema() {
        let f = FunctionInfo {
            function_id: "test".into(),
            description: None,
            request_format: Some(json!({"type": "string"})),
            response_format: None,
            metadata: None,
        };
        assert!(!has_valid_schema(&f));
    }

    #[test]
    fn test_capabilities_summary_empty() {
        let result = build_capabilities_summary(&[]);
        assert!(result.contains("No external functions"));
    }

    #[test]
    fn test_capabilities_summary_with_tools() {
        let tools = vec![ToolDef {
            name: "eval__metrics".into(),
            description: "Calculate metrics".into(),
            input_schema: json!({}),
        }];
        let result = build_capabilities_summary(&tools);
        assert!(result.contains("eval__metrics"));
        assert!(result.contains("Calculate metrics"));
    }

    #[test]
    fn test_system_prompt_contains_rules() {
        let tools = vec![];
        let prompt = build_system_prompt(&tools);
        assert!(prompt.contains("plain text"));
        assert!(prompt.contains("markdown"));
        assert!(prompt.contains("Do NOT wrap"));
    }

    #[test]
    fn test_function_to_tool() {
        let f = FunctionInfo {
            function_id: "eval::metrics".into(),
            description: Some("Compute P50/P95/P99".into()),
            request_format: Some(json!({"type": "object", "properties": {"function_id": {"type": "string"}}})),
            response_format: None,
            metadata: None,
        };
        let tool = function_to_tool(&f);
        assert_eq!(tool.name, "eval__metrics");
        assert_eq!(tool.description, "Compute P50/P95/P99");
        assert!(tool.input_schema.get("properties").is_some());
    }
}

pub fn build_system_prompt(tools: &[ToolDef]) -> String {
    let capabilities = build_capabilities_summary(tools);

    format!(
        "You are the iii agent, an intelligent assistant for the iii engine.\n\
         \n\
         You have access to functions registered by connected workers. Use them to answer \
         questions about the system, analyze performance, and manage the engine.\n\
         \n\
         Rules:\n\
         - Call the available functions to gather real data before answering.\n\
         - Respond with plain text. Use markdown for formatting (tables, lists, code blocks).\n\
         - Be concise and data-driven.\n\
         - When showing data, use markdown tables.\n\
         - Do NOT wrap your response in JSON objects.\n\
         \n\
         {capabilities}"
    )
}
