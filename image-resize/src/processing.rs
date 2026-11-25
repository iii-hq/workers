use iii_sdk::Error;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, Rgba, RgbaImage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::num::NonZeroU32;

use crate::config::{ResizeConfig, ResizeStrategy, check_quality};

/// A caller fault. The SDK sends a `Remote` error's code as is; any other
/// error reaches the caller as `invocation_failed`.
pub fn invalid(msg: impl std::fmt::Display) -> Error {
    Error::Remote {
        code: "INVALID_ARGS".into(),
        message: msg.to_string(),
        stacktrace: None,
    }
}

#[derive(Deserialize, Serialize, JsonSchema, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Jpeg,
    Png,
    Webp,
}

impl Format {
    fn image_format(self) -> ImageFormat {
        match self {
            Format::Jpeg => ImageFormat::Jpeg,
            Format::Png => ImageFormat::Png,
            Format::Webp => ImageFormat::WebP,
        }
    }
}

/// Per-call options. Every key is optional; any other key is rejected.
#[derive(Deserialize, JsonSchema, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct ImageMetadata {
    /// Output format. Defaults to the input's format.
    pub output_format: Option<Format>,
    /// JPEG and WebP quality. Ignored for PNG. Defaults to the configured quality.
    #[schemars(range(min = 1, max = 100))]
    pub quality: Option<u8>,
    /// `scale-to-fit` fits the image inside the box; `crop-to-fit` centre-crops it to the
    /// box's aspect ratio, then shrinks it to the box. Defaults to the configured strategy.
    pub strategy: Option<ResizeStrategy>,
    /// Box width in pixels. The image is never enlarged. Defaults to the configured box.
    pub target_width: Option<NonZeroU32>,
    /// Box height in pixels. The image is never enlarged. Defaults to the configured box.
    pub target_height: Option<NonZeroU32>,
}

/// The call result: the output's format and size, and the source's size after
/// EXIF orientation.
#[derive(Serialize, JsonSchema, Debug, PartialEq)]
pub struct ThumbnailMetadata {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub strategy: ResizeStrategy,
}

#[derive(Debug)]
pub struct ProcessingParams {
    pub target_width: u32,
    pub target_height: u32,
    pub strategy: ResizeStrategy,
    pub quality: u8,
    pub input_format: Format,
    pub output_format: Format,
}

/// The input format, from the bytes: a caller's label is not trusted.
pub fn sniff(bytes: &[u8]) -> Result<Format, Error> {
    match image::guess_format(bytes) {
        Ok(ImageFormat::Jpeg) => Ok(Format::Jpeg),
        Ok(ImageFormat::Png) => Ok(Format::Png),
        Ok(ImageFormat::WebP) => Ok(Format::Webp),
        Ok(other) => Err(invalid(format!(
            "unsupported input format {other:?}; send jpeg, png or webp"
        ))),
        Err(_) => Err(invalid(
            "input is not a recognized image; send jpeg, png or webp",
        )),
    }
}

pub fn resolve_params(
    config: &ResizeConfig,
    meta: &ImageMetadata,
    input_format: Format,
) -> Result<ProcessingParams, Error> {
    let output_format = meta.output_format.unwrap_or(input_format);
    let quality = meta.quality.unwrap_or(match output_format {
        Format::Jpeg => config.quality.jpeg,
        Format::Webp => config.quality.webp,
        Format::Png => 100, // PNG is lossless
    });

    Ok(ProcessingParams {
        target_width: meta.target_width.unwrap_or(config.width).get(),
        target_height: meta.target_height.unwrap_or(config.height).get(),
        strategy: meta.strategy.unwrap_or(config.strategy),
        quality: check_quality(quality).map_err(invalid)?,
        input_format,
        output_format,
    })
}

// Not ImageDecoder::orientation(): it allocates a WebP EXIF chunk from an unchecked header length.
fn exif_orientation(bytes: &[u8]) -> Orientation {
    exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()
        .and_then(|data| {
            data.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?
                .value
                .get_uint(0)
        })
        .and_then(|v| u8::try_from(v).ok())
        .and_then(Orientation::from_exif)
        .unwrap_or(Orientation::NoTransforms)
}

/// Largest image decoded, in pixels. The resize keeps an f32 buffer of up to
/// 16 bytes per source pixel, so this bounds memory per image.
pub const MAX_PIXELS: u64 = 50_000_000;

fn decode(bytes: &[u8], format: ImageFormat) -> Result<DynamicImage, Error> {
    let undecodable =
        |e: image::ImageError| invalid(format!("could not decode the {format:?} image: {e}"));
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    // into_decoder does not charge the pixel buffer (checked below); this
    // bounds what a PNG's ancillary chunks may inflate to.
    limits.max_alloc = Some(64 << 20);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(undecodable)?;
    let (w, h) = decoder.dimensions();
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(invalid(format!(
            "image is {w}x{h}; the limit is {MAX_PIXELS} pixels"
        )));
    }
    DynamicImage::from_decoder(decoder).map_err(undecodable)
}

fn over_white(img: &DynamicImage) -> DynamicImage {
    let mut bg = RgbaImage::from_pixel(img.width(), img.height(), Rgba([255; 4]));
    imageops::overlay(&mut bg, img, 0, 0);
    DynamicImage::ImageRgba8(bg)
}

pub fn resize_scale_to_fit(img: DynamicImage, max_w: u32, max_h: u32) -> DynamicImage {
    if img.width() <= max_w && img.height() <= max_h {
        return img;
    }
    img.resize(max_w, max_h, FilterType::Lanczos3)
}

pub fn resize_crop_to_fit(img: DynamicImage, box_w: u32, box_h: u32) -> DynamicImage {
    let (box_w, box_h) = (box_w.max(1), box_h.max(1));
    let (sw, sh) = (u64::from(img.width()), u64::from(img.height()));
    let (bw, bh) = (u64::from(box_w), u64::from(box_h));
    // The largest centred rectangle with the box's aspect ratio, in source
    // pixels: cropping first keeps the resize no larger than the source.
    let (cw, ch) = if sw * bh > sh * bw {
        ((sh * bw / bh).max(1), sh)
    } else {
        (sw, (sw * bh / bw).max(1))
    };
    let (cw, ch) = (cw as u32, ch as u32); // cw <= sw and ch <= sh
    let cropped = if (cw, ch) == (img.width(), img.height()) {
        img
    } else {
        img.crop_imm((img.width() - cw) / 2, (img.height() - ch) / 2, cw, ch)
    };
    if cw > box_w || ch > box_h {
        cropped.resize_exact(cw.min(box_w), ch.min(box_h), FilterType::Lanczos3)
    } else {
        cropped
    }
}

pub fn process_image(
    bytes: &[u8],
    params: &ProcessingParams,
) -> Result<(Vec<u8>, ThumbnailMetadata), Error> {
    let mut img = decode(bytes, params.input_format.image_format())?;
    img.apply_orientation(exif_orientation(bytes));
    let (source_width, source_height) = (img.width(), img.height());
    // JPEG has no alpha. Flatten before the resize: the filter mixes straight
    // alpha, so flattening afterwards would leave the hidden colour of
    // transparent pixels in every edge.
    if params.output_format == Format::Jpeg && img.color().has_alpha() {
        img = over_white(&img);
    }

    let thumbnail = match params.strategy {
        ResizeStrategy::ScaleToFit => {
            resize_scale_to_fit(img, params.target_width, params.target_height)
        }
        ResizeStrategy::CropToFit => {
            resize_crop_to_fit(img, params.target_width, params.target_height)
        }
    };

    let (width, height) = (thumbnail.width(), thumbnail.height());
    let max_side = match params.output_format {
        Format::Jpeg => 65_535,
        Format::Webp => 16_383,
        Format::Png => u32::MAX,
    };
    if width.max(height) > max_side {
        return Err(invalid(format!(
            "{width}x{height} is too large for this output format (max {max_side} px per side); use a smaller box or png"
        )));
    }

    let mut buf = Vec::new();
    let encoded = match params.output_format {
        Format::Jpeg => {
            thumbnail.write_with_encoder(JpegEncoder::new_with_quality(&mut buf, params.quality))
        }
        Format::Png => thumbnail.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png),
        Format::Webp => {
            buf = encode_webp(&thumbnail, params.quality)?;
            Ok(())
        }
    };
    encoded.map_err(|e| Error::Handler(format!("could not encode the output: {e}")))?;

    let result = ThumbnailMetadata {
        format: params.output_format,
        width,
        height,
        source_width,
        source_height,
        strategy: params.strategy,
    };
    Ok((buf, result))
}

fn encode_webp(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, Error> {
    use std::ffi::c_int;
    // process_image rejected sides above 16383, so these casts are exact.
    let (w, h) = (img.width() as c_int, img.height() as c_int);
    let mut out: *mut u8 = std::ptr::null_mut();
    // SAFETY: each buffer is h rows of `stride` bytes of this image; libwebp only reads it.
    let len = unsafe {
        if img.color().has_alpha() {
            let px = img.to_rgba8();
            libwebp_sys::WebPEncodeRGBA(px.as_ptr(), w, h, w * 4, f32::from(quality), &mut out)
        } else {
            let px = img.to_rgb8();
            libwebp_sys::WebPEncodeRGB(px.as_ptr(), w, h, w * 3, f32::from(quality), &mut out)
        }
    };
    if len == 0 {
        return Err(Error::Handler("libwebp could not encode the image".into()));
    }
    // SAFETY: on success libwebp returns `len` bytes at `out`, ours until WebPFree.
    let bytes = unsafe { std::slice::from_raw_parts(out, len).to_vec() };
    // SAFETY: `out` came from libwebp and is not used after this call.
    unsafe { libwebp_sys::WebPFree(out.cast()) };
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, RgbImage};

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

    fn is_invalid_args(err: &Error) -> bool {
        matches!(err, Error::Remote { code, .. } if code == "INVALID_ARGS")
    }

    fn default_config() -> ResizeConfig {
        serde_yaml_ng::from_str("{}").unwrap()
    }

    #[test]
    fn test_config_per_request_override() {
        let config = default_config();
        let metadata = ImageMetadata {
            quality: Some(50),
            strategy: Some(ResizeStrategy::CropToFit),
            target_width: NonZeroU32::new(100),
            target_height: NonZeroU32::new(100),
            ..Default::default()
        };
        let params = resolve_params(&config, &metadata, Format::Jpeg).unwrap();
        assert_eq!(params.target_width, 100);
        assert_eq!(params.target_height, 100);
        assert_eq!(params.strategy, ResizeStrategy::CropToFit);
        assert_eq!(params.quality, 50);

        let params2 = resolve_params(&config, &ImageMetadata::default(), Format::Jpeg).unwrap();
        assert_eq!(params2.target_width, 200);
        assert_eq!(params2.target_height, 200);
        assert_eq!(params2.strategy, ResizeStrategy::ScaleToFit);
        assert_eq!(params2.quality, 85);
    }

    #[test]
    fn test_scale_to_fit_landscape() {
        let img = make_rgb_image(1000, 500, 255, 0, 0);
        let result = resize_scale_to_fit(img, 200, 200);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 100);
    }

    #[test]
    fn test_scale_to_fit_portrait() {
        let img = make_rgb_image(500, 1000, 0, 255, 0);
        let result = resize_scale_to_fit(img, 200, 200);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn test_scale_to_fit_square() {
        let img = make_rgb_image(1000, 1000, 0, 0, 255);
        let result = resize_scale_to_fit(img, 200, 200);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn test_crop_to_fit_landscape() {
        let img = make_rgb_image(1000, 500, 255, 0, 0);
        let result = resize_crop_to_fit(img, 200, 200);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn test_crop_to_fit_portrait() {
        let img = make_rgb_image(500, 1000, 0, 255, 0);
        let result = resize_crop_to_fit(img, 200, 200);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 200);
    }

    fn run(
        input: &[u8],
        input_format: Format,
        output_format: Format,
        strategy: ResizeStrategy,
        w: u32,
        h: u32,
        quality: u8,
    ) -> DynamicImage {
        let params = ProcessingParams {
            target_width: w,
            target_height: h,
            strategy,
            quality,
            input_format,
            output_format,
        };
        let (output, _) = process_image(input, &params).unwrap();
        image::load_from_memory_with_format(&output, output_format.image_format()).unwrap()
    }

    #[test]
    fn scale_to_fit_never_enlarges() {
        for img in [
            make_rgb_image(100, 80, 10, 20, 30),
            make_rgb_image(1, 1, 10, 20, 30),
        ] {
            assert_eq!(resize_scale_to_fit(img.clone(), 200, 200), img);
        }
    }

    #[test]
    fn crop_to_fit_small_source_crops_without_enlarging() {
        for side in [200, 96] {
            let result = resize_crop_to_fit(make_rgb_image(100, 80, 10, 20, 30), side, side);
            assert_eq!((result.width(), result.height()), (80, 80), "box {side}");
        }
    }

    #[test]
    fn crop_to_fit_keeps_the_centre() {
        let columns = RgbImage::from_fn(300, 100, |x, _| match x / 100 {
            0 => image::Rgb([255, 0, 0]),
            1 => image::Rgb([0, 255, 0]),
            _ => image::Rgb([0, 0, 255]),
        });
        let png = encode_image(&DynamicImage::ImageRgb8(columns), ImageFormat::Png);
        let out = run(
            &png,
            Format::Png,
            Format::Png,
            ResizeStrategy::CropToFit,
            100,
            100,
            100,
        );
        assert_eq!((out.width(), out.height()), (100, 100));
        assert!(out.to_rgb8().pixels().all(|p| p.0 == [0, 255, 0]));
    }

    #[test]
    fn crop_to_fit_extreme_aspect_is_cheap() {
        let result = resize_crop_to_fit(make_rgb_image(20_000, 2, 10, 20, 30), 200, 200);
        assert_eq!((result.width(), result.height()), (2, 2));
    }

    #[test]
    fn crop_to_fit_never_exceeds_the_box() {
        let result = resize_crop_to_fit(make_rgb_image(5000, 1009, 10, 20, 30), 100, 1000);
        assert_eq!((result.width(), result.height()), (100, 1000));
    }

    #[test]
    fn decode_rejects_oversized_header() {
        let mut jpeg = encode_image(&make_rgb_image(16, 16, 1, 2, 3), ImageFormat::Jpeg);
        let p = jpeg.windows(2).position(|m| m == [0xFF, 0xC0]).unwrap();
        // SOF0: length (2), precision (1), then height and width, big-endian.
        jpeg[p + 5..p + 9].copy_from_slice(&[0x32, 0xC8, 0x32, 0xC8]);
        // Not expect_err: it would Debug-print a 13000x13000 image.
        let Err(err) = decode(&jpeg, ImageFormat::Jpeg) else {
            panic!("a 13000x13000 header must be rejected");
        };
        assert!(is_invalid_args(&err), "{err}");
        assert!(err.to_string().contains("pixels"), "{err}");
    }

    #[test]
    fn decode_allows_pixel_buffers_above_the_decoder_alloc_budget() {
        let (w, h) = (4200, 4000);
        assert!(w * h * 4 > 64 << 20);
        assert!(u64::from(w * h) <= MAX_PIXELS);
        let mut png = Vec::new();
        let enc = image::codecs::png::PngEncoder::new_with_quality(
            &mut png,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::NoFilter,
        );
        DynamicImage::ImageRgba8(RgbaImage::new(w, h))
            .write_with_encoder(enc)
            .unwrap();
        let img = decode(&png, ImageFormat::Png).unwrap();
        assert_eq!((img.width(), img.height()), (w, h));
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
            input_format: Format::Jpeg,
            output_format: Format::Jpeg,
        };
        let (output, _) = process_image(&bytes, &params).unwrap();
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
            input_format: Format::Png,
            output_format: Format::Png,
        };
        let (output, _) = process_image(&bytes, &params).unwrap();
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
            input_format: Format::Webp,
            output_format: Format::Webp,
        };
        let (output, _) = process_image(&bytes, &params).unwrap();
        let decoded = image::load_from_memory_with_format(&output, ImageFormat::WebP).unwrap();
        assert!(decoded.width() <= 100);
        assert!(decoded.height() <= 100);
    }

    fn webp_params(w: u32, h: u32, quality: u8, input_format: Format) -> ProcessingParams {
        ProcessingParams {
            target_width: w,
            target_height: h,
            strategy: ResizeStrategy::ScaleToFit,
            quality,
            input_format,
            output_format: Format::Webp,
        }
    }

    #[test]
    fn webp_output_is_lossy_and_honors_quality() {
        let mut seed: u32 = 1;
        let noise = RgbImage::from_fn(256, 256, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let [_, r, g, b] = seed.to_le_bytes();
            image::Rgb([r, g, b])
        });
        let png = encode_image(&DynamicImage::ImageRgb8(noise), ImageFormat::Png);
        let [low, high] = [20, 90].map(|q| {
            let (out, _) = process_image(&png, &webp_params(256, 256, q, Format::Png)).unwrap();
            assert_eq!(&out[..4], b"RIFF", "q{q}");
            assert_eq!(&out[12..16], b"VP8 ", "q{q} must be lossy");
            let decoded = image::load_from_memory_with_format(&out, ImageFormat::WebP).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (256, 256), "q{q}");
            out.len()
        });
        assert!(low < high, "q20 is {low} bytes, q90 is {high} bytes");
    }

    #[test]
    fn webp_output_keeps_alpha() {
        let half = RgbaImage::from_fn(64, 64, |x, _| {
            image::Rgba(if x < 32 { [200, 30, 30, 255] } else { [0; 4] })
        });
        let png = encode_image(&DynamicImage::ImageRgba8(half), ImageFormat::Png);
        let (out, _) = process_image(&png, &webp_params(64, 64, 80, Format::Png)).unwrap();
        let decoded = image::load_from_memory_with_format(&out, ImageFormat::WebP).unwrap();
        assert!(decoded.to_rgba8().pixels().any(|p| p.0[3] < 255));
    }

    #[test]
    fn webp_quality_defaults_to_config() {
        let metadata = ImageMetadata {
            output_format: Some(Format::Webp),
            ..Default::default()
        };
        let params = resolve_params(&default_config(), &metadata, Format::Png).unwrap();
        assert_eq!(params.quality, 80);
    }

    #[test]
    fn output_side_limits() {
        let strip = |w| {
            let mut png = Vec::new();
            DynamicImage::ImageLuma8(image::GrayImage::new(w, 1))
                .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
                .unwrap();
            png
        };
        let webp = webp_params(20_000, 1, 80, Format::Png);
        let err = process_image(&strip(16_384), &webp).unwrap_err();
        assert!(is_invalid_args(&err), "{err}");
        assert!(err.to_string().contains("16383"), "{err}");

        let jpeg = ProcessingParams {
            target_width: 70_000,
            output_format: Format::Jpeg,
            ..webp
        };
        let err = process_image(&strip(65_536), &jpeg).unwrap_err();
        assert!(is_invalid_args(&err), "{err}");
        assert!(err.to_string().contains("65535"), "{err}");
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
            input_format: Format::Jpeg,
            output_format: Format::Jpeg,
        };
        let params_high = ProcessingParams {
            target_width: 200,
            target_height: 200,
            strategy: ResizeStrategy::ScaleToFit,
            quality: 95,
            input_format: Format::Jpeg,
            output_format: Format::Jpeg,
        };

        let (output_low, _) = process_image(&bytes, &params_low).unwrap();
        let (output_high, _) = process_image(&bytes, &params_high).unwrap();

        assert!(
            output_low.len() < output_high.len(),
            "Quality 50 ({} bytes) should produce smaller output than quality 95 ({} bytes)",
            output_low.len(),
            output_high.len()
        );
    }

    // ── EXIF orientation ──────────────────────────────

    /// A minimal big-endian TIFF with one IFD entry: Orientation = n.
    fn tiff_orientation(n: u8) -> Vec<u8> {
        let mut tiff = b"MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0".to_vec();
        tiff.extend([n, 0, 0, 0, 0, 0, 0]);
        tiff
    }

    fn png_with_orientation(img: &DynamicImage, n: u8) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new(&mut buf);
        enc.set_exif_metadata(tiff_orientation(n)).unwrap();
        img.write_with_encoder(enc).unwrap();
        buf
    }

    const A: [u8; 4] = [255, 0, 0, 255];
    const B: [u8; 4] = [0, 255, 0, 255];
    const C: [u8; 4] = [0, 0, 255, 255];
    const D: [u8; 4] = [255, 255, 0, 255];

    /// [A B; C D]
    fn quad() -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_vec(2, 2, [A, B, C, D].concat()).unwrap())
    }

    fn png_out(input: &[u8]) -> DynamicImage {
        run(
            input,
            Format::Png,
            Format::Png,
            ResizeStrategy::ScaleToFit,
            200,
            200,
            100,
        )
    }

    #[test]
    fn exif_orientation_all_eight() {
        let expected = [
            (1, [A, B, C, D]),
            (2, [B, A, D, C]),
            (3, [D, C, B, A]),
            (4, [C, D, A, B]),
            (5, [A, C, B, D]),
            (6, [C, A, D, B]),
            (7, [D, B, C, A]),
            (8, [B, D, A, C]),
        ];
        for (n, pixels) in expected {
            let out = png_out(&png_with_orientation(&quad(), n));
            assert_eq!(
                out.to_rgba8().into_raw(),
                pixels.concat(),
                "orientation {n}"
            );
        }
    }

    #[test]
    fn orientation_6_swaps_dimensions() {
        let out = png_out(&png_with_orientation(&make_rgb_image(2, 1, 1, 2, 3), 6));
        assert_eq!((out.width(), out.height()), (1, 2));
    }

    #[test]
    fn orientation_is_read_from_jpeg() {
        let mut jpeg = Vec::new();
        let mut enc = JpegEncoder::new(&mut jpeg);
        enc.set_exif_metadata(tiff_orientation(6)).unwrap();
        make_rgb_image(2, 1, 1, 2, 3)
            .write_with_encoder(enc)
            .unwrap();
        let out = run(
            &jpeg,
            Format::Jpeg,
            Format::Png,
            ResizeStrategy::ScaleToFit,
            200,
            200,
            100,
        );
        assert_eq!((out.width(), out.height()), (1, 2));
    }

    #[test]
    fn no_exif_is_identity() {
        let out = png_out(&encode_image(&quad(), ImageFormat::Png));
        assert_eq!(out.to_rgba8().into_raw(), [A, B, C, D].concat());
    }

    #[test]
    fn jpeg_flattens_alpha_before_resizing() {
        let half = RgbaImage::from_fn(200, 200, |x, _| {
            image::Rgba(if x < 100 { [255; 4] } else { [0; 4] })
        });
        let png = encode_image(&DynamicImage::ImageRgba8(half), ImageFormat::Png);
        let out = run(
            &png,
            Format::Png,
            Format::Jpeg,
            ResizeStrategy::ScaleToFit,
            50,
            50,
            85,
        );
        assert_eq!((out.width(), out.height()), (50, 50));
        for (x, y, p) in out.to_rgb8().enumerate_pixels() {
            assert!(
                p.0.iter().all(|&c| c >= 250),
                "pixel ({x}, {y}) is {:?}",
                p.0
            );
        }
    }

    #[test]
    fn png_output_keeps_gray() {
        let gray =
            DynamicImage::ImageLuma8(image::GrayImage::from_pixel(40, 30, image::Luma([90])));
        let mut png = Vec::new();
        gray.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let out = run(
            &png,
            Format::Png,
            Format::Png,
            ResizeStrategy::ScaleToFit,
            20,
            20,
            100,
        );
        assert_eq!((out.width(), out.height()), (20, 15));
        assert_eq!(out.color(), image::ColorType::L8);
    }

    #[test]
    fn resolve_params_rejects_bad_quality() {
        let config = default_config();
        let with_quality = |quality| ImageMetadata {
            quality: Some(quality),
            ..Default::default()
        };
        for bad in [0, 101] {
            let err = resolve_params(&config, &with_quality(bad), Format::Jpeg).unwrap_err();
            assert!(is_invalid_args(&err), "{err}");
            assert!(err.to_string().contains("quality must be 1-100"), "{err}");
        }
        for good in [1, 100] {
            let params = resolve_params(&config, &with_quality(good), Format::Jpeg).unwrap();
            assert_eq!(params.quality, good);
        }
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
            input_format: Format::Jpeg,
            output_format: Format::Png,
        };
        let (output, _) = process_image(&bytes, &params).unwrap();
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
            input_format: Format::Png,
            output_format: Format::Jpeg,
        };
        let (output, _) = process_image(&bytes, &params).unwrap();
        let decoded = image::load_from_memory_with_format(&output, ImageFormat::Jpeg).unwrap();
        assert!(decoded.width() <= 100);
        assert!(decoded.height() <= 100);
    }

    #[test]
    fn test_resolve_params_with_output_format() {
        let config = default_config();
        let metadata = ImageMetadata {
            output_format: Some(Format::Png),
            ..Default::default()
        };
        let params = resolve_params(&config, &metadata, Format::Jpeg).unwrap();
        assert_eq!(params.input_format, Format::Jpeg);
        assert_eq!(params.output_format, Format::Png);
        // PNG is lossless, so quality defaults to 100
        assert_eq!(params.quality, 100);
    }
}
