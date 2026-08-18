//! Head-to-head benchmarks comparing xmloxide against roxmltree and quick-xml.
//!
//! Run with: `cargo bench --features bench-rust-xml --bench ecosystem_bench`
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fmt::Write;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use xmloxide::Document;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const ATOM_FEED: &str = include_str!("fixtures/atom_feed.xml");
const SVG_DRAWING: &str = include_str!("fixtures/svg_drawing.xml");
const MAVEN_POM: &str = include_str!("fixtures/maven_pom.xml");

/// Generates a large XML document at runtime (~100KB).
fn make_large_xml() -> String {
    let mut xml = String::from("<?xml version=\"1.0\"?>\n<database>\n");
    for i in 0..2000 {
        let _ = writeln!(
            xml,
            "  <record id=\"{i}\" status=\"active\" priority=\"{}\">\
             <name>Record {i}</name>\
             <value>{}</value>\
             <description>Description for record {i}.</description>\
             </record>",
            i % 5,
            i * 42
        );
    }
    xml.push_str("</database>\n");
    xml
}

// ---------------------------------------------------------------------------
// Parse benchmarks
// ---------------------------------------------------------------------------

fn bench_parse_throughput(c: &mut Criterion) {
    let large = make_large_xml();

    let fixtures: Vec<(&str, &str)> = vec![
        ("atom_feed", ATOM_FEED),
        ("svg_drawing", SVG_DRAWING),
        ("maven_pom", MAVEN_POM),
        ("large_2000", &large),
    ];

    let mut group = c.benchmark_group("parse");

    for (name, xml) in &fixtures {
        let bytes = xml.len() as u64;
        group.throughput(Throughput::Bytes(bytes));

        group.bench_with_input(BenchmarkId::new("xmloxide", name), xml, |b, xml| {
            b.iter(|| {
                let doc = Document::parse_str(black_box(xml)).unwrap();
                black_box(doc.root_element());
            });
        });

        group.bench_with_input(BenchmarkId::new("roxmltree", name), xml, |b, xml| {
            b.iter(|| {
                let doc = roxmltree::Document::parse(black_box(xml)).unwrap();
                black_box(doc.root_element());
            });
        });

        group.bench_with_input(BenchmarkId::new("quick-xml/reader", name), xml, |b, xml| {
            b.iter(|| {
                use quick_xml::events::Event;
                use quick_xml::Reader;
                let mut reader = Reader::from_str(black_box(xml));
                let mut count = 0u64;
                let mut buf = Vec::new();
                loop {
                    match reader.read_event_into(&mut buf) {
                        Ok(Event::Eof) => break,
                        Ok(_) => count += 1,
                        Err(e) => panic!("quick-xml error: {e}"),
                    }
                    buf.clear();
                }
                black_box(count);
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Tree navigation benchmarks
// ---------------------------------------------------------------------------

fn bench_tree_walk(c: &mut Criterion) {
    let large = make_large_xml();

    let mut group = c.benchmark_group("tree_walk");

    // xmloxide: walk all nodes and count elements
    group.bench_function("xmloxide", |b| {
        let doc = Document::parse_str(&large).unwrap();
        let root = doc.root_element().unwrap();
        b.iter(|| {
            let mut count = 0u64;
            for node in doc.descendants(black_box(root)) {
                if doc.is_element(node) {
                    count += 1;
                }
            }
            black_box(count)
        });
    });

    // roxmltree: walk all nodes and count elements
    group.bench_function("roxmltree", |b| {
        let doc = roxmltree::Document::parse(&large).unwrap();
        let root = doc.root_element();
        b.iter(|| {
            let mut count = 0u64;
            for node in black_box(root).descendants() {
                if node.is_element() {
                    count += 1;
                }
            }
            black_box(count)
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Attribute access benchmarks
// ---------------------------------------------------------------------------

fn bench_attr_access(c: &mut Criterion) {
    let large = make_large_xml();

    let mut group = c.benchmark_group("attr_access");

    // xmloxide: look up 'id' attribute on every element
    group.bench_function("xmloxide", |b| {
        let doc = Document::parse_str(&large).unwrap();
        let root = doc.root_element().unwrap();
        b.iter(|| {
            let mut count = 0u64;
            for node in doc.descendants(black_box(root)) {
                if doc.attribute(node, "id").is_some() {
                    count += 1;
                }
            }
            black_box(count)
        });
    });

    // roxmltree: look up 'id' attribute on every element
    group.bench_function("roxmltree", |b| {
        let doc = roxmltree::Document::parse(&large).unwrap();
        let root = doc.root_element();
        b.iter(|| {
            let mut count = 0u64;
            for node in black_box(root).descendants() {
                if node.attribute("id").is_some() {
                    count += 1;
                }
            }
            black_box(count)
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Serialization benchmarks
// ---------------------------------------------------------------------------

fn bench_serialize(c: &mut Criterion) {
    let large = make_large_xml();

    let mut group = c.benchmark_group("serialize");
    group.throughput(Throughput::Bytes(large.len() as u64));

    // xmloxide: serialize
    group.bench_function("xmloxide", |b| {
        let doc = Document::parse_str(&large).unwrap();
        b.iter(|| {
            let out = xmloxide::serial::serialize(black_box(&doc));
            black_box(out.len());
        });
    });

    // roxmltree doesn't have serialization, so we only compare xmloxide here
    // quick-xml writer is a different API (not DOM-to-string)

    group.finish();
}

// ---------------------------------------------------------------------------
// CSS selector benchmarks (xmloxide only — others don't have CSS)
// ---------------------------------------------------------------------------

fn bench_css_selector(c: &mut Criterion) {
    let large = make_large_xml();

    let mut group = c.benchmark_group("css_selector");

    group.bench_function("xmloxide/tag", |b| {
        let doc = Document::parse_str(&large).unwrap();
        let root = doc.root_element().unwrap();
        b.iter(|| {
            let results = xmloxide::css::select(black_box(&doc), root, "record").unwrap();
            black_box(results.len());
        });
    });

    group.bench_function("xmloxide/attr", |b| {
        let doc = Document::parse_str(&large).unwrap();
        let root = doc.root_element().unwrap();
        b.iter(|| {
            let results = xmloxide::css::select(black_box(&doc), root, "[priority=\"0\"]").unwrap();
            black_box(results.len());
        });
    });

    group.bench_function("xmloxide/complex", |b| {
        let doc = Document::parse_str(&large).unwrap();
        let root = doc.root_element().unwrap();
        b.iter(|| {
            let results = xmloxide::css::select(black_box(&doc), root, "record > name").unwrap();
            black_box(results.len());
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_throughput,
    bench_tree_walk,
    bench_attr_access,
    bench_serialize,
    bench_css_selector,
);
criterion_main!(benches);
