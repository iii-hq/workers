//! Demonstrates error recovery on malformed XML.
//!
//! Run with: `cargo run --example error_recovery`
#![allow(clippy::expect_used)]

use xmloxide::parser::{parse_str_with_options, ParseOptions};
use xmloxide::serial::serialize;

fn main() {
    let malformed_docs = vec![
        ("Missing end tag", "<root><child>text</root>"),
        ("Unclosed root", "<root><a>hello</a><b>world</b>"),
        (
            "Duplicate attributes",
            "<root attr=\"1\" attr=\"2\">text</root>",
        ),
        ("Mismatched tags", "<a><b>text</a></b>"),
        (
            "Invalid characters",
            "<root>valid text <!-- comment --> more</root>",
        ),
    ];

    let opts = ParseOptions::default().recover(true);

    for (label, xml) in malformed_docs {
        println!("=== {label} ===");
        println!("Input:  {xml}");

        match parse_str_with_options(xml, &opts) {
            Ok(doc) => {
                let output = serialize(&doc);
                println!("Output: {output}");
                if doc.diagnostics.is_empty() {
                    println!("(no diagnostics)");
                } else {
                    for diag in &doc.diagnostics {
                        println!("  warning: {diag}");
                    }
                }
            }
            Err(e) => {
                println!("Fatal error: {e}");
            }
        }
        println!();
    }
}
