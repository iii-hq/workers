//! XML serialization and roundtrip example.
//!
//! Run with: `cargo run --example serialize`
#![allow(clippy::expect_used)]

use xmloxide::serial::serialize;
use xmloxide::Document;

fn main() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<root xmlns:app="http://example.com/app">
  <app:config version="2.0">
    <app:setting name="debug">true</app:setting>
    <app:setting name="timeout">30</app:setting>
  </app:config>
  <data>
    <item id="1">First &amp; foremost</item>
    <item id="2">Less &lt;than&gt; more</item>
    <![CDATA[Some <raw> content & stuff]]>
  </data>
</root>"#;

    println!("=== Original XML ===");
    println!("{xml}");

    // Parse
    let doc = Document::parse_str(xml).expect("failed to parse");

    // Serialize
    let output = serialize(&doc);
    println!("\n=== Serialized ===");
    println!("{output}");

    // Roundtrip: parse the serialized output again
    let doc2 = Document::parse_str(&output).expect("roundtrip parse failed");
    let output2 = serialize(&doc2);

    println!("\n=== Roundtrip stable: {} ===", output == output2);
}
