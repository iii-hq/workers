//! `XInclude` document inclusion example.
//!
//! `XInclude` allows XML documents to include content from other sources
//! via `xi:include` elements. xmloxide processes these inclusions using
//! a resolver callback that you provide.
//!
//! Run with: `cargo run --example xinclude`
#![allow(clippy::expect_used)]

use xmloxide::serial::serialize;
use xmloxide::xinclude::{process_xincludes, XIncludeOptions};
use xmloxide::Document;

fn main() {
    // Main document with xi:include elements
    let main_xml = r#"<?xml version="1.0"?>
<manual xmlns:xi="http://www.w3.org/2001/XInclude">
  <title>User Guide</title>
  <xi:include href="chapter1.xml"/>
  <xi:include href="chapter2.xml"/>
  <xi:include href="missing.xml">
    <xi:fallback><section><title>Coming Soon</title></section></xi:fallback>
  </xi:include>
</manual>"#;

    // Simulated external files
    let chapter1 = "<chapter><title>Getting Started</title><p>Welcome to xmloxide.</p></chapter>";
    let chapter2 =
        "<chapter><title>Advanced Usage</title><p>XPath, validation, and more.</p></chapter>";

    let mut doc = Document::parse_str(main_xml).expect("parse failed");

    // Process XIncludes with a resolver that returns file content
    let result = process_xincludes(
        &mut doc,
        |href| match href {
            "chapter1.xml" => Some(chapter1.to_string()),
            "chapter2.xml" => Some(chapter2.to_string()),
            _ => None, // missing.xml will use the fallback
        },
        &XIncludeOptions::default(),
    );

    println!("Inclusions processed: {}", result.inclusions);
    println!("Errors: {}", result.errors.len());
    println!("\nResult:\n{}", serialize(&doc));
}
