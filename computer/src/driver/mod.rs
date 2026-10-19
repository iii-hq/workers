//! The environment driver: one desktop the worker drives. The [`Driver`]
//! trait keeps [`crate::session::Session`] driver-agnostic; four
//! implementations slot in behind it:
//!
//! - [`RemoteClient`] speaks the guest-executor wire (`{command, params}`
//!   over a WebSocket) to a remote or sandboxed desktop.
//! - [`NativeHost`] drives the local machine this worker runs on (macOS /
//!   Windows / Linux X11), capturing and injecting input directly.
//! - [`IiiSandboxHost`] boots a desktop inside an iii-sandbox microVM and
//!   drives it through iii primitives (`sandbox::exec` / `sandbox::fs`), with
//!   no executor and no socket into the guest.
//! - [`SemanticOnly`] is a native session whose desktop gives this worker no
//!   screen or input (a Wayland compositor): every coordinate call is refused
//!   with the reason, and only the accessibility layer works.

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub mod native;
pub mod remote;
pub mod sandbox;
pub mod semantic_only;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
pub use native::NativeHost;
pub use remote::RemoteClient;
pub use sandbox::IiiSandboxHost;
pub use semantic_only::SemanticOnly;

/// Desktop pixel dimensions. Coordinates handed to pointer actions are in this
/// space: integer pixels, top-left origin, 1:1 with the screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
}

/// Downscale target: longest edge capped at `max_dim`, aspect preserved.
pub(crate) fn target_dims(w: u32, h: u32, max_dim: u32) -> (u32, u32) {
    let longest = w.max(h);
    if longest <= max_dim || longest == 0 {
        return (w, h);
    }
    let s = max_dim as f64 / longest as f64;
    (
        ((w as f64 * s).round() as u32).max(1),
        ((h as f64 * s).round() as u32).max(1),
    )
}

/// Downscale to `(tw, th)` and JPEG-encode.
pub(crate) fn encode_jpeg(
    img: image::DynamicImage,
    tw: u32,
    th: u32,
    quality: u8,
) -> Result<Vec<u8>, String> {
    let img = if (tw, th) == (img.width(), img.height()) {
        img
    } else {
        img.resize_exact(tw, th, image::imageops::FilterType::Triangle)
    };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut std::io::Cursor::new(&mut out),
        quality,
    )
    .encode_image(&img.to_rgb8())
    .map_err(|e| format!("jpeg encode failed: {e}"))?;
    Ok(out)
}

/// One display available to the native host (from `computer::displays`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DisplayInfo {
    /// Index to pass as `monitor` to `computer::sessions::start`.
    pub index: u32,
    pub name: String,
    pub primary: bool,
    pub builtin: bool,
    /// Width: points on macOS, pixels on Windows and Linux.
    pub width: u32,
    /// Height, in the same unit.
    pub height: u32,
}

/// An encoded screenshot plus the detected mime of its bytes. The driver
/// returns whatever its guest encodes (png by default); the mime is read from
/// the magic bytes rather than assumed, so the content block advertises the
/// truth to the model.
#[derive(Debug, Clone)]
pub struct Shot {
    pub bytes: Vec<u8>,
    pub mime: String,
}

impl Shot {
    /// Detect `image/png` / `image/jpeg` from the leading magic bytes, falling
    /// back to `application/octet-stream` for anything unrecognized.
    pub fn new(bytes: Vec<u8>) -> Self {
        let mime = detect_mime(&bytes).to_string();
        Self { bytes, mime }
    }

    /// Base64 of the encoded image bytes.
    pub fn to_base64(&self) -> String {
        STANDARD.encode(&self.bytes)
    }

    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
}

pub fn detect_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else {
        "application/octet-stream"
    }
}

/// A live computer-use environment. Every method maps a desktop semantic onto
/// the concrete driver wire. Errors are human-readable strings surfaced back
/// through the function envelope.
#[async_trait]
pub trait Driver: Send + Sync {
    /// Current desktop dimensions.
    async fn screen_size(&self) -> Result<Screen, String>;
    /// Capture the desktop; bytes carry their own encoding (see [`Shot`]).
    async fn screenshot(&self) -> Result<Shot, String>;
    async fn left_click(&self, x: i64, y: i64) -> Result<(), String>;
    async fn right_click(&self, x: i64, y: i64) -> Result<(), String>;
    async fn double_click(&self, x: i64, y: i64) -> Result<(), String>;
    async fn move_cursor(&self, x: i64, y: i64) -> Result<(), String>;
    async fn scroll(&self, x: i64, y: i64, scroll_x: i64, scroll_y: i64) -> Result<(), String>;
    async fn drag(&self, from: (i64, i64), to: (i64, i64), button: &str) -> Result<(), String>;
    async fn type_text(&self, text: &str) -> Result<(), String>;
    /// Press a chord: a single key (`["enter"]`) or a combination
    /// (`["ctrl", "c"]`).
    async fn keypress(&self, keys: &[String]) -> Result<(), String>;
    /// Close the underlying connection. Idempotent: closing an
    /// already-closed driver succeeds.
    async fn close(&self) -> Result<(), String>;

    /// Why this session takes no coordinates (no screenshots, pointer or
    /// keyboard), or `None` when it does. Only [`SemanticOnly`] says why.
    fn coordinates(&self) -> Option<&str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_dims_caps_longest_edge() {
        assert_eq!(target_dims(3024, 1964, 1280), (1280, 831));
        assert_eq!(target_dims(800, 600, 1280), (800, 600));
        assert_eq!(target_dims(0, 0, 1280), (0, 0));
    }

    #[test]
    fn encode_downscales_to_a_jpeg() {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::new(64, 32));
        let jpeg = encode_jpeg(img, 32, 16, 80).unwrap();
        assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF]);
        let back = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((back.width(), back.height()), (32, 16));
    }

    #[test]
    fn detects_png_and_jpeg() {
        assert_eq!(detect_mime(&[0x89, b'P', b'N', b'G', 0x0d]), "image/png");
        assert_eq!(detect_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(detect_mime(&[0x00, 0x01]), "application/octet-stream");
        assert_eq!(detect_mime(&[]), "application/octet-stream");
    }

    #[test]
    fn shot_carries_detected_mime() {
        let shot = Shot::new(vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a]);
        assert_eq!(shot.mime, "image/png");
    }
}
