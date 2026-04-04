use crate::analyzer::CompletionContext;
use crate::engine_client::EngineClient;
use iii_sdk::{FunctionInfo, TriggerTypeInfo};
use std::sync::Arc;
use tower_lsp_server::ls_types::*;

pub fn get_completions(
    context: &CompletionContext,
    current_text: &str,
    engine: &Arc<EngineClient>,
) -> Vec<CompletionItem> {
    match context {
        CompletionContext::FunctionId => {
            // If user typed a namespace prefix like "todos::", filter to that namespace
            let namespace_filter = if current_text.contains("::") {
                current_text.split("::").next()
            } else {
                None
            };

            let mut items = Vec::new();
            for entry in engine.functions.iter() {
                let func: &FunctionInfo = entry.value();
                if let Some(ns) = namespace_filter {
                    if !func.function_id.starts_with(&format!("{}::", ns)) {
                        continue;
                    }
                }
                items.push(CompletionItem {
                    label: func.function_id.clone(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: func.description.clone(),
                    insert_text: Some(func.function_id.clone()),
                    ..Default::default()
                });
            }
            items
        }

        CompletionContext::TriggerType => {
            let mut items = Vec::new();
            for entry in engine.trigger_types.iter() {
                let tt: &TriggerTypeInfo = entry.value();
                items.push(CompletionItem {
                    label: tt.id.clone(),
                    kind: Some(CompletionItemKind::ENUM),
                    detail: Some(tt.description.clone()),
                    insert_text: Some(tt.id.clone()),
                    ..Default::default()
                });
            }
            items
        }

        CompletionContext::PayloadProperty { function_id } => {
            if let Some(func) = engine.get_function(function_id) {
                if let Some(schema) = &func.request_format {
                    return extract_properties_from_schema(schema);
                }
            }
            Vec::new()
        }

        CompletionContext::TriggerConfigProperty { trigger_type } => {
            if let Some(tt) = engine.get_trigger_type(trigger_type) {
                if let Some(schema) = &tt.trigger_request_format {
                    return extract_properties_from_schema(schema);
                }
            }
            Vec::new()
        }

        CompletionContext::None => Vec::new(),
    }
}

/// Extract property names from a JSON Schema and return them as completion items.
fn extract_properties_from_schema(schema: &serde_json::Value) -> Vec<CompletionItem> {
    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return Vec::new(),
    };

    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    properties
        .iter()
        .map(|(name, prop)| {
            let type_str = prop
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("any");
            let description = prop.get("description").and_then(|d| d.as_str());
            let is_required = required.contains(&name.as_str());

            let detail = if is_required {
                format!("{} (required)", type_str)
            } else {
                type_str.to_string()
            };

            CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::PROPERTY),
                detail: Some(detail),
                documentation: description
                    .map(|d| Documentation::String(d.to_string())),
                insert_text: Some(format!("{}: ", name)),
                ..Default::default()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashmap::DashMap;

    #[test]
    fn filters_engine_functions() {
        let functions: DashMap<String, FunctionInfo> = DashMap::new();
        functions.insert(
            "todos::create".to_string(),
            FunctionInfo {
                function_id: "todos::create".to_string(),
                description: Some("Create a todo".to_string()),
                request_format: None,
                response_format: None,
                metadata: None,
            },
        );
        functions.insert(
            "engine::functions::list".to_string(),
            FunctionInfo {
                function_id: "engine::functions::list".to_string(),
                description: Some("Internal".to_string()),
                request_format: None,
                response_format: None,
                metadata: None,
            },
        );

        let mut items = Vec::new();
        for entry in functions.iter() {
            let func: &FunctionInfo = entry.value();
            if !func.function_id.starts_with("engine::") {
                items.push(func.function_id.clone());
            }
        }

        assert_eq!(items.len(), 1);
        assert_eq!(items[0], "todos::create");
    }

    #[test]
    fn filters_engine_trigger_types() {
        let trigger_types: DashMap<String, TriggerTypeInfo> = DashMap::new();
        trigger_types.insert(
            "http".to_string(),
            TriggerTypeInfo {
                id: "http".to_string(),
                description: "HTTP trigger".to_string(),
                trigger_request_format: None,
                call_request_format: None,
            },
        );
        trigger_types.insert(
            "engine::functions-available".to_string(),
            TriggerTypeInfo {
                id: "engine::functions-available".to_string(),
                description: "Internal".to_string(),
                trigger_request_format: None,
                call_request_format: None,
            },
        );

        let mut items = Vec::new();
        for entry in trigger_types.iter() {
            let tt: &TriggerTypeInfo = entry.value();
            if !tt.id.starts_with("engine::") {
                items.push(tt.id.clone());
            }
        }

        assert_eq!(items.len(), 1);
        assert_eq!(items[0], "http");
    }

    #[test]
    fn none_context_returns_empty() {
        let items: Vec<CompletionItem> = Vec::new();
        assert!(items.is_empty());
    }

    #[test]
    fn extract_properties_from_json_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "The todo title" },
                "completed": { "type": "boolean" },
                "priority": { "type": "number" }
            },
            "required": ["title"]
        });

        let items = extract_properties_from_schema(&schema);
        assert_eq!(items.len(), 3);

        let title = items.iter().find(|i| i.label == "title").unwrap();
        assert_eq!(title.detail.as_deref(), Some("string (required)"));
        assert_eq!(title.insert_text.as_deref(), Some("title: "));
        assert_eq!(
            title.documentation,
            Some(Documentation::String("The todo title".to_string()))
        );

        let completed = items.iter().find(|i| i.label == "completed").unwrap();
        assert_eq!(completed.detail.as_deref(), Some("boolean"));
    }

    #[test]
    fn extract_properties_no_schema() {
        let schema = serde_json::json!({ "type": "string" });
        let items = extract_properties_from_schema(&schema);
        assert!(items.is_empty());
    }

    #[test]
    fn extract_properties_empty_object() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {}
        });
        let items = extract_properties_from_schema(&schema);
        assert!(items.is_empty());
    }
}
