//! Pull-based `XmlReader` streaming example.
//!
//! The `XmlReader` provides a cursor-style interface for reading XML
//! documents one node at a time without building a full DOM tree.
//!
//! Run with: `cargo run --example reader`
#![allow(clippy::expect_used)]

use xmloxide::reader::{XmlNodeType, XmlReader};

fn main() {
    let xml = r#"<?xml version="1.0"?>
<catalog>
  <product id="1" category="electronics">
    <name>Widget</name>
    <price currency="USD">29.99</price>
  </product>
  <product id="2" category="books">
    <name>XML Handbook</name>
    <price currency="USD">49.99</price>
  </product>
</catalog>"#;

    let mut reader = XmlReader::new(xml);
    let mut depth: usize = 0;

    println!("Walking the XML document node by node:\n");

    while reader.read().expect("read failed") {
        let indent = "  ".repeat(depth);
        match reader.node_type() {
            XmlNodeType::Element => {
                let name = reader.name().unwrap_or("?");
                let attr_count = reader.attribute_count();
                if attr_count > 0 {
                    print!("{indent}<{name}");
                    // Walk attributes
                    if reader.move_to_first_attribute() {
                        loop {
                            let aname = reader.name().unwrap_or("?");
                            let aval = reader.value().unwrap_or("?");
                            print!(" {aname}=\"{aval}\"");
                            if !reader.move_to_next_attribute() {
                                break;
                            }
                        }
                        reader.move_to_element();
                    }
                    println!(">");
                } else {
                    println!("{indent}<{name}>");
                }
                if !reader.is_empty_element() {
                    depth += 1;
                }
            }
            XmlNodeType::EndElement => {
                depth -= 1;
                let indent = "  ".repeat(depth);
                let name = reader.name().unwrap_or("?");
                println!("{indent}</{name}>");
            }
            XmlNodeType::Text => {
                let text = reader.value().unwrap_or("");
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    println!("{indent}TEXT: \"{trimmed}\"");
                }
            }
            _ => {}
        }
    }
}
