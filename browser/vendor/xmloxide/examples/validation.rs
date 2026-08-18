//! DTD, `RelaxNG`, and XSD validation examples.
//!
//! xmloxide supports validating XML documents against DTD, `RelaxNG`, and
//! XML Schema (XSD) schemas.
//!
//! Run with: `cargo run --example validation`
#![allow(clippy::expect_used)]

use xmloxide::validation::dtd::{parse_dtd, validate};
use xmloxide::validation::relaxng::{parse_relaxng, validate as validate_rng};
use xmloxide::validation::xsd::{parse_xsd, validate_xsd};
use xmloxide::Document;

fn main() {
    dtd_example();
    relaxng_example();
    xsd_example();
}

fn dtd_example() {
    println!("=== DTD Validation ===\n");

    let dtd_str = r"
        <!ELEMENT catalog (book+)>
        <!ELEMENT book (title, author)>
        <!ELEMENT title (#PCDATA)>
        <!ELEMENT author (#PCDATA)>
        <!ATTLIST book id ID #REQUIRED>
    ";

    // Valid document
    let valid_xml = r#"<catalog>
        <book id="b1"><title>Rust Programming</title><author>Alice</author></book>
        <book id="b2"><title>XML Essentials</title><author>Bob</author></book>
    </catalog>"#;

    let dtd = parse_dtd(dtd_str).expect("DTD parse failed");
    let mut doc = Document::parse_str(valid_xml).expect("XML parse failed");
    let result = validate(&mut doc, &dtd);
    println!("Valid document:   is_valid={}", result.is_valid);

    // Invalid document (missing required element)
    let invalid_xml = r#"<catalog>
        <book id="b1"><title>No Author</title></book>
    </catalog>"#;

    let mut doc = Document::parse_str(invalid_xml).expect("XML parse failed");
    let result = validate(&mut doc, &dtd);
    println!("Invalid document: is_valid={}", result.is_valid);
    for err in &result.errors {
        println!("  Error: {err}");
    }
    println!();
}

fn relaxng_example() {
    println!("=== RelaxNG Validation ===\n");

    let schema_xml = r#"<element name="person" xmlns="http://relaxng.org/ns/structure/1.0">
        <element name="name"><text/></element>
        <element name="email"><text/></element>
    </element>"#;

    let valid_xml = "<person><name>Alice</name><email>alice@example.com</email></person>";
    let invalid_xml = "<person><name>Bob</name></person>";

    let schema = parse_relaxng(schema_xml).expect("RelaxNG parse failed");

    let doc = Document::parse_str(valid_xml).expect("XML parse failed");
    let result = validate_rng(&doc, &schema);
    println!("Valid document:   is_valid={}", result.is_valid);

    let doc = Document::parse_str(invalid_xml).expect("XML parse failed");
    let result = validate_rng(&doc, &schema);
    println!("Invalid document: is_valid={}", result.is_valid);
    for err in &result.errors {
        println!("  Error: {err}");
    }
    println!();
}

fn xsd_example() {
    println!("=== XSD Validation ===\n");

    let schema_xml = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
        <xs:element name="temperature">
            <xs:complexType>
                <xs:simpleContent>
                    <xs:extension base="xs:decimal">
                        <xs:attribute name="unit" type="xs:string" use="required"/>
                    </xs:extension>
                </xs:simpleContent>
            </xs:complexType>
        </xs:element>
    </xs:schema>"#;

    let valid_xml = r#"<temperature unit="celsius">36.6</temperature>"#;
    let invalid_xml = r#"<temperature unit="celsius">not-a-number</temperature>"#;

    let schema = parse_xsd(schema_xml).expect("XSD parse failed");

    let doc = Document::parse_str(valid_xml).expect("XML parse failed");
    let result = validate_xsd(&doc, &schema);
    println!("Valid document:   is_valid={}", result.is_valid);

    let doc = Document::parse_str(invalid_xml).expect("XML parse failed");
    let result = validate_xsd(&doc, &schema);
    println!("Invalid document: is_valid={}", result.is_valid);
    for err in &result.errors {
        println!("  Error: {err}");
    }
}
