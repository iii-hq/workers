use tower_lsp_server::ls_types::Position;
use tree_sitter::{Node, Parser, Point};

/// Convert a UTF-16 column offset to a byte column offset within a line.
pub(crate) fn utf16_col_to_byte_col(line: &str, col_utf16: usize) -> usize {
    let mut utf16_units = 0;
    for (byte_idx, ch) in line.char_indices() {
        if utf16_units >= col_utf16 {
            return byte_idx;
        }
        utf16_units += ch.len_utf16();
    }
    line.len()
}

/// Convert a byte column offset to a UTF-16 column offset within a line.
pub(crate) fn byte_col_to_utf16_col(line: &str, byte_col: usize) -> u32 {
    let mut utf16_units: u32 = 0;
    for (byte_idx, ch) in line.char_indices() {
        if byte_idx >= byte_col {
            break;
        }
        utf16_units += ch.len_utf16() as u32;
    }
    utf16_units
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Language {
    TypeScript,
    Python,
    Rust,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompletionContext {
    FunctionId,
    TriggerType,
    PayloadProperty { function_id: String },
    TriggerConfigProperty { trigger_type: String },
    KnownValue { field_name: String },
    None,
}

#[derive(Debug)]
pub struct AnalysisResult {
    pub context: CompletionContext,
    pub current_text: String,
}

// --- Node kind helpers (language-aware) ---

fn is_call(kind: &str) -> bool {
    kind == "call_expression" || kind == "call"
}

fn is_object(kind: &str) -> bool {
    kind == "object" || kind == "dictionary"
}

fn is_arguments(kind: &str) -> bool {
    kind == "arguments" || kind == "argument_list"
}

fn is_string_content(kind: &str) -> bool {
    kind == "string_fragment" || kind == "string_content"
}

fn is_method_access(kind: &str) -> bool {
    kind == "member_expression" || kind == "attribute" || kind == "field_expression"
}

/// Method names that map to `trigger()`
fn is_trigger_method(name: &str) -> bool {
    name == "trigger" || name == "trigger_async"
}

/// Method names that map to `registerTrigger()` / `register_trigger()`
fn is_register_trigger_method(name: &str) -> bool {
    name == "registerTrigger" || name == "register_trigger"
}

// --- Public API ---

pub fn analyze(source: &str, position: Position, language: Language) -> AnalysisResult {
    let result = try_analyze(source, position, language);
    if result.context != CompletionContext::None {
        return result;
    }

    if let Some(patched) = patch_unclosed_string(source, position) {
        let result = try_analyze(&patched, position, language);
        if result.context != CompletionContext::None {
            return result;
        }
    }

    if let Some(patched) = patch_unclosed_brackets(source, position) {
        let result = try_analyze(&patched, position, language);
        if result.context != CompletionContext::None {
            return result;
        }
    }

    AnalysisResult {
        context: CompletionContext::None,
        current_text: String::new(),
    }
}

fn try_analyze(source: &str, position: Position, language: Language) -> AnalysisResult {
    let result = analyze_source(source, position, language);
    if result.context != CompletionContext::None {
        return result;
    }
    if position.character > 0 {
        let inner = Position {
            line: position.line,
            character: position.character - 1,
        };
        return analyze_source(source, inner, language);
    }
    result
}

// --- Source patching ---

fn patch_unclosed_string(source: &str, position: Position) -> Option<String> {
    let byte_offset = position_to_byte_offset(source, position)?;
    if byte_offset == 0 {
        return Option::None;
    }

    let line_start = source[..byte_offset]
        .rfind('\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    let line_before = &source[line_start..byte_offset];

    let single_quotes = line_before.chars().filter(|&c| c == '\'').count();
    let double_quotes = line_before.chars().filter(|&c| c == '"').count();

    if single_quotes % 2 == 1 {
        let mut patched = source.to_string();
        // Close the quote + any unclosed brackets AT the cursor position
        let suffix = build_closing_suffix(&source[..byte_offset], '\'');
        patched.insert_str(byte_offset, &suffix);
        return Some(patched);
    }

    if double_quotes % 2 == 1 {
        let mut patched = source.to_string();
        let suffix = build_closing_suffix(&source[..byte_offset], '"');
        patched.insert_str(byte_offset, &suffix);
        return Some(patched);
    }

    Option::None
}

/// Build a suffix that closes the quote and any unclosed brackets before the cursor.
fn build_closing_suffix(before_cursor: &str, quote: char) -> String {
    let mut open_parens: i32 = 0;
    let mut open_braces: i32 = 0;
    let mut in_string = false;
    let mut string_char = ' ';

    for ch in before_cursor.chars() {
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

    let mut suffix = String::new();
    suffix.push(quote);
    for _ in 0..open_braces.max(0) {
        suffix.push(' ');
        suffix.push('}');
    }
    for _ in 0..open_parens.max(0) {
        suffix.push(')');
    }
    suffix
}

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
    let col_utf16 = position.character as usize;

    let mut byte_offset = 0;
    let mut current_line = 0;

    // Split on \n to handle both LF and CRLF (tower-lsp doesn't normalize)
    for raw_line in source.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if current_line == line_num {
            return Some(byte_offset + utf16_col_to_byte_col(line, col_utf16));
        }
        byte_offset += raw_line.len() + 1; // +1 for the \n
        current_line += 1;
    }

    if line_num == current_line {
        Some(source.len())
    } else {
        Option::None
    }
}

// --- Core analysis ---

fn analyze_source(source: &str, position: Position, language: Language) -> AnalysisResult {
    let none = AnalysisResult {
        context: CompletionContext::None,
        current_text: String::new(),
    };

    let mut parser = Parser::new();
    let lang_ok = match language {
        Language::TypeScript => {
            parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        }
        Language::Python => parser.set_language(&tree_sitter_python::LANGUAGE.into()),
        Language::Rust => parser.set_language(&tree_sitter_rust::LANGUAGE.into()),
    };
    if lang_ok.is_err() {
        return none;
    }

    let tree = match parser.parse(source, Option::None) {
        Some(t) => t,
        Option::None => return none,
    };

    // Tree-sitter Point expects byte column, not UTF-16 code units
    let byte_col = source
        .lines()
        .nth(position.line as usize)
        .map(|line| utf16_col_to_byte_col(line, position.character as usize))
        .unwrap_or(0);
    let point = Point::new(position.line as usize, byte_col);
    let root = tree.root_node();

    let node = match root.descendant_for_point_range(point, point) {
        Some(n) => n,
        Option::None => return none,
    };

    // Try string context (function_id, type, known values)
    if let Some((string_node, current_text)) = find_string_at_cursor(node, source) {
        // Try dict-style context: string → pair → object → arguments → call
        let context = determine_context_pair(string_node, source);
        if context != CompletionContext::None {
            return AnalysisResult {
                context,
                current_text,
            };
        }

        // Try keyword argument context (Python): string → keyword_argument → argument_list → call
        let context = determine_context_kwarg(string_node, source);
        if context != CompletionContext::None {
            return AnalysisResult {
                context,
                current_text,
            };
        }

        // Try Rust struct field context: string → ... → field_initializer → struct_expression
        let context = determine_context_rust_field(string_node, source);
        if context != CompletionContext::None {
            return AnalysisResult {
                context,
                current_text,
            };
        }

        // Check known value fields (works for pair, kwarg, and field_initializer)
        if let Some(context) = check_known_value_field(string_node, source) {
            return AnalysisResult {
                context,
                current_text,
            };
        }
    }

    // Try object/dict context (payload properties, trigger config properties)
    let context = determine_object_context(node, source);

    AnalysisResult {
        context,
        current_text: String::new(),
    }
}

// --- String detection ---

fn find_string_at_cursor<'a>(node: Node<'a>, source: &str) -> Option<(Node<'a>, String)> {
    let kind = node.kind();

    // string_fragment (TS) or string_content (Python/Rust)
    if is_string_content(kind) {
        let text = node.utf8_text(source.as_bytes()).unwrap_or("");
        let string_node = node.parent()?;
        if string_node.kind() == "string" || string_node.kind() == "string_literal" {
            return Some((string_node, text.to_string()));
        }
    }

    if kind == "string" || kind == "string_literal" {
        let text = extract_string_content(node, source);
        return Some((node, text));
    }

    // Quote character that's part of a string
    // TS: anonymous "'" or "\"" nodes
    // Python: named "string_start" / "string_end" nodes
    if kind == "'" || kind == "\"" || kind == "string_start" || kind == "string_end" {
        if let Some(parent) = node.parent() {
            if parent.kind() == "string" || parent.kind() == "string_literal" {
                let text = extract_string_content(parent, source);
                return Some((parent, text));
            }
        }
    }

    None
}

fn extract_string_content(string_node: Node, source: &str) -> String {
    let mut cursor = string_node.walk();
    for child in string_node.children(&mut cursor) {
        if is_string_content(child.kind()) {
            return child.utf8_text(source.as_bytes()).unwrap_or("").to_string();
        }
    }
    String::new()
}

// --- Context detection: dict/object style (TS + Python dict) ---
// Chain: string → pair → object/dictionary → arguments/argument_list → call_expression/call

fn determine_context_pair(string_node: Node, source: &str) -> CompletionContext {
    let pair = match string_node.parent() {
        Some(p) if p.kind() == "pair" => p,
        _ => return CompletionContext::None,
    };

    let key = match pair.child_by_field_name("key") {
        Some(k) => k,
        None => return CompletionContext::None,
    };
    // Python dict keys are strings like 'function_id', strip quotes
    let key_text = strip_quotes(key.utf8_text(source.as_bytes()).unwrap_or(""));

    let object = match pair.parent() {
        Some(o) if is_object(o.kind()) => o,
        _ => return CompletionContext::None,
    };

    let arguments = match object.parent() {
        Some(a) if is_arguments(a.kind()) => a,
        _ => return CompletionContext::None,
    };

    let call = match arguments.parent() {
        Some(c) if is_call(c.kind()) => c,
        _ => return CompletionContext::None,
    };

    let method_name = match extract_method_name(call, source) {
        Some(name) => name,
        None => return CompletionContext::None,
    };

    match_method_and_key(&method_name, &key_text)
}

// --- Context detection: keyword argument style (Python only) ---
// Chain: string → keyword_argument → argument_list → call

fn determine_context_kwarg(string_node: Node, source: &str) -> CompletionContext {
    let kwarg = match string_node.parent() {
        Some(k) if k.kind() == "keyword_argument" => k,
        _ => return CompletionContext::None,
    };

    let name_node = match kwarg.child_by_field_name("name") {
        Some(n) => n,
        None => return CompletionContext::None,
    };
    let arg_name = name_node.utf8_text(source.as_bytes()).unwrap_or("");

    let arg_list = match kwarg.parent() {
        Some(a) if is_arguments(a.kind()) => a,
        _ => return CompletionContext::None,
    };

    let call = match arg_list.parent() {
        Some(c) if is_call(c.kind()) => c,
        _ => return CompletionContext::None,
    };

    let method_name = match extract_method_name(call, source) {
        Some(name) => name,
        None => return CompletionContext::None,
    };

    match_method_and_key(&method_name, arg_name)
}

/// Shared logic for matching method name + key/arg name to a context.
fn match_method_and_key(method: &str, key: &str) -> CompletionContext {
    if (is_trigger_method(method) || is_register_trigger_method(method)) && key == "function_id" {
        CompletionContext::FunctionId
    } else if is_register_trigger_method(method) && key == "type" {
        CompletionContext::TriggerType
    } else {
        CompletionContext::None
    }
}

// --- Method name extraction ---

fn extract_method_name(call: Node, source: &str) -> Option<String> {
    let func = call.child_by_field_name("function")?;

    if is_method_access(func.kind()) {
        let field_name = match func.kind() {
            "member_expression" => "property",
            "field_expression" => "field",
            _ => "attribute", // Python
        };
        let prop = func.child_by_field_name(field_name)?;
        return Some(prop.utf8_text(source.as_bytes()).ok()?.to_string());
    }

    if func.kind() == "identifier" {
        return Some(func.utf8_text(source.as_bytes()).ok()?.to_string());
    }

    None
}

// --- Rust struct field context ---
// Chain: string_literal → ... → field_initializer → field_initializer_list → struct_expression
// Rust SDK uses struct literals: TriggerRequest { function_id: "x".to_string() }

fn determine_context_rust_field(string_node: Node, source: &str) -> CompletionContext {
    // Walk up from string to find the field_initializer (may pass through .to_string() call)
    let mut current = string_node;
    let field_init = loop {
        match current.parent() {
            Some(p) if p.kind() == "field_initializer" => break p,
            // Stop if we've gone too far up
            Some(p) if p.kind() == "field_initializer_list" || p.kind() == "struct_expression" => {
                return CompletionContext::None
            }
            Some(p) => current = p,
            None => return CompletionContext::None,
        }
    };

    let field_name_node = match field_init.child_by_field_name("field") {
        Some(f) => f,
        None => return CompletionContext::None,
    };
    let field_text = field_name_node.utf8_text(source.as_bytes()).unwrap_or("");

    // field_initializer → field_initializer_list → struct_expression
    let field_list = match field_init.parent() {
        Some(p) if p.kind() == "field_initializer_list" => p,
        _ => return CompletionContext::None,
    };

    let struct_expr = match field_list.parent() {
        Some(p) if p.kind() == "struct_expression" => p,
        _ => return CompletionContext::None,
    };

    let struct_name_node = match struct_expr.child_by_field_name("name") {
        Some(n) => n,
        None => return CompletionContext::None,
    };
    let struct_name = struct_name_node.utf8_text(source.as_bytes()).unwrap_or("");

    // Match struct type + field name to context
    match (struct_name, field_text) {
        ("TriggerRequest", "function_id") => CompletionContext::FunctionId,
        ("RegisterTriggerInput", "function_id") => CompletionContext::FunctionId,
        ("RegisterTriggerInput", "trigger_type") => CompletionContext::TriggerType,
        _ => CompletionContext::None,
    }
}

// --- Known value fields ---

const KNOWN_VALUE_FIELDS: &[&str] = &["stream_name", "topic", "api_path", "scope", "queue"];

fn check_known_value_field(string_node: Node, source: &str) -> Option<CompletionContext> {
    // Walk up to find the container (may pass through .to_string() in Rust)
    let mut current = string_node;
    loop {
        let parent = current.parent()?;

        // Check pair style (TS objects / Python dicts)
        if parent.kind() == "pair" {
            let key = parent.child_by_field_name("key")?;
            let key_text = strip_quotes(key.utf8_text(source.as_bytes()).ok()?);
            if KNOWN_VALUE_FIELDS.contains(&key_text.as_str()) {
                return Some(CompletionContext::KnownValue {
                    field_name: key_text,
                });
            }
            return None;
        }

        // Check keyword_argument style (Python kwargs)
        if parent.kind() == "keyword_argument" {
            let name = parent.child_by_field_name("name")?;
            let arg_name = name.utf8_text(source.as_bytes()).ok()?;
            if KNOWN_VALUE_FIELDS.contains(&arg_name) {
                return Some(CompletionContext::KnownValue {
                    field_name: arg_name.to_string(),
                });
            }
            return None;
        }

        // Check field_initializer style (Rust structs)
        if parent.kind() == "field_initializer" {
            let field = parent.child_by_field_name("field")?;
            let field_text = field.utf8_text(source.as_bytes()).ok()?;
            if KNOWN_VALUE_FIELDS.contains(&field_text) {
                return Some(CompletionContext::KnownValue {
                    field_name: field_text.to_string(),
                });
            }
            return None;
        }

        // Stop at container boundaries
        if parent.kind() == "field_initializer_list"
            || is_object(parent.kind())
            || is_arguments(parent.kind())
        {
            return None;
        }

        current = parent;
    }
}

// --- Object/dict context (payload properties, trigger config properties) ---

fn determine_object_context(node: Node, source: &str) -> CompletionContext {
    let mut current = node;
    loop {
        if is_object(current.kind()) {
            // Try pair style: object → pair → outer_object → arguments → call
            if let Some(ctx) = check_nested_object_pair(current, source) {
                return ctx;
            }
            // Try kwarg style: object → keyword_argument → argument_list → call
            if let Some(ctx) = check_nested_object_kwarg(current, source) {
                return ctx;
            }
        }

        match current.parent() {
            Some(p) => current = p,
            None => break,
        }
    }
    CompletionContext::None
}

/// Object is value of a pair in a dict: object → pair → outer_object → arguments → call
fn check_nested_object_pair(object: Node, source: &str) -> Option<CompletionContext> {
    let pair = object.parent()?;
    if pair.kind() != "pair" {
        return None;
    }

    let key = pair.child_by_field_name("key")?;
    let key_text = strip_quotes(key.utf8_text(source.as_bytes()).ok()?);

    let outer_object = pair.parent()?;
    if !is_object(outer_object.kind()) {
        return None;
    }

    let arguments = outer_object.parent()?;
    if !is_arguments(arguments.kind()) {
        return None;
    }

    let call = arguments.parent()?;
    if !is_call(call.kind()) {
        return None;
    }

    let method_name = extract_method_name(call, source)?;

    match (method_name.as_str(), key_text.as_str()) {
        (m, "payload" | "data") if is_trigger_method(m) => {
            let fid = find_string_value_in_object(outer_object, "function_id", source)?;
            Some(CompletionContext::PayloadProperty { function_id: fid })
        }
        (m, "config") if is_register_trigger_method(m) => {
            let tt = find_string_value_in_object(outer_object, "type", source)?;
            Some(CompletionContext::TriggerConfigProperty { trigger_type: tt })
        }
        _ => None,
    }
}

/// Object is value of a keyword argument: dict → keyword_argument → argument_list → call
fn check_nested_object_kwarg(object: Node, source: &str) -> Option<CompletionContext> {
    let kwarg = object.parent()?;
    if kwarg.kind() != "keyword_argument" {
        return None;
    }

    let name = kwarg.child_by_field_name("name")?;
    let key_text = name.utf8_text(source.as_bytes()).ok()?;

    let arg_list = kwarg.parent()?;
    if !is_arguments(arg_list.kind()) {
        return None;
    }

    let call = arg_list.parent()?;
    if !is_call(call.kind()) {
        return None;
    }

    let method_name = extract_method_name(call, source)?;

    match (method_name.as_str(), key_text) {
        (m, "payload" | "data") if is_trigger_method(m) => {
            let fid = find_kwarg_string_value(arg_list, "function_id", source)?;
            Some(CompletionContext::PayloadProperty { function_id: fid })
        }
        (m, "config") if is_register_trigger_method(m) => {
            let tt = find_kwarg_string_value(arg_list, "type", source)?;
            Some(CompletionContext::TriggerConfigProperty { trigger_type: tt })
        }
        _ => None,
    }
}

// --- Value extraction helpers ---

/// Find a string value for a key in an object/dictionary node (pair children).
fn find_string_value_in_object(object: Node, target_key: &str, source: &str) -> Option<String> {
    let mut cursor = object.walk();
    for child in object.children(&mut cursor) {
        if child.kind() == "pair" {
            if let Some(key) = child.child_by_field_name("key") {
                let key_text = strip_quotes(key.utf8_text(source.as_bytes()).ok()?);
                if key_text == target_key {
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

/// Find a string value from a keyword_argument sibling in an argument_list.
fn find_kwarg_string_value(arg_list: Node, target_name: &str, source: &str) -> Option<String> {
    let mut cursor = arg_list.walk();
    for child in arg_list.children(&mut cursor) {
        if child.kind() == "keyword_argument" {
            if let Some(name) = child.child_by_field_name("name") {
                if name.utf8_text(source.as_bytes()).ok()? == target_name {
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

/// Strip surrounding quotes from a string (Python dict keys are quoted).
fn strip_quotes(s: &str) -> String {
    s.trim_matches('\'').trim_matches('"').to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    // --- TypeScript tests ---

    #[test]
    fn ts_trigger_function_id() {
        let source = r#"iii.trigger({ function_id: 'todos::create' })"#;
        let result = analyze(source, pos(0, 28), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "todos::create");
    }

    #[test]
    fn ts_trigger_function_id_empty() {
        let source = r#"iii.trigger({ function_id: '' })"#;
        let result = analyze(source, pos(0, 27), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn ts_register_trigger_type() {
        let source = r#"iii.registerTrigger({ type: 'http', function_id: 'greet' })"#;
        let result = analyze(source, pos(0, 29), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::TriggerType);
    }

    #[test]
    fn ts_register_trigger_function_id() {
        let source = r#"iii.registerTrigger({ type: 'http', function_id: 'greet' })"#;
        let result = analyze(source, pos(0, 50), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn ts_not_in_completable_position() {
        let source = r#"const name = 'hello world';"#;
        let result = analyze(source, pos(0, 15), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::None);
    }

    #[test]
    fn ts_cursor_in_comment() {
        let source = r#"// iii.trigger({ function_id: 'test' })"#;
        let result = analyze(source, pos(0, 31), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::None);
    }

    #[test]
    fn ts_await_trigger() {
        let source = r#"await iii.trigger({ function_id: 'test' })"#;
        let result = analyze(source, pos(0, 34), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn ts_multiline_trigger() {
        let source = "iii.trigger({\n  function_id: 'todos::create',\n  payload: {}\n})";
        let result = analyze(source, pos(1, 17), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn ts_nested_object_not_completable() {
        let source = r#"iii.trigger({ options: { function_id: 'test' } })"#;
        let result = analyze(source, pos(0, 39), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::None);
    }

    #[test]
    fn ts_double_quoted_string() {
        let source = r#"iii.trigger({ function_id: "todos::create" })"#;
        let result = analyze(source, pos(0, 28), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn ts_unclosed_string() {
        let source = "iii.trigger({function_id: '";
        let result = analyze(source, pos(0, 27), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn ts_unclosed_string_partial() {
        let source = "iii.trigger({function_id: 'to";
        let result = analyze(source, pos(0, 29), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "to");
    }

    #[test]
    fn ts_payload_property() {
        let source = "iii.trigger({ function_id: 'todos::create', payload: {} })";
        let result = analyze(source, pos(0, 54), Language::TypeScript);
        assert_eq!(
            result.context,
            CompletionContext::PayloadProperty {
                function_id: "todos::create".to_string()
            }
        );
    }

    #[test]
    fn ts_trigger_config() {
        let source = "iii.registerTrigger({ type: 'http', function_id: 'x', config: {} })";
        let result = analyze(source, pos(0, 63), Language::TypeScript);
        assert_eq!(
            result.context,
            CompletionContext::TriggerConfigProperty {
                trigger_type: "http".to_string()
            }
        );
    }

    #[test]
    fn ts_known_value_stream_name() {
        let source = "iii.registerTrigger({ type: 'stream', function_id: 'x', config: { stream_name: '' } })";
        let result = analyze(source, pos(0, 80), Language::TypeScript);
        assert_eq!(
            result.context,
            CompletionContext::KnownValue {
                field_name: "stream_name".to_string()
            }
        );
    }

    // --- Python dict-style tests ---

    #[test]
    fn py_trigger_function_id_dict() {
        let source = "iii.trigger({'function_id': 'todos::create'})";
        let result = analyze(source, pos(0, 29), Language::Python);
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "todos::create");
    }

    #[test]
    fn py_trigger_async_function_id_dict() {
        let source = "await iii.trigger_async({'function_id': 'todos::create'})";
        let result = analyze(source, pos(0, 41), Language::Python);
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "todos::create");
    }

    #[test]
    fn py_register_trigger_type_dict() {
        let source = "iii.register_trigger({'type': 'http', 'function_id': 'x'})";
        let result = analyze(source, pos(0, 31), Language::Python);
        assert_eq!(result.context, CompletionContext::TriggerType);
    }

    #[test]
    fn py_register_trigger_function_id_dict() {
        let source = "iii.register_trigger({'type': 'http', 'function_id': 'greet'})";
        let result = analyze(source, pos(0, 55), Language::Python);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn py_payload_dict() {
        let source = "iii.trigger({'function_id': 'x', 'payload': {}})";
        let result = analyze(source, pos(0, 45), Language::Python);
        assert_eq!(
            result.context,
            CompletionContext::PayloadProperty {
                function_id: "x".to_string()
            }
        );
    }

    #[test]
    fn py_payload_async_dict() {
        let source = "await iii.trigger_async({'function_id': 'x', 'payload': {}})";
        let result = analyze(source, pos(0, 57), Language::Python);
        assert_eq!(
            result.context,
            CompletionContext::PayloadProperty {
                function_id: "x".to_string()
            }
        );
    }

    #[test]
    fn py_trigger_config_dict() {
        let source = "iii.register_trigger({'type': 'http', 'function_id': 'x', 'config': {}})";
        let result = analyze(source, pos(0, 69), Language::Python);
        assert_eq!(
            result.context,
            CompletionContext::TriggerConfigProperty {
                trigger_type: "http".to_string()
            }
        );
    }

    // --- Python keyword argument tests ---

    #[test]
    fn py_trigger_function_id_kwarg() {
        let source = "iii.trigger(function_id='todos::create')";
        let result = analyze(source, pos(0, 25), Language::Python);
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "todos::create");
    }

    #[test]
    fn py_trigger_async_function_id_kwarg() {
        let source = "await iii.trigger_async(function_id='todos::create')";
        let result = analyze(source, pos(0, 37), Language::Python);
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "todos::create");
    }

    #[test]
    fn py_register_trigger_type_kwarg() {
        let source = "iii.register_trigger(type='http', function_id='x')";
        let result = analyze(source, pos(0, 27), Language::Python);
        assert_eq!(result.context, CompletionContext::TriggerType);
    }

    #[test]
    fn py_payload_kwarg() {
        let source = "iii.trigger(function_id='x', payload={})";
        let result = analyze(source, pos(0, 38), Language::Python);
        assert_eq!(
            result.context,
            CompletionContext::PayloadProperty {
                function_id: "x".to_string()
            }
        );
    }

    #[test]
    fn py_trigger_config_kwarg() {
        let source = "iii.register_trigger(type='http', function_id='x', config={})";
        let result = analyze(source, pos(0, 59), Language::Python);
        assert_eq!(
            result.context,
            CompletionContext::TriggerConfigProperty {
                trigger_type: "http".to_string()
            }
        );
    }

    #[test]
    fn py_known_value_dict() {
        // Known value inside a dict-style config with non-empty string
        let source = "iii.register_trigger({'type': 'stream', 'function_id': 'x', 'config': {'stream_name': 'users'}})";
        // Cursor inside 'users'
        let result = analyze(source, pos(0, 88), Language::Python);
        assert_eq!(
            result.context,
            CompletionContext::KnownValue {
                field_name: "stream_name".to_string()
            }
        );
    }

    #[test]
    fn py_not_in_completable_position() {
        let source = "x = 'hello world'";
        let result = analyze(source, pos(0, 6), Language::Python);
        assert_eq!(result.context, CompletionContext::None);
    }

    #[test]
    fn py_comment_not_completable() {
        let source = "# iii.trigger(function_id='test')";
        let result = analyze(source, pos(0, 27), Language::Python);
        assert_eq!(result.context, CompletionContext::None);
    }

    // --- Rust tests ---

    #[test]
    fn rs_trigger_function_id() {
        let source = r#"iii.trigger(TriggerRequest { function_id: "todos::create".to_string(), payload: json!({}), action: None, timeout_ms: None })"#;
        // Cursor inside "todos::create"
        let result = analyze(source, pos(0, 43), Language::Rust);
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "todos::create");
    }

    #[test]
    fn rs_register_trigger_type() {
        let source = r#"iii.register_trigger(RegisterTriggerInput { trigger_type: "http".to_string(), function_id: "x".to_string(), config: json!({}), metadata: None })"#;
        let result = analyze(source, pos(0, 59), Language::Rust);
        assert_eq!(result.context, CompletionContext::TriggerType);
        assert_eq!(result.current_text, "http");
    }

    #[test]
    fn rs_register_trigger_function_id() {
        let source = r#"iii.register_trigger(RegisterTriggerInput { trigger_type: "http".to_string(), function_id: "greet".to_string(), config: json!({}), metadata: None })"#;
        let result = analyze(source, pos(0, 92), Language::Rust);
        assert_eq!(result.context, CompletionContext::FunctionId);
        assert_eq!(result.current_text, "greet");
    }

    #[test]
    fn rs_not_in_completable_position() {
        let source = r#"let name = "hello";"#;
        let result = analyze(source, pos(0, 13), Language::Rust);
        assert_eq!(result.context, CompletionContext::None);
    }

    #[test]
    fn rs_comment_not_completable() {
        let source = r#"// iii.trigger(TriggerRequest { function_id: "test" })"#;
        let result = analyze(source, pos(0, 46), Language::Rust);
        assert_eq!(result.context, CompletionContext::None);
    }

    // --- Real multi-line file tests (the actual scenario from the editor) ---

    #[test]
    fn ts_real_file_unclosed_string() {
        // Simulates a real TS file where the user is typing on one line
        let source = "import { iii } from './iii'\n\nconst result = await iii.trigger({ function_id: '\n\nconst x = 1;\n";
        // Cursor after the quote on line 2
        let result = analyze(source, pos(2, 50), Language::TypeScript);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn py_real_file_unclosed_string_dict() {
        // Line 2: "result = iii.trigger({'function_id': '" = 38 chars
        let source = "from iii import iii\n\nresult = iii.trigger({'function_id': '\n\nx = 1\n";
        let result = analyze(source, pos(2, 38), Language::Python);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn py_real_file_unclosed_string_kwarg() {
        // Line 2: "result = iii.trigger(function_id='" = 34 chars
        let source = "from iii import iii\n\nresult = iii.trigger(function_id='\n\nx = 1\n";
        let result = analyze(source, pos(2, 34), Language::Python);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn py_real_file_unclosed_string_async_dict() {
        // Line 2: "result = await iii.trigger_async({'function_id': '" = 50 chars
        let source =
            "from iii import iii\n\nresult = await iii.trigger_async({'function_id': '\n\nx = 1\n";
        let result = analyze(source, pos(2, 50), Language::Python);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }

    #[test]
    fn py_real_file_unclosed_string_async_kwarg() {
        // Line 2: "result = await iii.trigger_async(function_id='" = 46 chars
        let source =
            "from iii import iii\n\nresult = await iii.trigger_async(function_id='\n\nx = 1\n";
        let result = analyze(source, pos(2, 46), Language::Python);
        assert_eq!(result.context, CompletionContext::FunctionId);
    }
}
