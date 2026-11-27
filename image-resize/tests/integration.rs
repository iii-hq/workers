//! End-to-end integration test for the image-resize worker.
//!
//! Exercises the public processing pipeline against the real fixture images in
//! `test-fixtures/`, the same way the worker handler would when driven by the
//! iii engine over a channel — without booting the WebSocket runtime.

use image_resize::config::ResizeStrategy;
use image_resize::processing::{Format, ProcessingParams, process_image};
use std::path::Path;

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

fn process(
    name: &str,
    input_format: Format,
    output_format: Format,
    strategy: ResizeStrategy,
    side: u32,
) -> (u32, u32) {
    let params = ProcessingParams {
        target_width: side,
        target_height: side,
        strategy,
        quality: 80,
        input_format,
        output_format,
        max_pixels: None,
        max_bytes: None,
    };
    let (out, result) = process_image(&fixture(name), &params).expect("process fixture");
    let decoded = image::load_from_memory(&out).expect("decode output");
    assert_eq!(result.format, output_format);
    assert_eq!(
        (decoded.width(), decoded.height()),
        (result.width, result.height)
    );
    (result.width, result.height)
}

#[test]
fn small_source_is_not_enlarged() {
    let dims = process(
        "sample.jpg",
        Format::Jpeg,
        Format::Jpeg,
        ResizeStrategy::ScaleToFit,
        128,
    );
    assert_eq!(dims, (100, 80));
}

#[test]
fn jpeg_fixture_scales_down_to_the_box() {
    let dims = process(
        "sample.jpg",
        Format::Jpeg,
        Format::Jpeg,
        ResizeStrategy::ScaleToFit,
        50,
    );
    assert_eq!(dims, (50, 40));
}

#[test]
fn png_fixture_crops_without_enlarging() {
    let dims = process(
        "sample.png",
        Format::Png,
        Format::Png,
        ResizeStrategy::CropToFit,
        96,
    );
    assert_eq!(dims, (80, 80));
}

#[test]
fn webp_to_jpeg_format_conversion() {
    let dims = process(
        "sample.webp",
        Format::Webp,
        Format::Jpeg,
        ResizeStrategy::ScaleToFit,
        64,
    );
    assert_eq!(dims, (64, 51));
}
