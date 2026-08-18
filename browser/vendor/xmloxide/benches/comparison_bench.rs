//! Head-to-head benchmark comparing xmloxide against libxml2.
//!
//! Run with: `cargo bench --features bench-libxml2 --bench comparison_bench`
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fmt::Write;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use xmloxide::parser::ParseOptions;
use xmloxide::sax::{parse_sax, SaxHandler};
use xmloxide::serial::serialize;
use xmloxide::xpath::evaluate;
use xmloxide::Document;

#[cfg(feature = "bench-libxml2")]
use libxml::parser::Parser as LibxmlParser;
#[cfg(feature = "bench-libxml2")]
use libxml::xpath::Context as LibxmlXPathContext;

// ---------------------------------------------------------------------------
// Fixture loading
// ---------------------------------------------------------------------------

const ATOM_FEED: &str = include_str!("fixtures/atom_feed.xml");
const SVG_DRAWING: &str = include_str!("fixtures/svg_drawing.xml");
const MAVEN_POM: &str = include_str!("fixtures/maven_pom.xml");
const XHTML_PAGE: &str = include_str!("fixtures/xhtml_page.xml");

/// Generates a large XML document at runtime (~100KB).
fn make_large_xml() -> String {
    let mut xml = String::from("<?xml version=\"1.0\"?>\n<database>\n");
    for i in 0..2000 {
        let _ = writeln!(
            xml,
            "  <record id=\"{i}\" status=\"active\" priority=\"{}\">\
             <name>Record {i}</name>\
             <value>{}</value>\
             <description>This is the description for record number {i} in our database.</description>\
             </record>",
            i % 5,
            i * 42
        );
    }
    xml.push_str("</database>\n");
    xml
}

// ---------------------------------------------------------------------------
// Parse throughput benchmarks
// ---------------------------------------------------------------------------

fn bench_parse_throughput(c: &mut Criterion) {
    let large_xml = make_large_xml();

    let fixtures: Vec<(&str, &str)> = vec![
        ("atom_feed", ATOM_FEED),
        ("svg_drawing", SVG_DRAWING),
        ("maven_pom", MAVEN_POM),
        ("xhtml_page", XHTML_PAGE),
        ("large_generated", &large_xml),
    ];

    let mut group = c.benchmark_group("parse_throughput");

    for (name, xml) in &fixtures {
        group.throughput(criterion::Throughput::Bytes(xml.len() as u64));

        group.bench_with_input(BenchmarkId::new("xmloxide", name), xml, |b, xml| {
            b.iter(|| Document::parse_str(black_box(xml)));
        });

        #[cfg(feature = "bench-libxml2")]
        group.bench_with_input(BenchmarkId::new("libxml2", name), xml, |b, xml| {
            let parser = LibxmlParser::default();
            b.iter(|| parser.parse_string(black_box(xml)));
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Serialize throughput benchmarks
// ---------------------------------------------------------------------------

fn bench_serialize_throughput(c: &mut Criterion) {
    let large_xml = make_large_xml();

    let fixtures: Vec<(&str, &str)> = vec![
        ("atom_feed", ATOM_FEED),
        ("maven_pom", MAVEN_POM),
        ("large_generated", &large_xml),
    ];

    let mut group = c.benchmark_group("serialize_throughput");

    for (name, xml) in &fixtures {
        // xmloxide serialize
        let doc = Document::parse_str(xml).expect("xmloxide parse failed");
        group.bench_with_input(BenchmarkId::new("xmloxide", name), &doc, |b, doc| {
            b.iter(|| serialize(black_box(doc)));
        });

        // libxml2 serialize
        #[cfg(feature = "bench-libxml2")]
        {
            let parser = LibxmlParser::default();
            let libxml_doc = parser.parse_string(xml).expect("libxml2 parse failed");
            group.bench_function(BenchmarkId::new("libxml2", name), |b| {
                b.iter(|| {
                    let _ = black_box(libxml_doc.to_string());
                });
            });
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// XPath benchmarks
// ---------------------------------------------------------------------------

fn bench_xpath(c: &mut Criterion) {
    let expressions: Vec<(&str, &str, &str)> = vec![
        ("simple_path", ATOM_FEED, "//entry/title"),
        ("attribute_pred", MAVEN_POM, "//dependency[scope='test']"),
        ("count_func", ATOM_FEED, "count(//entry)"),
        ("string_func", ATOM_FEED, "string(//feed/title)"),
    ];

    let mut group = c.benchmark_group("xpath");

    for (name, xml, expr) in &expressions {
        // xmloxide xpath
        let doc = Document::parse_str(xml).expect("xmloxide parse failed");
        let root = doc.root();
        group.bench_function(BenchmarkId::new("xmloxide", name), |b| {
            b.iter(|| evaluate(black_box(&doc), root, black_box(expr)));
        });

        // libxml2 xpath
        #[cfg(feature = "bench-libxml2")]
        {
            let parser = LibxmlParser::default();
            let libxml_doc = parser.parse_string(xml).expect("libxml2 parse failed");
            let ctx = LibxmlXPathContext::new(&libxml_doc).expect("xpath context failed");
            group.bench_function(BenchmarkId::new("libxml2", name), |b| {
                b.iter(|| ctx.evaluate(black_box(expr)));
            });
        }
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// SAX streaming benchmark (xmloxide only — libxml crate has no SAX API)
// ---------------------------------------------------------------------------

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

fn bench_sax_streaming(c: &mut Criterion) {
    let large_xml = make_large_xml();

    let fixtures: Vec<(&str, &str)> = vec![
        ("atom_feed", ATOM_FEED),
        ("maven_pom", MAVEN_POM),
        ("large_generated", &large_xml),
    ];

    let mut group = c.benchmark_group("sax_streaming");
    let options = ParseOptions::default();

    for (name, xml) in &fixtures {
        group.throughput(criterion::Throughput::Bytes(xml.len() as u64));
        group.bench_with_input(BenchmarkId::new("xmloxide", name), xml, |b, xml| {
            b.iter(|| {
                let mut handler = CountingHandler {
                    elements: 0,
                    characters: 0,
                };
                parse_sax(black_box(xml), &options, &mut handler).expect("SAX parse failed");
                black_box(handler.elements);
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion groups and main
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_parse_throughput,
    bench_serialize_throughput,
    bench_xpath,
    bench_sax_streaming,
);

criterion_main!(benches);
