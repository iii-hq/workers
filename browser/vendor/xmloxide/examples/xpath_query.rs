//! `XPath` query examples.
//!
//! Run with: `cargo run --example xpath_query`
#![allow(clippy::expect_used)]

use xmloxide::xpath::{evaluate, XPathValue};
use xmloxide::Document;

fn main() {
    let xml = r#"<?xml version="1.0"?>
<library>
  <book genre="fiction" id="1">
    <title>The Great Gatsby</title>
    <author>F. Scott Fitzgerald</author>
    <price>10.99</price>
  </book>
  <book genre="science" id="2">
    <title>A Brief History of Time</title>
    <author>Stephen Hawking</author>
    <price>14.99</price>
  </book>
  <book genre="fiction" id="3">
    <title>1984</title>
    <author>George Orwell</author>
    <price>8.99</price>
  </book>
</library>"#;

    let doc = Document::parse_str(xml).expect("failed to parse XML");
    let root = doc.root_element().expect("no root element");

    // Count all books
    let result = evaluate(&doc, root, "count(book)").expect("XPath failed");
    println!("Total books: {}", result.to_number());

    // Find all fiction books
    println!("\nFiction books:");
    let result = evaluate(&doc, root, "book[@genre='fiction']/title").expect("XPath failed");
    if let XPathValue::NodeSet(nodes) = &result {
        for &node in nodes {
            println!("  - {}", doc.text_content(node.anchor()));
        }
    }

    // Find books over $10
    println!("\nBooks over $10:");
    let result = evaluate(&doc, root, "book[number(price) > 10]/title").expect("XPath failed");
    if let XPathValue::NodeSet(nodes) = &result {
        for &node in nodes {
            println!("  - {}", doc.text_content(node.anchor()));
        }
    }

    // Get a string value
    let result = evaluate(&doc, root, "string(book[@id='2']/author)").expect("XPath failed");
    println!("\nAuthor of book 2: {}", result.to_xpath_string());

    // Sum prices
    let result = evaluate(&doc, root, "sum(book/price)").expect("XPath failed");
    println!("Total price: ${:.2}", result.to_number());
}
