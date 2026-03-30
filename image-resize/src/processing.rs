use anyhow::{anyhow, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{imageops::FilterType, DynamicImage, ImageFormat};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

use crate::config::{ResizeConfig, ResizeStrategy};

#[derive(Deserialize, Debug)]
pub struct ImageMetadata {
    pub format: String,
    #[serde(default)]
    pub output_format: Option<String>,
    #[allow(dead_code)] // Part of the channel protocol; source dimensions for logging/metadata
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
    #[serde(default)]
    pub quality: Option<u8>,
    #[serde(default)]
    pub strategy: Option<ResizeStrategy>,
    #[serde(default)]
    pub target_width: Option<u32>,
    #[serde(default)]
    pub target_height: Option<u32>,
}

#[derive(Serialize, Debug)]
pub struct ThumbnailMetadata {
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub strategy: String,
}

#[derive(Debug)]
pub struct ProcessingParams {
    pub target_width: u32,
    pub target_height: u32,
    pub strategy: ResizeStrategy,
    pub quality: u8,
    pub input_format: ImageFormat,
    pub output_format: ImageFormat,
}

fn parse_image_format(s: &str) -> Result<ImageFormat> {
    match s {
        "jpeg" | "jpg" => Ok(ImageFormat::Jpeg),
        "png" => Ok(ImageFormat::Png),
        "webp" => Ok(ImageFormat::WebP),
        other => Err(anyhow!("Unsupported format: {}", other)),
    }
}

pub fn resolve_params(config: &ResizeConfig, metadata: &ImageMetadata) -> Result<ProcessingParams> {
    let input_format = parse_image_format(&metadata.format)?;

    let output_format = match &metadata.output_format {
        Some(fmt) => parse_image_format(fmt)?,
        None => input_format,
    };

    let quality_default = match output_format {
        ImageFormat::Jpeg => config.quality.jpeg,
        ImageFormat::WebP => config.quality.webp,
        _ => 100, // PNG is lossless
    };

    Ok(ProcessingParams {
        target_width: metadata.target_width.unwrap_or(config.width),
        target_height: metadata.target_height.unwrap_or(config.height),
        strategy: metadata.strategy.unwrap_or(config.strategy),
        quality: metadata.quality.unwrap_or(quality_default),
        input_format,
        output_format,
    })
}

/// Read the EXIF orientation value from image bytes.
/// Returns 1 (identity/no rotation) if EXIF is missing or unreadable.
pub fn read_exif_orientation(bytes: &[u8]) -> u16 {
    let reader = exif::Reader::new();
    let Ok(exif_data) = reader.read_from_container(&mut std::io::Cursor::new(bytes)) else {
        return 1;
    };
    exif_data
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .map(|v| v as u16)
        .unwrap_or(1)
}

/// Apply EXIF orientation transform to an image.
///
/// EXIF orientation values:
///   1 = Normal (identity)
///   2 = Flip horizontal
///   3 = Rotate 180°
///   4 = Flip vertical
///   5 = Transpose (flip horizontal + rotate 270° CW)
///   6 = Rotate 90° CW
///   7 = Transverse (flip horizontal + rotate 90° CW)
///   8 = Rotate 270° CW (= 90° CCW)
pub fn apply_exif_orientation(img: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.fliph().rotate270(),
        6 => img.rotate90(),
        7 => img.fliph().rotate90(),
        8 => img.rotate270(),
        _ => img,
    }
}

pub fn resize_scale_to_fit(img: &DynamicImage, max_w: u32, max_h: u32) -> DynamicImage {
    img.resize(max_w, max_h, FilterType::Lanczos3)
}

pub fn resize_crop_to_fit(img: &DynamicImage, target_w: u32, target_h: u32) -> DynamicImage {
    let (orig_w, orig_h) = (img.width(), img.height());
    let scale = f64::max(
        target_w as f64 / orig_w as f64,
        target_h as f64 / orig_h as f64,
    );
    let scaled_w = (orig_w as f64 * scale).ceil() as u32;
    let scaled_h = (orig_h as f64 * scale).ceil() as u32;

    let scaled = img.resize_exact(scaled_w, scaled_h, FilterType::Lanczos3);

    let x = (scaled_w.saturating_sub(target_w)) / 2;
    let y = (scaled_h.saturating_sub(target_h)) / 2;
    scaled.crop_imm(x, y, target_w, target_h)
}

pub fn process_image(bytes: &[u8], params: &ProcessingParams) -> Result<Vec<u8>> {
    let raw_img = image::load_from_memory_with_format(bytes, params.input_format)?;
    let orientation = read_exif_orientation(bytes);
    let img = apply_exif_orientation(raw_img, orientation);

    let thumbnail = match params.strategy {
        ResizeStrategy::ScaleToFit => {
            resize_scale_to_fit(&img, params.target_width, params.target_height)
        }
        ResizeStrategy::CropToFit => {
            resize_crop_to_fit(&img, params.target_width, params.target_height)
        }
    };

    let mut buf = Vec::new();
    match params.output_format {
        ImageFormat::Jpeg => {
            let encoder = JpegEncoder::new_with_quality(&mut buf, params.quality);
            thumbnail.to_rgb8().write_with_encoder(encoder)?;
        }
        ImageFormat::Png => {
            let mut cursor = Cursor::new(&mut buf);
            thumbnail
                .to_rgba8()
                .write_to(&mut cursor, ImageFormat::Png)?;
        }
        ImageFormat::WebP => {
            let mut cursor = Cursor::new(&mut buf);
            thumbnail
                .to_rgba8()
                .write_to(&mut cursor, ImageFormat::WebP)?;
        }
        _ => return Err(anyhow!("Unsupported output format")),
    }

    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{RgbImage, RgbaImage};

    fn make_rgb_image(w: u32, h: u32, r: u8, g: u8, b: u8) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([r, g, b]);
        }
        DynamicImage::ImageRgb8(img)
    }

    fn make_rgba_image(w: u32, h: u32, r: u8, g: u8, b: u8, a: u8) -> DynamicImage {
        let mut img = RgbaImage::new(w, h);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgba([r, g, b, a]);
        }
        DynamicImage::ImageRgba8(img)
    }

    fn encode_image(img: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut buf = Vec::new();
        match format {
            ImageFormat::Jpeg => {
                let encoder = JpegEncoder::new_with_quality(&mut buf, 85);
                img.to_rgb8().write_with_encoder(encoder).unwrap();
            }
            ImageFormat::Png => {
                let mut cursor = Cursor::new(&mut buf);
                img.to_rgba8()
                    .write_to(&mut cursor, ImageFormat::Png)
                    .unwrap();
            }
            ImageFormat::WebP => {
                let mut cursor = Cursor::new(&mut buf);
                img.to_rgba8()
                    .write_to(&mut cursor, ImageFormat::WebP)
                    .unwrap();
            }
            _ => panic!("unsupported format in test helper"),
        }
        buf
    }

    fn default_config() -> ResizeConfig {
        serde_yaml::from_str("{}").unwrap()
    }

    #[test]
    fn test_config_per_request_override() {
        let config = default_config();
        let metadata = ImageMetadata {
            format: "jpeg".into(),
            output_format: None,
            width: 1000,
            height: 500,
            quality: Some(50),
            strategy: Some(ResizeStrategy::CropToFit),
            target_width: Some(100),
            target_height: Some(100),
        };
        let params = resolve_params(&config, &metadata).unwrap();
        assert_eq!(params.target_width, 100);
        assert_eq!(params.target_height, 100);
        assert_eq!(params.strategy, ResizeStrategy::CropToFit);
        assert_eq!(params.quality, 50);

        let metadata_no_override = ImageMetadata {
            format: "jpeg".into(),
            output_format: None,
            width: 1000,
            height: 500,
            quality: None,
            strategy: None,
            target_width: None,
            target_height: None,
        };
        let params2 = resolve_params(&config, &metadata_no_override).unwrap();
        assert_eq!(params2.target_width, 200);
        assert_eq!(params2.target_height, 200);
        assert_eq!(params2.strategy, ResizeStrategy::ScaleToFit);
        assert_eq!(params2.quality, 85);
    }

    #[test]
    fn test_scale_to_fit_landscape() {
        let img = make_rgb_image(1000, 500, 255, 0, 0);
        let result = resize_scale_to_fit(&img, 200, 200);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 100);
    }

    #[test]
    fn test_scale_to_fit_portrait() {
        let img = make_rgb_image(500, 1000, 0, 255, 0);
        let result = resize_scale_to_fit(&img, 200, 200);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn test_scale_to_fit_square() {
        let img = make_rgb_image(1000, 1000, 0, 0, 255);
        let result = resize_scale_to_fit(&img, 200, 200);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn test_crop_to_fit_landscape() {
        let img = make_rgb_image(1000, 500, 255, 0, 0);
        let result = resize_crop_to_fit(&img, 200, 200);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn test_crop_to_fit_portrait() {
        let img = make_rgb_image(500, 1000, 0, 255, 0);
        let result = resize_crop_to_fit(&img, 200, 200);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn test_decode_jpeg() {
        let img = make_rgb_image(100, 80, 255, 0, 0);
        let bytes = encode_image(&img, ImageFormat::Jpeg);
        let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg).unwrap();
        assert_eq!(decoded.width(), 100);
        assert_eq!(decoded.height(), 80);
    }

    #[test]
    fn test_decode_png() {
        let img = make_rgba_image(100, 80, 0, 0, 255, 128);
        let bytes = encode_image(&img, ImageFormat::Png);
        let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Png).unwrap();
        assert_eq!(decoded.width(), 100);
        assert_eq!(decoded.height(), 80);
    }

    #[test]
    fn test_decode_webp() {
        let img = make_rgba_image(100, 80, 0, 255, 0, 255);
        let bytes = encode_image(&img, ImageFormat::WebP);
        let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::WebP).unwrap();
        assert_eq!(decoded.width(), 100);
        assert_eq!(decoded.height(), 80);
    }

    #[test]
    fn test_encode_jpeg_roundtrip() {
        let img = make_rgb_image(400, 300, 200, 100, 50);
        let bytes = encode_image(&img, ImageFormat::Jpeg);
        let params = ProcessingParams {
            target_width: 100,
            target_height: 100,
            strategy: ResizeStrategy::ScaleToFit,
            quality: 85,
            input_format: ImageFormat::Jpeg,
            output_format: ImageFormat::Jpeg,
        };
        let output = process_image(&bytes, &params).unwrap();
        let decoded = image::load_from_memory_with_format(&output, ImageFormat::Jpeg).unwrap();
        assert!(decoded.width() <= 100);
        assert!(decoded.height() <= 100);
    }

    #[test]
    fn test_encode_png_preserves_alpha() {
        let img = make_rgba_image(400, 300, 0, 0, 255, 128);
        let bytes = encode_image(&img, ImageFormat::Png);
        let params = ProcessingParams {
            target_width: 100,
            target_height: 100,
            strategy: ResizeStrategy::ScaleToFit,
            quality: 100,
            input_format: ImageFormat::Png,
            output_format: ImageFormat::Png,
        };
        let output = process_image(&bytes, &params).unwrap();
        let decoded = image::load_from_memory_with_format(&output, ImageFormat::Png).unwrap();
        let rgba = decoded.to_rgba8();
        assert_eq!(rgba.sample_layout().channels, 4);
        let has_transparency = rgba.pixels().any(|p| p.0[3] < 255);
        assert!(
            has_transparency,
            "PNG output should preserve alpha transparency"
        );
    }

    #[test]
    fn test_encode_webp_roundtrip() {
        let img = make_rgba_image(400, 300, 0, 255, 0, 255);
        let bytes = encode_image(&img, ImageFormat::WebP);
        let params = ProcessingParams {
            target_width: 100,
            target_height: 100,
            strategy: ResizeStrategy::ScaleToFit,
            quality: 80,
            input_format: ImageFormat::WebP,
            output_format: ImageFormat::WebP,
        };
        let output = process_image(&bytes, &params).unwrap();
        let decoded = image::load_from_memory_with_format(&output, ImageFormat::WebP).unwrap();
        assert!(decoded.width() <= 100);
        assert!(decoded.height() <= 100);
    }

    #[test]
    fn test_jpeg_quality_control() {
        let img = make_rgb_image(400, 300, 200, 100, 50);
        let bytes = encode_image(&img, ImageFormat::Jpeg);

        let params_low = ProcessingParams {
            target_width: 200,
            target_height: 200,
            strategy: ResizeStrategy::ScaleToFit,
            quality: 50,
            input_format: ImageFormat::Jpeg,
            output_format: ImageFormat::Jpeg,
        };
        let params_high = ProcessingParams {
            target_width: 200,
            target_height: 200,
            strategy: ResizeStrategy::ScaleToFit,
            quality: 95,
            input_format: ImageFormat::Jpeg,
            output_format: ImageFormat::Jpeg,
        };

        let output_low = process_image(&bytes, &params_low).unwrap();
        let output_high = process_image(&bytes, &params_high).unwrap();

        assert!(
            output_low.len() < output_high.len(),
            "Quality 50 ({} bytes) should produce smaller output than quality 95 ({} bytes)",
            output_low.len(),
            output_high.len()
        );
    }

    // ── EXIF orientation tests ──────────────────────────────

    #[test]
    fn test_apply_exif_orientation_identity() {
        let img = make_rgb_image(100, 50, 255, 0, 0);
        let result = apply_exif_orientation(img, 1);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 50);
    }

    #[test]
    fn test_apply_exif_orientation_rotate_180() {
        let img = make_rgb_image(100, 50, 255, 0, 0);
        let result = apply_exif_orientation(img, 3);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 50);
    }

    #[test]
    fn test_apply_exif_orientation_rotate_90_cw() {
        let img = make_rgb_image(100, 50, 255, 0, 0);
        let result = apply_exif_orientation(img, 6);
        assert_eq!(result.width(), 50);
        assert_eq!(result.height(), 100);
    }

    #[test]
    fn test_apply_exif_orientation_rotate_90_ccw() {
        let img = make_rgb_image(100, 50, 255, 0, 0);
        let result = apply_exif_orientation(img, 8);
        assert_eq!(result.width(), 50);
        assert_eq!(result.height(), 100);
    }

    #[test]
    fn test_apply_exif_orientation_flip_horizontal() {
        let img = make_rgb_image(100, 50, 255, 0, 0);
        let result = apply_exif_orientation(img, 2);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 50);
    }

    #[test]
    fn test_read_exif_orientation_no_exif() {
        let img = make_rgb_image(100, 50, 255, 0, 0);
        let bytes = encode_image(&img, ImageFormat::Png);
        assert_eq!(read_exif_orientation(&bytes), 1);
    }

    #[test]
    fn test_unsupported_format_error() {
        let config = default_config();
        let metadata = ImageMetadata {
            format: "gif".into(),
            output_format: None,
            width: 100,
            height: 100,
            quality: None,
            strategy: None,
            target_width: None,
            target_height: None,
        };
        let result = resolve_params(&config, &metadata);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unsupported format"));
    }

    #[test]
    fn test_jpeg_to_png_conversion() {
        let img = make_rgb_image(400, 300, 200, 100, 50);
        let bytes = encode_image(&img, ImageFormat::Jpeg);
        let params = ProcessingParams {
            target_width: 100,
            target_height: 100,
            strategy: ResizeStrategy::ScaleToFit,
            quality: 100,
            input_format: ImageFormat::Jpeg,
            output_format: ImageFormat::Png,
        };
        let output = process_image(&bytes, &params).unwrap();
        let decoded = image::load_from_memory_with_format(&output, ImageFormat::Png).unwrap();
        assert!(decoded.width() <= 100);
        assert!(decoded.height() <= 100);
    }

    #[test]
    fn test_png_to_jpeg_conversion() {
        let img = make_rgba_image(400, 300, 0, 0, 255, 255);
        let bytes = encode_image(&img, ImageFormat::Png);
        let params = ProcessingParams {
            target_width: 100,
            target_height: 100,
            strategy: ResizeStrategy::ScaleToFit,
            quality: 85,
            input_format: ImageFormat::Png,
            output_format: ImageFormat::Jpeg,
        };
        let output = process_image(&bytes, &params).unwrap();
        let decoded = image::load_from_memory_with_format(&output, ImageFormat::Jpeg).unwrap();
        assert!(decoded.width() <= 100);
        assert!(decoded.height() <= 100);
    }

    #[test]
    fn test_resolve_params_with_output_format() {
        let config = default_config();
        let metadata = ImageMetadata {
            format: "jpeg".into(),
            output_format: Some("png".into()),
            width: 1000,
            height: 500,
            quality: None,
            strategy: None,
            target_width: None,
            target_height: None,
        };
        let params = resolve_params(&config, &metadata).unwrap();
        assert_eq!(params.input_format, ImageFormat::Jpeg);
        assert_eq!(params.output_format, ImageFormat::Png);
        assert_eq!(params.quality, 100);
    }

    // ── Document-to-thumbnail integration tests ────────────

    fn convert_and_resize(raw_bytes: &[u8], format: &str) -> Vec<u8> {
        use crate::conversion;

        let img = conversion::convert_to_image(raw_bytes, format).unwrap();
        let png_bytes = conversion::to_png_bytes(&img).unwrap();

        let params = ProcessingParams {
            target_width: 200,
            target_height: 200,
            strategy: ResizeStrategy::ScaleToFit,
            quality: 85,
            input_format: ImageFormat::Png,
            output_format: ImageFormat::Jpeg,
        };
        process_image(&png_bytes, &params).unwrap()
    }

    #[test]
    fn test_pdf_to_jpeg_thumbnail() {
        let pdf = std::fs::read("example/images/handbook.pdf").unwrap();
        let output = convert_and_resize(&pdf, "pdf");
        let decoded = image::load_from_memory_with_format(&output, ImageFormat::Jpeg).unwrap();
        assert!(decoded.width() <= 200);
        assert!(decoded.height() <= 200);
        assert!(decoded.width() > 0 && decoded.height() > 0);
    }

    #[test]
    fn test_psd_to_jpeg_thumbnail() {
        let psd = std::fs::read("example/images/sample_640\u{00d7}426.psd").unwrap();
        let output = convert_and_resize(&psd, "psd");
        let decoded = image::load_from_memory_with_format(&output, ImageFormat::Jpeg).unwrap();
        assert!(decoded.width() <= 200);
        assert!(decoded.height() <= 200);
        assert!(decoded.width() > 0 && decoded.height() > 0);
    }

}
