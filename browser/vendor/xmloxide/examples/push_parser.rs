//! Push/incremental parser example.
//!
//! The push parser accepts XML data in arbitrarily sized chunks, useful
//! when data arrives incrementally (network sockets, streaming, etc.).
//!
//! Run with: `cargo run --example push_parser`
#![allow(clippy::expect_used)]

use xmloxide::parser::PushParser;
use xmloxide::tree::NodeKind;

fn main() {
    // Simulate receiving XML in chunks (e.g., from a network stream)
    let chunks = [
        b"<?xml version=\"1.0\"?>" as &[u8],
        b"<inventory>",
        b"  <item sku=\"A10",
        b"1\"><name>Bolt</name>",
        b"<qty>500</qty></item>",
        b"  <item sku=\"B202\">",
        b"<name>Nut</name><qty>",
        b"1000</qty></item>",
        b"</inventory>",
    ];

    let mut parser = PushParser::new();

    for (i, chunk) in chunks.iter().enumerate() {
        parser.push(chunk);
        println!(
            "Pushed chunk {} ({} bytes, {} total buffered)",
            i + 1,
            chunk.len(),
            parser.buffered_bytes()
        );
    }

    let doc = parser.finish().expect("parsing failed");
    let root = doc.root_element().expect("no root element");

    println!("\nParsed document:");
    println!("Root: {}", doc.node_name(root).unwrap_or("?"));

    for child in doc.children(root) {
        if let NodeKind::Element {
            ref name,
            ref attributes,
            ..
        } = doc.node(child).kind
        {
            let sku = attributes
                .iter()
                .find(|a| a.name == "sku")
                .map_or("?", |a| a.value.as_str());
            print!("  <{name} sku=\"{sku}\">");

            for field in doc.children(child) {
                if let NodeKind::Element { ref name, .. } = doc.node(field).kind {
                    print!(" {name}={}", doc.text_content(field));
                }
            }
            println!();
        }
    }
}
