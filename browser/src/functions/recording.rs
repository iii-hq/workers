//! `browser::recording::start` / `stop` — capture a session's live viewport
//! to a video file by piping the screencast JPEG frames through ffmpeg.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordingStartInput {
    pub session_id: String,
    /// Output file path. The extension should match `format`.
    pub path: String,
    /// `webm` (VP9) or `mp4` (H.264). Defaults to webm.
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RecordingStartOutput {
    pub ok: bool,
    pub path: String,
    pub format: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordingStopInput {
    pub session_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RecordingStopOutput {
    /// False when no recording was running.
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Wall-clock duration captured, milliseconds.
    pub duration_ms: i64,
    /// Frames written to the encoder.
    pub frames: u64,
}

/// Resolve a requested format string to (canonical name, ffmpeg codec).
/// Unknown formats fail closed so a typo does not silently pick a default.
pub fn resolve_format(format: Option<&str>) -> Result<(&'static str, &'static str), String> {
    match format.unwrap_or("webm").to_ascii_lowercase().as_str() {
        "webm" => Ok(("webm", "libvpx-vp9")),
        "mp4" => Ok(("mp4", "libx264")),
        other => Err(format!("unknown format '{other}' (use webm or mp4)")),
    }
}

/// ffmpeg args to read a stream of JPEG frames on stdin and encode to
/// `path`. `-framerate` is the assumed input rate (the pump caps at ~15fps);
/// `yuv420p` keeps the result playable in browsers and QuickTime.
pub fn ffmpeg_args(codec: &str, path: &str) -> Vec<String> {
    vec![
        "-y".into(),
        "-f".into(),
        "image2pipe".into(),
        "-framerate".into(),
        "15".into(),
        "-i".into(),
        "-".into(),
        "-c:v".into(),
        codec.into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        path.into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_resolution() {
        assert_eq!(resolve_format(None).unwrap(), ("webm", "libvpx-vp9"));
        assert_eq!(resolve_format(Some("MP4")).unwrap(), ("mp4", "libx264"));
        assert!(resolve_format(Some("gif")).is_err());
    }

    #[test]
    fn ffmpeg_args_read_stdin_pipe() {
        let args = ffmpeg_args("libx264", "/tmp/out.mp4");
        assert!(args.contains(&"image2pipe".to_string()));
        assert_eq!(args.last().unwrap(), "/tmp/out.mp4");
        assert!(args.windows(2).any(|w| w == ["-c:v", "libx264"]));
    }
}
