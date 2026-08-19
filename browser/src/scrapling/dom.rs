//! Libxml-compatible HTML tree used by every Scrapling parser operation.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde_json::Value;
use xmloxide::html::{parse_html_with_options, HtmlParseOptions};
use xmloxide::serial::html::serialize_html_subtree;
use xmloxide::tree::NodeKind;
use xmloxide::{Document, NodeId};

const LXML_MIXED_CONTENT_TAGS: &[&str] = &[
    "body",
    "div",
    "p",
    "span",
    "a",
    "b",
    "i",
    "u",
    "s",
    "strike",
    "tt",
    "big",
    "small",
    "cite",
    "q",
    "kbd",
    "ins",
    "del",
    "em",
    "strong",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "dt",
    "dd",
    "td",
    "th",
    "caption",
    "pre",
    "code",
    "label",
    "button",
    "legend",
    "address",
    "blockquote",
    "form",
];
const LXML_RAW_TEXT_TAGS: &[&str] = &["script", "style", "title", "textarea"];

#[derive(Debug, Clone)]
pub struct Doc {
    pub tree: Document,
}

#[derive(Debug, Clone, Copy)]
pub struct ElementRef<'a> {
    doc: &'a Doc,
    id: NodeId,
}

impl PartialEq for ElementRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.doc, other.doc) && self.id == other.id
    }
}

impl Eq for ElementRef<'_> {}

impl Hash for ElementRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::from_ref(self.doc).hash(state);
        self.id.hash(state);
    }
}

impl<'a> ElementRef<'a> {
    pub fn id(self) -> NodeId {
        self.id
    }

    pub fn doc(self) -> &'a Doc {
        self.doc
    }

    pub fn name(self) -> &'a str {
        self.doc.tree.node_name(self.id).unwrap_or("")
    }

    pub fn attr(self, name: &str) -> Option<&'a str> {
        self.doc.tree.attribute(self.id, name)
    }

    pub fn attrs(self) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        self.doc
            .tree
            .attributes(self.id)
            .iter()
            .map(|attr| (attr.name.as_str(), attr.value.as_str()))
    }
}

pub fn parse(input: &str) -> Doc {
    let cleaned = input.trim().replace('\0', "");
    let trim_implicit_leading = (cleaned.starts_with("<!--") || cleaned.starts_with("<![CDATA["))
        && !cleaned.to_ascii_lowercase().contains("<body");
    let body = if cleaned.is_empty() {
        "<html></html>"
    } else {
        &cleaned
    };
    let options = HtmlParseOptions::default().recover(true).no_blanks(false);
    let mut tree = parse_html_with_options(body, &options).unwrap_or_else(|_| {
        parse_html_with_options("<html></html>", &HtmlParseOptions::default()).unwrap()
    });
    if tree.root_element().is_none() {
        tree = parse_html_with_options(&format!("<html><body>{body}</body></html>"), &options)
            .unwrap_or_else(|_| parse_html_with_options("<html></html>", &options).unwrap());
    }

    let removed: Vec<_> = tree
        .descendants(tree.root())
        .filter(|id| {
            matches!(
                tree.node(*id).kind,
                // lxml's cleaner drops PIs too; keeping them made ::text /
                // find-by-text / describe / get_all_text diverge.
                NodeKind::Comment { .. }
                    | NodeKind::CData { .. }
                    | NodeKind::ProcessingInstruction { .. }
            )
        })
        .collect();
    for id in removed {
        tree.remove_node(id);
    }
    if trim_implicit_leading {
        trim_body_leading_whitespace(&mut tree);
    }
    merge_adjacent_text(&mut tree);
    detach_blank_text(&mut tree);
    Doc { tree }
}

fn trim_body_leading_whitespace(tree: &mut Document) {
    let body = tree
        .descendants(tree.root())
        .find(|id| tree.node_name(*id) == Some("body"));
    let first = body.and_then(|id| tree.first_child(id));
    if let Some(id) = first.filter(|id| is_text(tree, *id)) {
        let trimmed = tree.node_text(id).unwrap_or("").trim_start().to_string();
        tree.set_text_content(id, &trimmed);
    }
}

fn is_text(tree: &Document, id: NodeId) -> bool {
    matches!(
        tree.node(id).kind,
        NodeKind::Text { .. } | NodeKind::CData { .. }
    )
}

fn merge_adjacent_text(tree: &mut Document) {
    let parents: Vec<_> = std::iter::once(tree.root())
        .chain(tree.descendants(tree.root()))
        .collect();
    for parent in parents {
        let children: Vec<_> = tree.children(parent).collect();
        let mut previous = None;
        for child in children {
            if !is_text(tree, child) {
                previous = None;
                continue;
            }
            if let Some(first) = previous {
                let merged = format!(
                    "{}{}",
                    tree.node_text(first).unwrap_or(""),
                    tree.node_text(child).unwrap_or("")
                );
                tree.set_text_content(first, &merged);
                tree.remove_node(child);
            } else {
                previous = Some(child);
            }
        }
    }
}

fn detach_blank_text(tree: &mut Document) {
    let ids: Vec<_> = tree.descendants(tree.root()).collect();
    let mut drop_ids = Vec::new();
    for id in ids {
        if !is_text(tree, id)
            || tree
                .prev_sibling(id)
                .is_some_and(|prev| is_text(tree, prev))
        {
            continue;
        }
        let mut run = vec![id];
        let mut all_blank = tree
            .node_text(id)
            .is_some_and(|text| text.chars().all(char::is_whitespace));
        let mut next = tree.next_sibling(id);
        while let Some(node) = next {
            if !is_text(tree, node) {
                break;
            }
            all_blank &= tree
                .node_text(node)
                .is_some_and(|text| text.chars().all(char::is_whitespace));
            run.push(node);
            next = tree.next_sibling(node);
        }
        if !all_blank {
            continue;
        }
        let previous_tag = tree.prev_sibling(id).and_then(|node| tree.node_name(node));
        let keep = previous_tag.map_or_else(
            || {
                tree.parent(id)
                    .and_then(|node| tree.node_name(node))
                    .is_some_and(|name| {
                        LXML_MIXED_CONTENT_TAGS.contains(&name)
                            || LXML_RAW_TEXT_TAGS.contains(&name)
                    })
            },
            |name| LXML_MIXED_CONTENT_TAGS.contains(&name),
        );
        if !keep {
            drop_ids.extend(run);
        }
    }
    for id in drop_ids {
        tree.remove_node(id);
    }
}

impl Doc {
    pub fn root(&self) -> ElementRef<'_> {
        let id = self
            .tree
            .root_element()
            .expect("HTML parser always emits a root");
        ElementRef { doc: self, id }
    }

    pub fn element(&self, id: NodeId) -> Option<ElementRef<'_>> {
        self.tree
            .is_element(id)
            .then_some(ElementRef { doc: self, id })
    }

    pub fn first_by_tag(&self, tag: &str) -> Option<ElementRef<'_>> {
        std::iter::once(self.root())
            .chain(descendant_elements(self.root()))
            .find(|element| element.name() == tag)
    }
}

pub fn merged_text_children(el: ElementRef<'_>) -> Vec<(bool, String)> {
    let tree = &el.doc.tree;
    let mut runs = Vec::new();
    let mut seen_element = false;
    let mut current: Option<String> = None;
    for child in tree.children(el.id) {
        if is_text(tree, child) {
            current
                .get_or_insert_with(String::new)
                .push_str(tree.node_text(child).unwrap_or(""));
        } else {
            if let Some(run) = current.take() {
                runs.push((!seen_element, run));
            }
            seen_element |= tree.is_element(child);
        }
    }
    if let Some(run) = current {
        runs.push((!seen_element, run));
    }
    runs
}

pub fn descendant_text_runs(el: ElementRef<'_>) -> Vec<(ElementRef<'_>, usize, String)> {
    let tree = &el.doc.tree;
    let mut out = Vec::new();
    let mut indexes: HashMap<NodeId, usize> = HashMap::new();
    let mut owner = None;
    let mut current = String::new();
    for node in tree.descendants(el.id) {
        if !is_text(tree, node) {
            flush_run(&mut owner, &mut current, &mut out, &mut indexes);
            continue;
        }
        let parent = tree.parent(node).and_then(|id| el.doc.element(id));
        if parent.map(ElementRef::id) != owner.map(ElementRef::id) {
            flush_run(&mut owner, &mut current, &mut out, &mut indexes);
            owner = parent;
        }
        current.push_str(tree.node_text(node).unwrap_or(""));
    }
    flush_run(&mut owner, &mut current, &mut out, &mut indexes);
    out
}

fn flush_run<'a>(
    owner: &mut Option<ElementRef<'a>>,
    text: &mut String,
    out: &mut Vec<(ElementRef<'a>, usize, String)>,
    indexes: &mut HashMap<NodeId, usize>,
) {
    if let Some(element) = owner.take() {
        let index = indexes.entry(element.id).or_default();
        out.push((element, *index, std::mem::take(text)));
        *index += 1;
    }
}

pub fn leading_text(el: ElementRef<'_>) -> String {
    merged_text_children(el)
        .first()
        .filter(|(leading, _)| *leading)
        .map(|(_, text)| text.clone())
        .unwrap_or_default()
}

pub fn first_text_run_nonblank(el: ElementRef<'_>) -> bool {
    merged_text_children(el)
        .first()
        .is_some_and(|(_, text)| !crate::scrapling::text::normalize_space(text).is_empty())
}

pub fn get_all_text(
    el: ElementRef<'_>,
    separator: &str,
    strip: bool,
    ignore: &[&str],
    valid_values: bool,
) -> String {
    let tree = &el.doc.tree;
    let mut fragments = Vec::new();
    let mut owner = None;
    let mut current = String::new();

    let flush = |current: &mut String, fragments: &mut Vec<String>| {
        if current.is_empty() {
            return;
        }
        let trimmed = current.trim();
        if !valid_values || !trimmed.is_empty() {
            fragments.push(if strip {
                trimmed.to_string()
            } else {
                current.clone()
            });
        }
        current.clear();
    };

    for node in tree.descendants(el.id) {
        if !is_text(tree, node) {
            flush(&mut current, &mut fragments);
            owner = None;
            continue;
        }
        let mut cursor = tree.parent(node);
        let mut ignored = false;
        while let Some(ancestor) = cursor {
            if tree
                .node_name(ancestor)
                .is_some_and(|name| ignore.iter().any(|item| item.eq_ignore_ascii_case(name)))
            {
                ignored = true;
                break;
            }
            if ancestor == el.id {
                break;
            }
            cursor = tree.parent(ancestor);
        }
        if ignored {
            flush(&mut current, &mut fragments);
            owner = None;
            continue;
        }
        let parent = tree.parent(node);
        if parent != owner {
            flush(&mut current, &mut fragments);
            owner = parent;
        }
        current.push_str(tree.node_text(node).unwrap_or(""));
    }
    flush(&mut current, &mut fragments);
    fragments.join(separator)
}

pub fn outer_html(el: ElementRef<'_>) -> String {
    serialize_html_subtree(&el.doc.tree, el.id)
}

pub fn attrs_json(el: ElementRef<'_>) -> serde_json::Map<String, Value> {
    el.attrs()
        .map(|(name, value)| (name.to_string(), Value::String(value.to_string())))
        .collect()
}

pub fn parent_element(el: ElementRef<'_>) -> Option<ElementRef<'_>> {
    el.doc.tree.parent(el.id).and_then(|id| el.doc.element(id))
}

pub fn element_children(el: ElementRef<'_>) -> Vec<ElementRef<'_>> {
    el.doc
        .tree
        .children(el.id)
        .filter_map(|id| el.doc.element(id))
        .collect()
}

pub fn descendant_elements(el: ElementRef<'_>) -> Vec<ElementRef<'_>> {
    el.doc
        .tree
        .descendants(el.id)
        .filter_map(|id| el.doc.element(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first<'a>(doc: &'a Doc, tag: &str) -> ElementRef<'a> {
        doc.first_by_tag(tag).unwrap()
    }

    #[test]
    fn parse_and_text_match_libxml_cleanup() {
        let doc = parse("<span>CONDITION: <!-- hi -->Excellent</span>");
        let span = first(&doc, "span");
        assert_eq!(leading_text(span), "CONDITION: Excellent");
        assert_eq!(outer_html(span), "<span>CONDITION: Excellent</span>");

        let doc = parse("<p>a\0b</p>");
        assert_eq!(get_all_text(first(&doc, "p"), "\n", false, &[], true), "ab");

        let doc = parse("<!--c--> b");
        assert_eq!(outer_html(doc.root()), "<html><body>b</body></html>");
        let doc = parse("<div><!--c--> b</div>");
        assert_eq!(outer_html(first(&doc, "div")), "<div> b</div>");
    }

    #[test]
    fn empty_input_keeps_the_oracles_explicit_html_root() {
        let doc = parse("");
        assert_eq!(doc.root().name(), "html");
        assert_eq!(outer_html(doc.root()), "<html></html>");

        let doc = parse(">");
        assert_eq!(outer_html(doc.root()), "<html><body>&gt;</body></html>");
    }

    #[test]
    fn text_runs_and_ignored_subtrees_match_scrapling() {
        let doc = parse("<div>a<script>bad()</script>b<style>.x{}</style>c</div>");
        assert_eq!(
            get_all_text(first(&doc, "div"), "\n", false, &["script", "style"], true),
            "a\nb\nc"
        );
        let doc = parse("<p>lead<b>x</b>tail</p>");
        assert_eq!(leading_text(first(&doc, "p")), "lead");
    }

    #[test]
    fn blank_text_uses_libxmls_legacy_content_model() {
        let doc = parse("<main>  <h1>a</h1>  <p>b</p>  <h2>c</h2>  <table><tr><td>d</td></tr></table>  <p>e</p></main>");
        let runs: Vec<_> = merged_text_children(first(&doc, "main"))
            .into_iter()
            .map(|(_, value)| value)
            .collect();
        assert_eq!(runs, ["  ", "  ", "  "]);
        assert_eq!(
            merged_text_children(first(&parse("<pre>   \n  </pre>"), "pre")),
            [(true, "   \n  ".to_string())]
        );
    }

    #[test]
    fn attributes_and_recovery_are_ordered_and_libxml_serialized() {
        let doc = parse("<input z=1 disabled a='' z=2 checked=checked>");
        let input = first(&doc, "input");
        assert_eq!(
            attrs_json(input).keys().collect::<Vec<_>>(),
            ["z", "disabled", "a", "checked"]
        );
        assert_eq!(outer_html(input), "<input z=\"1\" disabled a=\"\" checked>");

        let doc = parse("<table><td>A<td>B<div>C");
        assert_eq!(
            outer_html(first(&doc, "table")),
            "<table><td>A</td><td>B<div>C</div></td></table>"
        );
    }

    #[test]
    fn template_contents_are_ordinary_children() {
        let doc = parse("<template><p>x</p></template>");
        let template = first(&doc, "template");
        assert_eq!(
            element_children(template)
                .into_iter()
                .map(ElementRef::name)
                .collect::<Vec<_>>(),
            ["p"]
        );
    }
}
