//! End-to-end integration test for the image-resize worker.
//!
//! Exercises the public processing pipeline against the real fixture images in
//! `test-fixtures/`, the same way the worker handler would when driven by the
//! iii engine over a channel — without booting the WebSocket runtime.

use image::ImageFormat;
use image_resize::config::ResizeStrategy;
use image_resize::processing::{process_image, ProcessingParams};
use std::path::Path;

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

#[test]
fn jpeg_fixture_scales_within_box() {
    let bytes = fixture("sample.jpg");
    let params = ProcessingParams {
        target_width: 128,
        target_height: 128,
        strategy: ResizeStrategy::ScaleToFit,
        quality: 80,
        input_format: ImageFormat::Jpeg,
        output_format: ImageFormat::Jpeg,
    };

    let out = process_image(&bytes, &params).expect("process jpeg");
    let decoded = image::load_from_memory_with_format(&out, ImageFormat::Jpeg)
        .expect("decode jpeg output");

    assert!(decoded.width() <= 128, "width {} > 128", decoded.width());
    assert!(decoded.height() <= 128, "height {} > 128", decoded.height());
    assert!(decoded.width() == 128 || decoded.height() == 128, "scale-to-fit should hit a bound");
}

#[test]
fn png_fixture_crops_to_exact_size() {
    let bytes = fixture("sample.png");
    let params = ProcessingParams {
        target_width: 96,
        target_height: 96,
        strategy: ResizeStrategy::CropToFit,
        quality: 100,
        input_format: ImageFormat::Png,
        output_format: ImageFormat::Png,
    };

    let out = process_image(&bytes, &params).expect("process png");
    let decoded = image::load_from_memory_with_format(&out, ImageFormat::Png)
        .expect("decode png output");

    assert_eq!(decoded.width(), 96);
    assert_eq!(decoded.height(), 96);
}

#[test]
fn webp_to_jpeg_format_conversion() {
    let bytes = fixture("sample.webp");
    let params = ProcessingParams {
        target_width: 64,
        target_height: 64,
        strategy: ResizeStrategy::ScaleToFit,
        quality: 75,
        input_format: ImageFormat::WebP,
        output_format: ImageFormat::Jpeg,
    };

    let out = process_image(&bytes, &params).expect("process webp -> jpeg");
    let decoded = image::load_from_memory_with_format(&out, ImageFormat::Jpeg)
        .expect("decode jpeg output");

    assert!(decoded.width() <= 64);
    assert!(decoded.height() <= 64);
}
