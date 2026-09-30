//! Source units: the declarations evidence selection asks about, with the
//! comment ranges excerpts grow over.
//!
//! Ported from dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit
//! 82ef1fd: `packages/core/src/source.ts` `inspect`,
//! `parser-declarations.mjs` (Go, Rust) and the inspection parts of
//! `parser-helpers.mjs` (Python: `inspectPython`, `validPython`).
//!
//! Deviations: TypeScript and JavaScript units come from tree-sitter (the
//! TSX grammar for the JavaScript family), not the TypeScript compiler,
//! applying its rules to the equivalent nodes; the grammars are the lsp
//! worker's 0.23 pins, not jevgrep's 0.24/0.25 builds; Python names are not NFKC
//! normalized.

use tree_sitter::{Node, Parser, Tree};

use super::prompts::SourceRange;
use super::walk::{self, Unit};

/// Larger sources are inspected as text.
pub const MAX_PARSE_BYTES: usize = 1_000_000;

#[derive(Debug, Clone)]
pub struct Inspection {
    pub units: Vec<Unit>,
    /// Comment line ranges, sorted and unique.
    pub comments: Vec<SourceRange>,
    /// No parse: the units are `source` chunks.
    pub text: bool,
}

struct Decl {
    name: String,
    range: SourceRange,
    owner_headers: Vec<SourceRange>,
}

#[derive(Clone, Copy, PartialEq)]
enum Language {
    Python,
    TypeScript,
    Tsx,
    Go,
    Rust,
}

fn language(path: &str) -> Option<Language> {
    match walk::extname(path) {
        ".py" | ".pyi" => Some(Language::Python),
        ".ts" | ".mts" | ".cts" => Some(Language::TypeScript),
        ".tsx" | ".jsx" | ".js" | ".mjs" | ".cjs" => Some(Language::Tsx),
        ".go" => Some(Language::Go),
        ".rs" => Some(Language::Rust),
        _ => None,
    }
}

/// A path whose truncated previews carry a declaration index.
pub fn supported(path: &str) -> bool {
    language(path).is_some()
}

fn range(start_line: usize, end_line: usize) -> SourceRange {
    SourceRange {
        start_line,
        end_line,
    }
}

/// source.ts `inspect`: declarations of at most `max_unit_bytes` (larger
/// ones become partial text chunks), or `source` chunks when the language
/// is unsupported, the source is over `max_parse_bytes` or does not parse.
pub fn inspect(
    path: &str,
    source: &str,
    max_unit_bytes: usize,
    max_parse_bytes: usize,
) -> Inspection {
    let line_count = source.split('\n').count();
    let language = language(path);
    // Context windows use conservative whole-line Python comments, including
    // inside multiline strings.
    let python_comments: Vec<SourceRange> = if language == Some(Language::Python) {
        source
            .split('\n')
            .enumerate()
            .filter(|(_, line)| line.trim_start().starts_with('#'))
            .map(|(index, _)| range(index + 1, index + 1))
            .collect()
    } else {
        Vec::new()
    };
    let chunks = || {
        if source.is_empty() {
            Vec::new()
        } else {
            walk::text_units(source, 1, line_count, "source", max_unit_bytes, true)
        }
    };
    let fallback = |comments| Inspection {
        units: chunks(),
        comments,
        text: true,
    };
    if source.len() > max_parse_bytes {
        return fallback(python_comments);
    }
    let (decls, comments) = match language {
        None => return fallback(python_comments),
        Some(Language::Python) => match python(source) {
            Some(decls) => (decls, python_comments),
            None => return fallback(python_comments),
        },
        Some(language @ (Language::TypeScript | Language::Tsx)) => {
            match typescript(source, language == Language::Tsx) {
                (Some(decls), comments) => (decls, comments),
                // Invalid declarations fall back to text, but comments still
                // own the context-window boundaries.
                (None, comments) => return fallback(unique(comments)),
            }
        }
        Some(language) => match declarations(source, language == Language::Go) {
            Some(parsed) => parsed,
            None => return fallback(python_comments),
        },
    };
    let comments = unique(comments);
    if decls.is_empty() && !source.is_empty() {
        return Inspection {
            units: chunks(),
            comments,
            text: false,
        };
    }
    // Parser ranges and returned byte spans stay tied to the original bytes.
    let offsets = line_offsets(source);
    let at = |line: usize| {
        source
            .len()
            .min(offsets.get(line).copied().unwrap_or(usize::MAX))
    };
    let units = decls
        .into_iter()
        .flat_map(|decl| {
            let start = at(decl.range.start_line - 1);
            let end = at(decl.range.end_line);
            if end.saturating_sub(start) <= max_unit_bytes {
                return vec![Unit {
                    name: decl.name,
                    start_line: decl.range.start_line,
                    end_line: decl.range.end_line,
                    byte_start: start,
                    byte_end: end.max(start),
                    partial: false,
                    owner_headers: decl.owner_headers,
                }];
            }
            let mut parts = walk::text_units(
                source,
                decl.range.start_line,
                decl.range.end_line,
                &decl.name,
                max_unit_bytes,
                true,
            );
            for part in &mut parts {
                part.owner_headers = decl.owner_headers.clone();
            }
            parts
        })
        .collect();
    Inspection {
        units,
        comments,
        text: false,
    }
}

/// Byte offset of each line start, plus one past the last line.
fn line_offsets(source: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for line in source.split('\n') {
        offsets.push(offsets[offsets.len() - 1] + line.len() + 1);
    }
    offsets
}

fn unique(mut comments: Vec<SourceRange>) -> Vec<SourceRange> {
    let mut seen = std::collections::HashSet::new();
    comments.retain(|c| seen.insert((c.start_line, c.end_line)));
    comments.sort_by_key(|c| c.start_line);
    comments
}

fn parse(source: &str, language: tree_sitter::Language) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&language).ok()?;
    parser.parse(source, None)
}

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or_default()
}

fn field<'a>(node: Node<'_>, name: &str, source: &'a str) -> Option<&'a str> {
    node.child_by_field_name(name).map(|n| text(n, source))
}

/// parser-declarations.mjs `range`.
fn node_range(node: Node<'_>) -> SourceRange {
    let end = node.end_position();
    range(
        node.start_position().row + 1,
        end.row + usize::from(end.column > 0),
    )
}

fn is_comment(node: Node<'_>) -> bool {
    node.kind() == "comment" || node.kind().ends_with("_comment")
}

fn is_attribute(node: Node<'_>) -> bool {
    matches!(node.kind(), "attribute_item" | "inner_attribute_item")
}

/// Every comment node, anywhere.
fn comment_nodes(root: Node<'_>) -> Vec<Node<'_>> {
    let (mut comments, mut stack) = (Vec::new(), vec![root]);
    while let Some(node) = stack.pop() {
        if is_comment(node) {
            comments.push(node);
        } else {
            stack.extend(named(node));
        }
    }
    comments
}

/// parser-declarations.mjs `declarations`: Go and Rust units, `None` on a
/// syntax error.
fn declarations(source: &str, go: bool) -> Option<(Vec<Decl>, Vec<SourceRange>)> {
    let language = if go {
        tree_sitter_go::LANGUAGE.into()
    } else {
        tree_sitter_rust::LANGUAGE.into()
    };
    let tree = parse(source, language)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let comments = comment_nodes(root).into_iter().map(node_range).collect();
    let mut units = Vec::new();
    if go {
        for node in named(root) {
            if is_comment(node) {
                continue;
            }
            let mut name = field(node, "name", source)
                .unwrap_or(node.kind())
                .to_string();
            match node.kind() {
                "method_declaration" => {
                    let receiver = node.child_by_field_name("receiver").and_then(|list| {
                        named(list)
                            .into_iter()
                            .find(|c| c.kind() == "parameter_declaration")
                    });
                    let owner = receiver
                        .and_then(|r| field(r, "type", source))
                        .map(|t| t.strip_prefix('*').unwrap_or(t));
                    if let Some(owner) = owner.filter(|o| !o.is_empty()) {
                        name = format!("{owner}.{name}");
                    }
                }
                // Keep groups intact: iota and omitted constant values
                // depend on earlier specs.
                "type_declaration" | "var_declaration" | "const_declaration" => {
                    let mut names = Vec::new();
                    for child in named(node) {
                        let specs = if child.kind() == "var_spec_list" {
                            named(child)
                        } else {
                            vec![child]
                        };
                        for spec in specs {
                            let mut cursor = spec.walk();
                            names.extend(
                                spec.children_by_field_name("name", &mut cursor)
                                    .filter(|n| {
                                        matches!(n.kind(), "identifier" | "type_identifier")
                                    })
                                    .map(|n| text(n, source)),
                            );
                        }
                    }
                    if !names.is_empty() {
                        name = names.join(", ");
                    }
                }
                "package_clause" => {
                    let package = named(node)
                        .into_iter()
                        .find(|c| c.kind() == "package_identifier")
                        .map(|n| text(n, source))
                        .unwrap_or_default();
                    name = format!("package {package}");
                }
                _ => {}
            }
            units.push(Decl {
                name,
                range: node_range(node),
                owner_headers: Vec::new(),
            });
        }
    } else {
        rust_visit(&named(root), "", &[], source, &mut units);
    }
    Some((units, comments))
}

/// parser-declarations.mjs `startWithAttributes`: a Rust item starts at its
/// attributes and the comments among them, never at the previous item's
/// trailing comment.
fn start_with_attributes(node: Node<'_>) -> usize {
    let mut start = node;
    while let Some(previous) = start.prev_named_sibling() {
        if !(previous.kind() == "attribute_item" || is_comment(previous)) {
            break;
        }
        let trailing = is_comment(previous)
            && previous.prev_named_sibling().is_some_and(|before| {
                !is_comment(before)
                    && before.kind() != "attribute_item"
                    && before.end_position().row == previous.start_position().row
            });
        if trailing {
            break;
        }
        start = previous;
    }
    node_range(start).start_line
}

fn rust_visit(
    nodes: &[Node<'_>],
    prefix: &str,
    headers: &[SourceRange],
    source: &str,
    units: &mut Vec<Decl>,
) {
    let mut owned = headers.to_vec();
    owned.extend(
        nodes
            .iter()
            .filter(|n| n.kind() == "inner_attribute_item")
            .map(|n| node_range(*n)),
    );
    for &node in nodes {
        if is_comment(node) || is_attribute(node) {
            continue;
        }
        let body = matches!(
            node.kind(),
            "impl_item" | "trait_item" | "mod_item" | "foreign_mod_item"
        )
        .then(|| node.child_by_field_name("body"))
        .flatten();
        let owner = ["name", "type", "macro"]
            .into_iter()
            .filter_map(|name| field(node, name, source))
            .find(|owner| !owner.is_empty())
            .unwrap_or(node.kind());
        let start_line = start_with_attributes(node);
        let members = body
            .map(named)
            .filter(|members| members.iter().any(|c| !is_comment(*c) && !is_attribute(*c)));
        if let (Some(body), Some(members)) = (body, members) {
            let header = range(start_line, body.start_position().row + 1);
            let mut own = owned.clone();
            own.push(header);
            units.push(Decl {
                name: format!("{prefix}{owner}.context"),
                range: header,
                owner_headers: own.clone(),
            });
            let prefix = if node.kind() == "foreign_mod_item" {
                prefix.to_string()
            } else {
                format!("{prefix}{owner}.")
            };
            rust_visit(&members, &prefix, &own, source, units);
        } else {
            units.push(Decl {
                name: format!("{prefix}{owner}"),
                range: range(start_line, node_range(node).end_line),
                owner_headers: owned.clone(),
            });
        }
    }
}

/// source.ts TypeScript branch: top-level statements, a class with members
/// as its `.context` header plus `Class.member` units. `None` on a syntax
/// error; the comments are returned either way.
fn typescript(source: &str, tsx: bool) -> (Option<Vec<Decl>>, Vec<SourceRange>) {
    let language = if tsx {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    let Some(tree) = parse(source, language) else {
        return (None, Vec::new());
    };
    let offsets = line_offsets(source);
    let line = |byte: usize| offsets.partition_point(|&o| o <= byte);
    let root = tree.root_node();
    let comments = comment_nodes(root)
        .into_iter()
        .map(|c| range(line(c.start_byte()), line(c.end_byte().max(1) - 1)))
        .collect();
    if root.has_error() {
        return (None, comments);
    }
    let mut units = Vec::new();
    for statement in named(root) {
        if !is_comment(statement) && statement.kind() != "hash_bang_line" {
            ts_add(
                statement,
                statement.start_byte(),
                "",
                &[],
                source,
                &line,
                &mut units,
            );
        }
    }
    (Some(units), comments)
}

/// The declaration an `export` or `declare` statement wraps.
fn ts_declaration(mut node: Node<'_>) -> Node<'_> {
    loop {
        let inner = match node.kind() {
            "export_statement" => node.child_by_field_name("declaration").or_else(|| {
                node.child_by_field_name("value")
                    .filter(|value| value.kind() == "class")
            }),
            "ambient_declaration" => node
                .named_child(0)
                .filter(|c| !matches!(c.kind(), "statement_block" | "property_identifier")),
            _ => None,
        };
        match inner {
            Some(inner) => node = inner,
            None => return node,
        }
    }
}

/// The TypeScript compiler's `name` of a declaration, or a variable
/// statement's declarator names.
fn ts_name(declaration: Node<'_>, source: &str) -> Option<String> {
    match declaration.kind() {
        // Index signatures and static blocks have no name; neither has a
        // constructor.
        "index_signature" | "class_static_block" => None,
        "method_definition" | "method_signature"
            if field(declaration, "name", source) == Some("constructor") =>
        {
            None
        }
        "lexical_declaration" | "variable_declaration" => {
            let names: Vec<&str> = named(declaration)
                .into_iter()
                .filter(|d| d.kind() == "variable_declarator")
                .filter_map(|d| field(d, "name", source))
                .collect();
            (!names.is_empty()).then(|| names.join(", "))
        }
        "expression_statement" => declaration
            .named_child(0)
            .filter(|c| c.kind() == "internal_module")
            .and_then(|m| field(m, "name", source))
            .map(str::to_string),
        _ => field(declaration, "name", source).map(str::to_string),
    }
    .filter(|name| !name.is_empty())
}

fn ts_add(
    node: Node<'_>,
    start: usize,
    prefix: &str,
    headers: &[SourceRange],
    source: &str,
    line: &dyn Fn(usize) -> usize,
    units: &mut Vec<Decl>,
) {
    let declaration = ts_declaration(node);
    let name = format!(
        "{prefix}{}",
        ts_name(declaration, source).as_deref().unwrap_or("source")
    );
    let start_line = line(start);
    let is_class = matches!(
        declaration.kind(),
        "class_declaration" | "abstract_class_declaration" | "class"
    );
    // A member starts at its decorators, which tree-sitter keeps as the
    // member's preceding siblings.
    let mut members = Vec::new();
    if let Some(body) = declaration.child_by_field_name("body").filter(|_| is_class) {
        let mut decorated = None;
        for child in named(body) {
            match child.kind() {
                "comment" => {}
                "decorator" => {
                    decorated.get_or_insert(child.start_byte());
                }
                _ => members.push((child, decorated.take().unwrap_or(child.start_byte()))),
            }
        }
    }
    let Some(&(_, first)) = members.first() else {
        units.push(Decl {
            name,
            range: range(
                start_line,
                line(start.max(node.end_byte().saturating_sub(1))),
            ),
            owner_headers: headers.to_vec(),
        });
        return;
    };
    let first = line(first);
    let mut own = headers.to_vec();
    if first > start_line {
        let header = range(start_line, first - 1);
        own.push(header);
        units.push(Decl {
            name: format!("{name}.context"),
            range: header,
            owner_headers: own.clone(),
        });
    }
    for (member, member_start) in members {
        ts_add(
            member,
            member_start,
            &format!("{name}."),
            &own,
            source,
            line,
            units,
        );
    }
}

/// parser-helpers.mjs `inspectPython`: `None` on a syntax error, a bare CR
/// (retrieval coordinates count LF lines) or Python 2 syntax.
fn python(source: &str) -> Option<Vec<Decl>> {
    let bytes = source.as_bytes();
    if bytes
        .iter()
        .enumerate()
        .any(|(i, b)| *b == b'\r' && bytes.get(i + 1) != Some(&b'\n'))
    {
        return None;
    }
    let tree = parse(source, tree_sitter_python::LANGUAGE.into())?;
    let root = tree.root_node();
    if root.has_error() || !valid_python(root, source) {
        return None;
    }
    let mut units = Vec::new();
    py_visit(&named(root), "", &[], source, &mut units);
    Some(units)
}

fn py_definition(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "decorated_definition" {
        node.child_by_field_name("definition")
    } else {
        Some(node)
    }
}

fn py_is_definition(node: Node<'_>) -> bool {
    py_definition(node)
        .is_some_and(|d| matches!(d.kind(), "function_definition" | "class_definition"))
}

fn py_body(node: Node<'_>) -> Vec<Node<'_>> {
    node.child_by_field_name("body")
        .map(named)
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.kind() != "comment")
        .collect()
}

fn unparenthesized(mut node: Option<Node<'_>>) -> Option<Node<'_>> {
    while let Some(n) = node.filter(|n| n.kind() == "parenthesized_expression") {
        node = named(n).into_iter().find(|c| c.kind() != "comment");
    }
    node
}

/// Python AST ends exclude trailing comments; tree-sitter blocks include
/// them.
fn py_end_line(node: Node<'_>) -> usize {
    let mut last = node;
    loop {
        let mut cursor = last.walk();
        let child = last
            .children(&mut cursor)
            .filter(|c| c.kind() != "comment")
            .last();
        match child {
            Some(child) => last = child,
            None => break,
        }
    }
    last.end_position().row + 1
}

fn py_start_line(node: Node<'_>) -> usize {
    if node.kind() == "decorated_definition" {
        let decorator = named(node).into_iter().find(|c| c.kind() == "decorator");
        let expression = unparenthesized(decorator.and_then(|d| d.named_child(0)));
        return expression
            .or(decorator)
            .unwrap_or(node)
            .start_position()
            .row
            + 1;
    }
    node.start_position().row + 1
}

fn py_visit(
    nodes: &[Node<'_>],
    prefix: &str,
    headers: &[SourceRange],
    source: &str,
    units: &mut Vec<Decl>,
) {
    for &wrapped in nodes {
        if !py_is_definition(wrapped) {
            continue;
        }
        let Some(node) = py_definition(wrapped) else {
            continue;
        };
        let name = format!("{prefix}{}", field(node, "name", source).unwrap_or(""));
        let (start, end) = (py_start_line(wrapped), py_end_line(wrapped));
        let children: Vec<Node<'_>> = py_body(node)
            .into_iter()
            .filter(|c| py_is_definition(*c))
            .collect();
        if node.kind() != "class_definition" || children.is_empty() {
            units.push(Decl {
                name,
                range: range(start, end),
                owner_headers: headers.to_vec(),
            });
            continue;
        }
        let first = py_start_line(children[0]);
        let mut own = headers.to_vec();
        if first > start {
            own.push(range(start, first - 1));
        }
        let context = |from: usize, to: usize| Decl {
            name: format!("{name}.context"),
            range: range(from, to),
            owner_headers: own.clone(),
        };
        let mut cursor = start;
        for child in children {
            let child_start = py_start_line(child);
            if cursor < child_start {
                units.push(context(cursor, child_start - 1));
            }
            py_visit(&[child], &format!("{name}."), &own, source, units);
            cursor = py_end_line(child) + 1;
        }
        if cursor <= end {
            units.push(context(cursor, end));
        }
    }
}

/// parser-helpers.mjs `validPython`: tree-sitter also accepts some Python 2
/// syntax; reject those forms, accept modern Python.
fn valid_python(root: Node<'_>, source: &str) -> bool {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        stack.extend(named(n));
        let count = |field: &str| {
            let mut cursor = n.walk();
            let count = n.children_by_field_name(field, &mut cursor).count();
            count
        };
        let first_kind = n.named_child(0).map(|c| c.kind());
        let own = text(n, source);
        let invalid = match n.kind() {
            "exec_statement" => true,
            "print_statement" => first_kind != Some("chevron"),
            // tree-sitter-python 0.23 reads `except E, e` as `value , alias`
            // (jevgrep's 0.25 as two values).
            "except_clause" => {
                count("value") > 1 || {
                    let mut cursor = n.walk();
                    let comma = n.children(&mut cursor).any(|c| c.kind() == ",");
                    comma
                }
            }
            "raise_statement" => first_kind == Some("expression_list"),
            "for_in_clause" => count("right") > 1,
            "concatenated_string" => {
                let bytes: Vec<bool> = named(n)
                    .into_iter()
                    .filter(|c| c.kind() == "string")
                    .map(|c| {
                        text(c, source)
                            .trim_start_matches(['r', 'u', 'R', 'U'])
                            .starts_with(['b', 'B'])
                    })
                    .collect();
                bytes.iter().any(|b| *b) && bytes.iter().any(|b| !*b)
            }
            "function_definition" | "class_definition" => py_body(n).is_empty(),
            "integer" => {
                own.ends_with(['l', 'L'])
                    || (own.len() > 1
                        && own.starts_with('0')
                        && own[1..].bytes().all(|b| b.is_ascii_digit() || b == b'_')
                        && own[1..].bytes().any(|b| (b'1'..=b'9').contains(&b)))
            }
            "comparison_operator" => {
                let mut cursor = n.walk();
                let diamond = n.children(&mut cursor).any(|c| text(c, source) == "<>");
                diamond
            }
            "string_start" => {
                let lower = own.to_ascii_lowercase();
                lower.starts_with("ur") || lower.starts_with("ru")
            }
            "string" => own.starts_with('`'),
            "parameters" | "lambda_parameters" => named(n).into_iter().any(|c| {
                c.kind() == "tuple_pattern"
                    || c.child_by_field_name("name")
                        .is_some_and(|name| name.kind() == "tuple_pattern")
            }),
            "delete_statement" => named(n).into_iter().any(|c| {
                !unparenthesized(Some(c)).is_some_and(|t| {
                    matches!(
                        t.kind(),
                        "identifier"
                            | "attribute"
                            | "subscript"
                            | "expression_list"
                            | "tuple"
                            | "list"
                    )
                })
            }),
            _ => false,
        };
        if invalid {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(inspection: &Inspection) -> Vec<(&str, usize, usize)> {
        inspection
            .units
            .iter()
            .map(|u| (u.name.as_str(), u.start_line, u.end_line))
            .collect()
    }

    fn inspect_all(path: &str, source: &str) -> Inspection {
        inspect(path, source, source.len().max(4), MAX_PARSE_BYTES)
    }

    fn unit<'a>(inspection: &'a Inspection, name: &str) -> &'a Unit {
        inspection
            .units
            .iter()
            .find(|u| u.name == name)
            .unwrap_or_else(|| panic!("no unit {name}: {:?}", names(inspection)))
    }

    fn text<'a>(source: &'a str, unit: &Unit) -> &'a str {
        &source[unit.byte_start..unit.byte_end]
    }

    #[test]
    fn go_names_receiver_methods_and_keeps_declaration_groups() {
        let source = "package sample\r\nconst (\r\n First = iota\r\n Second\r\n)\r\ntype Box[T any] struct { value T }\r\nfunc (b *Box[T]) Read() T {\r\n // 🙂\r\n return b.value\r\n}\r\n";
        let result = inspect_all("sample.go", source);
        assert!(!result.text);
        let method = unit(&result, "Box[T].Read");
        assert_eq!(
            text(source, method),
            source
                .split("\r\n")
                .skip(6)
                .collect::<Vec<_>>()
                .join("\r\n")
        );
        let group = unit(&result, "First, Second");
        assert_eq!(
            text(source, group),
            "const (\r\n First = iota\r\n Second\r\n)\r\n"
        );
        unit(&result, "package sample");
        assert_eq!(result.comments, [range(8, 8)]);

        let source = "package x\nconst Alpha, Beta = 1, 2\ntype (\n Alias = string\n Value struct{}\n)\nvar First, Second int\nvar (\n Third int\n Fourth = 1\n)\nfunc K() {}\nfunc K() {}\n";
        let result = inspect_all("sample.go", source);
        for name in [
            "Alpha, Beta",
            "Alias, Value",
            "First, Second",
            "Third, Fourth",
        ] {
            unit(&result, name);
        }
        assert_eq!(result.units.iter().filter(|u| u.name == "K").count(), 2);
    }

    #[test]
    fn rust_units_carry_module_impl_and_attribute_headers() {
        let source = "#![allow(dead_code)]\r\n#[cfg(feature = \"demo\")]\r\nmod inner {\r\n #![allow(unused)]\r\n struct Box;\r\n #[cfg(feature = \"demo\")]\r\n impl Box {\r\n  /// Reads the value.\r\n  #[inline]\r\n  pub fn read(&self) -> &str { \"🙂\" }\r\n }\r\n}\r\n";
        let result = inspect_all("sample.rs", source);
        let method = unit(&result, "inner.Box.read");
        assert_eq!(
            method.owner_headers,
            [range(1, 1), range(2, 3), range(4, 4), range(6, 7)]
        );
        assert_eq!(
            text(source, method),
            "  /// Reads the value.\r\n  #[inline]\r\n  pub fn read(&self) -> &str { \"🙂\" }\r\n"
        );
        assert_eq!(
            (
                unit(&result, "inner.context").start_line,
                unit(&result, "inner.Box.context").start_line
            ),
            (2, 6)
        );

        // a comment between attribute and item stays with the item; the
        // previous item's trailing comment does not
        let source =
            "fn earlier() {} // about earlier\n#[inline]\n// about target\npub fn target() {}\n";
        let result = inspect_all("sample.rs", source);
        assert_eq!(
            text(source, unit(&result, "target")),
            "#[inline]\n// about target\npub fn target() {}\n"
        );

        let source = "mod inner {\n pub trait View {\n fn show(&self);\n }\n}\nextern \"C\" {\n fn foreign();\n static VALUE: u8;\n}\nmacro_rules! make { () => {} }\n";
        let result = inspect_all("sample.rs", source);
        for name in ["inner.View.show", "foreign", "VALUE", "make"] {
            unit(&result, name);
        }
    }

    #[test]
    fn typescript_classes_split_into_context_and_decorated_members() {
        let source = "/** header */\r\nexport class Box {\r\n  // getter\r\n  @trace\r\n  get value() { return \"🙂\"; }\r\n}\r\n";
        let result = inspect_all("box.ts", source);
        assert_eq!(names(&result), [("Box.context", 2, 3), ("Box.value", 4, 5)]);
        assert_eq!(result.comments, [range(1, 1), range(3, 3)]);
        assert_eq!(unit(&result, "Box.value").owner_headers, [range(2, 3)]);

        let source = "function x() {\n // end\n}\nconst a = 1, { b } = o;\nexport default 42;\nclass Same {\n  constructor() {}\n  first = 1;\n}\n";
        let result = inspect_all("comments.js", source);
        assert_eq!(
            names(&result),
            [
                ("x", 1, 3),
                ("a, { b }", 4, 4),
                ("source", 5, 5),
                ("Same.context", 6, 6),
                ("Same.source", 7, 7),
                ("Same.first", 8, 8),
            ]
        );
        assert_eq!(
            text(source, &result.units[0]),
            "function x() {\n // end\n}\n"
        );
    }

    #[test]
    fn python_classes_keep_decorators_context_gaps_and_nesting() {
        let source = "# module\n@decorator\nclass Café:\n    \"\"\"docs\"\"\"\n    value = 1\n    @property\n    def first(self):\n        def nested():\n            return \"é\"\n        return nested()\n\n    class Inner:\n        def method(self):\n            pass\n    tail = 2\n";
        let result = inspect_all("sample.py", source);
        assert!(!result.text);
        assert_eq!(
            names(&result),
            [
                ("Café.context", 2, 5),
                ("Café.first", 6, 10),
                ("Café.context", 11, 11),
                ("Café.Inner.context", 12, 12),
                ("Café.Inner.method", 13, 14),
                ("Café.context", 15, 15),
            ]
        );
        assert_eq!(result.comments, [range(1, 1)]);
        assert_eq!(
            unit(&result, "Café.Inner.method").owner_headers,
            [range(2, 5), range(12, 12)]
        );
        // trailing comments stay out of declaration ends
        let result = inspect_all(
            "a.py",
            "class A:\n    def x(self):\n        pass\n    # tail\n",
        );
        assert_eq!(names(&result), [("A.context", 1, 1), ("A.x", 2, 3)]);
        assert_eq!(result.comments, [range(4, 4)]);
    }

    #[test]
    fn syntax_errors_python_2_bare_cr_size_and_unknown_languages_are_text() {
        for (path, source) in [
            ("bad.py", "def broken(:\n  return 2\n"),
            ("bad.ts", "const = ;"),
            ("bad.go", "package x\nfunc broken( { 🙂\n"),
            ("bad.rs", "fn broken( { 🙂\n"),
            ("old.py", "def target():\n    print \"old\"\n"),
            ("old.py", "value = left <> right\n"),
            ("old.py", "value = 0755\n"),
            ("old.py", "try:\n    pass\nexcept Exception, e:\n    pass\n"),
            ("cr.py", "def a():\r    return 1\n"),
            ("readme.md", "éé\r\nnext\r\nlast"),
        ] {
            let result = inspect(path, source, 8, MAX_PARSE_BYTES);
            assert!(result.text, "{source}");
            assert!(result.units.iter().all(|u| u.partial && u.name == "source"));
            let joined: String = result.units.iter().map(|u| text(source, u)).collect();
            assert_eq!(joined, source);
            assert_eq!(result.units[0].start_line, 1);
            assert_eq!(
                result.units.last().unwrap().end_line,
                source.split('\n').count()
            );
        }
        let big = inspect("big.rs", "fn target() {}\n", 8, 1);
        assert!(big.text);
        for source in [
            "print(\"new\")\n",
            "print >> stream, value\n",
            "value = 0o755 + 0xFF + 000 + 0_0\n",
            "raise Error(\"message\") from cause\n",
        ] {
            assert!(!inspect_all("modern.py", source).text, "{source}");
        }
        // a TypeScript syntax error keeps its comments
        let source = format!("/*\n{}*/\nconst broken = ;\n", "comment\n".repeat(100));
        let result = inspect("broken.ts", &source, 3000, MAX_PARSE_BYTES);
        assert!(result.text);
        assert_eq!(result.comments, [range(1, 102)]);
    }

    #[test]
    fn oversized_declarations_split_into_partial_chunks_with_their_headers() {
        let body = "        x = 1\n".repeat(20);
        let source = format!("class A:\n    def big(self):\n{body}");
        let result = inspect("a.py", &source, 64, MAX_PARSE_BYTES);
        let big: Vec<&Unit> = result.units.iter().filter(|u| u.name == "A.big").collect();
        assert!(big.len() > 1);
        assert!(big
            .iter()
            .all(|u| u.partial && u.owner_headers == [range(1, 1)]));
    }
}
