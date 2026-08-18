#![allow(clippy::expect_used)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::fmt::Write;
use xmloxide::css;
use xmloxide::html::parse_html;
use xmloxide::html5::{parse_html5, parse_html5_with_options, Html5ParseOptions};
use xmloxide::parser::{ParseOptions, PushParser};
use xmloxide::reader::XmlReader;
use xmloxide::sax::{parse_sax, SaxHandler};
use xmloxide::serial::serialize;
use xmloxide::validation::{dtd, relaxng, schematron, xsd};
use xmloxide::xpath::evaluate;
use xmloxide::Document;

// ---------------------------------------------------------------------------
// Document generators
// ---------------------------------------------------------------------------

/// Generates a small XML document with approximately 10 elements.
fn make_small_xml() -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<root>\n");
    for i in 0..10 {
        let _ = writeln!(xml, "  <item id=\"{i}\">Value {i}</item>");
    }
    xml.push_str("</root>\n");
    xml
}

/// Generates a medium XML document with approximately 100 elements.
fn make_medium_xml() -> String {
    let mut xml = String::from("<?xml version=\"1.0\"?>\n<catalog>\n");
    for i in 0..100 {
        let _ = writeln!(
            xml,
            "  <book id=\"bk{i}\"><title>Title {i}</title>\
             <author>Author {i}</author>\
             <price>{}.99</price></book>",
            10 + i
        );
    }
    xml.push_str("</catalog>\n");
    xml
}

/// Generates a large XML document with approximately 1000 elements.
fn make_large_xml() -> String {
    let mut xml = String::from("<?xml version=\"1.0\"?>\n<database>\n");
    for i in 0..1000 {
        let _ = writeln!(
            xml,
            "  <record id=\"{i}\"><name>Record {i}</name>\
             <value>{}</value><status>active</status></record>",
            i * 42
        );
    }
    xml.push_str("</database>\n");
    xml
}

/// Generates a deeply nested XML document with the given nesting depth.
fn make_nested_xml(depth: usize) -> String {
    let mut xml = String::from("<?xml version=\"1.0\"?>\n");
    for i in 0..depth {
        let _ = write!(xml, "<level{i}>");
    }
    xml.push_str("leaf");
    for i in (0..depth).rev() {
        let _ = write!(xml, "</level{i}>");
    }
    xml.push('\n');
    xml
}

/// Generates an XML document where each element has `num_attrs` attributes.
fn make_attr_heavy_xml(num_attrs: usize) -> String {
    let mut xml = String::from("<?xml version=\"1.0\"?>\n<root>\n");
    for i in 0..10 {
        let _ = write!(xml, "  <element");
        for j in 0..num_attrs {
            let _ = write!(xml, " attr{j}=\"value_{i}_{j}\"");
        }
        xml.push_str("/>\n");
    }
    xml.push_str("</root>\n");
    xml
}

/// Generates an XML document with many namespace declarations and prefixed
/// elements.
fn make_namespace_heavy_xml() -> String {
    let mut xml = String::from("<?xml version=\"1.0\"?>\n<root");
    for i in 0..20 {
        let _ = write!(xml, " xmlns:ns{i}=\"http://example.com/ns{i}\"");
    }
    xml.push_str(">\n");
    for i in 0..100 {
        let ns = i % 20;
        let _ = writeln!(
            xml,
            "  <ns{ns}:item ns{ns}:id=\"{i}\">Content {i}</ns{ns}:item>"
        );
    }
    xml.push_str("</root>\n");
    xml
}

/// Generates an HTML document for benchmarking the HTML parser.
fn make_html_doc() -> String {
    let mut html = String::from(
        "<!DOCTYPE html>\n<html>\n<head>\n\
         <title>Benchmark Page</title>\n\
         <meta charset=\"utf-8\">\n\
         <link rel=\"stylesheet\" href=\"style.css\">\n\
         </head>\n<body>\n<h1>Benchmark</h1>\n",
    );
    for i in 0..50 {
        let _ = writeln!(
            html,
            "<div class=\"section\" id=\"s{i}\">\
             <p>Paragraph {i} with <b>bold</b> and <i>italic</i> text.</p>\
             <ul><li>Item A</li><li>Item B</li><li>Item C</li></ul>\
             <img src=\"img{i}.png\" alt=\"Image {i}\">\
             <a href=\"#s{i}\">Link {i}</a>\
             </div>"
        );
    }
    html.push_str("</body>\n</html>\n");
    html
}

/// Generates a medium XML document suitable for `XPath` benchmarks, with a
/// structure that exercises path navigation and predicates.
fn make_xpath_xml() -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\"?>\n\
         <library>\n",
    );
    for i in 0..50 {
        let genre = match i % 4 {
            0 => "fiction",
            1 => "science",
            2 => "history",
            _ => "poetry",
        };
        let _ = writeln!(
            xml,
            "  <book genre=\"{genre}\" id=\"{i}\">\
             <title>Book {i}</title>\
             <author>Author {}</author>\
             <year>{}</year>\
             <price>{}.99</price>\
             </book>",
            i % 10,
            2000 + i,
            10 + i
        );
    }
    xml.push_str("</library>\n");
    xml
}

// ---------------------------------------------------------------------------
// XML Parsing benchmarks
// ---------------------------------------------------------------------------

fn bench_parse_small(c: &mut Criterion) {
    let xml = make_small_xml();
    c.bench_function("parse_small", |b| {
        b.iter(|| Document::parse_str(black_box(&xml)));
    });
}

fn bench_parse_medium(c: &mut Criterion) {
    let xml = make_medium_xml();
    c.bench_function("parse_medium", |b| {
        b.iter(|| Document::parse_str(black_box(&xml)));
    });
}

fn bench_parse_large(c: &mut Criterion) {
    let xml = make_large_xml();
    c.bench_function("parse_large", |b| {
        b.iter(|| Document::parse_str(black_box(&xml)));
    });
}

fn bench_parse_deeply_nested(c: &mut Criterion) {
    let xml = make_nested_xml(50);
    c.bench_function("parse_deeply_nested", |b| {
        b.iter(|| Document::parse_str(black_box(&xml)));
    });
}

fn bench_parse_many_attributes(c: &mut Criterion) {
    let xml = make_attr_heavy_xml(50);
    c.bench_function("parse_many_attributes", |b| {
        b.iter(|| Document::parse_str(black_box(&xml)));
    });
}

fn bench_parse_namespace_heavy(c: &mut Criterion) {
    let xml = make_namespace_heavy_xml();
    c.bench_function("parse_namespace_heavy", |b| {
        b.iter(|| Document::parse_str(black_box(&xml)));
    });
}

// ---------------------------------------------------------------------------
// Serialization benchmarks
// ---------------------------------------------------------------------------

fn bench_serialize_small(c: &mut Criterion) {
    let xml = make_small_xml();
    let doc = Document::parse_str(&xml).expect("failed to parse small XML");
    c.bench_function("serialize_small", |b| {
        b.iter(|| serialize(black_box(&doc)));
    });
}

fn bench_serialize_large(c: &mut Criterion) {
    let xml = make_large_xml();
    let doc = Document::parse_str(&xml).expect("failed to parse large XML");
    c.bench_function("serialize_large", |b| {
        b.iter(|| serialize(black_box(&doc)));
    });
}

// ---------------------------------------------------------------------------
// HTML parsing benchmark
// ---------------------------------------------------------------------------

fn bench_parse_html(c: &mut Criterion) {
    let html = make_html_doc();
    c.bench_function("parse_html", |b| {
        b.iter(|| parse_html(black_box(&html)));
    });
}

// ---------------------------------------------------------------------------
// SAX parsing benchmark
// ---------------------------------------------------------------------------

/// A minimal SAX handler that counts elements, used for benchmarking the SAX
/// parsing path without allocation overhead from recording events.
struct CountingHandler {
    elements: u64,
    characters: u64,
}

impl SaxHandler for CountingHandler {
    fn start_element(
        &mut self,
        _local_name: &str,
        _prefix: Option<&str>,
        _namespace: Option<&str>,
        _attributes: &[(String, String, Option<String>, Option<String>)],
    ) {
        self.elements += 1;
    }

    fn characters(&mut self, _content: &str) {
        self.characters += 1;
    }
}

fn bench_sax_parse(c: &mut Criterion) {
    let xml = make_medium_xml();
    let options = ParseOptions::default();
    c.bench_function("sax_parse", |b| {
        b.iter(|| {
            let mut handler = CountingHandler {
                elements: 0,
                characters: 0,
            };
            parse_sax(black_box(&xml), &options, &mut handler).expect("SAX parse failed");
            black_box(handler.elements);
        });
    });
}

// ---------------------------------------------------------------------------
// XmlReader benchmark
// ---------------------------------------------------------------------------

fn bench_reader_parse(c: &mut Criterion) {
    let xml = make_medium_xml();
    c.bench_function("reader_parse", |b| {
        b.iter(|| {
            let mut reader = XmlReader::new(black_box(&xml));
            let mut count: u64 = 0;
            while reader.read().expect("reader failed") {
                count += 1;
            }
            black_box(count);
        });
    });
}

// ---------------------------------------------------------------------------
// XPath benchmarks
// ---------------------------------------------------------------------------

fn bench_xpath_simple(c: &mut Criterion) {
    let xml = make_xpath_xml();
    let doc = Document::parse_str(&xml).expect("failed to parse XPath XML");
    let root = doc.root_element().expect("no root element");
    c.bench_function("xpath_simple", |b| {
        b.iter(|| evaluate(black_box(&doc), root, "//book/title"));
    });
}

fn bench_xpath_complex(c: &mut Criterion) {
    let xml = make_xpath_xml();
    let doc = Document::parse_str(&xml).expect("failed to parse XPath XML");
    let root = doc.root_element().expect("no root element");
    c.bench_function("xpath_complex", |b| {
        b.iter(|| {
            evaluate(
                black_box(&doc),
                root,
                "//book[@genre='fiction' and number(price) > 20]/title",
            )
        });
    });
}

// ---------------------------------------------------------------------------
// Roundtrip benchmark: parse -> serialize -> parse
// ---------------------------------------------------------------------------

fn bench_roundtrip(c: &mut Criterion) {
    let xml = make_medium_xml();
    c.bench_function("roundtrip", |b| {
        b.iter(|| {
            let doc = Document::parse_str(black_box(&xml)).expect("parse failed");
            let serialized = serialize(&doc);
            let doc2 = Document::parse_str(&serialized).expect("re-parse failed");
            black_box(doc2);
        });
    });
}

// ---------------------------------------------------------------------------
// Push parser benchmark
// ---------------------------------------------------------------------------

fn bench_push_parser(c: &mut Criterion) {
    let xml = make_medium_xml();
    let bytes = xml.as_bytes();
    // Split into ~64-byte chunks to simulate incremental feeding.
    let chunk_size = 64;
    let chunks: Vec<&[u8]> = bytes.chunks(chunk_size).collect();
    c.bench_function("push_parser", |b| {
        b.iter(|| {
            let mut parser = PushParser::new();
            for chunk in &chunks {
                parser.push(black_box(chunk));
            }
            parser.finish().expect("push parse failed")
        });
    });
}

// ---------------------------------------------------------------------------
// Criterion groups and main
// ---------------------------------------------------------------------------

criterion_group!(
    parsing,
    bench_parse_small,
    bench_parse_medium,
    bench_parse_large,
    bench_parse_deeply_nested,
    bench_parse_many_attributes,
    bench_parse_namespace_heavy,
);

criterion_group!(serialization, bench_serialize_small, bench_serialize_large,);

// ---------------------------------------------------------------------------
// HTML5 parsing benchmarks
// ---------------------------------------------------------------------------

fn bench_parse_html5(c: &mut Criterion) {
    let html = make_html_doc();
    c.bench_function("parse_html5", |b| {
        b.iter(|| parse_html5(black_box(&html)));
    });
}

fn bench_parse_html5_fragment(c: &mut Criterion) {
    let html = make_html_doc();
    let opts = Html5ParseOptions {
        scripting: false,
        fragment_context: Some("body".to_string()),
    };
    c.bench_function("parse_html5_fragment", |b| {
        b.iter(|| parse_html5_with_options(black_box(&html), &opts));
    });
}

// ---------------------------------------------------------------------------
// Additional XPath benchmarks
// ---------------------------------------------------------------------------

fn bench_xpath_count(c: &mut Criterion) {
    let xml = make_xpath_xml();
    let doc = Document::parse_str(&xml).expect("failed to parse XPath XML");
    let root = doc.root_element().expect("no root element");
    c.bench_function("xpath_count", |b| {
        b.iter(|| evaluate(black_box(&doc), root, "count(//book)"));
    });
}

fn bench_xpath_string_function(c: &mut Criterion) {
    let xml = make_xpath_xml();
    let doc = Document::parse_str(&xml).expect("failed to parse XPath XML");
    let root = doc.root_element().expect("no root element");
    c.bench_function("xpath_string_function", |b| {
        b.iter(|| evaluate(black_box(&doc), root, "string(//book[1]/title)"));
    });
}

fn bench_xpath_position_predicate(c: &mut Criterion) {
    let xml = make_xpath_xml();
    let doc = Document::parse_str(&xml).expect("failed to parse XPath XML");
    let root = doc.root_element().expect("no root element");
    c.bench_function("xpath_position_predicate", |b| {
        b.iter(|| {
            evaluate(
                black_box(&doc),
                root,
                "//book[position() > 10 and position() < 20]",
            )
        });
    });
}

fn bench_xpath_ancestor(c: &mut Criterion) {
    let xml = make_xpath_xml();
    let doc = Document::parse_str(&xml).expect("failed to parse XPath XML");
    let root = doc.root_element().expect("no root element");
    // Get a deep node to evaluate ancestor axis from
    let result = evaluate(&doc, root, "//book[1]/title").expect("xpath failed");
    let title_node = result.as_node_set().expect("expected nodeset")[0].anchor();
    c.bench_function("xpath_ancestor", |b| {
        b.iter(|| evaluate(black_box(&doc), title_node, "ancestor::*"));
    });
}

fn bench_xpath_union(c: &mut Criterion) {
    let xml = make_xpath_xml();
    let doc = Document::parse_str(&xml).expect("failed to parse XPath XML");
    let root = doc.root_element().expect("no root element");
    c.bench_function("xpath_union", |b| {
        b.iter(|| evaluate(black_box(&doc), root, "//title | //author | //year"));
    });
}

// ---------------------------------------------------------------------------
// Validation benchmarks
// ---------------------------------------------------------------------------

/// DTD for validating the medium XML (catalog of books).
fn make_book_dtd() -> String {
    String::from(
        "<!ELEMENT catalog (book*)>\n\
         <!ELEMENT book (title, author, price)>\n\
         <!ATTLIST book id ID #REQUIRED>\n\
         <!ELEMENT title (#PCDATA)>\n\
         <!ELEMENT author (#PCDATA)>\n\
         <!ELEMENT price (#PCDATA)>\n",
    )
}

fn bench_validate_dtd(c: &mut Criterion) {
    let xml = make_medium_xml();
    let dtd_str = make_book_dtd();
    let dtd_schema = dtd::parse_dtd(&dtd_str).expect("DTD parse failed");
    c.bench_function("validate_dtd", |b| {
        b.iter(|| {
            let mut doc = Document::parse_str(black_box(&xml)).expect("parse failed");
            dtd::validate(black_box(&mut doc), &dtd_schema)
        });
    });
}

fn bench_validate_relaxng(c: &mut Criterion) {
    let schema_xml = r#"<?xml version="1.0"?>
<element name="catalog" xmlns="http://relaxng.org/ns/structure/1.0">
  <zeroOrMore>
    <element name="book">
      <attribute name="id"/>
      <element name="title"><text/></element>
      <element name="author"><text/></element>
      <element name="price"><text/></element>
    </element>
  </zeroOrMore>
</element>"#;
    let schema = relaxng::parse_relaxng(schema_xml).expect("RelaxNG parse failed");
    let xml = make_medium_xml();
    let doc = Document::parse_str(&xml).expect("parse failed");
    c.bench_function("validate_relaxng", |b| {
        b.iter(|| relaxng::validate(black_box(&doc), &schema));
    });
}

fn bench_validate_xsd(c: &mut Criterion) {
    let schema_xml = r#"<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:element name="catalog">
    <xs:complexType>
      <xs:sequence>
        <xs:element name="book" maxOccurs="unbounded" minOccurs="0">
          <xs:complexType>
            <xs:sequence>
              <xs:element name="title" type="xs:string"/>
              <xs:element name="author" type="xs:string"/>
              <xs:element name="price" type="xs:string"/>
            </xs:sequence>
            <xs:attribute name="id" type="xs:string" use="required"/>
          </xs:complexType>
        </xs:element>
      </xs:sequence>
    </xs:complexType>
  </xs:element>
</xs:schema>"#;
    let schema = xsd::parse_xsd(schema_xml).expect("XSD parse failed");
    let xml = make_medium_xml();
    let doc = Document::parse_str(&xml).expect("parse failed");
    c.bench_function("validate_xsd", |b| {
        b.iter(|| xsd::validate_xsd(black_box(&doc), &schema));
    });
}

fn bench_validate_schematron(c: &mut Criterion) {
    let schema_xml = r#"<schema xmlns="http://purl.oclc.org/dml/schematron">
  <pattern>
    <rule context="book">
      <assert test="title">book must have a title</assert>
      <assert test="author">book must have an author</assert>
      <assert test="@id">book must have an id attribute</assert>
    </rule>
  </pattern>
</schema>"#;
    let schema = schematron::parse_schematron(schema_xml).expect("Schematron parse failed");
    let xml = make_medium_xml();
    let doc = Document::parse_str(&xml).expect("parse failed");
    c.bench_function("validate_schematron", |b| {
        b.iter(|| schematron::validate_schematron(black_box(&doc), &schema));
    });
}

// ---------------------------------------------------------------------------
// CSS selector benchmark
// ---------------------------------------------------------------------------

fn bench_css_select(c: &mut Criterion) {
    let html = make_html_doc();
    let doc = parse_html5(&html).expect("html5 parse failed");
    let root = doc.root_element().expect("no root");
    c.bench_function("css_select_class", |b| {
        b.iter(|| css::select(black_box(&doc), root, "div.section"));
    });
}

fn bench_css_select_complex(c: &mut Criterion) {
    let html = make_html_doc();
    let doc = parse_html5(&html).expect("html5 parse failed");
    let root = doc.root_element().expect("no root");
    c.bench_function("css_select_complex", |b| {
        b.iter(|| css::select(black_box(&doc), root, "div.section > p > b"));
    });
}

// ---------------------------------------------------------------------------
// Criterion groups and main
// ---------------------------------------------------------------------------

criterion_group!(html_parsing, bench_parse_html);

criterion_group!(html5_parsing, bench_parse_html5, bench_parse_html5_fragment);

criterion_group!(sax, bench_sax_parse);

criterion_group!(reader, bench_reader_parse);

criterion_group!(
    xpath,
    bench_xpath_simple,
    bench_xpath_complex,
    bench_xpath_count,
    bench_xpath_string_function,
    bench_xpath_position_predicate,
    bench_xpath_ancestor,
    bench_xpath_union,
);

criterion_group!(roundtrip, bench_roundtrip);

criterion_group!(push, bench_push_parser);

criterion_group!(
    validation,
    bench_validate_dtd,
    bench_validate_relaxng,
    bench_validate_xsd,
    bench_validate_schematron,
);

criterion_group!(css_selectors, bench_css_select, bench_css_select_complex,);

criterion_main!(
    parsing,
    serialization,
    html_parsing,
    html5_parsing,
    sax,
    reader,
    xpath,
    roundtrip,
    push,
    validation,
    css_selectors,
);
