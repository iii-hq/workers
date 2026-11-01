use crate::analyzer::{
    byte_col_to_utf16_col, extract_method_name, is_arguments, is_call, is_call_alias,
    is_call_method, is_object, is_observe_method, rust_constructor_context, strip_quotes,
    type_name, CompletionContext, Language,
};
use crate::engine_client::{engine_source, EngineClient};
use crate::engine_introspection::FunctionInfo;
use std::sync::Arc;
use tower_lsp_server::ls_types::*;
use tree_sitter::{Node, Parser};

type ObjectDetails = (Vec<String>, Vec<(String, String)>, Range);

/// A `call` / `call_async` (or its deprecated alias) naming a function.
struct CallSite {
    function_id: String,
    function_id_range: Range,
    payload_keys: Vec<String>,
    payload_range: Range,
    has_payload: bool,
    /// The deprecated alias the call is written with, and where.
    alias: Option<(String, Range)>,
}

/// An `observe` source's args, as far as the source shows them.
enum ObserveArgs {
    /// The source passes no args (the string shorthand, or an object without `args`).
    Absent,
    /// An object literal.
    Object {
        keys: Vec<String>,
        values: Vec<(String, String)>,
        range: Range,
    },
    /// Built in a way the server does not read (a variable, a Rust `json!`).
    Unknown,
}

/// An `observe(source, target, options?)` call.
struct ObserveSite {
    /// The observed function, and where it is named.
    source: Option<(String, Range)>,
    args: ObserveArgs,
    /// The function the observer delivers to, and where it is named.
    target: Option<(String, Range)>,
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

    let (calls, observes) = find_all_calls(tree.root_node(), source);
    let lookup = |id: &str| engine.get_function(id);
    let mut diagnostics = Vec::new();

    for call in &calls {
        check_call(call, &lookup, &mut diagnostics);
    }
    for site in &observes {
        check_observe(site, &lookup, &mut diagnostics);
    }

    diagnostics
}

// --- AST walking ---

fn find_all_calls(root: Node, source: &str) -> (Vec<CallSite>, Vec<ObserveSite>) {
    let mut calls = Vec::new();
    let mut observes = Vec::new();
    walk_tree(root, source, &mut calls, &mut observes);
    (calls, observes)
}

fn walk_tree(node: Node, source: &str, calls: &mut Vec<CallSite>, observes: &mut Vec<ObserveSite>) {
    if is_call(node.kind()) {
        if let Some(info) = extract_call_site(node, source) {
            calls.push(info);
        } else if let Some(info) = extract_observe_site(node, source) {
            observes.push(info);
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_tree(child, source, calls, observes);
    }
}

/// The node naming a method call's method (`call` in `iii.call(...)`).
fn method_name_node(call: Node) -> Option<Node> {
    let func = call.child_by_field_name("function")?;
    match func.kind() {
        "member_expression" => func.child_by_field_name("property"),
        "field_expression" => func.child_by_field_name("field"),
        "attribute" => func.child_by_field_name("attribute"),
        "identifier" => Some(func),
        _ => None,
    }
}

/// The non-comment arguments of a call.
fn argument_nodes<'a>(arguments: Node<'a>) -> Vec<Node<'a>> {
    let mut cursor = arguments.walk();
    let nodes = arguments
        .named_children(&mut cursor)
        .filter(|argument| !argument.kind().contains("comment"))
        .collect();
    nodes
}

// --- Call extraction ---

fn extract_call_site(call: Node, source: &str) -> Option<CallSite> {
    let method = extract_method_name(call, source)?;
    if !is_call_method(&method) {
        return None;
    }

    let arguments = find_arguments(call)?;
    let mut alias = if is_call_alias(&method) {
        method_name_node(call).map(|node| (method.clone(), to_range(node, source)))
    } else {
        None
    };

    // Try dict-style (or a Rust struct literal): first child that's an object/dictionary
    if let Some(args_object) = find_child_object(arguments) {
        let (function_id, function_id_range) =
            find_string_pair(args_object, "function_id", source)?;
        if alias.is_none() {
            alias = struct_alias(args_object, source);
        }
        let (has_payload, payload_keys, payload_range) =
            if let Some(info) = find_object_pair(args_object, &["payload", "data"], source) {
                (true, info.0, info.1)
            } else {
                (false, Vec::new(), to_range(args_object, source))
            };
        return Some(CallSite {
            function_id,
            function_id_range,
            payload_keys,
            payload_range,
            has_payload,
            alias,
        });
    }

    // Try a Rust constructor: CallRequest::new("x", payload)
    if let Some(first) = argument_nodes(arguments).first() {
        if let Some((function_id, function_id_range, constructor)) =
            find_constructor_argument(*first, source, &CompletionContext::FunctionId)
        {
            if alias.is_none() {
                alias = constructor_alias(constructor, source);
            }
            return Some(CallSite {
                function_id,
                function_id_range,
                payload_keys: Vec::new(),
                payload_range: to_range(*first, source),
                has_payload: false,
                alias,
            });
        }
    }

    // Try keyword-argument style (Python)
    let (function_id, function_id_range) = find_kwarg_string(arguments, "function_id", source)?;
    let (has_payload, payload_keys, payload_range) =
        if let Some(info) = find_kwarg_object(arguments, &["payload", "data"], source) {
            (true, info.0, info.1)
        } else {
            (false, Vec::new(), to_range(arguments, source))
        };
    Some(CallSite {
        function_id,
        function_id_range,
        payload_keys,
        payload_range,
        has_payload,
        alias,
    })
}

/// `TriggerRequest { .. }`: the deprecated alias of `CallRequest`, and where.
fn struct_alias(object: Node, source: &str) -> Option<(String, Range)> {
    if object.kind() != "struct_expression" {
        return None;
    }
    let name = object.child_by_field_name("name")?;
    let ty = type_name(name.utf8_text(source.as_bytes()).ok()?);
    (ty == "TriggerRequest").then(|| (ty.to_string(), to_range(name, source)))
}

/// `TriggerRequest::new(..)`: the deprecated alias of `CallRequest`, and where.
fn constructor_alias(constructor: Node, source: &str) -> Option<(String, Range)> {
    let function = constructor.child_by_field_name("function")?;
    let path = function.utf8_text(source.as_bytes()).ok()?;
    let (ty, _) = path.rsplit_once("::")?;
    (type_name(ty) == "TriggerRequest")
        .then(|| ("TriggerRequest".to_string(), to_range(function, source)))
}

// --- Observe extraction ---

fn extract_observe_site(call: Node, source: &str) -> Option<ObserveSite> {
    let method = extract_method_name(call, source)?;
    if !is_observe_method(&method) {
        return None;
    }
    let arguments = find_arguments(call)?;
    let nodes = argument_nodes(arguments);
    let source_node = *nodes.first()?;

    let (observed, args) = if is_string(source_node.kind()) {
        (
            Some((
                extract_string_content(source_node, source),
                to_range(source_node, source),
            )),
            ObserveArgs::Absent,
        )
    } else if is_object(source_node.kind()) {
        let args = match find_pair_value(source_node, "args", source) {
            None if has_opaque_entry(source_node, "args", source) => ObserveArgs::Unknown,
            None => ObserveArgs::Absent,
            Some(value) if is_object(value.kind()) => {
                let (keys, values, range) = object_details(value, source);
                ObserveArgs::Object {
                    keys,
                    values,
                    range,
                }
            }
            Some(_) => ObserveArgs::Unknown,
        };
        (find_string_pair(source_node, "function_id", source), args)
    } else {
        // Rust: Source::function("x").args(json!({ .. })), or a value built elsewhere
        let observed = find_constructor_argument(
            source_node,
            source,
            &CompletionContext::ObservableFunctionId,
        )
        .map(|(id, range, _)| (id, range));
        (observed, ObserveArgs::Unknown)
    };

    let target = nodes.get(1).and_then(|&target| {
        if is_string(target.kind()) {
            Some((
                extract_string_content(target, source),
                to_range(target, source),
            ))
        } else if is_object(target.kind()) {
            find_string_pair(target, "function_id", source)
        } else {
            find_constructor_argument(target, source, &CompletionContext::TargetFunctionId)
                .map(|(id, range, _)| (id, range))
        }
    });

    Some(ObserveSite {
        source: observed,
        args,
        target,
    })
}

/// Whether an object may hold `key` without a `key: value` pair: the
/// shorthand `{ key }`, or a spread (`...rest`, `**rest`).
fn has_opaque_entry(object: Node, key: &str, source: &str) -> bool {
    let mut cursor = object.walk();
    let opaque = object
        .children(&mut cursor)
        .any(|child| match child.kind() {
            "spread_element" | "dictionary_splat" => true,
            "shorthand_property_identifier" => child.utf8_text(source.as_bytes()) == Ok(key),
            _ => false,
        });
    opaque
}

fn is_string(kind: &str) -> bool {
    kind == "string" || kind == "string_literal"
}

/// The string first argument of the Rust constructor in `node`'s subtree
/// whose argument names what `expected` completes (`CallRequest::new`,
/// `Source::function`, `Target::function`), with the constructor call.
fn find_constructor_argument<'a>(
    node: Node<'a>,
    source: &str,
    expected: &CompletionContext,
) -> Option<(String, Range, Node<'a>)> {
    if node.kind() == "call_expression" {
        let path = node
            .child_by_field_name("function")
            .and_then(|function| function.utf8_text(source.as_bytes()).ok())
            .unwrap_or("");
        if rust_constructor_context(path).as_ref() == Some(expected) {
            let arguments = find_arguments(node)?;
            let first = *argument_nodes(arguments).first()?;
            return find_string_in_subtree(first, source).map(|(id, range)| (id, range, node));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_constructor_argument(child, source, expected) {
            return Some(found);
        }
    }
    None
}

// --- Helpers: find in arguments ---

#[allow(clippy::manual_find)]
fn find_arguments<'a>(call: Node<'a>) -> Option<Node<'a>> {
    let mut cursor = call.walk();
    for child in call.children(&mut cursor) {
        if is_arguments(child.kind()) {
            return Some(child);
        }
    }
    None
}

#[allow(clippy::manual_find)]
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
            if is_string(value_node.kind()) {
                let text = extract_string_content(value_node, source);
                return Some((text, to_range(value_node, source)));
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
    if is_string(node.kind()) {
        return Some((extract_string_content(node, source), to_range(node, source)));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(result) = find_string_in_subtree(child, source) {
            return Some(result);
        }
    }
    None
}

/// The value of the pair named `key_name` in an object/dictionary.
fn find_pair_value<'a>(object: Node<'a>, key_name: &str, source: &str) -> Option<Node<'a>> {
    let mut cursor = object.walk();
    for child in object.children(&mut cursor) {
        if child.kind() == "pair" {
            let Some(key) = child.child_by_field_name("key") else {
                continue;
            };
            let Ok(key_text) = key.utf8_text(source.as_bytes()) else {
                continue;
            };
            if strip_quotes(key_text) == key_name {
                return child.child_by_field_name("value");
            }
        }
    }
    None
}

fn find_object_pair(
    object: Node,
    key_names: &[&str],
    source: &str,
) -> Option<(Vec<String>, Range)> {
    key_names.iter().find_map(|key| {
        let value = find_pair_value(object, key, source)?;
        is_object(value.kind())
            .then(|| (extract_object_keys(value, source), to_range(value, source)))
    })
}

/// An object's keys, its key/value texts, and its range.
fn object_details(object: Node, source: &str) -> ObjectDetails {
    (
        extract_object_keys(object, source),
        extract_object_string_values(object, source),
        to_range(object, source),
    )
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
                            return Some((
                                extract_string_content(value, source),
                                to_range(value, source),
                            ));
                        }
                    }
                }
            }
        }
    }
    None
}

fn find_kwarg_object(arg_list: Node, names: &[&str], source: &str) -> Option<(Vec<String>, Range)> {
    let mut cursor = arg_list.walk();
    for child in arg_list.children(&mut cursor) {
        if child.kind() == "keyword_argument" {
            if let Some(n) = child.child_by_field_name("name") {
                let arg_name = n.utf8_text(source.as_bytes()).ok()?;
                if names.contains(&arg_name) {
                    if let Some(value) = child.child_by_field_name("value") {
                        if is_object(value.kind()) {
                            return Some((
                                extract_object_keys(value, source),
                                to_range(value, source),
                            ));
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
            return child.utf8_text(source.as_bytes()).unwrap_or("").to_string();
        }
    }
    String::new()
}

fn to_range(node: Node, source: &str) -> Range {
    let start = node.start_position();
    let end = node.end_position();
    let start_char = source
        .lines()
        .nth(start.row)
        .map(|line| byte_col_to_utf16_col(line, start.column))
        .unwrap_or(start.column as u32);
    let end_char = source
        .lines()
        .nth(end.row)
        .map(|line| byte_col_to_utf16_col(line, end.column))
        .unwrap_or(end.column as u32);
    Range {
        start: Position {
            line: start.row as u32,
            character: start_char,
        },
        end: Position {
            line: end.row as u32,
            character: end_char,
        },
    }
}

// --- Diagnostic checks ---

fn diagnostic(range: Range, severity: DiagnosticSeverity, message: String) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(severity),
        source: Some("lsp".to_string()),
        message,
        ..Default::default()
    }
}

/// What a deprecated alias is the alias of.
fn alias_replacement(alias: &str) -> &'static str {
    match alias {
        "trigger_async" => "call_async",
        "TriggerRequest" => "CallRequest",
        _ => "call",
    }
}

fn check_namespace_format(id: &str, range: Range, diagnostics: &mut Vec<Diagnostic>) {
    if !id.is_empty() && !id.contains("::") {
        diagnostics.push(diagnostic(
            range,
            DiagnosticSeverity::HINT,
            format!("Function ID '{id}' should use namespace format 'namespace::name'"),
        ));
    }
}

fn unknown_function(id: &str, range: Range) -> Diagnostic {
    diagnostic(
        range,
        DiagnosticSeverity::WARNING,
        format!("Unknown function '{id}'"),
    )
}

fn check_call(
    call: &CallSite,
    lookup: &dyn Fn(&str) -> Option<FunctionInfo>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some((alias, range)) = &call.alias {
        diagnostics.push(Diagnostic {
            tags: Some(vec![DiagnosticTag::DEPRECATED]),
            ..diagnostic(
                *range,
                DiagnosticSeverity::HINT,
                format!(
                    "'{alias}' is the deprecated alias of '{}'",
                    alias_replacement(alias)
                ),
            )
        });
    }

    if call.function_id.starts_with("engine::") {
        return;
    }

    check_namespace_format(&call.function_id, call.function_id_range, diagnostics);

    let func = match lookup(&call.function_id) {
        Some(f) => f,
        None => {
            if !call.function_id.is_empty() {
                diagnostics.push(unknown_function(&call.function_id, call.function_id_range));
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
                    diagnostics.push(diagnostic(
                        call.payload_range,
                        DiagnosticSeverity::ERROR,
                        format!(
                            "Missing required property '{}' for '{}'",
                            name, call.function_id
                        ),
                    ));
                }
            }
        }
    }
}

const VALID_HTTP_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

fn check_observe(
    site: &ObserveSite,
    lookup: &dyn Fn(&str) -> Option<FunctionInfo>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some((id, range)) = &site.source {
        check_observe_source(id, *range, &site.args, lookup, diagnostics);
    }

    if let Some((id, range)) = &site.target {
        check_namespace_format(id, *range, diagnostics);
        match lookup(id) {
            None if !id.is_empty() => diagnostics.push(unknown_function(id, *range)),
            Some(func) if func.private => diagnostics.push(diagnostic(
                *range,
                DiagnosticSeverity::ERROR,
                format!("Function '{id}' is private: an observer cannot target it"),
            )),
            _ => {}
        }
    }
}

fn check_observe_source(
    id: &str,
    range: Range,
    args: &ObserveArgs,
    lookup: &dyn Fn(&str) -> Option<FunctionInfo>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let args_range = match args {
        ObserveArgs::Object { range, .. } => *range,
        _ => range,
    };
    let (keys, values): (&[String], &[(String, String)]) = match args {
        ObserveArgs::Object { keys, values, .. } => (keys, values),
        _ => (&[], &[]),
    };

    // Engine-hosted sources take no args; consumers filter on their input.
    let args_format = if id.starts_with("engine::") || engine_source(id).is_some() {
        None
    } else {
        match lookup(id) {
            None => {
                if !id.is_empty() {
                    diagnostics.push(unknown_function(id, range));
                }
                return;
            }
            Some(func) => {
                if !func.observable {
                    diagnostics.push(diagnostic(
                        range,
                        DiagnosticSeverity::ERROR,
                        format!("Function '{id}' is not observable"),
                    ));
                    return;
                }
                func.args_format
            }
        }
    };

    match (&args_format, args) {
        (None, ObserveArgs::Object { .. }) if !keys.is_empty() => {
            diagnostics.push(diagnostic(
                args_range,
                DiagnosticSeverity::ERROR,
                format!("Function '{id}' takes no args: select its calls with filters on input"),
            ));
        }
        (Some(schema), ObserveArgs::Absent | ObserveArgs::Object { .. }) => {
            let required = schema.get("required").and_then(|r| r.as_array());
            for name in required.into_iter().flatten().filter_map(|r| r.as_str()) {
                if !keys.iter().any(|key| key == name) {
                    diagnostics.push(diagnostic(
                        args_range,
                        DiagnosticSeverity::ERROR,
                        format!("Missing required arg '{name}' for '{id}'"),
                    ));
                }
            }
        }
        _ => {}
    }

    if id == "http::on-request" {
        for (key, value) in values {
            if key == "method" && !VALID_HTTP_METHODS.contains(&value.to_uppercase().as_str()) {
                diagnostics.push(diagnostic(
                    args_range,
                    DiagnosticSeverity::WARNING,
                    format!(
                        "Invalid HTTP method '{}'. Expected one of: {}",
                        value,
                        VALID_HTTP_METHODS.join(", ")
                    ),
                ));
            }
        }
    }

    // sec min hour day month weekday, and an optional year
    if id == "cron::on-tick" {
        for (key, value) in values {
            if key == "expression" {
                let fields = value.split_whitespace().count();
                if fields != 6 && fields != 7 {
                    diagnostics.push(diagnostic(
                        args_range,
                        DiagnosticSeverity::ERROR,
                        format!(
                            "Cron expression must have 6 fields (sec min hour day month weekday) and an optional year, got {fields}"
                        ),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn parse(source: &str, language: Language) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        match language {
            Language::TypeScript => {
                parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            }
            Language::Python => parser.set_language(&tree_sitter_python::LANGUAGE.into()),
            Language::Rust => parser.set_language(&tree_sitter_rust::LANGUAGE.into()),
        }
        .unwrap();
        parser.parse(source, None).unwrap()
    }

    fn parse_ts(source: &str) -> tree_sitter::Tree {
        parse(source, Language::TypeScript)
    }

    fn parse_py(source: &str) -> tree_sitter::Tree {
        parse(source, Language::Python)
    }

    /// Diagnostics of `source` against a catalog of `functions`.
    fn diagnose_with(source: &str, language: Language, functions: &[FunctionInfo]) -> Vec<String> {
        let catalog: HashMap<String, FunctionInfo> = functions
            .iter()
            .map(|f| (f.function_id.clone(), f.clone()))
            .collect();
        let lookup = move |id: &str| catalog.get(id).cloned();
        let tree = parse(source, language);
        let (calls, observes) = find_all_calls(tree.root_node(), source);
        let mut diagnostics = Vec::new();
        for call in &calls {
            check_call(call, &lookup, &mut diagnostics);
        }
        for site in &observes {
            check_observe(site, &lookup, &mut diagnostics);
        }
        diagnostics.into_iter().map(|d| d.message).collect()
    }

    fn function(id: &str) -> FunctionInfo {
        FunctionInfo {
            function_id: id.to_string(),
            ..Default::default()
        }
    }

    fn cron() -> FunctionInfo {
        FunctionInfo {
            args_format: Some(json!({
                "type": "object",
                "properties": { "expression": { "type": "string" } },
                "required": ["expression"]
            })),
            private: true,
            ..function("cron::on-tick")
        }
    }

    // --- TypeScript tests ---

    #[test]
    fn ts_finds_call_sites() {
        let source = "iii.call({ function_id: 'todos::create', payload: { title: 'test' } })";
        let tree = parse_ts(source);
        let (calls, _) = find_all_calls(tree.root_node(), source);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "todos::create");
        assert!(calls[0].has_payload);
        assert!(calls[0].alias.is_none());
    }

    #[test]
    fn ts_trigger_alias_is_a_call_site() {
        let source = "iii.trigger({ function_id: 'todos::create' })";
        let tree = parse_ts(source);
        let (calls, _) = find_all_calls(tree.root_node(), source);
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].alias.as_ref().map(|a| a.0.as_str()),
            Some("trigger")
        );
    }

    #[test]
    fn ts_finds_observe_sites() {
        let source = "iii.observe({ function_id: 'http::on-request', args: { method: 'GET', path: '/users' } }, 'api::users')";
        let tree = parse_ts(source);
        let (calls, observes) = find_all_calls(tree.root_node(), source);
        assert!(calls.is_empty());
        assert_eq!(observes.len(), 1);
        let site = &observes[0];
        assert_eq!(site.source.as_ref().unwrap().0, "http::on-request");
        assert_eq!(site.target.as_ref().unwrap().0, "api::users");
        match &site.args {
            ObserveArgs::Object { keys, values, .. } => {
                assert_eq!(keys, &["method", "path"]);
                assert_eq!(values[0], ("method".to_string(), "GET".to_string()));
            }
            _ => panic!("args object expected"),
        }
    }

    #[test]
    fn ts_observe_shorthand_has_no_args() {
        let source = "iii.observe('orders::create', 'alerts::big-order')";
        let tree = parse_ts(source);
        let (_, observes) = find_all_calls(tree.root_node(), source);
        assert_eq!(observes[0].source.as_ref().unwrap().0, "orders::create");
        assert!(matches!(observes[0].args, ObserveArgs::Absent));
        assert_eq!(observes[0].target.as_ref().unwrap().0, "alerts::big-order");
    }

    #[test]
    fn ts_observe_args_variable_is_unknown() {
        let source = "iii.observe({ function_id: 'cron::on-tick', args }, 'reports::daily')";
        assert!(diagnose_with(
            source,
            Language::TypeScript,
            &[cron(), function("reports::daily")]
        )
        .is_empty());
    }

    // --- Python dict-style tests ---

    #[test]
    fn py_finds_call_sites_dict() {
        let source = "iii.call({'function_id': 'todos::create', 'payload': {'title': 'test'}})";
        let tree = parse_py(source);
        let (calls, _) = find_all_calls(tree.root_node(), source);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "todos::create");
        assert!(calls[0].has_payload);
        assert_eq!(calls[0].payload_keys, vec!["title"]);
    }

    #[test]
    fn py_finds_call_async_sites_dict() {
        let source =
            "await iii.call_async({'function_id': 'todos::create', 'payload': {'title': 'test'}})";
        let tree = parse_py(source);
        let (calls, _) = find_all_calls(tree.root_node(), source);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "todos::create");
        assert!(calls[0].has_payload);
        assert_eq!(calls[0].payload_keys, vec!["title"]);
    }

    #[test]
    fn py_finds_observe_dict() {
        let source = "iii.observe({'function_id': 'cron::on-tick', 'args': {'expression': '0 0 9 * * *'}}, 'reports::daily')";
        let tree = parse_py(source);
        let (_, observes) = find_all_calls(tree.root_node(), source);
        assert_eq!(observes.len(), 1);
        assert_eq!(observes[0].source.as_ref().unwrap().0, "cron::on-tick");
        assert_eq!(observes[0].target.as_ref().unwrap().0, "reports::daily");
        assert!(
            matches!(&observes[0].args, ObserveArgs::Object { keys, .. } if keys == &["expression"])
        );
    }

    // --- Python keyword argument tests ---

    #[test]
    fn py_finds_call_sites_kwarg() {
        let source = "iii.call(function_id='todos::create', payload={'title': 'test'})";
        let tree = parse_py(source);
        let (calls, _) = find_all_calls(tree.root_node(), source);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "todos::create");
        assert!(calls[0].has_payload);
        assert_eq!(calls[0].payload_keys, vec!["title"]);
    }

    #[test]
    fn py_finds_call_async_sites_kwarg() {
        let source = "await iii.call_async(function_id='todos::create', payload={'title': 'test'})";
        let tree = parse_py(source);
        let (calls, _) = find_all_calls(tree.root_node(), source);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "todos::create");
        assert!(calls[0].has_payload);
        assert_eq!(calls[0].payload_keys, vec!["title"]);
    }

    #[test]
    fn py_ignores_other_calls() {
        let source = "foo.bar(function_id='test')";
        let tree = parse_py(source);
        let (calls, observes) = find_all_calls(tree.root_node(), source);
        assert!(calls.is_empty());
        assert!(observes.is_empty());
    }

    // --- Rust tests ---

    #[test]
    fn rs_finds_call_request_new() {
        let source =
            r#"iii.call(CallRequest::new("todos::create", json!({})).action(CallAction::Void))"#;
        let tree = parse(source, Language::Rust);
        let (calls, _) = find_all_calls(tree.root_node(), source);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function_id, "todos::create");
        assert!(calls[0].alias.is_none());
    }

    #[test]
    fn rs_trigger_request_alias() {
        let source = r#"iii.call(TriggerRequest { function_id: "todos::create".into(), payload: json!({}), action: None, timeout_ms: None })"#;
        let messages = diagnose_with(source, Language::Rust, &[function("todos::create")]);
        assert_eq!(
            messages,
            vec!["'TriggerRequest' is the deprecated alias of 'CallRequest'"]
        );
    }

    #[test]
    fn rs_finds_observe_chain() {
        let source = r#"iii.observe_with(
            Source::function("cron::on-tick").args(json!({ "expression": "0 0 9 * * *" })),
            Target::function("reports::daily").namespace("ops"),
            ObserverOptions::default(),
        )"#;
        let tree = parse(source, Language::Rust);
        let (_, observes) = find_all_calls(tree.root_node(), source);
        assert_eq!(observes.len(), 1);
        assert_eq!(observes[0].source.as_ref().unwrap().0, "cron::on-tick");
        assert_eq!(observes[0].target.as_ref().unwrap().0, "reports::daily");
        assert!(matches!(observes[0].args, ObserveArgs::Unknown));
    }

    // --- Diagnostics ---

    #[test]
    fn unknown_and_private_targets() {
        let source = "iii.observe('orders::create', 'cron::on-tick'); iii.observe('orders::create', 'nobody::here')";
        let messages = diagnose_with(
            source,
            Language::TypeScript,
            &[function("orders::create"), cron()],
        );
        assert_eq!(
            messages,
            vec![
                "Function 'cron::on-tick' is private: an observer cannot target it",
                "Unknown function 'nobody::here'",
            ]
        );
    }

    #[test]
    fn missing_required_args() {
        let source = "iii.observe('cron::on-tick', 'reports::daily')";
        let messages = diagnose_with(
            source,
            Language::TypeScript,
            &[cron(), function("reports::daily")],
        );
        assert_eq!(
            messages,
            vec!["Missing required arg 'expression' for 'cron::on-tick'"]
        );
    }

    #[test]
    fn args_to_a_function_without_args_format() {
        let source =
            "iii.observe({ function_id: 'orders::create', args: { total: 1 } }, 'alerts::big')";
        let messages = diagnose_with(
            source,
            Language::TypeScript,
            &[function("orders::create"), function("alerts::big")],
        );
        assert_eq!(
            messages,
            vec!["Function 'orders::create' takes no args: select its calls with filters on input"]
        );
    }

    #[test]
    fn unobservable_source() {
        let secret = FunctionInfo {
            observable: false,
            ..function("secrets::resolve")
        };
        let source = "iii.observe('secrets::resolve', 'audit::log')";
        let messages = diagnose_with(
            source,
            Language::TypeScript,
            &[secret, function("audit::log")],
        );
        assert_eq!(
            messages,
            vec!["Function 'secrets::resolve' is not observable"]
        );
    }

    #[test]
    fn engine_sources_take_no_args_and_are_known() {
        let ok =
            "iii.observe({ function_id: 'configuration::on-updated', filters: {} }, 'w::reload')";
        assert!(diagnose_with(ok, Language::TypeScript, &[function("w::reload")]).is_empty());

        let with_args = "iii.observe({ function_id: 'engine::on-functions-changed', args: { a: 'b' } }, 'w::r')";
        assert_eq!(
            diagnose_with(with_args, Language::TypeScript, &[function("w::r")]),
            vec!["Function 'engine::on-functions-changed' takes no args: select its calls with filters on input"]
        );
    }

    #[test]
    fn cron_expression_fields() {
        let functions = [cron(), function("r::d")];
        let six = "iii.observe({ function_id: 'cron::on-tick', args: { expression: '0 0 9 * * *' } }, 'r::d')";
        assert!(diagnose_with(six, Language::TypeScript, &functions).is_empty());
        let seven = "iii.observe({ function_id: 'cron::on-tick', args: { expression: '0 0 9 * * * *' } }, 'r::d')";
        assert!(diagnose_with(seven, Language::TypeScript, &functions).is_empty());
        let five = "iii.observe({ function_id: 'cron::on-tick', args: { expression: '0 9 * * *' } }, 'r::d')";
        assert_eq!(
            diagnose_with(five, Language::TypeScript, &functions),
            vec!["Cron expression must have 6 fields (sec min hour day month weekday) and an optional year, got 5"]
        );
    }

    #[test]
    fn http_method_arg() {
        let http = FunctionInfo {
            args_format: Some(json!({ "type": "object", "required": ["method", "path"] })),
            private: true,
            ..function("http::on-request")
        };
        let functions = [http, function("api::users")];
        let ok = "iii.observe({ function_id: 'http::on-request', args: { method: 'post', path: '/users' } }, 'api::users')";
        assert!(diagnose_with(ok, Language::TypeScript, &functions).is_empty());
        let bad = "iii.observe({ function_id: 'http::on-request', args: { method: 'FETCH', path: '/users' } }, 'api::users')";
        assert_eq!(
            diagnose_with(bad, Language::TypeScript, &functions),
            vec!["Invalid HTTP method 'FETCH'. Expected one of: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS"]
        );
    }

    #[test]
    fn call_alias_and_unknown_function() {
        let source = "await iii.trigger_async({'function_id': 'todos::missing'})";
        assert_eq!(
            diagnose_with(source, Language::Python, &[]),
            vec![
                "'trigger_async' is the deprecated alias of 'call_async'",
                "Unknown function 'todos::missing'",
            ]
        );
    }

    #[test]
    fn required_payload_property() {
        let create = FunctionInfo {
            request_format: Some(json!({ "type": "object", "required": ["title"] })),
            ..function("todos::create")
        };
        let source = "iii.call({ function_id: 'todos::create', payload: { done: false } })";
        assert_eq!(
            diagnose_with(source, Language::TypeScript, &[create]),
            vec!["Missing required property 'title' for 'todos::create'"]
        );
    }

    #[test]
    fn calls_through_an_observer_are_not_call_sites() {
        let source = "iii.call({ observer: obs, payload: { path: '/x' } })";
        assert!(diagnose_with(source, Language::TypeScript, &[]).is_empty());
    }
}
