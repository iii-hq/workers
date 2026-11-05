use crate::analyzer::CompletionContext;
use crate::engine_client::{EngineClient, ENGINE_SOURCES};
use crate::engine_introspection::FunctionInfo;
use std::sync::Arc;
use tower_lsp_server::ls_types::*;

/// Which functions a function-id position offers.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FunctionKind {
    /// Anything to call.
    Callable,
    /// Anything observable, engine sources included.
    Observable,
    /// Anything an observer may deliver to: not private.
    Target,
}

impl FunctionKind {
    fn admits(self, func: &FunctionInfo) -> bool {
        match self {
            FunctionKind::Callable => true,
            FunctionKind::Observable => func.observable,
            FunctionKind::Target => !func.private,
        }
    }
}

fn function_item(func: &FunctionInfo, kind: FunctionKind) -> CompletionItem {
    // An event source (a function observers pass args to, or that only its
    // worker calls) reads as an event when it is being observed.
    let event = kind == FunctionKind::Observable && (func.args_format.is_some() || func.private);
    CompletionItem {
        label: func.function_id.clone(),
        kind: Some(if event {
            CompletionItemKind::EVENT
        } else {
            CompletionItemKind::FUNCTION
        }),
        detail: func.description.clone(),
        insert_text: Some(func.function_id.clone()),
        ..Default::default()
    }
}

fn engine_source_item(id: &str, description: &str) -> CompletionItem {
    CompletionItem {
        label: id.to_string(),
        kind: Some(CompletionItemKind::EVENT),
        detail: Some(description.to_string()),
        insert_text: Some(id.to_string()),
        ..Default::default()
    }
}

/// The functions `kind` admits whose id starts with the namespace typed in
/// `current_text` (`todos::` → `todos::*`).
fn function_items(
    current_text: &str,
    kind: FunctionKind,
    engine: &Arc<EngineClient>,
) -> Vec<CompletionItem> {
    let namespace_filter = if current_text.contains("::") {
        current_text.split("::").next()
    } else {
        None
    };
    let in_namespace =
        |id: &str| namespace_filter.is_none_or(|ns| id.starts_with(&format!("{ns}::")));

    let mut items = Vec::new();
    for entry in engine.functions.iter() {
        let func: &FunctionInfo = entry.value();
        if in_namespace(&func.function_id) && kind.admits(func) {
            items.push(function_item(func, kind));
        }
    }
    if kind == FunctionKind::Observable {
        for &(id, description) in ENGINE_SOURCES {
            if in_namespace(id) && !engine.functions.contains_key(id) {
                items.push(engine_source_item(id, description));
            }
        }
    }
    items
}

fn value_items(values: Vec<String>, detail: &str) -> Vec<CompletionItem> {
    values
        .into_iter()
        .map(|value| CompletionItem {
            label: value.clone(),
            kind: Some(CompletionItemKind::VALUE),
            detail: Some(detail.to_string()),
            insert_text: Some(value),
            ..Default::default()
        })
        .collect()
}

pub fn get_completions(
    context: &CompletionContext,
    current_text: &str,
    engine: &Arc<EngineClient>,
) -> Vec<CompletionItem> {
    match context {
        CompletionContext::FunctionId => {
            function_items(current_text, FunctionKind::Callable, engine)
        }

        CompletionContext::ObservableFunctionId => {
            function_items(current_text, FunctionKind::Observable, engine)
        }

        CompletionContext::TargetFunctionId => {
            function_items(current_text, FunctionKind::Target, engine)
        }

        CompletionContext::PayloadProperty { function_id } => {
            if let Some(func) = engine.get_function(function_id) {
                if let Some(schema) = &func.request_format {
                    return extract_properties_from_schema(schema);
                }
            }
            Vec::new()
        }

        CompletionContext::ArgsProperty { function_id } => {
            if let Some(func) = engine.get_function(function_id) {
                if let Some(schema) = &func.args_format {
                    return extract_properties_from_schema(schema);
                }
            }
            Vec::new()
        }

        CompletionContext::ArgValue { function_id, arg } => {
            let mut values = engine
                .get_function(function_id)
                .and_then(|func| func.args_format)
                .map(|schema| enum_values(&schema, arg))
                .unwrap_or_default();
            for value in engine.get_arg_values(function_id, arg) {
                if !values.contains(&value) {
                    values.push(value);
                }
            }
            if values.is_empty() {
                values = engine.get_known_values(arg);
            }
            value_items(values, arg)
        }

        CompletionContext::KnownValue { field_name } => {
            value_items(engine.get_known_values(field_name), field_name)
        }

        CompletionContext::None => Vec::new(),
    }
}

/// The string `enum` values a JSON Schema allows for `property`.
fn enum_values(schema: &serde_json::Value, property: &str) -> Vec<String> {
    schema
        .get("properties")
        .and_then(|p| p.get(property))
        .and_then(|p| p.get("enum"))
        .and_then(|e| e.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
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
            let type_str = prop.get("type").and_then(|t| t.as_str()).unwrap_or("any");
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
                documentation: description.map(|d| Documentation::String(d.to_string())),
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

    fn function(id: &str) -> FunctionInfo {
        FunctionInfo {
            function_id: id.to_string(),
            ..Default::default()
        }
    }

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
                ..Default::default()
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
                ..Default::default()
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
    fn private_functions_are_observable_but_not_targets() {
        let source = FunctionInfo {
            private: true,
            ..function("cron::on-tick")
        };
        assert!(FunctionKind::Callable.admits(&source));
        assert!(FunctionKind::Observable.admits(&source));
        assert!(!FunctionKind::Target.admits(&source));
        assert_eq!(
            function_item(&source, FunctionKind::Observable).kind,
            Some(CompletionItemKind::EVENT)
        );
        assert_eq!(
            function_item(&source, FunctionKind::Callable).kind,
            Some(CompletionItemKind::FUNCTION)
        );
    }

    #[test]
    fn unobservable_functions_are_not_observe_sources() {
        let secret = FunctionInfo {
            observable: false,
            ..function("secrets::resolve")
        };
        assert!(!FunctionKind::Observable.admits(&secret));
        assert!(FunctionKind::Target.admits(&secret));
        assert_eq!(
            function_item(&function("orders::create"), FunctionKind::Observable).kind,
            Some(CompletionItemKind::FUNCTION)
        );
    }

    #[test]
    fn engine_source_items_are_events() {
        let item = engine_source_item(
            "configuration::on-updated",
            "A configuration entry changed.",
        );
        assert_eq!(item.kind, Some(CompletionItemKind::EVENT));
        assert_eq!(
            item.insert_text.as_deref(),
            Some("configuration::on-updated")
        );
    }

    #[test]
    fn enum_values_of_an_args_property() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["script", "style", "module"] },
                "path": { "type": "string" }
            }
        });
        assert_eq!(
            enum_values(&schema, "kind"),
            vec!["script", "style", "module"]
        );
        assert!(enum_values(&schema, "path").is_empty());
        assert!(enum_values(&schema, "missing").is_empty());
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
