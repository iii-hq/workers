//! The environment backend: one connected desktop the worker drives. v1 ships
//! a single implementation, [`ComputerServerClient`], which speaks the Cua
//! computer-server wire (`{command, params}` over a WebSocket). The [`Backend`]
//! trait keeps [`crate::session::Session`] backend-agnostic so a
//! host-background driver (out-of-process, a different integration shape) can
//! slot in behind the same surface later without touching session or function
//! code.

pub mod computer_server;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use computer_server::ComputerServerClient;

/// Desktop pixel dimensions. Coordinates handed to pointer actions are in this
/// space: integer pixels, top-left origin, 1:1 with the screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
}

/// Result of a shell command run inside the desktop.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
}

/// An encoded screenshot plus the detected mime of its bytes. The backend
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
/// the concrete backend wire. Errors are human-readable strings surfaced back
/// through the function envelope.
#[async_trait]
pub trait Backend: Send + Sync {
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
    async fn run_command(&self, command: &str) -> Result<CommandOutput, String>;
    async fn read_text(&self, path: &str) -> Result<String, String>;
    async fn write_text(&self, path: &str, content: &str) -> Result<(), String>;
    /// Best-effort accessibility tree; `null` when the guest does not expose a
    /// real one (Linux/Windows guests return a stub or nothing).
    async fn accessibility_tree(&self) -> Result<serde_json::Value, String>;
    /// Close the underlying connection. Idempotent: closing an
    /// already-closed backend succeeds.
    async fn close(&self) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

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
