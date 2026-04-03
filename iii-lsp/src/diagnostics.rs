use crate::engine_client::EngineClient;
use std::sync::Arc;
use tower_lsp_server::ls_types::*;
use tree_sitter::{Node, Parser};

/// Information extracted from a single trigger() call in the AST.
struct TriggerCall {
    function_id: String,
    function_id_range: Range,
    payload_keys: Vec<String>,
    payload_range: Range,
    has_payload: bool,
}

/// Analyze a document and return diagnostics for all trigger() calls.
pub fn diagnose(source: &str, engine: &Arc<EngineClient>) -> Vec<Diagnostic> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .is_err()
    {
        return Vec::new();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let calls = find_trigger_calls(tree.root_node(), source);
    let mut diagnostics = Vec::new();

    for call in &calls {
        check_trigger_call(call, engine, &mut diagnostics);
    }

    diagnostics
}

/// Walk the AST and collect all trigger() call sites.
fn find_trigger_calls(root: Node, source: &str) -> Vec<TriggerCall> {
    let mut calls = Vec::new();
    walk_tree(root, source, &mut calls);
    calls
}

fn walk_tree(node: Node, source: &str, calls: &mut Vec<TriggerCall>) {
    if node.kind() == "call_expression" {
        if let Some(info) = extract_trigger_call(node, source) {
            calls.push(info);
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_tree(child, source, calls);
    }
}

/// Extract trigger call info from a call_expression node.
fn extract_trigger_call(call: Node, source: &str) -> Option<TriggerCall> {
    // Check method name is "trigger"
    let func = call.child_by_field_name("function")?;
    if func.kind() != "member_expression" {
        return None;
    }
    let property = func.child_by_field_name("property")?;
    let method = property.utf8_text(source.as_bytes()).ok()?;
    if method != "trigger" {
        return None;
    }

    // Get the arguments object
    let arguments = call.child_by_field_name("arguments")?;
    let args_object = find_child_by_kind(arguments, "object")?;

    // Find function_id pair
    let (function_id, function_id_range) = find_string_pair(args_object, "function_id", source)?;

    // Find payload pair (optional)
    let (has_payload, payload_keys, payload_range) =
        if let Some(payload_info) = find_payload_pair(args_object, source) {
            (true, payload_info.0, payload_info.1)
        } else {
            (false, Vec::new(), to_range(args_object))
        };

    Some(TriggerCall {
        function_id,
        function_id_range,
        payload_keys,
        payload_range,
        has_payload,
    })
}

/// Find a pair with the given key name and extract its string value and range.
fn find_string_pair(object: Node, key_name: &str, source: &str) -> Option<(String, Range)> {
    let mut cursor = object.walk();
    for child in object.children(&mut cursor) {
        if child.kind() == "pair" {
            if let Some(key) = child.child_by_field_name("key") {
                if key.utf8_text(source.as_bytes()).ok()? == key_name {
                    if let Some(value) = child.child_by_field_name("value") {
                        if value.kind() == "string" {
                            let text = extract_string_content(value, source);
                            return Some((text, to_range(value)));
                        }
                    }
                }
            }
        }
    }
    None
}

/// Find the payload/data pair and extract its property keys and range.
fn find_payload_pair(object: Node, source: &str) -> Option<(Vec<String>, Range)> {
    let mut cursor = object.walk();
    for child in object.children(&mut cursor) {
        if child.kind() == "pair" {
            if let Some(key) = child.child_by_field_name("key") {
                let key_text = key.utf8_text(source.as_bytes()).ok()?;
                if key_text == "payload" || key_text == "data" {
                    if let Some(value) = child.child_by_field_name("value") {
                        if value.kind() == "object" {
                            let keys = extract_object_keys(value, source);
                            return Some((keys, to_range(value)));
                        }
                    }
                }
            }
        }
    }
    None
}

/// Extract all property key names from an object node.
fn extract_object_keys(object: Node, source: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut cursor = object.walk();
    for child in object.children(&mut cursor) {
        if child.kind() == "pair" {
            if let Some(key) = child.child_by_field_name("key") {
                if let Ok(text) = key.utf8_text(source.as_bytes()) {
                    keys.push(text.to_string());
                }
            }
        }
    }
    keys
}

fn extract_string_content(string_node: Node, source: &str) -> String {
    let mut cursor = string_node.walk();
    for child in string_node.children(&mut cursor) {
        if child.kind() == "string_fragment" {
            return child
                .utf8_text(source.as_bytes())
                .unwrap_or("")
                .to_string();
        }
    }
    String::new()
}

fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn to_range(node: Node) -> Range {
    let start = node.start_position();
    let end = node.end_position();
    Range {
        start: Position {
            line: start.row as u32,
            character: start.column as u32,
        },
        end: Position {
            line: end.row as u32,
            character: end.column as u32,
        },
    }
}

/// Check a trigger call against the engine cache and emit diagnostics.
fn check_trigger_call(call: &TriggerCall, engine: &Arc<EngineClient>, diagnostics: &mut Vec<Diagnostic>) {
    // Skip engine-internal functions
    if call.function_id.starts_with("engine::") {
        return;
    }

    let func = match engine.get_function(&call.function_id) {
        Some(f) => f,
        None => {
            if !call.function_id.is_empty() {
                diagnostics.push(Diagnostic {
                    range: call.function_id_range,
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some("iii-lsp".to_string()),
                    message: format!("Unknown function '{}'", call.function_id),
                    ..Default::default()
                });
            }
            return;
        }
    };

    // Check required payload properties
    if !call.has_payload {
        return;
    }

    let schema = match &func.request_format {
        Some(s) => s,
        None => return,
    };

    let required = match schema.get("required").and_then(|r| r.as_array()) {
        Some(r) => r,
        None => return,
    };

    for req in required {
        if let Some(name) = req.as_str() {
            if !call.payload_keys.contains(&name.to_string()) {
                diagnostics.push(Diagnostic {
                    range: call.payload_range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("iii-lsp".to_string()),
                    message: format!("Missing required property '{}' for '{}'", name, call.function_id),
                    ..Default::default()
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine_client::EngineClient;
    use dashmap::DashMap;
    use iii_sdk::FunctionInfo;

    fn make_function(id: &str, required: &[&str]) -> FunctionInfo {
        let schema = if required.is_empty() {
            None
        } else {
            Some(serde_json::json!({
                "type": "object",
                "properties": required.iter().map(|r| (r.to_string(), serde_json::json!({"type": "string"}))).collect::<serde_json::Map<String, serde_json::Value>>(),
                "required": required
            }))
        };
        FunctionInfo {
            function_id: id.to_string(),
            description: None,
            request_format: schema,
            response_format: None,
            metadata: None,
        }
    }

    // Test the tree-walking logic directly since we can't easily construct an EngineClient
    #[test]
    fn finds_trigger_calls() {
        let source = "iii.trigger({ function_id: 'todos::create', payload: { title: 'test' } })";
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let calls = find_trigger_calls(tree.root_node(), source);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "todos::create");
        assert!(calls[0].has_payload);
        assert_eq!(calls[0].payload_keys, vec!["title"]);
    }

    #[test]
    fn finds_multiple_trigger_calls() {
        let source = "iii.trigger({ function_id: 'a' })\niii.trigger({ function_id: 'b' })";
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let calls = find_trigger_calls(tree.root_node(), source);

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function_id, "a");
        assert_eq!(calls[1].function_id, "b");
    }

    #[test]
    fn detects_missing_required_property() {
        let functions: DashMap<String, FunctionInfo> = DashMap::new();
        functions.insert(
            "todos::create".to_string(),
            make_function("todos::create", &["title", "body"]),
        );

        let call = TriggerCall {
            function_id: "todos::create".to_string(),
            function_id_range: Range::default(),
            payload_keys: vec!["title".to_string()],
            payload_range: Range::default(),
            has_payload: true,
        };

        // Simulate check_trigger_call logic inline
        let func = functions.get("todos::create").unwrap();
        let schema = func.request_format.as_ref().unwrap();
        let required = schema
            .get("required")
            .and_then(|r| r.as_array())
            .unwrap();

        let missing: Vec<&str> = required
            .iter()
            .filter_map(|r| r.as_str())
            .filter(|name| !call.payload_keys.contains(&name.to_string()))
            .collect();

        assert_eq!(missing, vec!["body"]);
    }

    #[test]
    fn no_error_when_all_required_present() {
        let call = TriggerCall {
            function_id: "todos::create".to_string(),
            function_id_range: Range::default(),
            payload_keys: vec!["title".to_string(), "body".to_string()],
            payload_range: Range::default(),
            has_payload: true,
        };

        let required = vec!["title", "body"];
        let missing: Vec<&&str> = required
            .iter()
            .filter(|name| !call.payload_keys.contains(&name.to_string()))
            .collect();

        assert!(missing.is_empty());
    }

    #[test]
    fn ignores_non_trigger_calls() {
        let source = "foo.bar({ function_id: 'test' })";
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let calls = find_trigger_calls(tree.root_node(), source);

        assert!(calls.is_empty());
    }

    #[test]
    fn extracts_payload_keys() {
        let source =
            "iii.trigger({ function_id: 'x', payload: { name: 'a', age: 5, active: true } })";
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let calls = find_trigger_calls(tree.root_node(), source);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].payload_keys, vec!["name", "age", "active"]);
    }
}
