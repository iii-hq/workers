# xmloxide

[![CI](https://github.com/jonwiggins/xmloxide/actions/workflows/ci.yml/badge.svg)](https://github.com/jonwiggins/xmloxide/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/xmloxide.svg)](https://crates.io/crates/xmloxide)
[![docs.rs](https://docs.rs/xmloxide/badge.svg)](https://docs.rs/xmloxide)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.81-blue.svg)](https://www.rust-lang.org)

A pure Rust reimplementation of [libxml2](https://gitlab.gnome.org/GNOME/libxml2) — the de facto standard XML/HTML parsing library in the open-source world.

libxml2 became officially unmaintained in December 2025 with known security issues. xmloxide aims to be a memory-safe, high-performance replacement that passes the same conformance test suites.

## Features

- **Memory-safe** — arena-based tree with zero `unsafe` in the public API
- **Conformant** — 100% pass rate on the W3C XML Conformance Test Suite (1727/1727 applicable tests)
- **Error recovery** — parse malformed XML and still produce a usable tree, just like libxml2
- **Multiple parsing APIs** — DOM tree, SAX2 streaming, XmlReader pull, push/incremental
- **HTML parser** — error-tolerant HTML 4.01 parsing with auto-closing and void elements
- **WHATWG HTML5 parser** — full [HTML Living Standard](https://html.spec.whatwg.org/) tokenizer and tree builder (8810/8810 html5lib-tests passing)
- **HTML5 streaming** — SAX-like callback API for HTML5 (`html5::sax`) that wraps the tokenizer without building a DOM tree
- **CSS selectors** — query elements with familiar CSS syntax (`css::select`) including combinators, pseudo-classes, and fast `#id` lookup
- **XPath 1.0+** — full expression parser and evaluator with all XPath 1.0 core functions plus key XPath 2.0 functions (`matches()`, `replace()`, `tokenize()`, `upper-case()`, `lower-case()`, `abs()`, `min()`, `max()`, and more)
- **Validation** — DTD, RelaxNG, XML Schema (XSD), and ISO Schematron (ISO/IEC 19757-3) validation
- **Serde integration** — optional `serde` feature for XML (de)serialization to/from Rust types
- **Async parsing** — optional `async` feature for parsing from `tokio::io::AsyncRead` sources
- **Canonical XML** — C14N 1.0 and Exclusive C14N serialization
- **XInclude** — document inclusion processing
- **XML Catalogs** — OASIS XML Catalogs for URI resolution
- **`xmllint` CLI** — command-line tool for parsing, validating, and querying XML
- **Zero-copy where possible** — string interning for fast comparisons
- **No global state** — each `Document` is self-contained and `Send + Sync`
- **C/C++ FFI** — full C API with header file (`include/xmloxide.h`) for embedding in C/C++ projects
- **Minimal dependencies** — only `encoding_rs` (library has zero other deps; `clap` is CLI-only)

## Quick Start

```rust
use xmloxide::Document;

let doc = Document::parse_str("<root><child>Hello</child></root>").unwrap();
let root = doc.root_element().unwrap();
assert_eq!(doc.node_name(root), Some("root"));
assert_eq!(doc.text_content(root), "Hello");
```

### Serialization

```rust
use xmloxide::Document;
use xmloxide::serial::serialize;

let doc = Document::parse_str("<root><child>Hello</child></root>").unwrap();
let xml = serialize(&doc);
assert_eq!(xml, "<root><child>Hello</child></root>");
```

### XPath Queries

```rust
use xmloxide::Document;
use xmloxide::xpath::{evaluate, XPathValue};

let doc = Document::parse_str("<library><book><title>Rust</title></book></library>").unwrap();
let root = doc.root_element().unwrap();
let result = evaluate(&doc, root, "count(book)").unwrap();
assert_eq!(result.to_number(), 1.0);
```

### SAX2 Streaming

```rust
use xmloxide::sax::{parse_sax, SaxHandler, DefaultHandler};
use xmloxide::parser::ParseOptions;

struct MyHandler;
impl SaxHandler for MyHandler {
    fn start_element(&mut self, name: &str, _: Option<&str>, _: Option<&str>,
                     _: &[(String, String, Option<String>, Option<String>)]) {
        println!("Element: {name}");
    }
}

parse_sax("<root><child/></root>", &ParseOptions::default(), &mut MyHandler).unwrap();
```

### HTML Parsing

```rust
use xmloxide::html::parse_html;

let doc = parse_html("<p>Hello <br> World").unwrap();
let root = doc.root_element().unwrap();
assert_eq!(doc.node_name(root), Some("html"));
```

### CSS Selectors

```rust
use xmloxide::css::select;
use xmloxide::Document;

let doc = Document::parse_str(r#"<div><p class="intro">Hello</p><p>World</p></div>"#).unwrap();
let root = doc.root_element().unwrap();
let intros = select(&doc, root, "p.intro").unwrap();
assert_eq!(intros.len(), 1);
assert_eq!(doc.text_content(intros[0]), "Hello");
```

### HTML5 Parsing (WHATWG)

```rust
use xmloxide::html5::parse_html5;

let doc = parse_html5("<p>Hello <b>world</b>").unwrap();
let root = doc.root_element().unwrap();
assert_eq!(doc.node_name(root), Some("html"));
```

Fragment parsing (the algorithm behind `innerHTML`) is also supported:

```rust
use xmloxide::html5::{parse_html5_with_options, Html5ParseOptions};

let opts = Html5ParseOptions {
    scripting: false,
    fragment_context: Some("body".to_string()),
};
let doc = parse_html5_with_options("<p>fragment</p>", &opts).unwrap();
```

### HTML5 Streaming (SAX-like)

```rust
use xmloxide::html5::sax::{Html5SaxHandler, parse_html5_sax};

struct LinkExtractor { hrefs: Vec<String> }
impl Html5SaxHandler for LinkExtractor {
    fn start_element(&mut self, name: &str, attrs: &[(String, String)], _sc: bool) {
        if name == "a" {
            if let Some((_, href)) = attrs.iter().find(|(n, _)| n == "href") {
                self.hrefs.push(href.clone());
            }
        }
    }
}

let mut handler = LinkExtractor { hrefs: Vec::new() };
parse_html5_sax(r#"<a href="/page">Link</a>"#, &mut handler);
assert_eq!(handler.hrefs, vec!["/page"]);
```

### Error Recovery

```rust
use xmloxide::parser::{parse_str_with_options, ParseOptions};

let opts = ParseOptions::default().recover(true);
let doc = parse_str_with_options("<root><unclosed>", &opts).unwrap();
for diag in &doc.diagnostics {
    eprintln!("{}", diag);
}
```

## CLI Tool

```sh
# Parse and pretty-print
xmllint --format document.xml

# Validate against a schema
xmllint --schema schema.xsd document.xml
xmllint --relaxng schema.rng document.xml
xmllint --schematron schema.sch document.xml
xmllint --dtdvalid schema.dtd document.xml

# XPath query
xmllint --xpath "//title" document.xml

# Canonical XML
xmllint --c14n document.xml

# Parse HTML
xmllint --html page.html
```

## Module Overview

| Module | Description |
|--------|-------------|
| `tree` | Arena-based DOM tree (`Document`, `NodeId`, `NodeKind`) |
| `parser` | XML 1.0 recursive descent parser with error recovery |
| `parser::push` | Push/incremental parser for chunked input |
| `html` | Error-tolerant HTML 4.01 parser |
| `html5` | WHATWG HTML Living Standard parser (tokenizer + tree builder) |
| `html5::sax` | Streaming SAX-like API for HTML5 (no DOM tree built) |
| `css` | CSS selector engine for querying document trees |
| `sax` | SAX2 streaming event-driven parser |
| `reader` | XmlReader pull-based parsing API |
| `serial` | XML, HTML, and HTML5 serializers, plus Canonical XML (C14N) |
| `xpath` | XPath 1.0+ expression parser and evaluator |
| `validation::dtd` | DTD parsing and validation |
| `validation::relaxng` | RelaxNG schema validation |
| `validation::xsd` | XML Schema (XSD) validation |
| `validation::schematron` | ISO Schematron rule-based validation |
| `serde_xml` | Serde XML (de)serialization (optional `serde` feature) |
| `async_xml` | Async parsing via `tokio::io::AsyncRead` (optional `async` feature) |
| `xinclude` | XInclude 1.0 document inclusion |
| `catalog` | OASIS XML Catalogs for URI resolution |
| `encoding` | Character encoding detection and transcoding |
| `ffi` | C/C++ FFI bindings (`include/xmloxide.h`) |

## Performance

Parsing throughput is competitive with libxml2 — within 3-4% on most documents, and **12% faster** on SVG. Serialization is **1.5-2.4x faster** thanks to the arena-based tree design. XPath is **1.1-2.7x faster** across all benchmarks.

**Parsing:**

| Document | Size | xmloxide | libxml2 | Result |
|----------|------|----------|---------|--------|
| Atom feed | 4.9 KB | 26.7 µs (176 MiB/s) | 25.5 µs (184 MiB/s) | ~4% slower |
| SVG drawing | 6.3 KB | 58.5 µs (103 MiB/s) | 65.6 µs (92 MiB/s) | **12% faster** |
| Maven POM | 11.5 KB | 76.9 µs (142 MiB/s) | 74.2 µs (148 MiB/s) | ~4% slower |
| XHTML page | 10.2 KB | 69.5 µs (139 MiB/s) | 61.5 µs (157 MiB/s) | ~13% slower |
| Large (374 KB) | 374 KB | 2.15 ms (169 MiB/s) | 2.08 ms (175 MiB/s) | ~3% slower |

**Serialization:**

| Document | Size | xmloxide | libxml2 | Result |
|----------|------|----------|---------|--------|
| Atom feed | 4.9 KB | 11.3 µs | 17.5 µs | **1.5x faster** |
| Maven POM | 11.5 KB | 20.1 µs | 47.5 µs | **2.4x faster** |
| Large (374 KB) | 374 KB | 614 µs | 1397 µs | **2.3x faster** |

**XPath:**

| Expression | xmloxide | libxml2 | Result |
|------------|----------|---------|--------|
| Simple path (`//entry/title`) | 1.51 µs | 1.63 µs | **8% faster** |
| Attribute predicate (`//book[@id]`) | 5.91 µs | 15.99 µs | **2.7x faster** |
| `count()` function | 1.09 µs | 1.67 µs | **1.5x faster** |
| `string()` function | 1.32 µs | 1.77 µs | **1.3x faster** |

Key optimizations: arena-based tree for fast serialization, byte-level pre-checks for character validation, bulk text scanning, ASCII fast paths for name parsing, zero-copy element name splitting, inline entity resolution, XPath `//` step fusion with fused axis expansion, inlined tree accessors, and name-test fast paths for child/descendant axes.

```sh
# Run benchmarks (requires libxml2 system library)
cargo bench --features bench-libxml2 --bench comparison_bench
```

## Testing

- **1078 unit tests** across all modules
- **138 FFI tests** covering the full C API surface (including SAX, Schematron, and CSS)
- **libxml2 compatibility suite** — 119/119 tests passing (100%) covering XML parsing, namespaces, error detection, and HTML parsing
- **W3C XML Conformance Test Suite** — 1727/1727 applicable tests passing (100%)
- **html5lib-tests** — 7032/7032 tokenizer tests + 1778/1778 tree construction tests (100%)
- **Integration tests** covering real-world XML/HTML documents, edge cases, and error recovery

```sh
cargo test --all-features
```

## C/C++ FFI

xmloxide provides a C-compatible API for embedding in C/C++ projects (like Chromium, game engines, or any codebase that currently uses libxml2).

```sh
# Build shared + static libraries (uses the included Makefile)
make

# Or build individually:
make shared   # .so / .dylib / .dll
make static   # .a / .lib

# Build and run the C example
make example
```

```c
#include "xmloxide.h"

xmloxide_document *doc = xmloxide_parse_str("<root>Hello</root>");
uint32_t root = xmloxide_doc_root_element(doc);
char *name = xmloxide_node_name(doc, root);   // "root"
char *text = xmloxide_node_text_content(doc, root); // "Hello"

xmloxide_free_string(name);
xmloxide_free_string(text);
xmloxide_free_doc(doc);
```

The full API — including tree navigation and mutation, XPath evaluation, serialization (plain and pretty-printed), HTML/HTML5 parsing, DTD/RelaxNG/XSD/Schematron validation, C14N, SAX streaming, XmlReader, push parser, and XML Catalogs — is declared in [`include/xmloxide.h`](include/xmloxide.h).

## Migrating from libxml2

| libxml2 | xmloxide (Rust) | xmloxide (C FFI) |
|---------|----------------|------------------|
| `xmlReadMemory` | `Document::parse_str` | `xmloxide_parse_str` |
| `xmlReadFile` | `Document::parse_file` | `xmloxide_parse_file` |
| `xmlParseDoc` | `Document::parse_bytes` | `xmloxide_parse_bytes` |
| `htmlReadMemory` | `html::parse_html` | `xmloxide_parse_html` |
| (HTML5 parsing) | `html5::parse_html5` | — |
| (HTML5 fragment / innerHTML) | `html5::parse_html5_with_options` | — |
| (HTML5 streaming) | `html5::sax::parse_html5_sax` | — |
| (CSS selectors / `querySelector`) | `css::select` | — |
| `xmlFreeDoc` | (drop `Document`) | `xmloxide_free_doc` |
| `xmlDocGetRootElement` | `doc.root_element()` | `xmloxide_doc_root_element` |
| `xmlNodeGetContent` | `doc.text_content(id)` | `xmloxide_node_text_content` |
| `xmlNodeSetContent` | `doc.set_text_content(id, s)` | `xmloxide_set_text_content` |
| `xmlGetProp` | `doc.attribute(id, name)` | `xmloxide_node_attribute` |
| `xmlSetProp` | `doc.set_attribute(...)` | `xmloxide_set_attribute` |
| `xmlNewNode` | `doc.create_node(...)` | `xmloxide_create_element` |
| `xmlNewText` | `doc.create_node(Text{..})` | `xmloxide_create_text` |
| `xmlAddChild` | `doc.append_child(p, c)` | `xmloxide_append_child` |
| `xmlAddPrevSibling` | `doc.insert_before(ref, c)` | `xmloxide_insert_before` |
| `xmlUnlinkNode` | `doc.remove_node(id)` | `xmloxide_remove_node` |
| `xmlCopyNode` | `doc.clone_node(id, deep)` | `xmloxide_clone_node` |
| `xmlGetID` | `doc.element_by_id(s)` | `xmloxide_element_by_id` |
| `xmlDocDumpMemory` | `serial::serialize(&doc)` | `xmloxide_serialize` |
| `xmlDocDumpFormatMemory` | `serial::serialize_with_options` | `xmloxide_serialize_pretty` |
| `htmlDocDumpMemory` | `serial::html::serialize_html` | `xmloxide_serialize_html` |
| `xmlC14NDocDumpMemory` | `serial::c14n::canonicalize` | `xmloxide_canonicalize` |
| `xmlXPathEvalExpression` | `xpath::evaluate` | `xmloxide_xpath_eval` |
| `xmlValidateDtd` | `validation::dtd::validate` | `xmloxide_validate_dtd` |
| `xmlRelaxNGValidateDoc` | `validation::relaxng::validate` | `xmloxide_validate_relaxng` |
| `xmlSchemaValidateDoc` | `validation::xsd::validate_xsd` | `xmloxide_validate_xsd` |
| (Schematron validation) | `validation::schematron::validate_schematron` | `xmloxide_validate_schematron` |
| `xmlXIncludeProcess` | `xinclude::process_xincludes` | `xmloxide_process_xincludes` |
| `xmlLoadCatalog` | `Catalog::parse` | `xmloxide_parse_catalog` |
| `xmlSAX2...` callbacks | `sax::SaxHandler` trait | `xmloxide_sax_parse` |
| `xmlTextReaderRead` | `reader::XmlReader` | `xmloxide_reader_read` |
| `xmlCreatePushParserCtxt` | `parser::PushParser` | `xmloxide_push_parser_new` |
| `xmlParseChunk` | `PushParser::push` | `xmloxide_push_parser_push` |

**Thread safety:** Unlike libxml2, xmloxide has no global state. Each `Document` is self-contained and `Send + Sync`. The FFI layer uses thread-local storage for the last error message — each thread has its own error state. No initialization or cleanup functions are needed.

## Fuzzing

xmloxide includes fuzz targets for security testing:

```sh
# Install cargo-fuzz (requires nightly)
cargo install cargo-fuzz

# Run a fuzz target
cargo +nightly fuzz run fuzz_xml_parse
cargo +nightly fuzz run fuzz_html_parse
cargo +nightly fuzz run fuzz_html5_parse
cargo +nightly fuzz run fuzz_html5_fragment
cargo +nightly fuzz run fuzz_xpath
cargo +nightly fuzz run fuzz_roundtrip
cargo +nightly fuzz run fuzz_sax
cargo +nightly fuzz run fuzz_reader
cargo +nightly fuzz run fuzz_push
cargo +nightly fuzz run fuzz_validation
cargo +nightly fuzz run fuzz_schematron
```

## Building

```sh
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo bench
```

Minimum supported Rust version: **1.81**

## Limitations

- **No XML 1.1** — xmloxide implements XML 1.0 (Fifth Edition) only. XML 1.1 is rarely used and not planned.
- **No XSLT** — XSLT is a separate specification (libxslt) and is out of scope.
- **HTML parsers** — both an HTML 4.01 parser (matching libxml2's behavior) and a full WHATWG HTML5 parser are provided. The HTML5 parser passes 100% of html5lib-tests.
- **Push parser buffers internally** — the push/incremental parser API (`PushParser`) currently buffers all pushed data and performs the full parse on `finish()`, rather than truly streaming like libxml2's `xmlParseChunk`. SAX streaming (`parse_sax` for XML, `html5::sax::parse_html5_sax` for HTML5) is available as an alternative for memory-constrained large-document processing.
- **XPath `namespace::` axis** — the `namespace::` axis returns the element node when in-scope namespaces match (rather than materializing separate namespace nodes), following the same pattern as the attribute axis.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for version history.

## License

MIT
