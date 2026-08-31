//! scrapling::to-markdown rendering: html/markdown/text extraction, with an
//! optional main-content sanitizer and CSS scope
//! (`Convertor._extract_content`, scrapling/core/shell.py, normative).

use std::collections::HashSet;

use xmloxide::tree::NodeKind;
use xmloxide::NodeId;

use crate::scrapling::dom::{self, Doc, ElementRef};
use crate::scrapling::query;

/// `_strip_noise_tags` + the tag half of `_sanitize_for_ai`'s `_HIDDEN_XPATH`
/// union: `<template>` is dropped the same way as the noise tags — folding
/// it into one name-set costs nothing since detaching is order-independent
/// (matching is by predicate only, never by "is a sibling already gone").
fn is_noise_tag(name: &str) -> bool {
    matches!(name, "script" | "style" | "noscript" | "svg" | "template")
}

/// The `contains(@style, ...)` half of `_HIDDEN_XPATH` (shell.py) — plain
/// substring tests, not CSS parsing.
const HIDDEN_STYLE_SUBSTRINGS: &[&str] = &[
    "display:none",
    "display: none",
    "visibility:hidden",
    "visibility: hidden",
    "opacity:0",
    "opacity: 0",
    "font-size:0",
    "font-size: 0",
    "height:0",
    "height: 0",
    "width:0",
    "width: 0",
];

/// `_ZWC_PATTERN` (shell.py): zero-width/invisible-formatting characters
/// stripped from text nodes by the prompt-injection sanitizer.
const ZERO_WIDTH_CHARS: [char; 6] = [
    '\u{200b}', '\u{200c}', '\u{200d}', '\u{feff}', '\u{2060}', '\u{180e}',
];

fn is_hidden(doc: &Doc, id: NodeId) -> bool {
    is_noise_tag(doc.tree.node_name(id).unwrap_or(""))
        || doc.tree.attribute(id, "aria-hidden") == Some("true")
        || doc
            .tree
            .attribute(id, "style")
            .is_some_and(|s| HIDDEN_STYLE_SUBSTRINGS.iter().any(|pat| s.contains(pat)))
}

/// `_strip_noise_tags` + `_sanitize_for_ai`, fused into one in-place pass over
/// a tree the caller already cloned (never called on the caller's own doc):
/// detach every noise/hidden/aria-hidden/template element (`drop_tree` —
/// lxml keeps a dropped element's TAIL text by splicing it onto the
/// predecessor, but html5ever/scraper never attaches trailing text to an
/// element in the first place — it's already an ordinary sibling text node —
/// so a plain `detach()` reproduces that splice for free), then strip
/// zero-width chars from every surviving text node.
///
/// `scope_id` (the element `render` is about to treat as the page — `body`,
/// or the whole document if there's no `body`) is exempt from the hidden
/// check: `_HIDDEN_XPATH` is rooted at `.//` (XPath descendant axis, self
/// excluded), so `<body aria-hidden="true">` or `<body style="display:
/// none">` never drops itself in Python, only matching *descendants* do.
/// Oracle-probed: both yield their content, not `""`. The name-based checks
/// (script/style/noscript/svg/template) need no such exemption in practice —
/// `scope_id` is always a `<body>` or the document's `<html>` root, never one
/// of those tag names — but exempting it from the whole predicate uniformly
/// is simpler than splitting the check in two, and is a no-op for that half.
pub(crate) fn sanitize_main_content(doc: &mut Doc, scope_id: NodeId) {
    let drop_ids: Vec<_> = doc
        .tree
        .descendants(doc.tree.root())
        .filter(|id| *id != scope_id && doc.tree.is_element(*id) && is_hidden(doc, *id))
        .collect();
    for id in drop_ids {
        doc.tree.remove_node(id);
    }

    let text_ids: Vec<_> = doc
        .tree
        .descendants(doc.tree.root())
        .filter(|id| matches!(doc.tree.node(*id).kind, NodeKind::Text { .. }))
        .collect();
    for id in text_ids {
        let cleaned: String = doc
            .tree
            .node_text(id)
            .unwrap_or("")
            .chars()
            .filter(|c| !ZERO_WIDTH_CHARS.contains(c))
            .collect();
        doc.tree.set_text_content(id, &cleaned);
    }
}

/// First `<body>` in document order, else the whole document's root element
/// (`page.css("body").first or page`, shell.py).
fn body_or_root(doc: &Doc) -> ElementRef<'_> {
    doc.first_by_tag("body").unwrap_or_else(|| doc.root())
}

/// Collapse runs of `\n`, then `\r`, then `\t`, then space — in that order,
/// each to a single occurrence (the `Convertor._extract_content` text-mode
/// loop, shell.py: sequential `re.sub(f"[{s}]+", s, ...)` per character).
/// markdownify's `all_whitespace_re` = `[\t \r\n]+` → single space. Crucially
/// ASCII-only: NBSP (and other Unicode spaces) are NOT collapsed, unlike
/// Rust's `split_whitespace`, which treats U+00A0 as whitespace.
fn collapse_ascii_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if matches!(ch, '\t' | ' ' | '\r' | '\n') {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out
}

fn collapse_whitespace(mut s: String) -> String {
    for c in ['\n', '\r', '\t', ' '] {
        let mut out = String::with_capacity(s.len());
        let mut prev_was_c = false;
        for ch in s.chars() {
            if ch == c {
                if prev_was_c {
                    continue;
                }
                prev_was_c = true;
            } else {
                prev_was_c = false;
            }
            out.push(ch);
        }
        s = out;
    }
    s
}

// The public wrapper exposes markdownify with no option overrides. This is a
// repository-owned port of markdownify 1.2.3's complete default conversion
// behavior. Its first step deliberately reparses the lxml-serialized subtree
// with the same tree shape as BeautifulSoup 4.15's `html.parser` builder.
// That second parse is observable for raw-text elements such as <plaintext>.

#[derive(Clone, Debug)]
enum MarkdownNodeKind {
    Document,
    Element {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Text(String),
    Comment,
    Doctype,
}

#[derive(Clone, Debug)]
struct MarkdownNode {
    kind: MarkdownNodeKind,
    parent: Option<usize>,
    children: Vec<usize>,
}

#[derive(Debug)]
struct MarkdownTree {
    nodes: Vec<MarkdownNode>,
}

impl MarkdownTree {
    fn new() -> Self {
        Self {
            nodes: vec![MarkdownNode {
                kind: MarkdownNodeKind::Document,
                parent: None,
                children: Vec::new(),
            }],
        }
    }

    fn append(&mut self, parent: usize, kind: MarkdownNodeKind) -> usize {
        let id = self.nodes.len();
        self.nodes.push(MarkdownNode {
            kind,
            parent: Some(parent),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }

    fn name(&self, id: usize) -> Option<&str> {
        match &self.nodes[id].kind {
            MarkdownNodeKind::Document => Some("[document]"),
            MarkdownNodeKind::Element { name, .. } => Some(name),
            _ => None,
        }
    }

    fn attr(&self, id: usize, name: &str) -> Option<&str> {
        match &self.nodes[id].kind {
            MarkdownNodeKind::Element { attrs, .. } => attrs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str()),
            _ => None,
        }
    }

    fn element_children_recursive(&self, id: usize, names: &[&str], out: &mut Vec<usize>) {
        for child in &self.nodes[id].children {
            if self.name(*child).is_some_and(|name| names.contains(&name)) {
                out.push(*child);
            }
            self.element_children_recursive(*child, names, out);
        }
    }

    fn sibling_index(&self, id: usize) -> Option<(usize, usize)> {
        let parent = self.nodes[id].parent?;
        let index = self.nodes[parent]
            .children
            .iter()
            .position(|candidate| *candidate == id)?;
        Some((parent, index))
    }

    fn previous_sibling(&self, id: usize) -> Option<usize> {
        let (parent, index) = self.sibling_index(id)?;
        index.checked_sub(1).map(|i| self.nodes[parent].children[i])
    }

    fn next_sibling(&self, id: usize) -> Option<usize> {
        let (parent, index) = self.sibling_index(id)?;
        self.nodes[parent].children.get(index + 1).copied()
    }

    fn previous_element_sibling(&self, id: usize) -> Option<usize> {
        let (parent, index) = self.sibling_index(id)?;
        self.nodes[parent].children[..index]
            .iter()
            .rev()
            .copied()
            .find(|candidate| self.name(*candidate).is_some())
    }

    fn has_ancestor(&self, id: usize, name: &str) -> bool {
        let mut cursor = self.nodes[id].parent;
        while let Some(parent) = cursor {
            if self.name(parent) == Some(name) {
                return true;
            }
            cursor = self.nodes[parent].parent;
        }
        false
    }
}

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn decode_html_parser_entities(value: &str) -> String {
    html_escape::decode_html_entities(value).into_owned()
}

fn find_tag_end(input: &str, start: usize) -> usize {
    let bytes = input.as_bytes();
    let mut quote = None;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' if quote.is_none() => quote = Some(bytes[i]),
            current if quote == Some(current) => quote = None,
            b'>' if quote.is_none() => return i,
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

fn parse_start_tag(raw: &str) -> (String, Vec<(String, String)>, bool) {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let name_start = i;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'/' | b'>') {
        i += 1;
    }
    let name = raw[name_start..i].to_ascii_lowercase();
    let mut attrs: Vec<(String, String)> = Vec::new();
    let mut self_closing = false;

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'/' {
            self_closing = true;
            i += 1;
            continue;
        }
        let attr_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b'=' | b'/' | b'>')
        {
            i += 1;
        }
        if attr_start == i {
            i += 1;
            continue;
        }
        let attr_name = raw[attr_start..i].to_ascii_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && matches!(bytes[i], b'\'' | b'"') {
                let quote = bytes[i];
                i += 1;
                let value_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                value = decode_html_parser_entities(&raw[value_start..i]);
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                let value_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' {
                    i += 1;
                }
                value = decode_html_parser_entities(&raw[value_start..i]);
            }
        }
        // html.parser/BeautifulSoup keeps the last duplicate attribute.
        if let Some(existing) = attrs.iter_mut().find(|(key, _)| key == &attr_name) {
            existing.1 = value;
        } else {
            attrs.push((attr_name, value));
        }
    }
    (name, attrs, self_closing)
}

/// Parse the already libxml-serialized selection the way the default
/// BeautifulSoup HTMLParserTreeBuilder observes it. Libxml has already done
/// malformed-markup recovery, so this stage primarily needs HTMLParser's
/// tokenization, entity handling, raw-text behavior, and sibling tree.
fn parse_markdown_tree(input: &str) -> MarkdownTree {
    let mut tree = MarkdownTree::new();
    let mut stack = vec![0usize];
    let mut i = 0usize;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        if bytes[i] != b'<' {
            let end = input[i..]
                .find('<')
                .map_or(bytes.len(), |offset| i + offset);
            if end > i {
                let text = decode_html_parser_entities(&input[i..end]);
                tree.append(*stack.last().unwrap(), MarkdownNodeKind::Text(text));
            }
            i = end;
            continue;
        }
        if input[i..].starts_with("<!--") {
            let end = input[i + 4..]
                .find("-->")
                .map_or(bytes.len(), |offset| i + 4 + offset + 3);
            tree.append(*stack.last().unwrap(), MarkdownNodeKind::Comment);
            i = end;
            continue;
        }
        if input[i..].get(..2).is_some_and(|s| s == "</") {
            let end = find_tag_end(input, i + 2);
            let name = input[i + 2..end]
                .split_ascii_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches('/')
                .to_ascii_lowercase();
            if let Some(position) = stack
                .iter()
                .rposition(|node| tree.name(*node) == Some(name.as_str()))
            {
                stack.truncate(position);
            }
            i = end.saturating_add(1);
            continue;
        }
        if input[i..].get(..2).is_some_and(|s| s == "<!") {
            let end = find_tag_end(input, i + 2);
            tree.append(*stack.last().unwrap(), MarkdownNodeKind::Doctype);
            i = end.saturating_add(1);
            continue;
        }
        if input[i..].get(..2).is_some_and(|s| s == "<?") {
            let end = find_tag_end(input, i + 2);
            tree.append(*stack.last().unwrap(), MarkdownNodeKind::Comment);
            i = end.saturating_add(1);
            continue;
        }

        let end = find_tag_end(input, i + 1);
        if end == bytes.len() {
            tree.append(
                *stack.last().unwrap(),
                MarkdownNodeKind::Text(decode_html_parser_entities(&input[i..])),
            );
            break;
        }
        let (name, attrs, self_closing) = parse_start_tag(&input[i + 1..end]);
        if name.is_empty() {
            tree.append(
                *stack.last().unwrap(),
                MarkdownNodeKind::Text("<".to_string()),
            );
            i += 1;
            continue;
        }
        let node = tree.append(
            *stack.last().unwrap(),
            MarkdownNodeKind::Element {
                name: name.clone(),
                attrs,
            },
        );
        i = end + 1;

        if name == "plaintext" {
            // Python's HTMLParser treats every remaining byte, including
            // apparent closing tags and entities, as literal plaintext.
            if i < bytes.len() {
                tree.append(node, MarkdownNodeKind::Text(input[i..].to_string()));
            }
            break;
        }
        if matches!(name.as_str(), "script" | "style") {
            let closing = format!("</{name}");
            let lower_tail = input[i..].to_ascii_lowercase();
            if let Some(offset) = lower_tail.find(&closing) {
                if offset > 0 {
                    tree.append(
                        node,
                        MarkdownNodeKind::Text(input[i..i + offset].to_string()),
                    );
                }
                let close_start = i + offset;
                i = find_tag_end(input, close_start + 2).saturating_add(1);
                continue;
            }
        }
        if !self_closing && !VOID_TAGS.contains(&name.as_str()) {
            stack.push(node);
        }
    }
    tree
}

fn heading_number(name: &str) -> Option<usize> {
    let digits: String = name
        .strip_prefix('h')?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

fn is_block_name(name: Option<&str>) -> bool {
    name.is_some_and(|name| {
        heading_number(name).is_some()
            || matches!(
                name,
                "p" | "blockquote"
                    | "article"
                    | "div"
                    | "section"
                    | "ol"
                    | "ul"
                    | "li"
                    | "dl"
                    | "dt"
                    | "dd"
                    | "table"
                    | "thead"
                    | "tbody"
                    | "tfoot"
                    | "tr"
                    | "td"
                    | "th"
            )
    })
}

fn removes_whitespace_outside(tree: &MarkdownTree, node: Option<usize>) -> bool {
    node.is_some_and(|id| is_block_name(tree.name(id)) || tree.name(id) == Some("pre"))
}

fn block_content(tree: &MarkdownTree, node: usize) -> bool {
    match &tree.nodes[node].kind {
        MarkdownNodeKind::Element { .. } => true,
        MarkdownNodeKind::Text(text) => !text.trim().is_empty(),
        _ => false,
    }
}

fn next_block_content(tree: &MarkdownTree, id: usize) -> Option<usize> {
    let (parent, index) = tree.sibling_index(id)?;
    tree.nodes[parent].children[index + 1..]
        .iter()
        .copied()
        .find(|candidate| block_content(tree, *candidate))
}

fn normalize_markdown_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i], ' ' | '\t' | '\r' | '\n') {
            let mut has_newline = false;
            while i < chars.len() && matches!(chars[i], ' ' | '\t' | '\r' | '\n') {
                has_newline |= matches!(chars[i], '\r' | '\n');
                i += 1;
            }
            out.push(if has_newline { '\n' } else { ' ' });
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn escape_markdown_text(text: &str) -> String {
    text.replace('*', r"\*").replace('_', r"\_")
}

fn trim_ascii_markdown(value: &str) -> &str {
    value.trim_matches([' ', '\t', '\r', '\n'])
}

fn chomp(value: &str) -> (&'static str, &'static str, &str) {
    let prefix = if value.starts_with(' ') { " " } else { "" };
    let suffix = if value.ends_with(' ') { " " } else { "" };
    (prefix, suffix, value.trim())
}

fn split_boundary_newlines(value: &str) -> (String, String, String) {
    let leading = value.bytes().take_while(|byte| *byte == b'\n').count();
    let remainder = &value[leading..];
    if remainder.is_empty() {
        return ("\n".repeat(leading), String::new(), String::new());
    }
    let trailing = remainder
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\n')
        .count();
    let content_end = remainder.len() - trailing;
    (
        "\n".repeat(leading),
        remainder[..content_end].to_string(),
        "\n".repeat(trailing),
    )
}

fn indent_lines(value: &str, prefix: &str, empty_prefix: &str) -> String {
    value
        .split('\n')
        .map(|line| {
            if line.is_empty() {
                empty_prefix.to_string()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct MarkdownConverter<'a> {
    tree: &'a MarkdownTree,
}

impl<'a> MarkdownConverter<'a> {
    fn process(&self) -> String {
        self.process_tag(0, &HashSet::new())
    }

    fn process_element(&self, id: usize, parent_tags: &HashSet<String>) -> String {
        match &self.tree.nodes[id].kind {
            MarkdownNodeKind::Text(_) => self.process_text(id, parent_tags),
            MarkdownNodeKind::Element { .. } | MarkdownNodeKind::Document => {
                self.process_tag(id, parent_tags)
            }
            MarkdownNodeKind::Comment | MarkdownNodeKind::Doctype => String::new(),
        }
    }

    fn can_ignore_child(&self, child: usize, remove_inside: bool) -> bool {
        match &self.tree.nodes[child].kind {
            MarkdownNodeKind::Element { .. } => false,
            MarkdownNodeKind::Comment | MarkdownNodeKind::Doctype => true,
            MarkdownNodeKind::Text(text) => {
                if !text.trim().is_empty() {
                    return false;
                }
                let previous = self.tree.previous_sibling(child);
                let next = self.tree.next_sibling(child);
                (remove_inside && (previous.is_none() || next.is_none()))
                    || removes_whitespace_outside(self.tree, previous)
                    || removes_whitespace_outside(self.tree, next)
            }
            MarkdownNodeKind::Document => true,
        }
    }

    fn process_tag(&self, id: usize, parent_tags: &HashSet<String>) -> String {
        let name = self.tree.name(id).unwrap_or("");
        let remove_inside = is_block_name(Some(name));
        let mut child_context = parent_tags.clone();
        child_context.insert(name.to_string());
        if heading_number(name).is_some() || matches!(name, "td" | "th") {
            child_context.insert("_inline".to_string());
        }
        if matches!(name, "pre" | "code" | "kbd" | "samp") {
            child_context.insert("_noformat".to_string());
        }

        let mut child_strings: Vec<String> = self.tree.nodes[id]
            .children
            .iter()
            .copied()
            .filter(|child| !self.can_ignore_child(*child, remove_inside))
            .map(|child| self.process_element(child, &child_context))
            .filter(|value| !value.is_empty())
            .collect();

        if name != "pre" && !self.tree.has_ancestor(id, "pre") {
            let mut collapsed = vec![String::new()];
            for child in child_strings {
                let (mut leading, content, trailing) = split_boundary_newlines(&child);
                if collapsed.last().is_some_and(|last| !last.is_empty()) && !leading.is_empty() {
                    let previous = collapsed.pop().unwrap();
                    leading = "\n".repeat(2.min(previous.len().max(leading.len())));
                }
                collapsed.extend([leading, content, trailing]);
            }
            child_strings = collapsed;
        }
        let text = child_strings.concat();
        self.convert_tag(id, name, text, parent_tags)
    }

    fn process_text(&self, id: usize, parent_tags: &HashSet<String>) -> String {
        let MarkdownNodeKind::Text(raw) = &self.tree.nodes[id].kind else {
            unreachable!()
        };
        let mut text = if parent_tags.contains("pre") {
            raw.clone()
        } else {
            normalize_markdown_text(raw)
        };
        if !parent_tags.contains("_noformat") {
            text = escape_markdown_text(&text);
        }

        let parent = self.tree.nodes[id].parent;
        let previous = self.tree.previous_sibling(id);
        let next = self.tree.next_sibling(id);
        if removes_whitespace_outside(self.tree, previous)
            || (parent.is_some_and(|node| is_block_name(self.tree.name(node)))
                && previous.is_none())
        {
            text = text.trim_start_matches([' ', '\t', '\r', '\n']).to_string();
        }
        if removes_whitespace_outside(self.tree, next)
            || (parent.is_some_and(|node| is_block_name(self.tree.name(node))) && next.is_none())
        {
            text = text.trim_end().to_string();
        }
        text
    }

    fn inline(&self, text: String, markup: &str, parent_tags: &HashSet<String>) -> String {
        if parent_tags.contains("_noformat") {
            return text;
        }
        let (prefix, suffix, inner) = chomp(&text);
        if inner.is_empty() {
            String::new()
        } else {
            format!("{prefix}{markup}{inner}{markup}{suffix}")
        }
    }

    fn convert_tag(
        &self,
        id: usize,
        name: &str,
        text: String,
        parent_tags: &HashSet<String>,
    ) -> String {
        if name == "[document]" {
            return text.trim_matches('\n').to_string();
        }
        if let Some(number) = heading_number(name) {
            return self.convert_heading(number, &text, parent_tags);
        }
        match name {
            "a" => self.convert_a(id, text, parent_tags),
            "b" | "strong" => self.inline(text, "**", parent_tags),
            "em" | "i" => self.inline(text, "*", parent_tags),
            "del" | "s" => self.inline(text, "~~", parent_tags),
            "sub" | "sup" => self.inline(text, "", parent_tags),
            "code" | "kbd" | "samp" => self.convert_code(text, parent_tags),
            "blockquote" => self.convert_blockquote(text, parent_tags),
            "br" => {
                if parent_tags.contains("_inline") {
                    if text.is_empty() {
                        " ".into()
                    } else {
                        format!("{text} ")
                    }
                } else {
                    format!("  \n{text}")
                }
            }
            "div" | "article" | "section" => self.convert_div(text, parent_tags),
            "dd" => self.convert_dd(text, parent_tags),
            "dt" => self.convert_dt(text, parent_tags),
            "dl" => self.convert_div(text, parent_tags),
            "hr" => "\n\n---\n\n".to_string(),
            "img" => self.convert_img(id, parent_tags),
            "video" => self.convert_video(id, text, parent_tags),
            "ul" | "ol" => self.convert_list(id, text, parent_tags),
            "li" => self.convert_li(id, text),
            "p" => self.convert_p(text, parent_tags),
            "pre" => self.convert_pre(text),
            "q" => format!("\"{text}\""),
            "script" | "style" => String::new(),
            "table" => format!("\n\n{}\n\n", text.trim()),
            "caption" => format!("{}\n\n", text.trim()),
            "figcaption" => format!("\n\n{}\n\n", text.trim()),
            "td" | "th" => self.convert_cell(id, text),
            "tr" => self.convert_row(id, text),
            _ => text,
        }
    }

    fn convert_a(&self, id: usize, text: String, parent_tags: &HashSet<String>) -> String {
        if parent_tags.contains("_noformat") {
            return text;
        }
        let (prefix, suffix, inner) = chomp(&text);
        if inner.is_empty() {
            return String::new();
        }
        // markdownify treats an empty href/title as absent (`if href`,
        // `not title` are falsy on ""), so an empty href is no link and an
        // empty title never blocks the autolink form.
        let href = self.tree.attr(id, "href").filter(|value| !value.is_empty());
        let title = self
            .tree
            .attr(id, "title")
            .filter(|value| !value.is_empty());
        if href.is_some_and(|href| inner.replace(r"\_", "_") == href) && title.is_none() {
            return format!("<{href}>", href = href.unwrap());
        }
        let Some(href) = href else {
            return inner.to_string();
        };
        let title = title
            .map(|value| format!(" \"{}\"", value.replace('"', r#"\""#)))
            .unwrap_or_default();
        format!("{prefix}[{inner}]({href}{title}){suffix}")
    }

    fn convert_blockquote(&self, text: String, parent_tags: &HashSet<String>) -> String {
        let text = trim_ascii_markdown(&text);
        if parent_tags.contains("_inline") {
            return format!(" {text} ");
        }
        if text.is_empty() {
            return "\n".to_string();
        }
        format!("\n{}\n\n", indent_lines(text, "> ", ">"))
    }

    fn convert_code(&self, text: String, parent_tags: &HashSet<String>) -> String {
        if parent_tags.contains("_noformat") {
            return text;
        }
        let (prefix, suffix, inner) = chomp(&text);
        if inner.is_empty() {
            return String::new();
        }
        let bytes = inner.as_bytes();
        let mut maximum = 0;
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'`' {
                let start = i;
                while i < bytes.len() && bytes[i] == b'`' {
                    i += 1;
                }
                maximum = maximum.max(i - start);
            } else {
                i += 1;
            }
        }
        let delimiter = "`".repeat(maximum + 1);
        let inner = if maximum > 0 {
            format!(" {inner} ")
        } else {
            inner.to_string()
        };
        format!("{prefix}{delimiter}{inner}{delimiter}{suffix}")
    }

    fn convert_div(&self, text: String, parent_tags: &HashSet<String>) -> String {
        if parent_tags.contains("_inline") {
            return format!(" {} ", text.trim());
        }
        let text = text.trim();
        if text.is_empty() {
            String::new()
        } else {
            format!("\n\n{text}\n\n")
        }
    }

    fn convert_dd(&self, text: String, parent_tags: &HashSet<String>) -> String {
        let text = text.trim();
        if parent_tags.contains("_inline") {
            return format!(" {text} ");
        }
        if text.is_empty() {
            return "\n".to_string();
        }
        let indented = indent_lines(text, "    ", "");
        format!(":{}\n", &indented[1..])
    }

    fn convert_dt(&self, text: String, parent_tags: &HashSet<String>) -> String {
        let text = collapse_ascii_ws(text.trim());
        if parent_tags.contains("_inline") {
            return format!(" {text} ");
        }
        if text.is_empty() {
            "\n".to_string()
        } else {
            format!("\n\n{text}\n")
        }
    }

    fn convert_heading(&self, number: usize, text: &str, parent_tags: &HashSet<String>) -> String {
        if parent_tags.contains("_inline") {
            return text.to_string();
        }
        let number = number.clamp(1, 6);
        let text = text.trim();
        if number <= 2 {
            if text.is_empty() {
                return String::new();
            }
            let underline = if number == 1 { '=' } else { '-' };
            return format!(
                "\n\n{text}\n{}\n\n",
                underline.to_string().repeat(text.chars().count())
            );
        }
        let text = collapse_ascii_ws(text);
        format!("\n\n{} {text}\n\n", "#".repeat(number))
    }

    fn convert_img(&self, id: usize, parent_tags: &HashSet<String>) -> String {
        let alt = self.tree.attr(id, "alt").unwrap_or("");
        if parent_tags.contains("_inline") {
            return alt.to_string();
        }
        let src = self.tree.attr(id, "src").unwrap_or("");
        let title = self
            .tree
            .attr(id, "title")
            .filter(|value| !value.is_empty())
            .map(|value| format!(" \"{}\"", value.replace('"', r#"\""#)))
            .unwrap_or_default();
        format!("![{alt}]({src}{title})")
    }

    fn convert_video(&self, id: usize, text: String, parent_tags: &HashSet<String>) -> String {
        if parent_tags.contains("_inline") {
            return text;
        }
        let mut src = self.tree.attr(id, "src").unwrap_or("");
        if src.is_empty() {
            let mut sources = Vec::new();
            self.tree
                .element_children_recursive(id, &["source"], &mut sources);
            src = sources
                .iter()
                .find_map(|source| self.tree.attr(*source, "src"))
                .unwrap_or("");
        }
        let poster = self.tree.attr(id, "poster").unwrap_or("");
        match (src.is_empty(), poster.is_empty()) {
            (false, false) => format!("[![{text}]({poster})]({src})"),
            (false, true) => format!("[{text}]({src})"),
            (true, false) => format!("![{text}]({poster})"),
            (true, true) => text,
        }
    }

    fn convert_list(&self, id: usize, text: String, parent_tags: &HashSet<String>) -> String {
        // markdownify: before_paragraph is true when the next block-content
        // sibling is anything but another list — INCLUDING a bare text node
        // (name None), which the old `.and_then(name)` dropped, so a list
        // trailed by text lost its separating blank line.
        let before_paragraph = next_block_content(self.tree, id)
            .is_some_and(|node| !matches!(self.tree.name(node), Some("ul" | "ol")));
        if parent_tags.contains("li") {
            return format!("\n{}", text.trim_end());
        }
        format!("\n\n{text}{}", if before_paragraph { "\n" } else { "" })
    }

    fn convert_li(&self, id: usize, text: String) -> String {
        let text = text.trim();
        if text.is_empty() {
            return "\n".to_string();
        }
        let parent = self.tree.nodes[id].parent;
        let bullet = if parent.and_then(|node| self.tree.name(node)) == Some("ol") {
            let parent = parent.unwrap();
            // ponytail: ASCII-only, like colspan below. Python's int() also
            // parses Unicode decimal digits (Arabic-Indic, etc.), which std
            // can't decode without a unicode-data crate — a marginal `start`
            // value not worth the dependency. Upgrade: add `unicode` and map
            // Nd digits if a golden ever needs it.
            let start = self
                .tree
                .attr(parent, "start")
                .filter(|value| !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            let previous_items = self
                .tree
                .sibling_index(id)
                .map(|(parent, index)| {
                    self.tree.nodes[parent].children[..index]
                        .iter()
                        .filter(|node| self.tree.name(**node) == Some("li"))
                        .count()
                })
                .unwrap_or(0);
            format!("{}.", start + previous_items)
        } else {
            let mut depth: isize = -1;
            let mut cursor = Some(id);
            while let Some(node) = cursor {
                if self.tree.name(node) == Some("ul") {
                    depth += 1;
                }
                cursor = self.tree.nodes[node].parent;
            }
            ['*', '+', '-'][depth.rem_euclid(3) as usize].to_string()
        };
        let marker = format!("{bullet} ");
        let indent = " ".repeat(marker.len());
        let indented = indent_lines(text, &indent, "");
        format!("{marker}{}\n", &indented[marker.len()..])
    }

    fn convert_p(&self, text: String, parent_tags: &HashSet<String>) -> String {
        if parent_tags.contains("_inline") {
            return format!(" {} ", trim_ascii_markdown(&text));
        }
        let text = trim_ascii_markdown(&text);
        if text.is_empty() {
            String::new()
        } else {
            format!("\n\n{text}\n\n")
        }
    }

    fn convert_pre(&self, text: String) -> String {
        if text.is_empty() {
            return String::new();
        }
        // markdownify's STRIP mode: remove everything through the last
        // leading newline (but preserve indentation after it), then remove
        // all trailing spaces/newlines.
        let start = text
            .char_indices()
            .take_while(|(_, c)| matches!(c, ' ' | '\n'))
            .filter(|(_, c)| *c == '\n')
            .map(|(index, _)| index + 1)
            .last()
            .unwrap_or(0);
        let text = text[start..].trim_end_matches([' ', '\n']);
        format!("\n\n```\n{text}\n```\n\n")
    }

    fn convert_cell(&self, id: usize, text: String) -> String {
        let colspan = self
            .tree
            .attr(id, "colspan")
            .filter(|value| value.chars().all(|c| c.is_ascii_digit()))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, 1000);
        format!(
            " {}{}",
            text.trim().replace('\n', " "),
            " |".repeat(colspan)
        )
    }

    fn convert_row(&self, id: usize, text: String) -> String {
        let mut cells = Vec::new();
        self.tree
            .element_children_recursive(id, &["td", "th"], &mut cells);
        let is_first_row = self.tree.previous_element_sibling(id).is_none();
        let parent = self.tree.nodes[id].parent.unwrap_or(0);
        let parent_name = self.tree.name(parent).unwrap_or("");
        let is_headrow = cells.iter().all(|cell| self.tree.name(*cell) == Some("th"))
            || (parent_name == "thead" && {
                let mut rows = Vec::new();
                self.tree
                    .element_children_recursive(parent, &["tr"], &mut rows);
                rows.len() == 1
            });
        let table_has_thead = self.tree.nodes[parent].parent.is_some_and(|table| {
            let mut heads = Vec::new();
            self.tree
                .element_children_recursive(table, &["thead"], &mut heads);
            !heads.is_empty()
        });
        let head_missing = (is_first_row && parent_name != "tbody")
            || (is_first_row && parent_name == "tbody" && !table_has_thead);
        let full_colspan: usize = cells
            .iter()
            .map(|cell| {
                self.tree
                    .attr(*cell, "colspan")
                    .filter(|value| value.chars().all(|c| c.is_ascii_digit()))
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 1000)
            })
            .sum();
        let mut overline = String::new();
        let mut underline = String::new();
        if is_headrow && is_first_row {
            underline = format!("| {} |\n", vec!["---"; full_colspan].join(" | "));
        } else {
            let tbody_at_table_start =
                parent_name == "tbody" && self.tree.previous_element_sibling(parent).is_none();
            if head_missing || (is_first_row && (parent_name == "table" || tbody_at_table_start)) {
                overline.push_str(&format!("| {} |\n", vec![""; full_colspan].join(" | ")));
                overline.push_str(&format!("| {} |\n", vec!["---"; full_colspan].join(" | ")));
            }
        }
        format!("{overline}|{text}\n{underline}")
    }
}

fn markdownify(content: &str) -> String {
    let tree = parse_markdown_tree(content);
    MarkdownConverter { tree: &tree }.process()
}

/// One target's rendering, `extraction_type` mode (shell.py). A
/// `QueryResult::Text` (a `::text`/`::attr()` pseudo-match) short-circuits
/// exactly like Python's text-node `Selector`: both `html_content` and
/// `get_all_text` return the bare value with no re-serialization
/// (`parser.py` — `if self._is_text_node(self._root): return ...`), so
/// `html`/`markdown` treat the value as their "outer html" and `text` skips
/// straight past `get_all_text` to collapsing the value itself.
///
/// Oracle-probed (`li a::text` / `a::attr(href)` on basic.html): all three
/// modes give the identical bare value there, because it contains no HTML or
/// markdown-special characters. Markdown mode still runs the value through
/// the converter rather than passing it through raw, because markdownify
/// escapes markdown-significant characters even in a tag-less string —
/// reprobed with `1. *starred* & _under_ # hash`:
/// `markdownify` gives `1. \*starred\* & \_under\_ # hash`; the local port is
/// still run for this tag-less input because escaping is part of its contract.
fn render_one(target: &query::QueryResult, format: &str) -> Result<String, String> {
    if format == "text" {
        let raw = match target {
            query::QueryResult::Element(el) => dom::get_all_text(
                *el,
                "\n",
                true,
                &["script", "style", "noscript", "svg", "iframe"],
                true,
            ),
            query::QueryResult::Text { value, .. } => value.clone(),
        };
        return Ok(collapse_whitespace(raw));
    }
    let content_source = match target {
        query::QueryResult::Element(el) => dom::outer_html(*el),
        query::QueryResult::Text { value, .. } => value.clone(),
    };
    match format {
        "html" => Ok(content_source),
        "markdown" => Ok(markdownify(&content_source)),
        _ => unreachable!("format validated by render()"),
    }
}

/// `Convertor._extract_content` + `render_content`'s `"".join(...).strip()`
/// (shell.py / core.py, normative): validate format; optionally re-scope to
/// `body` and sanitize a clone; optionally scope further to every
/// `css_selector` match (concatenated, no separator; `::text`/`::attr()`
/// pseudo-matches render too, see `render_one`); render each target in
/// `format`; join and trim once at the end.
pub fn render(
    doc: &Doc,
    format: &str,
    css_selector: Option<&str>,
    main_content_only: bool,
) -> Result<String, String> {
    if !matches!(format, "markdown" | "text" | "html") {
        return Err(format!("unsupported format: {format}"));
    }

    // Resolve the scope root on the ORIGINAL tree first — `body_or_root`'s
    // tag lookup doesn't depend on sanitization — then clone and sanitize
    // with that id exempted (see `sanitize_main_content`'s doc comment).
    // NodeIds are stable across `Document::clone()`, so the id found here
    // still names the same node in the clone.
    let sanitized = main_content_only.then(|| {
        let scope_id = body_or_root(doc).id();
        let mut sanitized = doc.clone();
        sanitize_main_content(&mut sanitized, scope_id);
        (sanitized, scope_id)
    });
    let (active_doc, root): (&Doc, ElementRef) = match &sanitized {
        Some((d, id)) => (d, d.element(*id).expect("body_or_root remains an element")),
        None => (doc, doc.root()),
    };

    let targets: Vec<query::QueryResult> = match css_selector.filter(|s| !s.is_empty()) {
        Some(sel) => query::css_query(active_doc, Some(root), sel)?,
        None => vec![query::QueryResult::Element(root)],
    };

    let mut content = String::new();
    for t in &targets {
        content.push_str(&render_one(t, format)?);
    }
    Ok(content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrapling::dom;

    #[test]
    fn markdown_contains_heading_and_list() {
        let d = dom::parse(r#"<html><body><h1>Hello</h1><ul><li>a</li></ul></body></html>"#);
        let md = render(&d, "markdown", None, false).unwrap();
        assert!(md.contains("Hello"));
        assert!(md.contains("a"));
    }

    #[test]
    fn hidden_and_zero_width_stripping() {
        let d = dom::parse(concat!(
            r#"<html><body><p>keep</p><p style="display:none">drop</p>"#,
            "<p>ze\u{200b}ro</p></body></html>"
        ));
        let txt = render(&d, "text", None, true).unwrap();
        assert!(txt.contains("keep"));
        assert!(!txt.contains("drop"));
        assert!(txt.contains("zero"));
    }

    #[test]
    fn bad_format_error_matches_python() {
        let d = dom::parse("<p>x</p>");
        assert_eq!(
            render(&d, "pdf", None, false).unwrap_err(),
            "unsupported format: pdf"
        );
    }

    /// `_HIDDEN_XPATH` is `.//*[@aria-hidden='true']` — descendant axis, self
    /// excluded — so the scope root carrying the attribute must not drop
    /// itself. Oracle-confirmed: `{"content": "keep me"}`, not `""`.
    #[test]
    fn body_aria_hidden_does_not_drop_itself() {
        let d = dom::parse(r#"<html><body aria-hidden="true"><p>keep me</p></body></html>"#);
        assert_eq!(render(&d, "text", None, true).unwrap(), "keep me");
    }

    /// Same exemption, via the style-substring half of `_HIDDEN_XPATH`
    /// (`.//*[contains(@style, "display:none") ...]`). Oracle-confirmed:
    /// `{"content": "keep me too"}`.
    #[test]
    fn body_style_hidden_does_not_drop_itself() {
        let d = dom::parse(r#"<html><body style="display:none"><p>keep me too</p></body></html>"#);
        assert_eq!(render(&d, "text", None, true).unwrap(), "keep me too");
    }
}
