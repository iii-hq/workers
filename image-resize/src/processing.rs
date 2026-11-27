use iii_sdk::Error;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::metadata::Orientation;
use image::{
    DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbImage, Rgba, RgbaImage,
};
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

/// A top-level chunk of a WebP file, as a slice of the input.
fn webp_chunk<'a>(bytes: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut rest = bytes.get(12..)?; // past "RIFF", the size and "WEBP"
    loop {
        let (head, tail) = rest.split_at_checked(8)?;
        let size = u32::from_le_bytes(head[4..].try_into().ok()?) as usize;
        let body = tail.get(..size)?;
        if &head[..4] == fourcc {
            return Some(body);
        }
        rest = tail.get(size + (size & 1)..)?;
    }
}

/// Whether a JPEG ends after its first SOS without an EOI; decoders would fill
/// the missing rows with grey. Segments are skipped by length, so an EOI inside
/// one (an EXIF thumbnail's) does not count.
fn jpeg_truncated(bytes: &[u8]) -> bool {
    let mut rest = bytes.get(2..).unwrap_or_default(); // past SOI
    let mut scanned = false;
    loop {
        // In scan data FF00 is a stuffed byte, FFD0-FFD7 a restart and FFFF fill.
        let Some(p) = rest
            .windows(2)
            .position(|m| m[0] == 0xFF && !matches!(m[1], 0x00 | 0xD0..=0xD7 | 0xFF))
        else {
            return scanned;
        };
        let marker = rest[p + 1];
        rest = &rest[p + 2..];
        scanned |= marker == 0xDA;
        match marker {
            0xD9 => return false,
            0x01 | 0xD8 => {} // no length
            _ => {
                let [hi, lo, ..] = *rest else {
                    return scanned;
                };
                let Some(tail) = rest.get(usize::from(u16::from_be_bytes([hi, lo]))..) else {
                    return scanned;
                };
                rest = tail;
            }
        }
    }
}

/// Largest image decoded, in pixels. The resize keeps an f32 buffer of up to
/// 16 bytes per source pixel, so this bounds memory per image.
pub const MAX_PIXELS: u64 = 50_000_000;

fn check_pixels(w: u32, h: u32) -> Result<(), Error> {
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(invalid(format!(
            "image is {w}x{h}; the limit is {MAX_PIXELS} pixels"
        )));
    }
    Ok(())
}

/// The image, oriented, and its embedded ICC profile.
fn decode(bytes: &[u8], format: ImageFormat) -> Result<(DynamicImage, Option<Vec<u8>>), Error> {
    let undecodable =
        |e: image::ImageError| invalid(format!("could not decode the {format:?} image: {e}"));
    if format == ImageFormat::Jpeg && jpeg_truncated(bytes) {
        return Err(invalid(
            "the JPEG is truncated: its data ends before the end-of-image marker",
        ));
    }
    let (mut img, orientation, icc) = if format == ImageFormat::WebP {
        let orientation = webp_chunk(bytes, b"EXIF").and_then(Orientation::from_exif_chunk);
        let icc = webp_chunk(bytes, b"ICCP").map(<[u8]>::to_vec);
        (decode_webp(bytes)?, orientation, icc)
    } else {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = Limits::default();
        // into_decoder does not charge the pixel buffer (checked below); this
        // bounds what a PNG's ancillary chunks may inflate to.
        limits.max_alloc = Some(64 << 20);
        reader.limits(limits);
        let mut decoder = reader.into_decoder().map_err(undecodable)?;
        let (w, h) = decoder.dimensions();
        check_pixels(w, h)?;
        // JPEG: one APP1 segment, at most 64 KiB. PNG: the eXIf chunk the decoder already holds.
        let orientation = decoder.orientation().ok();
        // JPEG: its APP2 segments, at most 16 MiB. PNG: the iCCP chunk the decoder already holds.
        let icc = decoder.icc_profile().ok().flatten();
        (
            DynamicImage::from_decoder(decoder).map_err(undecodable)?,
            orientation,
            icc,
        )
    };
    img.apply_orientation(orientation.unwrap_or(Orientation::NoTransforms));
    Ok((img, icc))
}

/// Largest ICC profile applied; real RGB profiles take a few KiB.
const MAX_ICC_BYTES: usize = 4 << 20;

/// `img` converted from the colour profile `icc` to sRGB, alpha kept. Any
/// profile but an RGB one on an RGB image leaves it as it is.
fn to_srgb(mut img: DynamicImage, icc: &[u8]) -> DynamicImage {
    if let Err(why) = convert_to_srgb(&mut img, icc) {
        tracing::debug!(%why, "colour profile not applied");
    }
    img
}

fn convert_to_srgb(img: &mut DynamicImage, icc: &[u8]) -> Result<(), String> {
    use moxcms::{ColorProfile, DataColorSpace, Layout, RenderingIntent, TransformOptions};
    if icc.len() > MAX_ICC_BYTES {
        return Err(format!("the profile is {} bytes", icc.len()));
    }
    let profile = ColorProfile::new_from_slice(icc).map_err(|e| format!("bad profile: {e}"))?;
    if profile.color_space != DataColorSpace::Rgb {
        return Err(format!("a {:?} profile", profile.color_space));
    }
    let srgb = ColorProfile::new_srgb();
    // Relative colorimetric, libvips' thumbnail default: in-gamut colours keep
    // their values and the rest clip. Matrix profiles (Display P3, Adobe RGB) define no other.
    let options = TransformOptions {
        rendering_intent: RenderingIntent::RelativeColorimetric,
        ..Default::default()
    };
    let (rgb, rgba) = (Layout::Rgb, Layout::Rgba);
    // A profile that moves no colour of a 6x6x6 cube by more than a level is
    // sRGB in effect; skipping it keeps sRGB-tagged images byte-identical.
    let probe: Vec<u8> = (0..216u8)
        .flat_map(|i| [i / 36, i / 6 % 6, i % 6].map(|c| c * 51))
        .collect();
    let mut probed = probe.clone();
    apply(
        profile.create_transform_8bit(rgb, &srgb, rgb, options),
        &mut probed,
    )
    .map_err(|e| e.to_string())?;
    if probe.iter().zip(&probed).all(|(a, b)| a.abs_diff(*b) <= 1) {
        return Ok(());
    }
    match img {
        DynamicImage::ImageRgb8(b) => {
            apply(profile.create_transform_8bit(rgb, &srgb, rgb, options), b)
        }
        DynamicImage::ImageRgba8(b) => {
            apply(profile.create_transform_8bit(rgba, &srgb, rgba, options), b)
        }
        DynamicImage::ImageRgb16(b) => {
            apply(profile.create_transform_16bit(rgb, &srgb, rgb, options), b)
        }
        DynamicImage::ImageRgba16(b) => apply(
            profile.create_transform_16bit(rgba, &srgb, rgba, options),
            b,
        ),
        other => return Err(format!("a {:?} image", other.color())),
    }
    .map_err(|e| e.to_string())
}

fn apply<V: Copy + Default>(
    transform: Result<
        std::sync::Arc<dyn moxcms::TransformExecutor<V> + Send + Sync>,
        moxcms::CmsError,
    >,
    px: &mut [V],
) -> Result<(), moxcms::CmsError> {
    let transform = transform?;
    // Slices of whole RGB and RGBA pixels, so a full-size image needs no full-size copy.
    for part in px.chunks_mut(12 << 10) {
        let src = part.to_vec();
        transform.transform(&src, part)?;
    }
    Ok(())
}

/// A still WebP, or an animation's first frame on its canvas. libwebp checks
/// each frame's bitstream against the canvas before it allocates pixels.
fn decode_webp(bytes: &[u8]) -> Result<DynamicImage, Error> {
    use libwebp_sys::{VP8StatusCode, WebPBitstreamFeatures};
    let undecodable = |why: &str| invalid(format!("could not decode the WebP image: {why}"));
    let mut features = WebPBitstreamFeatures {
        width: 0,
        height: 0,
        has_alpha: 0,
        has_animation: 0,
        format: 0,
        pad: [0; 5],
    };
    // SAFETY: libwebp reads at most `bytes.len()` bytes and writes only `features`.
    let status =
        unsafe { libwebp_sys::WebPGetFeatures(bytes.as_ptr(), bytes.len(), &mut features) };
    if status != VP8StatusCode::VP8_STATUS_OK {
        return Err(undecodable(&format!("{status:?}")));
    }
    let (Ok(w), Ok(h)) = (
        u32::try_from(features.width),
        u32::try_from(features.height),
    ) else {
        return Err(undecodable("negative size"));
    };
    check_pixels(w, h)?;
    if features.has_animation != 0 {
        return first_frame(bytes, w, h)
            .map(DynamicImage::ImageRgba8)
            .ok_or_else(|| undecodable("libwebp rejected the animation"));
    }

    let alpha = features.has_alpha != 0;
    let decode_into = if alpha {
        libwebp_sys::WebPDecodeRGBAInto
    } else {
        libwebp_sys::WebPDecodeRGBInto
    };
    // check_pixels capped w*h at MAX_PIXELS, so neither product overflows.
    let stride = w as usize * if alpha { 4 } else { 3 };
    let mut px = vec![0; stride * h as usize];
    // SAFETY: `px` is h rows of `stride` bytes, and libwebp writes only inside `px.len()` bytes.
    let out = unsafe {
        decode_into(
            bytes.as_ptr(),
            bytes.len(),
            px.as_mut_ptr(),
            px.len(),
            stride as std::ffi::c_int,
        )
    };
    if out.is_null() {
        return Err(undecodable("libwebp rejected its data"));
    }
    Ok(if alpha {
        DynamicImage::ImageRgba8(RgbaImage::from_raw(w, h, px).expect("w*h RGBA pixels"))
    } else {
        DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, px).expect("w*h RGB pixels"))
    })
}

/// The first frame of an animation whose canvas `decode_webp` measured as `w`x`h`.
fn first_frame(bytes: &[u8], w: u32, h: u32) -> Option<RgbaImage> {
    use libwebp_sys::{WEBP_CSP_MODE, WebPAnimDecoder, WebPAnimDecoderOptions, WebPAnimInfo};
    struct Decoder(*mut WebPAnimDecoder);
    impl Drop for Decoder {
        fn drop(&mut self) {
            // SAFETY: the pointer came from WebPAnimDecoderNew, which may return null (a no-op here).
            unsafe { libwebp_sys::WebPAnimDecoderDelete(self.0) }
        }
    }
    let options = WebPAnimDecoderOptions {
        color_mode: WEBP_CSP_MODE::MODE_RGBA,
        use_threads: 0,
        padding: [0; 7],
    };
    let data = libwebp_sys::WebPData {
        bytes: bytes.as_ptr(),
        size: bytes.len(),
    };
    // SAFETY: the decoder reads `bytes`, which outlives it: `dec` drops before this returns.
    let dec = Decoder(unsafe { libwebp_sys::WebPAnimDecoderNew(&data, &options) });
    if dec.0.is_null() {
        return None;
    }
    let mut info = WebPAnimInfo::default();
    let mut frame = std::ptr::null_mut();
    let mut timestamp = 0;
    // SAFETY: `dec` is live, and both calls write only through the pointers passed.
    let decoded = unsafe {
        libwebp_sys::WebPAnimDecoderGetInfo(dec.0, &mut info) != 0
            && libwebp_sys::WebPAnimDecoderGetNext(dec.0, &mut frame, &mut timestamp) != 0
    };
    // The canvas must be the one check_pixels passed, which bounds the copy below.
    if !decoded || (info.canvas_width, info.canvas_height) != (w, h) {
        return None;
    }
    // SAFETY: `frame` is the decoder's w*h RGBA canvas, valid until `dec` drops.
    let canvas = unsafe { std::slice::from_raw_parts(frame, w as usize * h as usize * 4) };
    RgbaImage::from_raw(w, h, canvas.to_vec())
}

fn over_white(img: &DynamicImage) -> DynamicImage {
    let mut bg = RgbaImage::from_pixel(img.width(), img.height(), Rgba([255; 4]));
    imageops::overlay(&mut bg, img, 0, 0);
    DynamicImage::ImageRgba8(bg)
}

/// Runs `resize` on colour premultiplied by alpha: Lanczos3 mixes channels as
/// stored, so the hidden colour of transparent pixels would bleed into edges.
fn premultiplied(
    mut img: DynamicImage,
    resize: impl FnOnce(&DynamicImage) -> DynamicImage,
) -> DynamicImage {
    let weighed = weigh_by_alpha(&mut img, false);
    let mut out = resize(&img);
    if weighed {
        weigh_by_alpha(&mut out, true);
    }
    out
}

/// Multiplies colour by alpha in place, or divides it back out with `undo`.
/// False, with the image untouched, when it has no alpha or, unless `undo`,
/// every pixel is opaque.
fn weigh_by_alpha(img: &mut DynamicImage, undo: bool) -> bool {
    match img {
        DynamicImage::ImageLumaA8(b) => weigh(b, 2, undo),
        DynamicImage::ImageRgba8(b) => weigh(b, 4, undo),
        DynamicImage::ImageLumaA16(b) => weigh(b, 2, undo),
        DynamicImage::ImageRgba16(b) => weigh(b, 4, undo),
        // No alpha, or a float image, which decode never makes.
        _ => false,
    }
}

fn weigh<S>(px: &mut [S], channels: usize, undo: bool) -> bool
where
    S: image::Primitive + Into<u64> + TryFrom<u64>,
{
    let max: u64 = S::DEFAULT_MAX_VALUE.into();
    let alpha = |p: &[S]| -> u64 { p[channels - 1].into() };
    if !undo && px.chunks_exact(channels).all(|p| alpha(p) == max) {
        return false;
    }
    for p in px.chunks_exact_mut(channels) {
        let a = alpha(p);
        for c in &mut p[..channels - 1] {
            let c64: u64 = (*c).into();
            let v = match (undo, a) {
                (false, _) => (c64 * a + max / 2) / max,
                (true, 0) => 0,
                (true, _) => (c64 * max + a / 2) / a,
            };
            // Saturates: the filter's negative lobes can leave colour above alpha.
            *c = S::try_from(v).unwrap_or(S::DEFAULT_MAX_VALUE);
        }
    }
    true
}

pub fn resize_scale_to_fit(img: DynamicImage, max_w: u32, max_h: u32) -> DynamicImage {
    if img.width() <= max_w && img.height() <= max_h {
        return img;
    }
    premultiplied(img, |img| img.resize(max_w, max_h, FilterType::Lanczos3))
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
        premultiplied(cropped, |img| {
            img.resize_exact(cw.min(box_w), ch.min(box_h), FilterType::Lanczos3)
        })
    } else {
        cropped
    }
}

pub fn process_image(
    bytes: &[u8],
    params: &ProcessingParams,
) -> Result<(Vec<u8>, ThumbnailMetadata), Error> {
    let (mut img, icc) = decode(bytes, params.input_format.image_format())?;
    let (source_width, source_height) = (img.width(), img.height());
    // JPEG has no alpha. Flattening before the resize leaves it an opaque
    // image, which skips the premultiplication.
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
    // After the resize, so the conversion touches only the thumbnail's pixels.
    let thumbnail = match icc {
        Some(icc) => to_srgb(thumbnail, &icc),
        None => thumbnail,
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
    use image::ImageEncoder;

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
            ImageFormat::WebP => buf = encode_webp(img, 90).unwrap(),
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
        for (w, h) in [(300, 100), (100, 300)] {
            let bands = RgbImage::from_fn(w, h, |x, y| match x.max(y) / 100 {
                0 => image::Rgb([255, 0, 0]),
                1 => image::Rgb([0, 255, 0]),
                _ => image::Rgb([0, 0, 255]),
            });
            let png = encode_image(&DynamicImage::ImageRgb8(bands), ImageFormat::Png);
            let out = run(
                &png,
                Format::Png,
                Format::Png,
                ResizeStrategy::CropToFit,
                100,
                100,
                100,
            );
            assert_eq!((out.width(), out.height()), (100, 100), "{w}x{h}");
            assert!(
                out.to_rgb8().pixels().all(|p| p.0 == [0, 255, 0]),
                "{w}x{h}"
            );
        }
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
    fn jpeg_truncated_follows_segments_restarts_and_scans() {
        // SOI, a DQT holding FFD9, a scan with stuffing, a restart and fill,
        // a DHT, a second scan, EOI.
        let jpeg = b"\xFF\xD8\xFF\xDB\x00\x04\xFF\xD9\xFF\xDA\x00\x03\x00\x12\xFF\x00\xFF\xD3\
            \x34\xFF\xFF\xFF\xC4\x00\x03\x00\xFF\xDA\x00\x02\x56\xFF\xD9";
        assert!(!jpeg_truncated(jpeg));
        assert!(!jpeg_truncated(
            &[jpeg.as_slice(), b"\xFF\xDAtrailing"].concat()
        ));
        // Before the first SOS the decoder reports the damage itself.
        for n in 0..jpeg.len() {
            assert_eq!(jpeg_truncated(&jpeg[..n]), n >= 10, "first {n} bytes");
        }
    }

    #[test]
    fn jpeg_truncated_survives_bad_lengths() {
        for (bytes, truncated) in [
            (b"\xFF\xD8\xFF\xE1\x00\x00\xFF\xDA".as_slice(), true),
            (b"\xFF\xD8\xFF\xE1\x00\x01", false),
            (b"\xFF\xD8\xFF\xE1\xFF\xFF\xFF\xDA\x00\x02\xFF\xD9", false),
            (b"\xFF\xD8\xFF\xDA\xFF\xFF\xFF\xD9", true),
            (b"\xFF\xD8\xFF\xDA\x00", true),
            (b"\xFF", false),
        ] {
            assert_eq!(jpeg_truncated(bytes), truncated, "{bytes:02X?}");
        }
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
        let img = decode(&png, ImageFormat::Png).unwrap().0;
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
        let decoded = decode(&output, ImageFormat::WebP).unwrap().0;
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
            let decoded = decode(&out, ImageFormat::WebP).unwrap().0;
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
        let decoded = decode(&out, ImageFormat::WebP).unwrap().0;
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

    fn png_with_exif(img: &DynamicImage, tiff: Vec<u8>) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new(&mut buf);
        enc.set_exif_metadata(tiff).unwrap();
        img.write_with_encoder(enc).unwrap();
        buf
    }

    /// A big-endian TIFF whose IFD0 points `k` Exif IFD entries at overlapping child IFDs of `c` entries.
    fn tiff_overlapping_child_ifds(k: u16, c: u16) -> Vec<u8> {
        let children = 8 + 2 + 12 * u32::from(k) + 4;
        let mut tiff = b"MM\0*\0\0\0\x08".to_vec();
        tiff.extend(k.to_be_bytes());
        for i in 0..u32::from(k) {
            tiff.extend([0x87, 0x69, 0, 4, 0, 0, 0, 1]);
            tiff.extend((children + 2 * i).to_be_bytes());
        }
        tiff.extend([0; 4]);
        // Child i starts at byte 2i of this region: its count, then 0xFF filler entries.
        let mut region = vec![0xFF; 2 + 12 * usize::from(c)];
        for count in region[..2 * usize::from(k)].as_chunks_mut::<2>().0 {
            *count = c.to_be_bytes();
        }
        tiff.extend(region);
        // Every child's next-IFD offset lands here: 0, no further IFD.
        tiff.extend(vec![0; 2 * usize::from(k) + 4]);
        tiff
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
            let out = png_out(&png_with_exif(&quad(), tiff_orientation(n)));
            assert_eq!(
                out.to_rgba8().into_raw(),
                pixels.concat(),
                "orientation {n}"
            );
        }
    }

    #[test]
    fn orientation_6_swaps_dimensions() {
        let png = png_with_exif(&make_rgb_image(2, 1, 1, 2, 3), tiff_orientation(6));
        let params = ProcessingParams {
            output_format: Format::Png,
            ..webp_params(200, 200, 100, Format::Png)
        };
        let (out, meta) = process_image(&png, &params).unwrap();
        assert_eq!((meta.source_width, meta.source_height), (1, 2));
        assert_eq!((meta.width, meta.height), (1, 2));
        let out = image::load_from_memory_with_format(&out, ImageFormat::Png).unwrap();
        assert_eq!((out.width(), out.height()), (1, 2));
    }

    #[test]
    fn exif_with_overlapping_child_ifds_is_cheap() {
        let tiff = tiff_overlapping_child_ifds(4000, 4000);
        assert!(tiff.len() < 110_000, "{} bytes", tiff.len());
        let png = png_with_exif(&make_rgb_image(1, 1, 1, 2, 3), tiff);
        let params = ProcessingParams {
            output_format: Format::Png,
            ..webp_params(200, 200, 100, Format::Png)
        };
        let start = std::time::Instant::now();
        let (_, meta) = process_image(&png, &params).unwrap();
        let elapsed = start.elapsed();
        assert_eq!((meta.width, meta.height), (1, 1));
        assert!(elapsed.as_secs_f64() < 1.0, "took {elapsed:?}");
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
    fn orientation_is_read_from_webp() {
        let still = encode_webp(&make_rgb_image(2, 1, 1, 2, 3), 90).unwrap();
        let mut webp = riff(&[
            vp8x(0x08, 2, 1),
            still[12..].to_vec(),
            chunk(b"EXIF", &tiff_orientation(6)),
        ]);
        let out = run(
            &webp,
            Format::Webp,
            Format::Png,
            ResizeStrategy::ScaleToFit,
            200,
            200,
            100,
        );
        assert_eq!((out.width(), out.height()), (1, 2));

        let p = webp.windows(4).position(|w| w == b"EXIF").unwrap();
        webp[p + 4..p + 8].fill(0xFF);
        assert_eq!(webp_chunk(&webp, b"EXIF"), None);
    }

    #[test]
    fn no_exif_is_identity() {
        let out = png_out(&encode_image(&quad(), ImageFormat::Png));
        assert_eq!(out.to_rgba8().into_raw(), [A, B, C, D].concat());
    }

    // ── WebP containers ──────────────────────────────

    /// A RIFF chunk: fourcc, little-endian size, payload, pad to an even length.
    fn chunk(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(payload.len()).unwrap().to_le_bytes();
        [fourcc, &size, payload, &vec![0; payload.len() % 2]].concat()
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body = chunks.concat();
        let size = u32::try_from(body.len() + 4).unwrap().to_le_bytes();
        [b"RIFF".as_slice(), &size, b"WEBP", &body].concat()
    }

    fn le24(n: u32) -> [u8; 3] {
        let [a, b, c, _] = n.to_le_bytes();
        [a, b, c]
    }

    /// A VP8X chunk for a `w`x`h` canvas; 0x02 marks an animation, 0x08 EXIF.
    fn vp8x(flags: u8, w: u32, h: u32) -> Vec<u8> {
        chunk(
            b"VP8X",
            &[[flags, 0, 0, 0].as_slice(), &le24(w - 1), &le24(h - 1)].concat(),
        )
    }

    /// An ANMF chunk that says it is `w`x`h` at the origin and holds `frame`'s chunks.
    fn anmf(w: u32, h: u32, frame: &[u8]) -> Vec<u8> {
        let header = [
            [0; 6].as_slice(),
            &le24(w - 1),
            &le24(h - 1),
            &[100, 0, 0, 0],
        ]
        .concat();
        chunk(b"ANMF", &[header.as_slice(), frame].concat())
    }

    fn animation(w: u32, h: u32, frames: &[Vec<u8>]) -> Vec<u8> {
        riff(&[&[vp8x(0x02, w, h), chunk(b"ANIM", &[0; 6])], frames].concat())
    }

    /// A lossy 16x16 WebP whose VP8 frame header says `side`x`side`.
    fn webp_claiming(side: u32) -> Vec<u8> {
        let mut webp = encode_webp(&make_rgb_image(16, 16, 1, 2, 3), 80).unwrap();
        assert_eq!(&webp[12..16], b"VP8 ");
        let side = u16::try_from(side).unwrap().to_le_bytes();
        // After the chunk header, the 3-byte frame tag and the 3-byte start code.
        webp[26..30].copy_from_slice(&[side, side].concat());
        webp
    }

    /// The largest Rust allocation this thread made since the last `take`.
    mod largest_alloc {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;

        thread_local!(static LARGEST: Cell<usize> = const { Cell::new(0) });

        fn note(size: usize) {
            let _ = LARGEST.try_with(|c| c.set(c.get().max(size)));
        }

        pub fn take() -> usize {
            LARGEST.with(|c| c.replace(0))
        }

        struct Probe;

        // SAFETY: every call goes to System unchanged; `note` neither allocates nor panics.
        unsafe impl GlobalAlloc for Probe {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                note(layout.size());
                unsafe { System.alloc(layout) }
            }
            unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
                note(layout.size());
                unsafe { System.alloc_zeroed(layout) }
            }
            unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
                note(size);
                unsafe { System.realloc(ptr, layout, size) }
            }
            unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
                unsafe { System.dealloc(ptr, layout) }
            }
        }

        #[global_allocator]
        static PROBE: Probe = Probe;
    }

    #[test]
    fn webp_frame_unlike_its_canvas_is_rejected_up_front() {
        let webp = riff(&[vp8x(0, 1, 1), webp_claiming(16_383)[12..].to_vec()]);
        largest_alloc::take();
        let Err(err) = decode(&webp, ImageFormat::WebP) else {
            panic!("a 16383x16383 frame on a 1x1 canvas must be rejected");
        };
        let largest = largest_alloc::take();
        assert!(is_invalid_args(&err), "{err}");
        assert!(largest < 1 << 20, "allocated {largest} bytes");
    }

    #[test]
    fn webp_over_the_pixel_limit_is_rejected_before_decoding() {
        let frame = encode_webp(&make_rgb_image(16, 16, 1, 2, 3), 80).unwrap();
        for webp in [
            webp_claiming(8000),
            animation(8000, 8000, &[anmf(16, 16, &frame[12..])]),
        ] {
            let Err(err) = decode(&webp, ImageFormat::WebP) else {
                panic!("an 8000x8000 WebP must be rejected");
            };
            assert!(is_invalid_args(&err), "{err}");
            assert!(err.to_string().contains("8000x8000"), "{err}");
        }
    }

    #[test]
    fn animated_webp_resizes_its_first_frame() {
        let frame = |r, b| {
            anmf(
                8,
                8,
                &encode_webp(&make_rgb_image(8, 8, r, 0, b), 90).unwrap()[12..],
            )
        };
        let webp = animation(8, 8, &[frame(255, 0), frame(0, 255)]);
        let out = run(
            &webp,
            Format::Webp,
            Format::Png,
            ResizeStrategy::ScaleToFit,
            4,
            4,
            100,
        );
        assert_eq!((out.width(), out.height()), (4, 4));
        for p in out.to_rgba8().pixels() {
            assert!(p[0] > 200 && p[2] < 50 && p[3] == 255, "{p:?}");
        }
    }

    #[test]
    fn animated_frame_is_sized_by_its_bitstream() {
        let blue = encode_webp(&make_rgba_image(16, 16, 0, 0, 255, 128), 90).unwrap();
        assert_eq!(&blue[30..34], b"ALPH", "VP8X, then ALPH and VP8");
        let webp = animation(16, 16, &[anmf(1, 1, &blue[30..])]);
        let img = decode(&webp, ImageFormat::WebP).unwrap().0;
        assert_eq!((img.width(), img.height()), (16, 16));
        for p in img.to_rgba8().pixels() {
            assert!(p[2] > 200 && p[3] == 128, "{p:?}");
        }
    }

    #[test]
    fn webp_input_keeps_alpha() {
        let half = RgbaImage::from_fn(16, 16, |x, _| {
            image::Rgba(if x < 8 { [200, 30, 30, 255] } else { [0; 4] })
        });
        let webp = encode_webp(&DynamicImage::ImageRgba8(half), 90).unwrap();
        let out = run(
            &webp,
            Format::Webp,
            Format::Png,
            ResizeStrategy::ScaleToFit,
            16,
            16,
            100,
        )
        .to_rgba8();
        assert_eq!((out.get_pixel(0, 0)[3], out.get_pixel(15, 15)[3]), (255, 0));
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
    fn transparent_pixels_do_not_bleed_their_hidden_colour() {
        let rgba8 = DynamicImage::ImageRgba8(RgbaImage::from_fn(40, 20, |x, _| {
            image::Rgba(if x < 20 {
                [255, 0, 0, 0]
            } else {
                [0, 255, 0, 255]
            })
        }));
        for img in [rgba8.clone(), DynamicImage::ImageRgba16(rgba8.to_rgba16())] {
            let color = img.color();
            for out in [
                resize_scale_to_fit(img.clone(), 10, 10),
                resize_crop_to_fit(img.clone(), 10, 10),
            ] {
                assert_eq!(out.color(), color);
                for (x, y, p) in out.to_rgba8().enumerate_pixels() {
                    assert!(p[3] == 0 || p[0] <= 2, "{color:?} ({x}, {y}) is {:?}", p.0);
                }
            }
        }
    }

    #[test]
    fn opaque_alpha_resizes_as_before() {
        let mut seed: u32 = 1;
        let noise = RgbaImage::from_fn(64, 48, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let [_, r, g, b] = seed.to_le_bytes();
            image::Rgba([r, g, b, 255])
        });
        let img = DynamicImage::ImageRgba8(noise);
        assert_eq!(
            resize_scale_to_fit(img.clone(), 20, 20),
            img.resize(20, 20, FilterType::Lanczos3)
        );
    }

    #[test]
    fn alpha_weighting_rounds_and_zeroes_transparent_colour() {
        let rgba = |px: Vec<u8>| DynamicImage::ImageRgba8(RgbaImage::from_raw(3, 1, px).unwrap());
        let mut img = rgba(vec![255, 128, 1, 128, 200, 100, 50, 0, 10, 20, 30, 255]);
        assert!(weigh_by_alpha(&mut img, false));
        assert_eq!(
            img.as_bytes(),
            [128, 64, 1, 128, 0, 0, 0, 0, 10, 20, 30, 255]
        );

        let mut img = rgba(vec![128, 64, 1, 128, 5, 5, 5, 0, 200, 0, 0, 100]);
        assert!(weigh_by_alpha(&mut img, true));
        assert_eq!(
            img.as_bytes(),
            [255, 128, 2, 128, 0, 0, 0, 0, 255, 0, 0, 100]
        );

        let mut opaque = rgba(vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255]);
        assert!(!weigh_by_alpha(&mut opaque, false));
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

    // ── Colour profiles ──────────────────────────────

    fn display_p3() -> Vec<u8> {
        moxcms::ColorProfile::new_display_p3().encode().unwrap()
    }

    fn png_with_icc(img: &DynamicImage, icc: Vec<u8>) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new(&mut buf);
        enc.set_icc_profile(icc).unwrap();
        img.write_with_encoder(enc).unwrap();
        buf
    }

    /// An 8-bit Display P3 colour in sRGB, through the standard P3-D65 to sRGB
    /// linear matrix; both spaces use the sRGB curve.
    fn p3_to_srgb(p: [u8; 3]) -> [u8; 3] {
        const M: [[f64; 3]; 3] = [
            [1.224_940_2, -0.224_940_4, 0.0],
            [-0.042_056_9, 1.042_057_1, 0.0],
            [-0.019_637_6, -0.078_636_1, 1.098_273_5],
        ];
        let linear = p.map(|c| {
            let c = f64::from(c) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        });
        M.map(|row| {
            let v = (0..3)
                .map(|i| row[i] * linear[i])
                .sum::<f64>()
                .clamp(0.0, 1.0);
            let v = if v <= 0.003_130_8 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            };
            (v * 255.0).round() as u8
        })
    }

    /// The PNG output for `input` at its own size; it must carry no profile.
    fn untagged_png(input: &[u8], input_format: Format) -> DynamicImage {
        let params = ProcessingParams {
            output_format: Format::Png,
            ..webp_params(1000, 1000, 100, input_format)
        };
        let (out, _) = process_image(input, &params).unwrap();
        assert!(
            !out.windows(4).any(|w| w == b"iCCP"),
            "output has a profile"
        );
        image::load_from_memory_with_format(&out, ImageFormat::Png).unwrap()
    }

    /// Asserts that `tagged`, a Display P3 image, comes out as its pixels in sRGB.
    fn assert_p3_converted(tagged: &[u8], input_format: Format) {
        let (raw, icc) = decode(tagged, input_format.image_format()).unwrap();
        assert_eq!(icc, Some(display_p3()));
        let out = untagged_png(tagged, input_format);
        assert_eq!(out.color(), raw.color());
        for (p, r) in out.to_rgba8().pixels().zip(raw.to_rgba8().pixels()) {
            let want = p3_to_srgb([r[0], r[1], r[2]]);
            let near = (0..3).all(|c| p[c].abs_diff(want[c]) <= 2);
            assert!(
                near && p[3] == r[3],
                "P3 {:?} came out {:?}, not {want:?}",
                r.0,
                p.0
            );
        }
    }

    #[test]
    fn display_p3_png_is_converted_to_srgb() {
        // In gamut, white, black, and a green sRGB cannot show, which clips.
        let px = [
            [200, 100, 50, 128],
            [255, 255, 255, 255],
            [0, 0, 0, 255],
            [0, 255, 0, 255],
        ];
        let img = DynamicImage::ImageRgba8(RgbaImage::from_raw(4, 1, px.concat()).unwrap());
        for img in [
            DynamicImage::ImageRgb8(img.to_rgb8()),
            DynamicImage::ImageRgb16(img.to_rgb16()),
            DynamicImage::ImageRgba16(img.to_rgba16()),
            img,
        ] {
            assert_p3_converted(&png_with_icc(&img, display_p3()), Format::Png);
        }
    }

    #[test]
    fn display_p3_jpeg_is_converted_to_srgb() {
        let mut jpeg = Vec::new();
        let mut enc = JpegEncoder::new_with_quality(&mut jpeg, 95);
        enc.set_icc_profile(display_p3()).unwrap();
        make_rgb_image(16, 16, 200, 100, 50)
            .write_with_encoder(enc)
            .unwrap();
        assert_p3_converted(&jpeg, Format::Jpeg);
    }

    #[test]
    fn display_p3_webp_is_converted_to_srgb() {
        let still = encode_webp(&make_rgb_image(16, 16, 200, 100, 50), 90).unwrap();
        // VP8X flag 0x20 marks an ICCP chunk, which comes right after it.
        let webp = riff(&[
            vp8x(0x20, 16, 16),
            chunk(b"ICCP", &display_p3()),
            still[12..].to_vec(),
        ]);
        assert_p3_converted(&webp, Format::Webp);
    }

    #[test]
    fn profiles_that_need_no_conversion_leave_pixels_unchanged() {
        let mut seed: u32 = 7;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 16) as u16
        };
        let wide = image::ImageBuffer::from_fn(16, 16, |_, _| image::Rgb([next(), next(), next()]));
        let wide = DynamicImage::ImageRgb16(wide);
        // sRGB as a 1024-entry curve, the form of the widely shipped IEC 61966-2.1 profile.
        let mut tabled = moxcms::ColorProfile::new_srgb();
        let curve = (0..1024).map(|i| {
            let c = f64::from(i) / 1023.0;
            let lin = if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
            (lin * 65535.0).round() as u16
        });
        let curve = moxcms::ToneReprCurve::Lut(curve.collect());
        (tabled.red_trc, tabled.green_trc) = (Some(curve.clone()), Some(curve.clone()));
        (tabled.blue_trc, tabled.cicp) = (Some(curve), None);
        let gray = moxcms::ColorProfile::new_gray_with_gamma(2.2);
        let profiles = [
            ("sRGB", moxcms::ColorProfile::new_srgb().encode().unwrap()),
            ("tabled sRGB", tabled.encode().unwrap()),
            ("gray", gray.encode().unwrap()),
            ("garbage", b"not an ICC profile".to_vec()),
            ("oversized", [display_p3(), vec![0; MAX_ICC_BYTES]].concat()),
        ];
        for noise in [DynamicImage::ImageRgb8(wide.to_rgb8()), wide] {
            for (why, icc) in &profiles {
                let out = untagged_png(&png_with_icc(&noise, icc.clone()), Format::Png);
                assert_eq!(out, noise, "{why} {:?}", noise.color());
            }
        }
    }
}
