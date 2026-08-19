//! Basic XML parsing and tree navigation.
//!
//! Run with: `cargo run --example basic_parse`
#![allow(clippy::expect_used)]

use xmloxide::tree::NodeKind;
use xmloxide::Document;

fn main() {
    let xml = r#"<?xml version="1.0"?>
<bookstore>
  <book category="fiction">
    <title lang="en">The Great Gatsby</title>
    <author>F. Scott Fitzgerald</author>
    <year>1925</year>
    <price>10.99</price>
  </book>
  <book category="science">
    <title lang="en">A Brief History of Time</title>
    <author>Stephen Hawking</author>
    <year>1988</year>
    <price>14.99</price>
  </book>
</bookstore>"#;

    let doc = Document::parse_str(xml).expect("failed to parse XML");
    let root = doc.root_element().expect("no root element");

    println!("Root element: {}", doc.node_name(root).unwrap_or("?"));

    // Iterate over child elements
    for child in doc.children(root) {
        if let NodeKind::Element {
            ref name,
            ref attributes,
            ..
        } = doc.node(child).kind
        {
            let category = attributes
                .iter()
                .find(|a| a.name == "category")
                .map_or("unknown", |a| a.value.as_str());
            println!("\n<{name}> (category={category})");

            // Print child elements
            for grandchild in doc.children(child) {
                if let NodeKind::Element { ref name, .. } = doc.node(grandchild).kind {
                    let text = doc.text_content(grandchild);
                    println!("  {name}: {text}");
                }
            }
        }
    }
}
