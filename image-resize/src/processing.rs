use iii_sdk::Error;
use image::codecs::jpeg::JpegEncoder;
use image::imageops;
use image::metadata::Orientation;
use image::{
    DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbImage, Rgba, RgbaImage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::Cursor;
use std::num::{NonZeroU32, NonZeroU64};

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
    /// Largest output in pixels, width times height: a larger one shrinks further,
    /// keeping its aspect ratio. Defaults to the configured limit, if any.
    pub max_pixels: Option<NonZeroU64>,
    /// Largest output in bytes: JPEG and WebP lower their quality, then shrink;
    /// PNG shrinks. Defaults to the configured limit, if any.
    pub max_bytes: Option<NonZeroU64>,
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
    /// The JPEG or WebP quality used, which `max_bytes` can lower; absent for PNG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
}

#[derive(Debug)]
pub struct ProcessingParams {
    pub target_width: u32,
    pub target_height: u32,
    pub strategy: ResizeStrategy,
    pub quality: u8,
    pub input_format: Format,
    pub output_format: Format,
    pub max_pixels: Option<NonZeroU64>,
    pub max_bytes: Option<NonZeroU64>,
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
        max_pixels: meta.max_pixels.or(config.max_pixels),
        max_bytes: meta.max_bytes.or(config.max_bytes),
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

/// Whether `o` swaps width and height.
fn swaps_sides(o: Orientation) -> bool {
    use Orientation::{Rotate90, Rotate90FlipH, Rotate270, Rotate270FlipH};
    matches!(o, Rotate90 | Rotate270 | Rotate90FlipH | Rotate270FlipH)
}

/// An image, oriented, its embedded ICC profile and the source's oriented size.
type Decoded = (DynamicImage, Option<Vec<u8>>, (u32, u32));

/// The image, decoded. `fit` maps an oriented size to the size the caller will shrink the
/// image to, if any; a decoder may then return a smaller image that still covers the whole source.
fn decode(
    bytes: &[u8],
    format: ImageFormat,
    fit: impl Fn(u32, u32) -> Option<(u32, u32)>,
) -> Result<Decoded, Error> {
    let undecodable =
        |e: image::ImageError| invalid(format!("could not decode the {format:?} image: {e}"));
    if format == ImageFormat::Jpeg && jpeg_truncated(bytes) {
        return Err(invalid(
            "the JPEG is truncated: its data ends before the end-of-image marker",
        ));
    }
    let (mut img, orientation, icc, (w, h)) = if format == ImageFormat::WebP {
        let orientation = webp_chunk(bytes, b"EXIF").and_then(Orientation::from_exif_chunk);
        let icc = webp_chunk(bytes, b"ICCP")
            .filter(|icc| icc.len() <= MAX_ICC_BYTES)
            .map(<[u8]>::to_vec);
        let img = decode_webp(bytes)?;
        let size = (img.width(), img.height());
        (img, orientation, icc, size)
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
        // Dropped here when too large, so it is not held through the decode and the resize.
        let icc = decoder
            .icc_profile()
            .ok()
            .flatten()
            .filter(|icc| icc.len() <= MAX_ICC_BYTES);
        // The output size on the stored axes, before orientation.
        let target = if orientation.is_some_and(swaps_sides) {
            fit(h, w).map(|(h, w)| (w, h))
        } else {
            fit(w, h)
        };
        let shrunk = match (format, target) {
            (ImageFormat::Jpeg, Some(target)) => shrunk_jpeg(bytes, (w, h), target),
            _ => None,
        };
        let img = match shrunk {
            Some(img) => img,
            None => DynamicImage::from_decoder(decoder).map_err(undecodable)?,
        };
        (img, orientation, icc, (w, h))
    };
    let orientation = orientation.unwrap_or(Orientation::NoTransforms);
    img.apply_orientation(orientation);
    let size = if swaps_sides(orientation) {
        (h, w)
    } else {
        (w, h)
    };
    Ok((img, icc, size))
}

/// How many times the output size, at least, a JPEG may shrink to while decoding.
const JPEG_SHRINK_MARGIN: u32 = 2;

/// The colour transform byte of the last Adobe APP14 segment before the first scan.
fn adobe_transform(bytes: &[u8]) -> Option<u8> {
    let mut rest = bytes.get(2..)?; // past SOI
    let mut transform = None;
    loop {
        match *rest {
            [0xFF, 0xFF, ..] => rest = &rest[1..], // fill byte
            [0xFF, 0xDA, ..] => return transform,
            [0xFF, marker, hi, lo, ..] => {
                let end = 2 + usize::from(u16::from_be_bytes([hi, lo]));
                let body = rest.get(4..end)?;
                // zune-jpeg's test: "Adobe" and at least the 12 bytes up to the transform.
                if marker == 0xEE && body.len() >= 12 && body.starts_with(b"Adobe") {
                    transform = Some(body[11]);
                }
                rest = &rest[end..];
            }
            _ => return transform,
        }
    }
}

/// A baseline 3-component JPEG of `size` decoded at a half or a quarter of it, the smallest
/// that stays `JPEG_SHRINK_MARGIN` times `target` on both sides; None when none does, or on
/// any error, which the full decode then reports.
fn shrunk_jpeg(bytes: &[u8], (w, h): (u32, u32), (tw, th): (u32, u32)) -> Option<DynamicImage> {
    use jpeg_decoder::{CodingProcess, ColorTransform, Decoder, PixelFormat};
    let m = JPEG_SHRINK_MARGIN;
    // Not an eighth: it keeps only each block's average, a box that blurs the thumbnail.
    // Whole blocks only, so the shrunk image covers the source exactly.
    let d = [4, 2]
        .into_iter()
        .find(|&d| w % d == 0 && h % d == 0 && w / d >= tw * m && h / d >= th * m)?;
    let mut decoder = Decoder::new(bytes);
    decoder.read_info().ok()?;
    let info = decoder.info()?;
    // Progressive scans cost jpeg-decoder as much scaled as zune-jpeg's full decode.
    // shortcut: grey and CMYK JPEGs decode at full size; shrink them too if they become common.
    if info.coding_process != CodingProcess::DctSequential
        || info.pixel_format != PixelFormat::RGB24
    {
        return None;
    }
    // JPEG sides fit in 16 bits.
    let size = decoder.scale((w / d) as u16, (h / d) as u16).ok()?;
    if size != ((w / d) as u16, (h / d) as u16) {
        return None;
    }
    // zune-jpeg, the full decode, takes Adobe transform 0 as RGB whatever the component ids
    // or a JFIF segment say; jpeg-decoder would convert it from YCbCr.
    if adobe_transform(bytes) == Some(0) {
        decoder.set_color_transform(ColorTransform::RGB);
    }
    // shortcut: decode() runs 3 more threads, one per component, that the per-core permit does not
    // count (up to 15% less throughput at saturation); go single-threaded once jpeg-decoder can.
    let px = decoder.decode().ok()?;
    RgbImage::from_raw(w / d, h / d, px).map(DynamicImage::ImageRgb8)
}

/// Largest ICC profile applied; real RGB profiles take a few KiB.
const MAX_ICC_BYTES: usize = 4 << 20;

/// Most tags an ICC profile applied may list; real ones list 10 to 20.
/// moxcms parses every entry, repeats included, so a few KiB of entries
/// naming one large tag would take minutes.
const MAX_ICC_TAGS: u32 = 64;

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
    // The tag count: a big-endian u32 after the 128-byte header.
    let tags = icc
        .get(128..132)
        .and_then(|n| n.try_into().ok())
        .map_or(0, u32::from_be_bytes);
    if tags > MAX_ICC_TAGS {
        return Err(format!("the profile lists {tags} tags"));
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

/// `img` composited over white into opaque RGB.
/// Taken by value, so a source that is not RGBA8 is freed before the RGB output is built.
fn over_white(img: DynamicImage) -> DynamicImage {
    let src = img.into_rgba8();
    let mut px = Vec::with_capacity(src.len() / 4 * 3);
    for &Rgba([r, g, b, a]) in src.pixels() {
        px.extend_from_slice(&match a {
            0 => [255; 3],
            255 => [r, g, b],
            _ => {
                // image's Rgba blend over white, step for step, so colours stay as they were.
                let a = f32::from(a) / 255.0;
                let alpha = 1.0 + a - a;
                [r, g, b].map(|c| (255.0 * ((f32::from(c) / 255.0 * a + (1.0 - a)) / alpha)) as u8)
            }
        });
    }
    DynamicImage::ImageRgb8(RgbImage::from_raw(src.width(), src.height(), px).expect("w*h pixels"))
}

/// Runs `resize` on colour premultiplied by alpha: Lanczos3 mixes channels as
/// stored, so the hidden colour of transparent pixels would bleed into edges.
/// A borrowed image is copied only when some pixel is not opaque.
fn premultiplied(
    img: Cow<'_, DynamicImage>,
    resize: impl FnOnce(&DynamicImage) -> DynamicImage,
) -> DynamicImage {
    if !translucent(&img) {
        return resize(&img);
    }
    let mut img = img.into_owned();
    weigh_by_alpha(&mut img, false);
    let mut out = resize(&img);
    weigh_by_alpha(&mut out, true);
    out
}

/// Whether some pixel is not fully opaque.
fn translucent(img: &DynamicImage) -> bool {
    fn any<S: image::Primitive>(px: &[S], channels: usize) -> bool {
        px.chunks_exact(channels)
            .any(|p| p[channels - 1] != S::DEFAULT_MAX_VALUE)
    }
    match img {
        DynamicImage::ImageLumaA8(b) => any(b, 2),
        DynamicImage::ImageRgba8(b) => any(b, 4),
        DynamicImage::ImageLumaA16(b) => any(b, 2),
        DynamicImage::ImageRgba16(b) => any(b, 4),
        // No alpha, or a float image, which decode never makes.
        _ => false,
    }
}

/// Multiplies colour by alpha in place, or divides it back out with `undo`.
fn weigh_by_alpha(img: &mut DynamicImage, undo: bool) {
    match img {
        DynamicImage::ImageLumaA8(b) => weigh(b, 2, undo),
        DynamicImage::ImageRgba8(b) => weigh(b, 4, undo),
        DynamicImage::ImageLumaA16(b) => weigh(b, 2, undo),
        DynamicImage::ImageRgba16(b) => weigh(b, 4, undo),
        _ => {}
    }
}

fn weigh<S>(px: &mut [S], channels: usize, undo: bool)
where
    S: image::Primitive + Into<u64> + TryFrom<u64>,
{
    let max: u64 = S::DEFAULT_MAX_VALUE.into();
    let alpha = |p: &[S]| -> u64 { p[channels - 1].into() };
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
}

/// `img` resized to exactly `w`x`h` with Lanczos3, in its own pixel type.
fn lanczos3(img: &DynamicImage, w: u32, h: u32) -> DynamicImage {
    use fast_image_resize::pixels::{U8x3, U8x4};
    match img {
        DynamicImage::ImageRgb8(src) => {
            DynamicImage::ImageRgb8(simd_lanczos3::<U8x3, _>(src, w, h))
        }
        DynamicImage::ImageRgba8(src) => {
            DynamicImage::ImageRgba8(simd_lanczos3::<U8x4, _>(src, w, h))
        }
        // shortcut: grey and 16-bit images keep image's slower Lanczos3, as each pixel type
        // fast_image_resize handles grows the binary; move them over if they become common.
        _ => img.resize_exact(w, h, imageops::FilterType::Lanczos3),
    }
}

fn simd_lanczos3<P: fast_image_resize::PixelTrait, Px: image::Pixel<Subpixel = u8>>(
    src: &image::ImageBuffer<Px, Vec<u8>>,
    w: u32,
    h: u32,
) -> image::ImageBuffer<Px, Vec<u8>> {
    use fast_image_resize::images::{TypedImage, TypedImageRef};
    use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
    let mut out = image::ImageBuffer::new(w, h);
    let options = ResizeOptions::new()
        .resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3))
        // premultiplied() has already weighed colour by alpha.
        .use_alpha(false);
    let from = TypedImageRef::<P>::from_buffer(src.width(), src.height(), src.as_raw())
        .expect("the source's own pixels");
    let mut to = TypedImage::<P>::from_buffer(w, h, &mut out).expect("w*h pixels");
    Resizer::new()
        .resize_typed(&from, &mut to, &options)
        .expect("one pixel type and no crop");
    out
}

/// `w`x`h` shrunk to at most `max` pixels, keeping its aspect ratio; never
/// enlarged, and no side below 1.
fn within_pixels(w: u32, h: u32, max: Option<NonZeroU64>) -> (u32, u32) {
    let area = u64::from(w) * u64::from(h);
    let Some(max) = max.map(NonZeroU64::get).filter(|&max| area > max) else {
        return (w, h);
    };
    let scale = (max as f64 / area as f64).sqrt();
    let short = ((f64::from(w.min(h)) * scale) as u32).max(1);
    // Bounded in integers, so float rounding never lands above `max`.
    let bound = u32::try_from(max / u64::from(short)).unwrap_or(u32::MAX);
    let long = ((f64::from(w.max(h)) * scale) as u32).clamp(1, bound);
    if w <= h { (short, long) } else { (long, short) }
}

pub fn resize_scale_to_fit(
    img: DynamicImage,
    max_w: u32,
    max_h: u32,
    max_pixels: Option<NonZeroU64>,
) -> DynamicImage {
    let size = fit_size(img.width(), img.height(), max_w, max_h, max_pixels);
    shrink_to(img, size)
}

/// The scale-to-fit output size of a `w`x`h` image, or None when it already fits.
fn fit_size(
    w: u32,
    h: u32,
    max_w: u32,
    max_h: u32,
    max_pixels: Option<NonZeroU64>,
) -> Option<(u32, u32)> {
    // The pixel limit narrows the box, so the image is resized once.
    let (pw, ph) = within_pixels(w, h, max_pixels);
    let (max_w, max_h) = (max_w.min(pw), max_h.min(ph));
    if w <= max_w && h <= max_h {
        return None;
    }
    // As image's resize: one scale for both sides, rounded, so sizes stay as they were.
    let scale = f64::min(
        f64::from(max_w) / f64::from(w),
        f64::from(max_h) / f64::from(h),
    );
    let fit = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
    Some((fit(w), fit(h)))
}

/// `img` resized to `size`, if any.
fn shrink_to(img: DynamicImage, size: Option<(u32, u32)>) -> DynamicImage {
    match size {
        Some((w, h)) => premultiplied(Cow::Owned(img), |img| lanczos3(img, w, h)),
        None => img,
    }
}

pub fn resize_crop_to_fit(
    img: DynamicImage,
    box_w: u32,
    box_h: u32,
    max_pixels: Option<NonZeroU64>,
) -> DynamicImage {
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
    let (w, h) = within_pixels(cw.min(box_w), ch.min(box_h), max_pixels);
    if (w, h) == (cw, ch) {
        cropped
    } else {
        premultiplied(Cow::Owned(cropped), |img| lanczos3(img, w, h))
    }
}

pub fn process_image(
    bytes: &[u8],
    params: &ProcessingParams,
) -> Result<(Vec<u8>, ThumbnailMetadata), Error> {
    let (box_w, box_h, max_pixels) = (params.target_width, params.target_height, params.max_pixels);
    let fit = |w, h| match params.strategy {
        ResizeStrategy::ScaleToFit => fit_size(w, h, box_w, box_h, max_pixels),
        // shortcut: crop-to-fit decodes at full size, as its crop would land between the
        // pixels of a shrunk decode; move it over if crop-to-fit calls become common.
        ResizeStrategy::CropToFit => None,
    };
    let (mut img, icc, (source_width, source_height)) =
        decode(bytes, params.input_format.image_format(), fit)?;
    // JPEG has no alpha. Flattening before the resize leaves it an opaque
    // image, which skips the premultiplication.
    if params.output_format == Format::Jpeg && img.color().has_alpha() {
        img = over_white(img);
    }

    let thumbnail = match params.strategy {
        // From the source's size, as the decode may have shrunk the image already.
        ResizeStrategy::ScaleToFit => shrink_to(img, fit(source_width, source_height)),
        ResizeStrategy::CropToFit => resize_crop_to_fit(img, box_w, box_h, max_pixels),
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

    let (buf, thumbnail, quality) = encode_within(thumbnail, params)?;
    let result = ThumbnailMetadata {
        format: params.output_format,
        width: thumbnail.width(),
        height: thumbnail.height(),
        source_width,
        source_height,
        strategy: params.strategy,
        quality: (params.output_format != Format::Png).then_some(quality),
    };
    Ok((buf, result))
}

/// Most encodes one call runs to meet `max_bytes`.
const MAX_ENCODES: usize = 8;
/// The JPEG and WebP qualities tried, in turn, before an output over `max_bytes` shrinks.
const QUALITY_STEPS: [u8; 3] = [70, 55, 40];

/// `img` encoded, at lower qualities and then smaller sizes until it fits in
/// `max_bytes`: the bytes, the image they hold and its quality.
fn encode_within(
    img: DynamicImage,
    params: &ProcessingParams,
) -> Result<(Vec<u8>, DynamicImage, u8), Error> {
    let format = params.output_format;
    let mut quality = params.quality;
    let mut buf = encode(&img, format, quality)?;
    let Some(max) = params.max_bytes.map(NonZeroU64::get) else {
        return Ok((buf, img, quality));
    };
    let mut steps = QUALITY_STEPS
        .into_iter()
        .filter(move |&q| format != Format::Png && q < quality);
    let mut shrunk: Option<DynamicImage> = None;
    for _ in 1..MAX_ENCODES {
        let len = buf.len() as u64;
        if len <= max {
            break;
        }
        if let Some(q) = steps.next() {
            quality = q;
        } else {
            let current = shrunk.as_ref().unwrap_or(&img);
            let pixels = f64::from(current.width()) * f64::from(current.height());
            // Bytes track pixels roughly; the margin covers what does not, such as headers.
            let budget = (pixels * max as f64 / len as f64 * 0.9) as u64;
            let (w, h) = within_pixels(img.width(), img.height(), NonZeroU64::new(budget.max(1)));
            if (w, h) == (current.width(), current.height()) {
                break;
            }
            // From the first thumbnail each time, so resizes never stack more than two deep.
            shrunk = Some(premultiplied(Cow::Borrowed(&img), |img| {
                lanczos3(img, w, h)
            }));
        }
        buf = encode(shrunk.as_ref().unwrap_or(&img), format, quality)?;
    }
    let img = shrunk.unwrap_or(img);
    if buf.len() as u64 > max {
        return Err(invalid(format!(
            "the output does not fit in max_bytes ({max}): at {}x{} it is still {} bytes",
            img.width(),
            img.height(),
            buf.len()
        )));
    }
    Ok((buf, img, quality))
}

fn encode(img: &DynamicImage, format: Format, quality: u8) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    let encoded = match format {
        Format::Jpeg => img.write_with_encoder(JpegEncoder::new_with_quality(&mut buf, quality)),
        Format::Png => img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png),
        Format::Webp => return encode_webp(img, quality),
    };
    encoded.map_err(|e| Error::Handler(format!("could not encode the output: {e}")))?;
    Ok(buf)
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

    /// A `decode` fit that asks for no shrink, so the image decodes at full size.
    fn no_fit(_: u32, _: u32) -> Option<(u32, u32)> {
        None
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
    fn limits_default_to_the_config_and_a_call_overrides_them() {
        let config = ResizeConfig {
            max_pixels: NonZeroU64::new(10_000),
            max_bytes: NonZeroU64::new(1_000_000),
            ..default_config()
        };
        let limits = |p: &ProcessingParams| {
            (
                p.max_pixels.map(NonZeroU64::get),
                p.max_bytes.map(NonZeroU64::get),
            )
        };
        let params = resolve_params(&config, &ImageMetadata::default(), Format::Png).unwrap();
        assert_eq!(limits(&params), (Some(10_000), Some(1_000_000)));
        let png = encode_image(&make_rgb_image(400, 300, 1, 2, 3), ImageFormat::Png);
        let (_, result) = process_image(&png, &params).unwrap();
        assert_eq!((result.width, result.height), (115, 86));

        let metadata = ImageMetadata {
            max_pixels: NonZeroU64::new(40_000),
            max_bytes: NonZeroU64::new(5_000),
            ..Default::default()
        };
        let params = resolve_params(&config, &metadata, Format::Png).unwrap();
        assert_eq!(limits(&params), (Some(40_000), Some(5_000)));
        let params = resolve_params(&default_config(), &ImageMetadata::default(), Format::Png);
        assert_eq!(limits(&params.unwrap()), (None, None));
    }

    #[test]
    fn test_scale_to_fit_landscape() {
        let img = make_rgb_image(1000, 500, 255, 0, 0);
        let result = resize_scale_to_fit(img, 200, 200, None);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 100);
    }

    #[test]
    fn test_scale_to_fit_portrait() {
        let img = make_rgb_image(500, 1000, 0, 255, 0);
        let result = resize_scale_to_fit(img, 200, 200, None);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn test_scale_to_fit_square() {
        let img = make_rgb_image(1000, 1000, 0, 0, 255);
        let result = resize_scale_to_fit(img, 200, 200, None);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn test_crop_to_fit_landscape() {
        let img = make_rgb_image(1000, 500, 255, 0, 0);
        let result = resize_crop_to_fit(img, 200, 200, None);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn test_crop_to_fit_portrait() {
        let img = make_rgb_image(500, 1000, 0, 255, 0);
        let result = resize_crop_to_fit(img, 200, 200, None);
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
            max_pixels: None,
            max_bytes: None,
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
            assert_eq!(resize_scale_to_fit(img.clone(), 200, 200, None), img);
        }
    }

    #[test]
    fn crop_to_fit_small_source_crops_without_enlarging() {
        for side in [200, 96] {
            let result = resize_crop_to_fit(make_rgb_image(100, 80, 10, 20, 30), side, side, None);
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
        let result = resize_crop_to_fit(make_rgb_image(20_000, 2, 10, 20, 30), 200, 200, None);
        assert_eq!((result.width(), result.height()), (2, 2));
    }

    #[test]
    fn crop_to_fit_never_exceeds_the_box() {
        let result = resize_crop_to_fit(make_rgb_image(5000, 1009, 10, 20, 30), 100, 1000, None);
        assert_eq!((result.width(), result.height()), (100, 1000));
    }

    #[test]
    fn within_pixels_keeps_the_aspect_under_the_limit() {
        let max = NonZeroU64::new;
        assert_eq!(within_pixels(400, 300, None), (400, 300));
        assert_eq!(within_pixels(400, 300, max(120_000)), (400, 300));
        assert_eq!(within_pixels(400, 300, max(30_000)), (200, 150));
        assert_eq!(within_pixels(400, 300, max(10_000)), (115, 86));
        assert_eq!(within_pixels(20_000, 2, max(100)), (100, 1));
        assert_eq!(within_pixels(2, 20_000, max(100)), (1, 100));
        for (w, h) in [
            (4000, 3000),
            (1, 50_000),
            (12_345, 678),
            (7, 7),
            (u32::MAX, 3),
        ] {
            for limit in [1, 2, 3, 99, 1000, 123_457, 11_999_999] {
                let (nw, nh) = within_pixels(w, h, max(limit));
                let (area, out) = (u64::from(w) * u64::from(h), u64::from(nw) * u64::from(nh));
                let ok = if area > limit {
                    out <= limit
                } else {
                    (nw, nh) == (w, h)
                };
                assert!(
                    ok && (1..=w).contains(&nw) && (1..=h).contains(&nh),
                    "{w}x{h} under {limit}: {nw}x{nh}"
                );
            }
        }
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
        let Err(err) = decode(&jpeg, ImageFormat::Jpeg, no_fit) else {
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
        let img = decode(&png, ImageFormat::Png, no_fit).unwrap().0;
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
            max_pixels: None,
            max_bytes: None,
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
            max_pixels: None,
            max_bytes: None,
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
            max_pixels: None,
            max_bytes: None,
        };
        let (output, _) = process_image(&bytes, &params).unwrap();
        let decoded = decode(&output, ImageFormat::WebP, no_fit).unwrap().0;
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
            max_pixels: None,
            max_bytes: None,
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
            let decoded = decode(&out, ImageFormat::WebP, no_fit).unwrap().0;
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
        let decoded = decode(&out, ImageFormat::WebP, no_fit).unwrap().0;
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
            max_pixels: None,
            max_bytes: None,
        };
        let params_high = ProcessingParams {
            target_width: 200,
            target_height: 200,
            strategy: ResizeStrategy::ScaleToFit,
            quality: 95,
            input_format: Format::Jpeg,
            output_format: Format::Jpeg,
            max_pixels: None,
            max_bytes: None,
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

    /// A `w`x600 gradient stored as a baseline JPEG that rotates to 600x`w`.
    fn rotated_gradient_jpeg(w: u32) -> Vec<u8> {
        // Each channel ramps along its own axis, so a stretch, shift or swapped side shows.
        let src = RgbImage::from_fn(w, 600, |x, y| {
            image::Rgb([(x / 4) as u8, (y / 3) as u8, ((x + y) / 6) as u8])
        });
        let mut jpeg = Vec::new();
        let mut enc = JpegEncoder::new_with_quality(&mut jpeg, 90);
        enc.set_exif_metadata(tiff_orientation(6)).unwrap();
        src.write_with_encoder(enc).unwrap();
        jpeg
    }

    #[test]
    fn jpeg_shrinks_while_decoding_to_the_same_thumbnail() {
        assert_shrinks_to_the_full_decode_thumbnail(&rotated_gradient_jpeg(800));
    }

    #[test]
    fn jpeg_with_adobe_transform_0_shrinks_as_rgb_like_the_full_decode() {
        // An APP14 Adobe segment, transform 0 (RGB), beside the encoder's JFIF and ids 1, 2, 3.
        let mut jpeg = rotated_gradient_jpeg(800);
        let app14 = [
            0xFF, 0xEE, 0, 14, b'A', b'd', b'o', b'b', b'e', 0, 100, 0, 0, 0, 0, 0,
        ];
        jpeg.splice(2..2, app14);
        assert_eq!(adobe_transform(&jpeg), Some(0));
        assert_shrinks_to_the_full_decode_thumbnail(&jpeg);
    }

    /// The 600x800 `jpeg` shrinks to a quarter for a 100x100 box, to the thumbnail of its full decode.
    fn assert_shrinks_to_the_full_decode_thumbnail(jpeg: &[u8]) {
        let fit = |w, h| fit_size(w, h, 100, 100, None);
        let (img, _, size) = decode(jpeg, ImageFormat::Jpeg, fit).unwrap();
        assert_eq!(size, (600, 800));
        // A quarter, twice the 75x100 output, on the rotated axes.
        assert_eq!((img.width(), img.height()), (150, 200));

        let params = ProcessingParams {
            output_format: Format::Png,
            ..webp_params(100, 100, 100, Format::Jpeg)
        };
        let (out, meta) = process_image(jpeg, &params).unwrap();
        assert_eq!((meta.source_width, meta.source_height), (600, 800));
        assert_eq!((meta.width, meta.height), (75, 100));
        let out = image::load_from_memory_with_format(&out, ImageFormat::Png)
            .unwrap()
            .to_rgb8();
        let full = decode(jpeg, ImageFormat::Jpeg, no_fit).unwrap().0;
        let expected = resize_scale_to_fit(full, 100, 100, None).to_rgb8();
        let diff = out
            .as_raw()
            .iter()
            .zip(expected.as_raw())
            .map(|(a, b)| a.abs_diff(*b))
            .max();
        assert!(diff <= Some(2), "max diff {diff:?}");
    }

    #[test]
    fn jpeg_shrinks_only_by_whole_blocks_within_the_margin() {
        let shrunk = |jpeg: &[u8], side| {
            let fit = |w, h| fit_size(w, h, side, side, None);
            let img = decode(jpeg, ImageFormat::Jpeg, fit).unwrap().0;
            (img.width(), img.height())
        };
        let jpeg = rotated_gradient_jpeg(800);
        // A 76x101 output takes a half; a 151x201 one keeps the full size.
        assert_eq!(shrunk(&jpeg, 101), (300, 400));
        assert_eq!(shrunk(&jpeg, 201), (600, 800));
        // 802 takes a half instead of a quarter, and 801 nothing.
        assert_eq!(shrunk(&rotated_gradient_jpeg(802), 100), (300, 401));
        assert_eq!(shrunk(&rotated_gradient_jpeg(801), 100), (600, 801));
        let mut grey = Vec::new();
        DynamicImage::new_luma8(800, 600)
            .write_with_encoder(JpegEncoder::new_with_quality(&mut grey, 85))
            .unwrap();
        assert_eq!(shrunk(&grey, 50), (800, 600));
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
        let Err(err) = decode(&webp, ImageFormat::WebP, no_fit) else {
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
            let Err(err) = decode(&webp, ImageFormat::WebP, no_fit) else {
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
        let img = decode(&webp, ImageFormat::WebP, no_fit).unwrap().0;
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
        // Red hidden beside opaque green, and white hidden beside opaque black.
        let rgba8 = DynamicImage::ImageRgba8(RgbaImage::from_fn(40, 20, |x, _| {
            image::Rgba(if x < 20 {
                [255, 0, 0, 0]
            } else {
                [0, 255, 0, 255]
            })
        }));
        let gray8 = DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_fn(40, 20, |x, _| {
            image::LumaA(if x < 20 { [255, 0] } else { [0, 255] })
        }));
        for img in [
            rgba8.clone(),
            DynamicImage::ImageRgba16(rgba8.to_rgba16()),
            gray8.clone(),
            DynamicImage::ImageLumaA16(gray8.to_luma_alpha16()),
        ] {
            let color = img.color();
            for out in [
                resize_scale_to_fit(img.clone(), 10, 10, None),
                resize_crop_to_fit(img.clone(), 10, 10, None),
            ] {
                assert_eq!(out.color(), color);
                for (x, y, p) in out.to_rgba8().enumerate_pixels() {
                    assert!(p[3] == 0 || p[0] <= 2, "{color:?} ({x}, {y}) is {:?}", p.0);
                }
            }
        }
    }

    #[test]
    fn max_bytes_shrinks_without_bleeding_hidden_colour() {
        let mut seed: u32 = 5;
        let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(300, 300, |x, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            image::Rgba(if x < 150 {
                [255, 0, 0, 0]
            } else {
                [0, 128 | (seed >> 24) as u8, 0, 255]
            })
        }));
        let params = ProcessingParams {
            output_format: Format::Png,
            max_bytes: NonZeroU64::new(10_000),
            ..webp_params(300, 300, 85, Format::Png)
        };
        let (_, out, _) = encode_within(img, &params).unwrap();
        assert!(out.width() < 300, "no shrink");
        for (x, y, p) in out.to_rgba8().enumerate_pixels() {
            assert!(p[3] == 0 || p[0] <= 2, "({x}, {y}) is {:?}", p.0);
        }
    }

    #[test]
    fn max_bytes_shrinks_an_opaque_thumbnail_without_copying_it() {
        let mut seed: u32 = 3;
        let noise = DynamicImage::ImageRgb8(RgbImage::from_fn(1000, 1000, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let [_, r, g, b] = seed.to_le_bytes();
            image::Rgb([r, g, b])
        }));
        let params = ProcessingParams {
            output_format: Format::Jpeg,
            max_bytes: NonZeroU64::new(5_000),
            ..webp_params(1000, 1000, 85, Format::Jpeg)
        };
        for img in [DynamicImage::ImageRgba8(noise.to_rgba8()), noise] {
            let (color, copy) = (img.color(), img.as_bytes().len());
            largest_alloc::take();
            let (_, out, _) = encode_within(img, &params).unwrap();
            let largest = largest_alloc::take();
            assert!(out.width() < 1000, "no shrink");
            assert!(
                largest < copy,
                "{color:?}: allocated {largest} bytes, a copy"
            );
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
            resize_scale_to_fit(img.clone(), 20, 20, None),
            lanczos3(&img, 20, 15)
        );
    }

    #[test]
    fn lanczos3_is_within_a_level_or_two_of_image_s() {
        let mut seed: u32 = 7;
        let noise = DynamicImage::ImageRgba8(RgbaImage::from_fn(64, 48, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            image::Rgba(seed.to_le_bytes())
        }));
        // On this noise CatmullRom is 9 levels off and Triangle 21; the fixed-point Lanczos3 is 1.
        for img in [DynamicImage::ImageRgb8(noise.to_rgb8()), noise] {
            let theirs = img.resize_exact(20, 15, imageops::FilterType::Lanczos3);
            let ours = lanczos3(&img, 20, 15);
            assert_eq!(ours.color(), img.color());
            let worst = ours
                .as_bytes()
                .iter()
                .zip(theirs.as_bytes())
                .map(|(a, b)| a.abs_diff(*b))
                .max();
            assert!(worst <= Some(2), "{:?}: {worst:?}", img.color());
        }
    }

    #[test]
    fn alpha_weighting_rounds_and_zeroes_transparent_colour() {
        let rgba = |px: Vec<u8>| DynamicImage::ImageRgba8(RgbaImage::from_raw(3, 1, px).unwrap());
        let mut img = rgba(vec![255, 128, 1, 128, 200, 100, 50, 0, 10, 20, 30, 255]);
        assert!(translucent(&img));
        weigh_by_alpha(&mut img, false);
        assert_eq!(
            img.as_bytes(),
            [128, 64, 1, 128, 0, 0, 0, 0, 10, 20, 30, 255]
        );

        let mut img = rgba(vec![128, 64, 1, 128, 5, 5, 5, 0, 200, 0, 0, 100]);
        weigh_by_alpha(&mut img, true);
        assert_eq!(
            img.as_bytes(),
            [255, 128, 2, 128, 0, 0, 0, 0, 255, 0, 0, 100]
        );

        let opaque = rgba(vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255]);
        assert!(!translucent(&opaque));
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
    fn flattening_matches_image_s_blend_over_white() {
        // Every alpha against a spread of colours.
        let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(256, 256, |c, a| {
            Rgba([c as u8, 255 - c as u8, (c * 7) as u8, a as u8])
        }));
        let mut expected = RgbaImage::from_pixel(256, 256, Rgba([255; 4]));
        imageops::overlay(&mut expected, &img, 0, 0);
        let out = over_white(img);
        assert_eq!(out.color(), image::ColorType::Rgb8);
        assert_eq!(
            out,
            DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(expected).to_rgb8())
        );
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
            max_pixels: None,
            max_bytes: None,
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
            max_pixels: None,
            max_bytes: None,
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
        let (raw, icc, _) = decode(tagged, input_format.image_format(), no_fit).unwrap();
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

    /// `icc` with its first tag entry repeated until it lists `count` tags.
    fn with_tags(icc: &[u8], count: u32) -> Vec<u8> {
        let listed = u32::from_be_bytes(icc[128..132].try_into().unwrap());
        let table_end = 132 + 12 * listed as usize;
        let grown = 12 * (count - listed);
        let entries: Vec<u8> = icc[132..table_end]
            .chunks(12)
            .chain(std::iter::repeat_n(
                &icc[132..144],
                (count - listed) as usize,
            ))
            .flat_map(|entry| {
                let offset = u32::from_be_bytes(entry[4..8].try_into().unwrap()) + grown;
                [&entry[..4], &offset.to_be_bytes(), &entry[8..]].concat()
            })
            .collect();
        let mut out = [
            &icc[..128],
            &count.to_be_bytes(),
            &entries,
            &icc[table_end..],
        ]
        .concat();
        let size = u32::try_from(out.len()).unwrap();
        out[..4].copy_from_slice(&size.to_be_bytes());
        out
    }

    #[test]
    fn a_profile_listing_too_many_tags_is_not_parsed() {
        let mut img = make_rgb_image(4, 4, 200, 100, 50);
        assert_eq!(
            convert_to_srgb(&mut img, &with_tags(&display_p3(), 64)),
            Ok(())
        );
        assert_eq!(
            convert_to_srgb(&mut img, &with_tags(&display_p3(), 2000)),
            Err("the profile lists 2000 tags".into())
        );
    }

    #[test]
    fn oversized_profiles_are_dropped_as_they_are_read() {
        let oversized = [display_p3(), vec![0; MAX_ICC_BYTES]].concat();
        let img = make_rgb_image(16, 16, 200, 100, 50);
        let png = png_with_icc(&img, oversized.clone());
        assert_eq!(
            decode(&png, ImageFormat::Png, no_fit)
                .unwrap()
                .1
                .map(|p| p.len()),
            None
        );
        let still = encode_webp(&img, 90).unwrap();
        let webp = riff(&[
            vp8x(0x20, 16, 16),
            chunk(b"ICCP", &oversized),
            still[12..].to_vec(),
        ]);
        assert_eq!(
            decode(&webp, ImageFormat::WebP, no_fit)
                .unwrap()
                .1
                .map(|p| p.len()),
            None
        );
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
