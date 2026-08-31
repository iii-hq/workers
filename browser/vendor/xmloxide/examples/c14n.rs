//! Canonical XML (C14N) serialization example.
//!
//! Canonical XML produces a deterministic byte-for-byte representation
//! of an XML document, commonly used in digital signatures (XML-DSIG).
//!
//! Run with: `cargo run --example c14n`
#![allow(clippy::expect_used)]

use xmloxide::serial::c14n::{canonicalize, C14nOptions};
use xmloxide::Document;

fn main() {
    // Input with varied formatting, attribute order, and extra whitespace
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- This comment will be stripped by default C14N -->
<doc xmlns:b="http://example.com/b" xmlns:a="http://example.com/a">
  <item   z="3"   a="1"   m="2" >
    <![CDATA[Some text]]>
  </item>
  <empty/>
  <b:element b:attr="val"/>
</doc>"#;

    let doc = Document::parse_str(xml).expect("parse failed");

    // Standard C14N (default includes comments, sorts attributes and namespaces)
    let c14n = canonicalize(&doc, &C14nOptions::default());
    println!("=== C14N (with comments, default) ===\n{c14n}\n");

    // C14N without comments
    let opts_no_comments = C14nOptions {
        with_comments: false,
        ..C14nOptions::default()
    };
    let c14n_no_comments = canonicalize(&doc, &opts_no_comments);
    println!("=== C14N (without comments) ===\n{c14n_no_comments}\n");

    // Exclusive C14N (namespace-aware, used in XML signatures)
    let opts_exclusive = C14nOptions {
        exclusive: true,
        ..C14nOptions::default()
    };
    let c14n_exc = canonicalize(&doc, &opts_exclusive);
    println!("=== Exclusive C14N ===\n{c14n_exc}");
}
