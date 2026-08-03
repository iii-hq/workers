//! Local smoke probe: run every function against a real PDF and print the
//! shape of what comes back.
//!
//!     cargo run --example probe -- <file.pdf> [password]
//!
//! Not a test. Tests must be deterministic and fixture-committed; this is for
//! pointing the worker at a document on the machine and looking at the result.

use pdf::config::WorkerConfig;
use pdf::functions::{classify, items, markdown, regions, text};
use pdf::source::PdfSource;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: probe <file.pdf> [password]");
    let password = args.next();

    pdf::cmaps::materialize();
    let cfg = WorkerConfig::default();
    let src = || PdfSource {
        path: Some(path.clone()),
        bytes_base64: None,
    };

    println!("== {path}");

    let classified = classify::handle(
        classify::Request {
            source: src(),
            password: password.clone(),
            sample_pages: None,
        },
        &cfg,
    )
    .expect("classify");
    println!(
        "classify: {:?} confidence {:.2} pages {} sampled {:?} need-ocr {:?} in {}ms",
        classified.document_type,
        classified.confidence,
        classified.page_count,
        classified.pages_sampled,
        classified.pages_needing_ocr,
        classified.elapsed_ms
    );
    for reason in &classified.ocr_reasons {
        println!("  page {} -> {:?}", reason.page, reason.reasons);
    }

    let md = markdown::handle(
        markdown::Request {
            source: src(),
            password: password.clone(),
            pages: None,
            max_chars: None,
            profile: markdown::Profile::Fidelity,
            include_images: false,
            strip_headers_footers: true,
            per_page: false,
        },
        &cfg,
    )
    .expect("to-markdown");
    println!(
        "markdown: {} of {} chars (truncated {}) tables {:?} columns {:?} encoding-issues {} in {}ms",
        md.body.chars,
        md.body.total_chars,
        md.body.truncated,
        md.pages_with_tables,
        md.pages_with_columns,
        md.has_encoding_issues,
        md.elapsed_ms
    );
    let head: String = md.body.text.chars().take(400).collect();
    println!("--- first 400 chars ---\n{head}\n---");

    let txt = text::handle(
        text::Request {
            source: src(),
            max_chars: Some(0),
        },
        &cfg,
    )
    .expect("extract-text");
    println!("text: {} chars", txt.body.total_chars);

    let it = items::handle(
        items::Request {
            source: src(),
            pages: Some(vec![1]),
            max_items: Some(3),
        },
        &cfg,
    )
    .expect("extract-items");
    println!(
        "items: page 1 has {} items ({} returned, {})",
        it.total_count, it.count, it.coordinate_origin
    );
    for item in &it.items {
        println!(
            "  {:?} {:?} @ ({:.1},{:.1}) {}pt {}",
            item.kind, item.text, item.x, item.y, item.font_size, item.font
        );
    }

    let rg = regions::handle(
        regions::Request {
            source: src(),
            regions: vec![regions::PageRegions {
                page: 1,
                boxes: vec![[0.0, 0.0, 612.0, 400.0]],
            }],
            mode: regions::Mode::Text,
        },
        &cfg,
    )
    .expect("extract-regions");
    let region_text = rg.pages[0].regions[0]
        .text
        .chars()
        .take(120)
        .collect::<String>();
    println!(
        "regions: {} boxes, {} unreliable ({})\n  {:?}",
        rg.region_count, rg.regions_needing_ocr, rg.coordinate_origin, region_text
    );
}
