//! SAX2 streaming parser example.
//!
//! Run with: `cargo run --example sax_streaming`
#![allow(clippy::expect_used)]

use xmloxide::parser::ParseOptions;
use xmloxide::sax::{parse_sax, SaxHandler};

/// A handler that tracks element depth and prints events.
struct PrintHandler {
    depth: usize,
}

impl SaxHandler for PrintHandler {
    fn start_document(&mut self) {
        println!("--- Document start ---");
    }

    fn end_document(&mut self) {
        println!("--- Document end ---");
    }

    fn start_element(
        &mut self,
        local_name: &str,
        prefix: Option<&str>,
        _namespace: Option<&str>,
        attributes: &[(String, String, Option<String>, Option<String>)],
    ) {
        let indent = "  ".repeat(self.depth);
        let name = match prefix {
            Some(p) => format!("{p}:{local_name}"),
            None => local_name.to_string(),
        };
        if attributes.is_empty() {
            println!("{indent}<{name}>");
        } else {
            let attrs: Vec<String> = attributes
                .iter()
                .map(|(local, value, _, _)| format!("{local}=\"{value}\""))
                .collect();
            println!("{indent}<{name} {}>", attrs.join(" "));
        }
        self.depth += 1;
    }

    fn end_element(&mut self, local_name: &str, prefix: Option<&str>, _namespace: Option<&str>) {
        self.depth -= 1;
        let indent = "  ".repeat(self.depth);
        let name = match prefix {
            Some(p) => format!("{p}:{local_name}"),
            None => local_name.to_string(),
        };
        println!("{indent}</{name}>");
    }

    fn characters(&mut self, content: &str) {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            let indent = "  ".repeat(self.depth);
            println!("{indent}TEXT: \"{trimmed}\"");
        }
    }
}

fn main() {
    let xml = r#"<?xml version="1.0"?>
<catalog>
  <product id="1" category="electronics">
    <name>Widget</name>
    <price>29.99</price>
  </product>
  <product id="2" category="books">
    <name>XML Handbook</name>
    <price>49.99</price>
  </product>
</catalog>"#;

    let mut handler = PrintHandler { depth: 0 };
    let options = ParseOptions::default();
    parse_sax(xml, &options, &mut handler).expect("SAX parsing failed");
}
