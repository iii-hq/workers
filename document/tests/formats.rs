//! Every format this worker claims, converted end to end through the handler.
//!
//! The unit tests cover the routing and the caps with CSV, which needs no
//! binary fixture. These cover the claim on the box: a Word file, a workbook, a
//! deck and an RTF document all come out as markdown, and a deck's embedded
//! image comes back as bytes a model can be handed.
//!
//! The fixtures are assembled by `tests/fixtures/make_fixtures.py` from the
//! parts each format requires, so a converter upgrade that changes what they
//! produce shows up as a failing assertion here, which is the point.

use std::path::PathBuf;

use document::config::WorkerConfig;
use document::format::{DetectedFrom, Family, Format};
use document::functions::{assets, detect, markdown};
use document::source::DocumentSource;

fn fixture(name: &str) -> String {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    path.to_string_lossy().to_string()
}

fn source(name: &str) -> DocumentSource {
    DocumentSource {
        path: Some(fixture(name)),
        ..DocumentSource::default()
    }
}

fn convert(name: &str) -> markdown::Response {
    markdown::handle(
        markdown::Request {
            source: source(name),
            format: None,
            max_chars: Some(0),
        },
        &WorkerConfig::default(),
    )
    .unwrap_or_else(|e| panic!("{name} converts: {e}"))
}

fn detect(name: &str) -> detect::Response {
    detect::handle(
        detect::Request {
            source: source(name),
        },
        &WorkerConfig::default(),
    )
    .unwrap_or_else(|e| panic!("{name} is detected: {e}"))
}

#[test]
fn a_word_document_keeps_its_headings_and_tables() {
    let response = convert("sample.docx");
    assert_eq!(response.format, Format::Docx);
    assert_eq!(response.family, Family::Prose);
    assert_eq!(response.detected_from, DetectedFrom::Content);

    let text = &response.body.text;
    assert!(text.contains("Quarterly Notes"), "{text}");
    assert!(
        text.contains("without a restart"),
        "body text survived: {text}"
    );
    // A table that arrives as a run-on paragraph is the failure this asserts
    // against: the pipe is what makes it a table to the model.
    assert!(text.contains('|'), "the table became prose: {text}");
    assert!(text.contains("21480"), "{text}");
    assert!(!response.body.truncated);
}

#[test]
fn a_workbook_becomes_rows_a_model_can_read() {
    let response = convert("sample.xlsx");
    assert_eq!(response.format, Format::Excel);
    assert_eq!(response.family, Family::Spreadsheet);

    let text = &response.body.text;
    assert!(text.contains("scenario"), "{text}");
    assert!(text.contains("echo"), "{text}");
    assert!(text.contains("21480"), "{text}");
}

#[test]
fn a_deck_converts_and_reports_the_images_markdown_dropped() {
    let response = convert("sample.pptx");
    assert_eq!(response.format, Format::Pptx);
    assert_eq!(response.family, Family::Presentation);

    let text = &response.body.text;
    assert!(text.contains("Three Primitives"), "{text}");
    assert!(text.contains("Worker, function, trigger."), "{text}");

    // The count is the whole reason it is on the response: markdown renders an
    // embedded image as alt text, so without this a deck of diagrams looks
    // like a document that simply had little to say.
    assert_eq!(response.asset_count, 1);
}

#[test]
fn an_rtf_document_converts() {
    let response = convert("sample.rtf");
    assert_eq!(response.format, Format::Rtf);
    assert_eq!(response.family, Family::Prose);
    assert!(response.body.text.contains("Release notes"));
    assert!(response.body.text.contains("40 ms"));
}

#[test]
fn a_csv_file_on_disk_is_recognised_by_its_name() {
    let response = convert("sample.csv");
    assert_eq!(response.format, Format::Csv);
    assert_eq!(response.detected_from, DetectedFrom::Extension);
    assert!(response.body.text.contains("fanout"));
}

/// The images are the point of the extraction: a deck whose content is
/// diagrams reads as empty without them, and a model that can see images can
/// use the bytes directly.
#[test]
fn a_decks_image_comes_back_as_usable_bytes() {
    let response = assets::handle(
        assets::Request {
            source: source("sample.pptx"),
            format: None,
            max_assets: None,
            media_type_prefix: Some("image/".to_string()),
            include_bytes: true,
        },
        &WorkerConfig::default(),
    )
    .expect("the deck's assets extract");

    assert_eq!(response.total_count, 1);
    assert!(!response.truncated);
    let asset = &response.assets[0];
    assert_eq!(asset.media_type, "image/png");
    assert!(asset.omitted.is_none());

    let encoded = asset.bytes_base64.as_ref().expect("bytes were included");
    let decoded = base64_decode(encoded);
    assert_eq!(
        &decoded[..8],
        b"\x89PNG\r\n\x1a\n",
        "what came back is not a PNG"
    );
    assert_eq!(asset.size_bytes as usize, decoded.len());
}

/// An inventory pass is how a caller decides whether the bytes are worth
/// moving at all, so it must still report the type and the size.
#[test]
fn an_inventory_lists_assets_without_moving_them() {
    let response = assets::handle(
        assets::Request {
            source: source("sample.pptx"),
            format: None,
            max_assets: None,
            media_type_prefix: None,
            include_bytes: false,
        },
        &WorkerConfig::default(),
    )
    .expect("the deck's assets are listed");

    let asset = &response.assets[0];
    assert!(asset.bytes_base64.is_none());
    assert_eq!(asset.omitted, Some("not_requested"));
    assert!(asset.size_bytes > 0);
}

/// An asset over the per-asset ceiling is still announced. Dropping it from
/// the list entirely would tell the caller the document has no images.
#[test]
fn an_oversized_asset_is_listed_without_its_bytes() {
    let cfg = WorkerConfig {
        max_asset_bytes: 4,
        ..WorkerConfig::default()
    };
    let response = assets::handle(
        assets::Request {
            source: source("sample.pptx"),
            format: None,
            max_assets: None,
            media_type_prefix: None,
            include_bytes: true,
        },
        &cfg,
    )
    .expect("extraction succeeds");

    let asset = &response.assets[0];
    assert!(asset.bytes_base64.is_none());
    assert_eq!(asset.omitted, Some("too_large"));
    assert_eq!(asset.media_type, "image/png");
}

/// The per-asset ceiling does not bound a response on its own: a couple of dozen
/// assets each just under it still add up to a payload nobody asked for. The
/// total budget stops the encoding while still listing what exists.
#[test]
fn the_response_budget_stops_encoding_but_not_listing() {
    let cfg = WorkerConfig {
        max_assets_total_bytes: 1,
        ..WorkerConfig::default()
    };
    let response = assets::handle(
        assets::Request {
            source: source("sample.pptx"),
            format: None,
            max_assets: None,
            media_type_prefix: None,
            include_bytes: true,
        },
        &cfg,
    )
    .expect("extraction succeeds");

    let asset = &response.assets[0];
    assert!(asset.bytes_base64.is_none());
    assert_eq!(asset.omitted, Some("budget_spent"));
    // Still announced, with everything a caller needs to fetch it alone.
    assert_eq!(asset.media_type, "image/png");
    assert!(asset.size_bytes > 0);
}

/// Detection runs on the bytes, so a package format is recognised without its
/// name — which is the case for a file pasted into a composer.
#[test]
fn detection_reads_the_package_not_the_extension() {
    let response = detect("sample.pptx");
    assert_eq!(response.format, Some(Format::Pptx));
    assert_eq!(response.detected_from, Some(DetectedFrom::Content));
    assert!(response.convertible);
    assert!(response.has_assets);
}

fn base64_decode(encoded: &str) -> Vec<u8> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD.decode(encoded).expect("valid base64")
}
