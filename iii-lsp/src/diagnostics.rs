use crate::analyzer::Language;
use crate::engine_client::EngineClient;
use std::sync::Arc;
use tower_lsp_server::ls_types::*;
use tree_sitter::{Node, Parser};

struct TriggerCall {
    function_id: String,
    function_id_range: Range,
    payload_keys: Vec<String>,
    payload_range: Range,
    has_payload: bool,
}

struct RegisterTriggerCall {
    trigger_type: String,
    trigger_type_range: Range,
    function_id: String,
    function_id_range: Range,
    config_keys: Vec<String>,
    config_values: Vec<(String, String)>,
    config_range: Range,
    has_config: bool,
}

pub fn diagnose(source: &str, engine: &Arc<EngineClient>, language: Language) -> Vec<Diagnostic> {
    let mut parser = Parser::new();
    let lang_ok = match language {
        Language::TypeScript => {
            parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        }
        Language::Python => parser.set_language(&tree_sitter_python::LANGUAGE.into()),
        Language::Rust => parser.set_language(&tree_sitter_rust::LANGUAGE.into()),
    };
    if lang_ok.is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let (trigger_calls, register_calls) = find_all_calls(tree.root_node(), source);
    let mut diagnostics = Vec::new();

    for call in &trigger_calls {
        check_trigger_call(call, engine, &mut diagnostics);
    }
    for call in &register_calls {
        check_register_trigger_call(call, engine, &mut diagnostics);
    }

    diagnostics
}

// --- AST walking ---

fn find_all_calls(root: Node, source: &str) -> (Vec<TriggerCall>, Vec<RegisterTriggerCall>) {
    let mut trigger_calls = Vec::new();
    let mut register_calls = Vec::new();
    walk_tree(root, source, &mut trigger_calls, &mut register_calls);
    (trigger_calls, register_calls)
}

fn walk_tree(
    node: Node,
    source: &str,
    trigger_calls: &mut Vec<TriggerCall>,
    register_calls: &mut Vec<RegisterTriggerCall>,
) {
    if is_call(node.kind()) {
        if let Some(info) = extract_trigger_call(node, source) {
            trigger_calls.push(info);
        } else if let Some(info) = extract_register_trigger_call(node, source) {
            register_calls.push(info);
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_tree(child, source, trigger_calls, register_calls);
    }
}

// --- Node kind helpers ---

fn is_call(kind: &str) -> bool {
    kind == "call_expression" || kind == "call"
}

fn is_method_access(kind: &str) -> bool {
    kind == "member_expression" || kind == "attribute" || kind == "field_expression"
}

fn is_object(kind: &str) -> bool {
    kind == "object" || kind == "dictionary"
}

fn is_arguments(kind: &str) -> bool {
    kind == "arguments" || kind == "argument_list"
}

fn strip_quotes(s: &str) -> String {
    s.trim_matches('\'').trim_matches('"').to_string()
}

// --- Method name extraction ---

fn extract_method_name(call: Node, source: &str) -> Option<String> {
    let func = call.child_by_field_name("function")?;
    if is_method_access(func.kind()) {
        let field = match func.kind() {
            "member_expression" => "property",
            "field_expression" => "field",
            _ => "attribute",
        };
        let prop = func.child_by_field_name(field)?;
        return Some(prop.utf8_text(source.as_bytes()).ok()?.to_string());
    }
    if func.kind() == "identifier" {
        return Some(func.utf8_text(source.as_bytes()).ok()?.to_string());
    }
    None
}

fn is_trigger_method(name: &str) -> bool {
    name == "trigger"
}

fn is_register_trigger_method(name: &str) -> bool {
    name == "registerTrigger" || name == "register_trigger"
}

// --- Call extraction ---

fn extract_trigger_call(call: Node, source: &str) -> Option<TriggerCall> {
    let method = extract_method_name(call, source)?;
    if !is_trigger_method(&method) {
        return None;
    }

    let arguments = find_arguments(call)?;

    // Try dict-style: first child that's an object/dictionary
    if let Some(args_object) = find_child_object(arguments) {
        let (function_id, function_id_range) =
            find_string_pair(args_object, "function_id", source)?;
        let (has_payload, payload_keys, payload_range) =
            if let Some(info) = find_object_pair(args_object, &["payload", "data"], source) {
                (true, info.0, info.1)
            } else {
                (false, Vec::new(), to_range(args_object))
            };
        return Some(TriggerCall {
            function_id,
            function_id_range,
            payload_keys,
            payload_range,
            has_payload,
        });
    }

    // Try keyword-argument style (Python)
    let (function_id, function_id_range) = find_kwarg_string(arguments, "function_id", source)?;
    let (has_payload, payload_keys, payload_range) =
        if let Some(info) = find_kwarg_object(arguments, &["payload", "data"], source) {
            (true, info.0, info.1)
        } else {
            (false, Vec::new(), to_range(arguments))
        };
    Some(TriggerCall {
        function_id,
        function_id_range,
        payload_keys,
        payload_range,
        has_payload,
    })
}

fn extract_register_trigger_call(call: Node, source: &str) -> Option<RegisterTriggerCall> {
    let method = extract_method_name(call, source)?;
    if !is_register_trigger_method(&method) {
        return None;
    }

    let arguments = find_arguments(call)?;

    // Try dict-style
    if let Some(args_object) = find_child_object(arguments) {
        let (trigger_type, trigger_type_range) =
            find_string_pair(args_object, "type", source)?;
        let (function_id, function_id_range) = find_string_pair(args_object, "function_id", source)
            .unwrap_or((String::new(), to_range(args_object)));
        let (has_config, config_keys, config_values, config_range) =
            if let Some(info) = find_config_object_pair(args_object, source) {
                (true, info.0, info.1, info.2)
            } else {
                (false, Vec::new(), Vec::new(), to_range(args_object))
            };
        return Some(RegisterTriggerCall {
            trigger_type,
            trigger_type_range,
            function_id,
            function_id_range,
            config_keys,
            config_values,
            config_range,
            has_config,
        });
    }

    // Try keyword-argument style
    let (trigger_type, trigger_type_range) = find_kwarg_string(arguments, "type", source)?;
    let (function_id, function_id_range) = find_kwarg_string(arguments, "function_id", source)
        .unwrap_or((String::new(), to_range(arguments)));
    let (has_config, config_keys, config_values, config_range) =
        if let Some(info) = find_kwarg_config_object(arguments, source) {
            (true, info.0, info.1, info.2)
        } else {
            (false, Vec::new(), Vec::new(), to_range(arguments))
        };
    Some(RegisterTriggerCall {
        trigger_type,
        trigger_type_range,
        function_id,
        function_id_range,
        config_keys,
        config_values,
        config_range,
        has_config,
    })
}

// --- Helpers: find in arguments ---

fn find_arguments<'a>(call: Node<'a>) -> Option<Node<'a>> {
    let mut cursor = call.walk();
    for child in call.children(&mut cursor) {
        if is_arguments(child.kind()) {
            return Some(child);
        }
    }
    None
}

fn find_child_object<'a>(parent: Node<'a>) -> Option<Node<'a>> {
    let mut cursor = parent.walk();
    for child in parent.children(&mut cursor) {
        if is_object(child.kind()) || child.kind() == "struct_expression" {
            return Some(child);
        }
    }
    None
}

// --- Helpers: dict/object pair extraction ---

fn find_string_pair(object: Node, key_name: &str, source: &str) -> Option<(String, Range)> {
    // For struct_expression, look inside field_initializer_list
    let container = if object.kind() == "struct_expression" {
        object.child_by_field_name("body")?
    } else {
        object
    };

    let mut cursor = container.walk();
    for child in container.children(&mut cursor) {
        let (field_name, value_node) = if child.kind() == "pair" {
            // TS/Python: pair with key/value fields
            let key = child.child_by_field_name("key")?;
            let key_text = strip_quotes(key.utf8_text(source.as_bytes()).ok()?);
            let value = child.child_by_field_name("value")?;
            (key_text, value)
        } else if child.kind() == "field_initializer" {
            // Rust: field_initializer with field/value fields
            let field = child.child_by_field_name("field")?;
            let field_text = field.utf8_text(source.as_bytes()).ok()?.to_string();
            let value = child.child_by_field_name("value")?;
            (field_text, value)
        } else {
            continue;
        };

        if field_name == key_name {
            // Direct string
            if value_node.kind() == "string" || value_node.kind() == "string_literal" {
                let text = extract_string_content(value_node, source);
                return Some((text, to_range(value_node)));
            }
            // Rust: "x".to_string() or "x".into() — find string_literal in subtree
            if let Some(s) = find_string_in_subtree(value_node, source) {
                return Some(s);
            }
        }
    }
    None
}

/// Find a string_literal node anywhere in a subtree (handles "x".to_string()).
fn find_string_in_subtree(node: Node, source: &str) -> Option<(String, Range)> {
    if node.kind() == "string_literal" || node.kind() == "string" {
        return Some((extract_string_content(node, source), to_range(node)));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(result) = find_string_in_subtree(child, source) {
            return Some(result);
        }
    }
    None
}

fn find_object_pair(
    object: Node,
    key_names: &[&str],
    source: &str,
) -> Option<(Vec<String>, Range)> {
    let mut cursor = object.walk();
    for child in object.children(&mut cursor) {
        if child.kind() == "pair" {
            if let Some(key) = child.child_by_field_name("key") {
                let key_text = strip_quotes(key.utf8_text(source.as_bytes()).ok()?);
                if key_names.contains(&key_text.as_str()) {
                    if let Some(value) = child.child_by_field_name("value") {
                        if is_object(value.kind()) {
                            return Some((extract_object_keys(value, source), to_range(value)));
                        }
                    }
                }
            }
        }
    }
    None
}

fn find_config_object_pair(
    object: Node,
    source: &str,
) -> Option<(Vec<String>, Vec<(String, String)>, Range)> {
    let mut cursor = object.walk();
    for child in object.children(&mut cursor) {
        if child.kind() == "pair" {
            if let Some(key) = child.child_by_field_name("key") {
                let key_text = strip_quotes(key.utf8_text(source.as_bytes()).ok()?);
                if key_text == "config" {
                    if let Some(value) = child.child_by_field_name("value") {
                        if is_object(value.kind()) {
                            let keys = extract_object_keys(value, source);
                            let kvs = extract_object_string_values(value, source);
                            return Some((keys, kvs, to_range(value)));
                        }
                    }
                }
            }
        }
    }
    None
}

// --- Helpers: keyword argument extraction (Python) ---

fn find_kwarg_string(arg_list: Node, name: &str, source: &str) -> Option<(String, Range)> {
    let mut cursor = arg_list.walk();
    for child in arg_list.children(&mut cursor) {
        if child.kind() == "keyword_argument" {
            if let Some(n) = child.child_by_field_name("name") {
                if n.utf8_text(source.as_bytes()).ok()? == name {
                    if let Some(value) = child.child_by_field_name("value") {
                        if value.kind() == "string" {
                            return Some((extract_string_content(value, source), to_range(value)));
                        }
                    }
                }
            }
        }
    }
    None
}

fn find_kwarg_object(
    arg_list: Node,
    names: &[&str],
    source: &str,
) -> Option<(Vec<String>, Range)> {
    let mut cursor = arg_list.walk();
    for child in arg_list.children(&mut cursor) {
        if child.kind() == "keyword_argument" {
            if let Some(n) = child.child_by_field_name("name") {
                let arg_name = n.utf8_text(source.as_bytes()).ok()?;
                if names.contains(&arg_name) {
                    if let Some(value) = child.child_by_field_name("value") {
                        if is_object(value.kind()) {
                            return Some((extract_object_keys(value, source), to_range(value)));
                        }
                    }
                }
            }
        }
    }
    None
}

fn find_kwarg_config_object(
    arg_list: Node,
    source: &str,
) -> Option<(Vec<String>, Vec<(String, String)>, Range)> {
    let mut cursor = arg_list.walk();
    for child in arg_list.children(&mut cursor) {
        if child.kind() == "keyword_argument" {
            if let Some(n) = child.child_by_field_name("name") {
                if n.utf8_text(source.as_bytes()).ok()? == "config" {
                    if let Some(value) = child.child_by_field_name("value") {
                        if is_object(value.kind()) {
                            let keys = extract_object_keys(value, source);
                            let kvs = extract_object_string_values(value, source);
                            return Some((keys, kvs, to_range(value)));
                        }
                    }
                }
            }
        }
    }
    None
}

// --- Common helpers ---

fn extract_object_keys(object: Node, source: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut cursor = object.walk();
    for child in object.children(&mut cursor) {
        if child.kind() == "pair" {
            if let Some(key) = child.child_by_field_name("key") {
                if let Ok(text) = key.utf8_text(source.as_bytes()) {
                    keys.push(strip_quotes(text));
                }
            }
        }
    }
    keys
}

fn extract_object_string_values(object: Node, source: &str) -> Vec<(String, String)> {
    let mut kvs = Vec::new();
    let mut cursor = object.walk();
    for child in object.children(&mut cursor) {
        if child.kind() == "pair" {
            if let Some(key) = child.child_by_field_name("key") {
                if let Some(value) = child.child_by_field_name("value") {
                    if let (Ok(k), Ok(v)) = (
                        key.utf8_text(source.as_bytes()),
                        value.utf8_text(source.as_bytes()),
                    ) {
                        kvs.push((strip_quotes(k), strip_quotes(v)));
                    }
                }
            }
        }
    }
    kvs
}

fn extract_string_content(string_node: Node, source: &str) -> String {
    let mut cursor = string_node.walk();
    for child in string_node.children(&mut cursor) {
        let k = child.kind();
        if k == "string_fragment" || k == "string_content" {
            return child
                .utf8_text(source.as_bytes())
                .unwrap_or("")
                .to_string();
        }
    }
    String::new()
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

// --- Diagnostic checks ---

fn check_trigger_call(
    call: &TriggerCall,
    engine: &Arc<EngineClient>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if call.function_id.starts_with("engine::") {
        return;
    }

    if !call.function_id.is_empty() && !call.function_id.contains("::") {
        diagnostics.push(Diagnostic {
            range: call.function_id_range,
            severity: Some(DiagnosticSeverity::HINT),
            source: Some("iii-lsp".to_string()),
            message: format!(
                "Function ID '{}' should use namespace format 'namespace::name'",
                call.function_id
            ),
            ..Default::default()
        });
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

    if !call.has_payload {
        return;
    }

    let schema = match &func.request_format {
        Some(s) => s,
        None => return,
    };

    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        for req in required {
            if let Some(name) = req.as_str() {
                if !call.payload_keys.contains(&name.to_string()) {
                    diagnostics.push(Diagnostic {
                        range: call.payload_range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("iii-lsp".to_string()),
                        message: format!(
                            "Missing required property '{}' for '{}'",
                            name, call.function_id
                        ),
                        ..Default::default()
                    });
                }
            }
        }
    }
}

const VALID_HTTP_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

fn check_register_trigger_call(
    call: &RegisterTriggerCall,
    engine: &Arc<EngineClient>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !call.function_id.is_empty() && !call.function_id.contains("::") {
        diagnostics.push(Diagnostic {
            range: call.function_id_range,
            severity: Some(DiagnosticSeverity::HINT),
            source: Some("iii-lsp".to_string()),
            message: format!(
                "Function ID '{}' should use namespace format 'namespace::name'",
                call.function_id
            ),
            ..Default::default()
        });
    }

    if !call.function_id.is_empty() && engine.get_function(&call.function_id).is_none() {
        diagnostics.push(Diagnostic {
            range: call.function_id_range,
            severity: Some(DiagnosticSeverity::WARNING),
            source: Some("iii-lsp".to_string()),
            message: format!("Unknown function '{}'", call.function_id),
            ..Default::default()
        });
    }

    if engine.get_trigger_type(&call.trigger_type).is_none() && !call.trigger_type.is_empty() {
        diagnostics.push(Diagnostic {
            range: call.trigger_type_range,
            severity: Some(DiagnosticSeverity::WARNING),
            source: Some("iii-lsp".to_string()),
            message: format!("Unknown trigger type '{}'", call.trigger_type),
            ..Default::default()
        });
        return;
    }

    if !call.has_config {
        return;
    }

    if let Some(tt) = engine.get_trigger_type(&call.trigger_type) {
        if let Some(schema) = &tt.trigger_request_format {
            if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
                for req in required {
                    if let Some(name) = req.as_str() {
                        if !call.config_keys.contains(&name.to_string()) {
                            diagnostics.push(Diagnostic {
                                range: call.config_range,
                                severity: Some(DiagnosticSeverity::ERROR),
                                source: Some("iii-lsp".to_string()),
                                message: format!(
                                    "Missing required config property '{}' for trigger type '{}'",
                                    name, call.trigger_type
                                ),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }
    }

    if call.trigger_type == "http" {
        for (key, value) in &call.config_values {
            if key == "http_method" && !VALID_HTTP_METHODS.contains(&value.as_str()) {
                diagnostics.push(Diagnostic {
                    range: call.config_range,
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some("iii-lsp".to_string()),
                    message: format!(
                        "Invalid HTTP method '{}'. Expected one of: {}",
                        value,
                        VALID_HTTP_METHODS.join(", ")
                    ),
                    ..Default::default()
                });
            }
        }
    }

    if call.trigger_type == "cron" {
        for (key, value) in &call.config_values {
            if key == "expression" {
                let fields: Vec<&str> = value.split_whitespace().collect();
                if fields.len() != 6 {
                    diagnostics.push(Diagnostic {
                        range: call.config_range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("iii-lsp".to_string()),
                        message: format!(
                            "Cron expression must have 6 fields (sec min hour day month weekday), got {}",
                            fields.len()
                        ),
                        ..Default::default()
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iii_sdk::FunctionInfo;

    fn parse_ts(source: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    fn parse_py(source: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    // --- TypeScript tests ---

    #[test]
    fn ts_finds_trigger_calls() {
        let source = "iii.trigger({ function_id: 'todos::create', payload: { title: 'test' } })";
        let tree = parse_ts(source);
        let (calls, _) = find_all_calls(tree.root_node(), source);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "todos::create");
        assert!(calls[0].has_payload);
    }

    #[test]
    fn ts_finds_register_trigger_calls() {
        let source =
            "iii.registerTrigger({ type: 'http', function_id: 'x', config: { api_path: '/test' } })";
        let tree = parse_ts(source);
        let (_, reg) = find_all_calls(tree.root_node(), source);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg[0].trigger_type, "http");
        assert_eq!(reg[0].config_keys, vec!["api_path"]);
    }

    // --- Python dict-style tests ---

    #[test]
    fn py_finds_trigger_calls_dict() {
        let source = "iii.trigger({'function_id': 'todos::create', 'payload': {'title': 'test'}})";
        let tree = parse_py(source);
        let (calls, _) = find_all_calls(tree.root_node(), source);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "todos::create");
        assert!(calls[0].has_payload);
        assert_eq!(calls[0].payload_keys, vec!["title"]);
    }

    #[test]
    fn py_finds_register_trigger_dict() {
        let source = "iii.register_trigger({'type': 'http', 'function_id': 'x', 'config': {'api_path': '/test'}})";
        let tree = parse_py(source);
        let (_, reg) = find_all_calls(tree.root_node(), source);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg[0].trigger_type, "http");
        assert_eq!(reg[0].config_keys, vec!["api_path"]);
    }

    // --- Python keyword argument tests ---

    #[test]
    fn py_finds_trigger_calls_kwarg() {
        let source = "iii.trigger(function_id='todos::create', payload={'title': 'test'})";
        let tree = parse_py(source);
        let (calls, _) = find_all_calls(tree.root_node(), source);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "todos::create");
        assert!(calls[0].has_payload);
        assert_eq!(calls[0].payload_keys, vec!["title"]);
    }

    #[test]
    fn py_finds_register_trigger_kwarg() {
        let source =
            "iii.register_trigger(type='http', function_id='x', config={'api_path': '/test'})";
        let tree = parse_py(source);
        let (_, reg) = find_all_calls(tree.root_node(), source);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg[0].trigger_type, "http");
        assert_eq!(reg[0].config_keys, vec!["api_path"]);
    }

    #[test]
    fn py_ignores_non_trigger_calls() {
        let source = "foo.bar(function_id='test')";
        let tree = parse_py(source);
        let (calls, reg) = find_all_calls(tree.root_node(), source);
        assert!(calls.is_empty());
        assert!(reg.is_empty());
    }
}
