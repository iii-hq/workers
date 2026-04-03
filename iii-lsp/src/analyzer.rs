use tower_lsp_server::ls_types::Position;
use tree_sitter::{Node, Parser, Point};

#[derive(Debug, Clone, PartialEq)]
pub enum CompletionContext {
    /// Cursor is inside a function_id string in trigger() or registerTrigger()
    FunctionId,
    /// Cursor is inside a type string in registerTrigger()
    TriggerType,
    /// Cursor is inside a payload/data object in trigger() — suggest property names
    /// from the function's request_format schema
    PayloadProperty { function_id: String },
    /// Cursor is not in a completable position
    None,
}

#[derive(Debug)]
pub struct AnalysisResult {
    pub context: CompletionContext,
    /// The current text content of the string at cursor (without quotes)
    pub current_text: String,
}

/// Analyze a TypeScript source file at the given cursor position.
/// Tries the original source first, then falls back to patching unclosed
/// strings at the cursor so tree-sitter can produce a valid AST.
pub fn analyze(source: &str, position: Position) -> AnalysisResult {
    // Try original source at position and position-1
    let result = try_analyze(source, position);
    if result.context != CompletionContext::None {
        return result;
    }

    // Try with patched unclosed string
    if let Some(patched) = patch_unclosed_string(source, position) {
        let result = try_analyze(&patched, position);
        if result.context != CompletionContext::None {
            return result;
        }
    }

    // Try with patched unclosed brackets (for payload context in incomplete code)
    if let Some(patched) = patch_unclosed_brackets(source, position) {
        let result = try_analyze(&patched, position);
        if result.context != CompletionContext::None {
            return result;
        }
    }

    AnalysisResult {
        context: CompletionContext::None,
        current_text: String::new(),
    }
}

/// Try analysis at both the given position and position-1.
fn try_analyze(source: &str, position: Position) -> AnalysisResult {
    let result = analyze_source(source, position);
    if result.context != CompletionContext::None {
        return result;
    }
    if position.character > 0 {
        let inner = Position {
            line: position.line,
            character: position.character - 1,
        };
        return analyze_source(source, inner);
    }
    result
}

/// Patch the source by closing an unclosed string at the cursor position.
/// When the user types `function_id: '` and hasn't closed the quote yet,
/// tree-sitter produces ERROR nodes. This inserts a closing quote so
/// tree-sitter can build a valid AST.
fn patch_unclosed_string(source: &str, position: Position) -> Option<String> {
    let byte_offset = position_to_byte_offset(source, position)?;

    if byte_offset == 0 {
        return Option::None;
    }

    let line_start = source[..byte_offset].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let line_before = &source[line_start..byte_offset];

    // Count quotes before cursor on this line
    let single_quotes = line_before.chars().filter(|&c| c == '\'').count();
    let double_quotes = line_before.chars().filter(|&c| c == '"').count();

    if single_quotes % 2 == 1 {
        let mut patched = source.to_string();
        patched.insert_str(byte_offset, "' })");
        return Some(patched);
    }

    if double_quotes % 2 == 1 {
        let mut patched = source.to_string();
        patched.insert_str(byte_offset, "\" })");
        return Some(patched);
    }

    Option::None
}

/// Patch the source by closing unclosed brackets/parens at the cursor position.
/// Handles cases like `iii.trigger({function_id: 'x', payload: {}` missing `})`.
fn patch_unclosed_brackets(source: &str, position: Position) -> Option<String> {
    let byte_offset = position_to_byte_offset(source, position)?;
    let before = &source[..byte_offset];

    let mut open_parens: i32 = 0;
    let mut open_braces: i32 = 0;
    let mut in_string = false;
    let mut string_char = ' ';

    for ch in before.chars() {
        if in_string {
            if ch == string_char {
                in_string = false;
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                in_string = true;
                string_char = ch;
            }
            '(' => open_parens += 1,
            ')' => open_parens -= 1,
            '{' => open_braces += 1,
            '}' => open_braces -= 1,
            _ => {}
        }
    }

    if open_parens <= 0 && open_braces <= 0 {
        return Option::None;
    }

    let mut suffix = String::new();
    for _ in 0..open_braces.max(0) {
        suffix.push('}');
    }
    for _ in 0..open_parens.max(0) {
        suffix.push(')');
    }

    let mut patched = source.to_string();
    patched.insert_str(byte_offset, &suffix);
    Some(patched)
}

fn position_to_byte_offset(source: &str, position: Position) -> Option<usize> {
    let line_num = position.line as usize;
    let col = position.character as usize;

    let mut byte_offset = 0;
    for (i, line) in source.lines().enumerate() {
        if i == line_num {
            byte_offset += col.min(line.len());
            return Some(byte_offset);
        }
        byte_offset += line.len() + 1;
    }

    // Cursor past last line
    if line_num == source.lines().count() {
        Some(source.len())
    } else {
        Option::None
    }
}

fn analyze_source(source: &str, position: Position) -> AnalysisResult {
    let none = AnalysisResult {
        context: CompletionContext::None,
        current_text: String::new(),
    };

    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .is_err()
    {
        return none;
    }

    let tree = match parser.parse(source, Option::None) {
        Some(t) => t,
        Option::None => return none,
    };

    let point = Point::new(position.line as usize, position.character as usize);
    let root = tree.root_node();

    let node = match root.descendant_for_point_range(point, point) {
        Some(n) => n,
        Option::None => return none,
    };

    // Try string context first (function_id, type completions)
    if let Some((string_node, current_text)) = find_string_at_cursor(node, source) {
        let context = determine_context(string_node, source);
        if context != CompletionContext::None {
            return AnalysisResult {
                context,
                current_text,
            };
        }
    }

    // Try payload object context (property suggestions from request_format)
    let context = determine_payload_context(node, source);

    AnalysisResult {
        context,
        current_text: String::new(),
    }
}

/// Find the string node containing the cursor and extract its text content.
fn find_string_at_cursor<'a>(node: Node<'a>, source: &str) -> Option<(Node<'a>, String)> {
    let kind = node.kind();

    if kind == "string_fragment" || kind == "template_string" {
        let text = node.utf8_text(source.as_bytes()).unwrap_or("");
        let string_node = node.parent()?;
        if string_node.kind() == "string" || string_node.kind() == "template_string" {
            return Some((string_node, text.to_string()));
        }
    }

    if kind == "string" {
        let text = extract_string_content(node, source);
        return Some((node, text));
    }

    // Check if we're on a quote character that's part of a string
    if kind == "'" || kind == "\"" {
        if let Some(parent) = node.parent() {
            if parent.kind() == "string" {
                let text = extract_string_content(parent, source);
                return Some((parent, text));
            }
        }
    }

    None
}

/// Extract the text content from a string node (without quotes).
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

/// Walk up the AST from a string node to determine the completion context.
/// Expected chain: string -> pair -> object -> arguments -> call_expression
fn determine_context(string_node: Node, source: &str) -> CompletionContext {
    // string -> pair
    let pair = match string_node.parent() {
        Some(p) if p.kind() == "pair" => p,
        _ => return CompletionContext::None,
    };

    // Get the key of the pair
    let key = match pair.child_by_field_name("key") {
        Some(k) => k,
        None => return CompletionContext::None,
    };
    let key_text = key.utf8_text(source.as_bytes()).unwrap_or("");

    // pair -> object
    let object = match pair.parent() {
        Some(o) if o.kind() == "object" => o,
        _ => return CompletionContext::None,
    };

    // object -> arguments
    let arguments = match object.parent() {
        Some(a) if a.kind() == "arguments" => a,
        _ => return CompletionContext::None,
    };

    // arguments -> call_expression
    let call = match arguments.parent() {
        Some(c) if c.kind() == "call_expression" => c,
        _ => return CompletionContext::None,
    };

    // Get the method name from the call expression
    let method_name = match extract_method_name(call, source) {
        Some(name) => name,
        None => return CompletionContext::None,
    };

    match (method_name.as_str(), key_text) {
        ("trigger", "function_id") => CompletionContext::FunctionId,
        ("registerTrigger", "function_id") => CompletionContext::FunctionId,
        ("registerTrigger", "type") => CompletionContext::TriggerType,
        _ => CompletionContext::None,
    }
}

/// Extract the method name from a call_expression node.
fn extract_method_name(call: Node, source: &str) -> Option<String> {
    let func = call.child_by_field_name("function")?;

    if func.kind() == "member_expression" {
        let property = func.child_by_field_name("property")?;
        let name = property.utf8_text(source.as_bytes()).ok()?;
        return Some(name.to_string());
    }

    if func.kind() == "identifier" {
        let name = func.utf8_text(source.as_bytes()).ok()?;
        return Some(name.to_string());
    }

    None
}

/// Detect if the cursor is inside a payload/data object in a trigger() call.
/// Walks up the AST looking for: cursor → ... → object(payload value) → pair(payload) → object(args) → arguments → call_expression(trigger)
fn determine_payload_context(node: Node, source: &str) -> CompletionContext {
    let mut current = node;

    loop {
        if current.kind() == "object" {
            if let Some(context) = check_payload_object(current, source) {
                return context;
            }
        }

        match current.parent() {
            Some(p) => current = p,
            None => break,
        }
    }

    CompletionContext::None
}

/// Check if an object node is the value of a "payload" or "data" pair inside a trigger() call.
fn check_payload_object(object: Node, source: &str) -> Option<CompletionContext> {
    // object must be the value of a pair
    let pair = object.parent()?;
    if pair.kind() != "pair" {
        return None;
    }

    // pair key must be "payload" or "data"
    let key = pair.child_by_field_name("key")?;
    let key_text = key.utf8_text(source.as_bytes()).ok()?;
    if key_text != "payload" && key_text != "data" {
        return None;
    }

    // pair must be inside the outer arguments object
    let outer_object = pair.parent()?;
    if outer_object.kind() != "object" {
        return None;
    }

    // outer object must be inside arguments of a call_expression
    let arguments = outer_object.parent()?;
    if arguments.kind() != "arguments" {
        return None;
    }

    let call = arguments.parent()?;
    if call.kind() != "call_expression" {
        return None;
    }

    let method_name = extract_method_name(call, source)?;
    if method_name != "trigger" {
        return None;
    }

    // Find function_id from sibling pairs in the outer object
    let function_id = find_function_id_in_object(outer_object, source)?;

    Some(CompletionContext::PayloadProperty { function_id })
}

/// Find the function_id string value from pairs in an object node.
fn find_function_id_in_object(object: Node, source: &str) -> Option<String> {
    let mut cursor = object.walk();
    for child in object.children(&mut cursor) {
        if child.kind() == "pair" {
            if let Some(key) = child.child_by_field_name("key") {
                if key.utf8_text(source.as_bytes()).ok()? == "function_id" {
                    if let Some(value) = child.child_by_field_name("value") {
                        if value.kind() == "string" {
                            return Some(extract_string_content(value, source));
                        }
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn trigger_function_id() {
        let source = r#"iii.trigger({ function_id: 'todos::create' })"#;
        let result = analyze(source, pos(0, 28));
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "todos::create");
    }

    #[test]
    fn trigger_function_id_empty_string() {
        let source = r#"iii.trigger({ function_id: '' })"#;
        let result = analyze(source, pos(0, 27));
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "");
    }

    #[test]
    fn register_trigger_type() {
        let source = r#"iii.registerTrigger({ type: 'http', function_id: 'greet' })"#;
        let result = analyze(source, pos(0, 29));
        assert_eq!(result.context, CompletionContext::TriggerType);
        assert_eq!(result.current_text, "http");
    }

    #[test]
    fn register_trigger_function_id() {
        let source = r#"iii.registerTrigger({ type: 'http', function_id: 'greet' })"#;
        let result = analyze(source, pos(0, 50));
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "greet");
    }

    #[test]
    fn not_in_completable_position() {
        let source = r#"const name = 'hello world';"#;
        let result = analyze(source, pos(0, 15));
        assert_eq!(result.context, CompletionContext::None);
    }

    #[test]
    fn cursor_in_comment() {
        let source = r#"// iii.trigger({ function_id: 'test' })"#;
        let result = analyze(source, pos(0, 31));
        assert_eq!(result.context, CompletionContext::None);
    }

    #[test]
    fn await_trigger() {
        let source = r#"await iii.trigger({ function_id: 'test' })"#;
        let result = analyze(source, pos(0, 34));
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "test");
    }

    #[test]
    fn multiline_trigger() {
        let source = "iii.trigger({\n  function_id: 'todos::create',\n  payload: {}\n})";
        let result = analyze(source, pos(1, 17));
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "todos::create");
    }

    #[test]
    fn nested_object_not_completable() {
        let source = r#"iii.trigger({ options: { function_id: 'test' } })"#;
        let result = analyze(source, pos(0, 39));
        assert_eq!(result.context, CompletionContext::None);
    }

    #[test]
    fn double_quoted_string() {
        let source = r#"iii.trigger({ function_id: "todos::create" })"#;
        let result = analyze(source, pos(0, 28));
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "todos::create");
    }

    // --- Unclosed string tests (the real-world typing scenario) ---
    // After typing `'`, the cursor is one past the quote character.

    #[test]
    fn unclosed_string_trigger() {
        let source = "iii.trigger({function_id: '";
        // Cursor after the quote = column 27 (past end of 27-char line)
        let result = analyze(source, pos(0, 27));
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn unclosed_string_register_trigger_type() {
        let source = "iii.registerTrigger({ type: '";
        let result = analyze(source, pos(0, 29));
        assert_eq!(result.context, CompletionContext::TriggerType);
    }

    #[test]
    fn unclosed_string_partial_text() {
        // User has typed 'to' and is still typing
        let source = "iii.trigger({function_id: 'to";
        let result = analyze(source, pos(0, 29));
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "to");
    }

    #[test]
    fn unclosed_string_in_real_file() {
        // Cursor after the quote = column 27 (line 1 has 27 chars)
        let source = "const x = 1;\niii.trigger({function_id: '\nconst y = 2;";
        let result = analyze(source, pos(1, 27));
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn unclosed_double_quote() {
        let source = "iii.trigger({function_id: \"";
        let result = analyze(source, pos(0, 27));
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    // --- Payload property completion tests ---

    #[test]
    fn payload_property_empty_object() {
        // Cursor inside empty payload object
        let source = "iii.trigger({ function_id: 'todos::create', payload: {} })";
        // Position inside the {} of payload — column 54 is between { and }
        let result = analyze(source, pos(0, 54));
        assert_eq!(
            result.context,
            CompletionContext::PayloadProperty {
                function_id: "todos::create".to_string()
            }
        );
    }

    #[test]
    fn payload_property_multiline() {
        let source = "iii.trigger({\n  function_id: 'todos::create',\n  payload: {\n    \n  }\n})";
        // Cursor on the empty line inside payload (line 3)
        let result = analyze(source, pos(3, 4));
        assert_eq!(
            result.context,
            CompletionContext::PayloadProperty {
                function_id: "todos::create".to_string()
            }
        );
    }

    #[test]
    fn payload_not_in_trigger() {
        // payload inside some other function — should NOT trigger
        let source = "foo({ function_id: 'test', payload: {} })";
        let result = analyze(source, pos(0, 36));
        assert_eq!(result.context, CompletionContext::None);
    }

    #[test]
    fn data_property_also_works() {
        let source = "iii.trigger({ function_id: 'todos::create', data: {} })";
        let result = analyze(source, pos(0, 51));
        assert_eq!(
            result.context,
            CompletionContext::PayloadProperty {
                function_id: "todos::create".to_string()
            }
        );
    }

    #[test]
    fn payload_unclosed_brackets() {
        // Real-world case: user is typing and hasn't closed }) yet
        let source = "iii.trigger({function_id: 'myscope::create_todo', payload: {}";
        // Cursor at col 60 (on the } of payload object)
        let result = analyze(source, pos(0, 60));
        assert_eq!(
            result.context,
            CompletionContext::PayloadProperty {
                function_id: "myscope::create_todo".to_string()
            }
        );
    }

    #[test]
    fn payload_unclosed_with_cursor_inside() {
        // User typed { and cursor is right after it
        let source = "iii.trigger({function_id: 'myscope::create_todo', payload: {";
        let result = analyze(source, pos(0, 60));
        assert_eq!(
            result.context,
            CompletionContext::PayloadProperty {
                function_id: "myscope::create_todo".to_string()
            }
        );
    }
}

#[cfg(test)]
mod debug_payload {
    use super::*;
    use tree_sitter::Point;

    #[test]
    fn debug_real_payload() {
        let source = "iii.trigger({function_id: 'myscope::create_todo', payload: {}";
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        eprintln!("AST: {}", root.to_sexp());
        
        for col in 58..62 {
            let node = root.descendant_for_point_range(Point::new(0, col), Point::new(0, col));
            if let Some(n) = node {
                eprintln!("Col {}: kind={:?} named={}", col, n.kind(), n.is_named());
                let mut p = n.parent();
                let mut depth = 0;
                while let Some(parent) = p {
                    depth += 1;
                    if depth > 8 { break; }
                    eprintln!("  {}parent: kind={:?}", " ".repeat(depth), parent.kind());
                    p = parent.parent();
                }
            }
        }
    }
    
    #[test]
    fn debug_complete_payload() {
        let source = "iii.trigger({function_id: 'myscope::create_todo', payload: {} })";
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        eprintln!("COMPLETE AST: {}", root.to_sexp());
        
        for col in 58..62 {
            let node = root.descendant_for_point_range(Point::new(0, col), Point::new(0, col));
            if let Some(n) = node {
                eprintln!("Col {}: kind={:?} named={}", col, n.kind(), n.is_named());
                let mut p = n.parent();
                let mut depth = 0;
                while let Some(parent) = p {
                    depth += 1;
                    if depth > 8 { break; }
                    eprintln!("  {}parent: kind={:?}", " ".repeat(depth), parent.kind());
                    p = parent.parent();
                }
            }
        }
    }
}
