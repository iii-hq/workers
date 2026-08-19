//! HTML parsing example demonstrating error-tolerant parsing.
//!
//! Run with: `cargo run --example html_parse`
#![allow(clippy::expect_used)]

use xmloxide::html::parse_html;
use xmloxide::tree::NodeKind;
use xmloxide::Document;

fn print_tree(doc: &Document, node: xmloxide::NodeId, depth: usize) {
    let indent = "  ".repeat(depth);
    let node_data = doc.node(node);
    match &node_data.kind {
        NodeKind::Element {
            name, attributes, ..
        } => {
            if attributes.is_empty() {
                println!("{indent}<{name}>");
            } else {
                let attrs: Vec<String> = attributes
                    .iter()
                    .map(|a| format!("{}=\"{}\"", a.name, a.value))
                    .collect();
                println!("{indent}<{name} {}>", attrs.join(" "));
            }
            for child in doc.children(node) {
                print_tree(doc, child, depth + 1);
            }
        }
        NodeKind::Text { content } => {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                println!("{indent}\"{trimmed}\"");
            }
        }
        _ => {
            for child in doc.children(node) {
                print_tree(doc, child, depth + 1);
            }
        }
    }
}

fn main() {
    // HTML with missing tags, unclosed elements, void elements
    let html = r#"
<p>Hello <b>bold <i>and italic</b> text</i>
<br>
<img src="photo.jpg" alt="A photo">
<ul>
  <li>First item
  <li>Second item
  <li>Third item
</ul>
<div>Unclosed div
"#;

    println!("Input HTML:");
    println!("{html}");
    println!("Parsed tree:");

    let doc = parse_html(html).expect("HTML parsing failed");
    let root = doc.root_element().expect("no root");
    print_tree(&doc, root, 0);
}
