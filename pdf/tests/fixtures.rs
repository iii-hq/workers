//! Behaviour against real documents.
//!
//! The unit tests cover the pure logic; these run the actual parser over the
//! committed fixtures, which is where a dependency upgrade that changes what a
//! document produces will show up.
//!
//! Both fixtures are built by `tests/fixtures/make_fixtures.py` from raw PDF
//! syntax, so their content is known exactly and the assertions can be precise
//! rather than "returns something".

use pdf::config::WorkerConfig;
use pdf::functions::classify::DocumentType;
use pdf::functions::{classify, items, markdown, regions, text};
use pdf::source::PdfSource;

fn fixture(name: &str) -> PdfSource {
    PdfSource {
        path: Some(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        )),
        bytes_base64: None,
        // Unstamped: these exercise the handlers, not the jail, which has its
        // own tests in `src/source.rs`.
        fs_scope: None,
    }
}

fn cfg() -> WorkerConfig {
    WorkerConfig::default()
}

// ---------------------------------------------------------------------------
// classify
// ---------------------------------------------------------------------------

#[test]
fn a_text_document_classifies_as_text_based_and_needs_no_ocr() {
    let result = classify::handle(
        classify::Request {
            source: fixture("text-two-page.pdf"),
            password: None,
            sample_pages: None,
        },
        &cfg(),
    )
    .expect("classify");

    assert_eq!(result.document_type, DocumentType::TextBased);
    assert_eq!(result.page_count, 2);
    assert!(result.pages_needing_ocr.is_empty());
    assert!(result.ocr_reasons.is_empty());
    assert_eq!(result.source, "text-two-page.pdf");
}

/// The document-level verdict is not enough on its own: the per-page reason is
/// what tells a caller whether to send pages to a vision model.
#[test]
fn a_document_with_no_text_is_flagged_page_by_page() {
    let result = classify::handle(
        classify::Request {
            source: fixture("no-text.pdf"),
            password: None,
            sample_pages: None,
        },
        &cfg(),
    )
    .expect("classify");

    assert_ne!(result.document_type, DocumentType::TextBased);
    assert_eq!(result.pages_needing_ocr, vec![1]);
    assert_eq!(result.ocr_reasons.len(), 1);
    assert_eq!(result.ocr_reasons[0].page, 1);
    assert!(
        result.ocr_reasons[0]
            .reasons
            .contains(&"no_text".to_string()),
        "expected a no_text reason, got {:?}",
        result.ocr_reasons[0].reasons
    );
}

/// Pages needing OCR are reported 1-indexed. The parser has a second
/// classification entry point that counts from zero, so a refactor onto it
/// would turn page 1 into page 0 with no compile error.
#[test]
fn ocr_pages_are_reported_one_indexed() {
    let result = classify::handle(
        classify::Request {
            source: fixture("no-text.pdf"),
            password: None,
            sample_pages: None,
        },
        &cfg(),
    )
    .expect("classify");

    assert!(
        !result.pages_needing_ocr.contains(&0),
        "page 0 is not a page; the numbering slipped to 0-indexed"
    );
}

#[test]
fn the_sample_counters_are_present_for_an_unencrypted_document() {
    let result = classify::handle(
        classify::Request {
            source: fixture("text-two-page.pdf"),
            password: None,
            sample_pages: None,
        },
        &cfg(),
    )
    .expect("classify");

    assert_eq!(result.pages_sampled, Some(2));
    assert_eq!(result.pages_with_text, Some(2));
    assert!(result.ocr_recommended.is_some());
}

/// `min_text_ops_per_page` is a real lever, not decoration: a page whose text
/// operators fall under it stops counting as a text page. The fixture draws
/// three operators per page, so raising the bar past three must empty the
/// count.
///
/// The document-level verdict deliberately is NOT asserted here. A document
/// with no images is not called scanned merely because its operator counts are
/// low, so the type stays text-based while the counter goes to zero. That is
/// the parser's behaviour and worth pinning as a fact rather than assuming the
/// verdict tracks the counter.
#[test]
fn the_text_operator_threshold_changes_what_counts_as_a_text_page() {
    let default = classify::handle(
        classify::Request {
            source: fixture("text-two-page.pdf"),
            password: None,
            sample_pages: None,
        },
        &cfg(),
    )
    .expect("classify");
    assert_eq!(default.pages_with_text, Some(2));

    let strict = WorkerConfig {
        min_text_ops_per_page: 4,
        ..WorkerConfig::default()
    };
    let result = classify::handle(
        classify::Request {
            source: fixture("text-two-page.pdf"),
            password: None,
            sample_pages: None,
        },
        &strict,
    )
    .expect("classify");

    assert_eq!(
        result.pages_with_text,
        Some(0),
        "with the bar above the fixture's operator count, no page should qualify"
    );
    assert!(
        result.confidence < default.confidence,
        "a document that no longer looks like text should be reported less confidently"
    );
}

// ---------------------------------------------------------------------------
// to-markdown
// ---------------------------------------------------------------------------

#[test]
fn markdown_recovers_structure_from_the_page() {
    let result = markdown::handle(
        markdown::Request {
            source: fixture("text-two-page.pdf"),
            password: None,
            pages: None,
            max_chars: None,
            profile: markdown::Profile::Fidelity,
            include_images: false,
            strip_headers_footers: true,
            per_page: false,
        },
        &cfg(),
    )
    .expect("to-markdown");

    // The 24pt line becomes a heading; the 12pt lines stay body text. Nothing
    // in the document says "heading" — it is recovered from the font size.
    assert!(
        result.body.text.contains("# Quarterly Report"),
        "expected a recovered heading, got:\n{}",
        result.body.text
    );
    assert!(result.body.text.contains("Revenue rose to 4.2 million"));
    assert!(result.body.text.contains("# Appendix"));
    assert!(!result.body.truncated);
    assert_eq!(result.page_count, 2);
    assert!(!result.has_encoding_issues);
}

/// A page filter is the cheap way to read a long document, and the numbers are
/// 1-indexed on the wire while the parser's per-page entry point counts from
/// zero. Asking for page 2 must return page 2's content, not page 1's.
#[test]
fn a_page_filter_selects_that_page_and_not_its_neighbour() {
    let result = markdown::handle(
        markdown::Request {
            source: fixture("text-two-page.pdf"),
            password: None,
            pages: Some(vec![2]),
            max_chars: None,
            profile: markdown::Profile::Fidelity,
            include_images: false,
            strip_headers_footers: true,
            per_page: true,
        },
        &cfg(),
    )
    .expect("to-markdown");

    assert!(
        result.body.text.contains("Appendix"),
        "page 2 should hold the appendix, got:\n{}",
        result.body.text
    );
    assert!(
        !result.body.text.contains("Quarterly Report"),
        "page 1 leaked into a page-2 request:\n{}",
        result.body.text
    );
    assert_eq!(result.pages_converted, 1);

    let pages = result.pages.expect("per_page requested");
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].page, 2, "per-page numbering must stay 1-indexed");
    assert!(pages[0].markdown.contains("Appendix"));
}

/// Truncation must report what it dropped, or a caller answers from a fragment
/// believing it has the document.
#[test]
fn a_capped_response_says_how_much_it_withheld() {
    let result = markdown::handle(
        markdown::Request {
            source: fixture("text-two-page.pdf"),
            password: None,
            pages: None,
            max_chars: Some(20),
            profile: markdown::Profile::Fidelity,
            include_images: false,
            strip_headers_footers: true,
            per_page: false,
        },
        &cfg(),
    )
    .expect("to-markdown");

    assert!(result.body.truncated);
    assert_eq!(result.body.chars, 20);
    assert!(result.body.total_chars > 20);
    assert!(result.body.preview.is_some());
}

#[test]
fn max_chars_zero_returns_the_whole_document() {
    let result = markdown::handle(
        markdown::Request {
            source: fixture("text-two-page.pdf"),
            password: None,
            pages: None,
            max_chars: Some(0),
            profile: markdown::Profile::Fidelity,
            include_images: false,
            strip_headers_footers: true,
            per_page: false,
        },
        &cfg(),
    )
    .expect("to-markdown");

    assert!(!result.body.truncated);
    assert_eq!(result.body.chars, result.body.total_chars);
}

/// A scan produces no markdown. The response must still carry the verdict, so
/// an empty body is distinguishable from a document that is genuinely blank.
#[test]
fn a_document_with_no_text_yields_no_markdown_but_still_reports_why() {
    let result = markdown::handle(
        markdown::Request {
            source: fixture("no-text.pdf"),
            password: None,
            pages: None,
            max_chars: None,
            profile: markdown::Profile::Fidelity,
            include_images: false,
            strip_headers_footers: true,
            per_page: false,
        },
        &cfg(),
    )
    .expect("to-markdown");

    assert!(result.body.text.trim().is_empty());
    assert_ne!(result.document_type, DocumentType::TextBased);
    assert_eq!(result.pages_needing_ocr, vec![1]);
}

// ---------------------------------------------------------------------------
// extract-text
// ---------------------------------------------------------------------------

#[test]
fn plain_text_extraction_returns_the_words_without_the_structure() {
    let result = text::handle(
        text::Request {
            source: fixture("text-two-page.pdf"),
            max_chars: Some(0),
        },
        &cfg(),
    )
    .expect("extract-text");

    assert!(result.body.text.contains("Quarterly Report"));
    assert!(result.body.text.contains("Appendix"));
    assert!(
        !result.body.text.contains("# "),
        "plain text should carry no markdown markers"
    );
}

// ---------------------------------------------------------------------------
// extract-items
// ---------------------------------------------------------------------------

/// The positions are the reason this function exists, so assert the actual
/// numbers the fixture was drawn with rather than that items came back.
#[test]
fn items_carry_the_geometry_the_page_was_drawn_with() {
    let result = items::handle(
        items::Request {
            source: fixture("text-two-page.pdf"),
            pages: Some(vec![1]),
            max_items: None,
        },
        &cfg(),
    )
    .expect("extract-items");

    assert_eq!(result.total_count, 3);
    assert!(!result.truncated);
    assert_eq!(result.coordinate_origin, "pdf-points, bottom-left");

    let heading = result
        .items
        .iter()
        .find(|i| i.text.contains("Quarterly Report"))
        .expect("the heading is on page 1");
    assert_eq!(heading.page, 1);
    assert_eq!(heading.x, 72.0);
    assert_eq!(heading.y, 700.0);
    assert_eq!(heading.font_size, 24.0);

    let body = result
        .items
        .iter()
        .find(|i| i.text.contains("Revenue rose"))
        .expect("the body line is on page 1");
    assert_eq!(body.font_size, 12.0);
    // Bottom-left origin: the heading sits higher on the page, so its y is
    // LARGER. Under a top-left origin this comparison would invert.
    assert!(
        heading.y > body.y,
        "with a bottom-left origin the heading must have the larger y"
    );
}

#[test]
fn an_item_cap_reports_what_it_left_behind() {
    let result = items::handle(
        items::Request {
            source: fixture("text-two-page.pdf"),
            pages: Some(vec![1]),
            max_items: Some(1),
        },
        &cfg(),
    )
    .expect("extract-items");

    assert_eq!(result.count, 1);
    assert_eq!(result.total_count, 3);
    assert!(result.truncated);
}

#[test]
fn a_page_filter_limits_which_items_come_back() {
    let page_two = items::handle(
        items::Request {
            source: fixture("text-two-page.pdf"),
            pages: Some(vec![2]),
            max_items: None,
        },
        &cfg(),
    )
    .expect("extract-items");

    assert!(page_two.items.iter().all(|i| i.page == 2));
    assert!(page_two.items.iter().any(|i| i.text.contains("Appendix")));
}

// ---------------------------------------------------------------------------
// extract-regions
// ---------------------------------------------------------------------------

/// Region boxes use a TOP-left origin while items use bottom-left. A box over
/// the top of the page must therefore return the heading, and the same numbers
/// read as bottom-left would return the wrong end of the page.
#[test]
fn a_region_over_the_top_of_the_page_returns_the_heading() {
    let result = regions::handle(
        regions::Request {
            source: fixture("text-two-page.pdf"),
            regions: vec![regions::PageRegions {
                page: 1,
                boxes: vec![[0.0, 0.0, 612.0, 200.0]],
            }],
            mode: regions::Mode::Text,
        },
        &cfg(),
    )
    .expect("extract-regions");

    assert_eq!(result.coordinate_origin, "pdf-points, top-left");
    assert_eq!(result.region_count, 1);
    assert_eq!(
        result.pages[0].page, 1,
        "page numbering must stay 1-indexed"
    );
    assert!(
        result.pages[0].regions[0].text.contains("Quarterly Report"),
        "a top-left box over the first 200 points should hold the heading, got {:?}",
        result.pages[0].regions[0].text
    );
}

#[test]
fn an_empty_region_is_reported_as_unreliable_rather_than_as_empty_text() {
    let result = regions::handle(
        regions::Request {
            source: fixture("text-two-page.pdf"),
            regions: vec![regions::PageRegions {
                page: 1,
                // The lower half of the page carries nothing.
                boxes: vec![[0.0, 600.0, 612.0, 790.0]],
            }],
            mode: regions::Mode::Text,
        },
        &cfg(),
    )
    .expect("extract-regions");

    assert_eq!(result.regions_needing_ocr, 1);
    assert!(result.pages[0].regions[0].needs_ocr);
}

// ---------------------------------------------------------------------------
// input handling
// ---------------------------------------------------------------------------

/// A document handed over inline must produce the same result as the same
/// document read from disk.
#[test]
fn inline_bytes_and_a_path_agree() {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;

    let path = format!(
        "{}/tests/fixtures/text-two-page.pdf",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).expect("fixture readable");

    let from_path = classify::handle(
        classify::Request {
            source: fixture("text-two-page.pdf"),
            password: None,
            sample_pages: None,
        },
        &cfg(),
    )
    .expect("classify from path");

    let from_bytes = classify::handle(
        classify::Request {
            source: PdfSource {
                path: None,
                bytes_base64: Some(BASE64.encode(&bytes)),
                fs_scope: None,
            },
            password: None,
            sample_pages: None,
        },
        &cfg(),
    )
    .expect("classify from bytes");

    assert_eq!(from_path.document_type, from_bytes.document_type);
    assert_eq!(from_path.page_count, from_bytes.page_count);
    assert_eq!(from_path.pages_needing_ocr, from_bytes.pages_needing_ocr);
    assert_eq!(from_bytes.source, "<inline>");
}

#[test]
fn a_document_over_the_size_ceiling_is_refused_before_parsing() {
    let tiny = WorkerConfig {
        max_input_bytes: 16,
        ..WorkerConfig::default()
    };
    let err = classify::handle(
        classify::Request {
            source: fixture("text-two-page.pdf"),
            password: None,
            sample_pages: None,
        },
        &tiny,
    )
    .expect_err("over the ceiling");
    assert!(err.contains("max_input_bytes"), "{err}");
}

#[test]
fn a_missing_file_fails_with_the_path_in_the_message() {
    let err = classify::handle(
        classify::Request {
            source: fixture("does-not-exist.pdf"),
            password: None,
            sample_pages: None,
        },
        &cfg(),
    )
    .expect_err("missing file");
    assert!(err.contains("does-not-exist.pdf"), "{err}");
}
