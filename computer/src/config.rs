//! Config for the computer worker. `max_sessions` is a hard cap. The screencast
//! rate is read per pump tick, so it hot-reloads; the action and connect
//! timeouts are read when a session connects, and `default_endpoint` and `os`
//! are read at session start, so a change to any of those applies to sessions
//! started after it. Running sessions keep the driver they were launched with.

use std::sync::Arc;

use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type SharedConfig = Arc<ArcSwap<WorkerConfig>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WorkerConfig {
    /// Guest-executor endpoint new sessions connect to when
    /// `sessions::start` omits one. Accepts `ws://host:port`,
    /// `http://host:port`, or a bare `host:port` (defaults to ws, `/ws`
    /// appended). Empty means every `sessions::start` must pass its own
    /// `endpoint`.
    pub default_endpoint: String,
    /// Informational OS label recorded on started sessions and surfaced in
    /// `session-started` events (`linux`, `macos`, `windows`, `android`).
    /// Does not change the driver; it labels which guest you connected to.
    pub os: String,
    /// Maximum concurrently running sessions; `sessions::start` beyond this
    /// fails until one stops.
    pub max_sessions: u64,
    /// Stop sessions idle longer than this (ms). 0 disables the sweep.
    pub idle_stop_ms: u64,
    /// Live-view frame rate cap for the screencast (frames/sec). Clamped to
    /// at least 1. The pump polls the driver screenshot at most this often.
    pub screencast_fps: u64,
    /// Longest edge (px) a native screenshot/frame is downscaled to before
    /// JPEG encoding. Full-resolution Retina captures are huge; this caps the
    /// image the model sees and the coordinate space it acts in.
    pub max_screenshot_dimension: u64,
    /// JPEG quality (1-100) for native screenshots and frames.
    pub screenshot_quality: u64,
    /// Timeout for each driver action (ms). Fixed when a session connects.
    pub command_timeout_ms: u64,
    /// Timeout for establishing the driver connection at session start (ms).
    pub connect_timeout_ms: u64,
    /// OCI image (an iii-sandbox preset name or `custom_images` key) booted for
    /// a sandbox-backed session when `sessions::start` omits `image`. Empty
    /// means a sandbox session must name its own image. The image must ship
    /// Xvfb, xdotool, imagemagick, and openbox (see images/desktop).
    pub sandbox_image: String,
    /// Virtual display width (px) for a sandbox-backed session. Fixed
    /// resolution keeps coordinates 1:1 with the screenshot.
    pub sandbox_width: u64,
    /// Virtual display height (px) for a sandbox-backed session.
    pub sandbox_height: u64,
    /// Give a sandbox-backed desktop network access. On by default: a desktop
    /// nobody can browse from is not much of a desktop. Turn it off for a model
    /// that should be able to click around a sandbox without reaching anything
    /// outside it, since `computer::act` is unrestricted once a session exists.
    pub sandbox_network: bool,
    /// Idle timeout (seconds) passed to `sandbox::create`. Set well above the
    /// worker's own `idle_stop_ms` so the sandbox reaper never kills a live
    /// desktop out from under a session; the worker owns teardown.
    pub sandbox_idle_timeout_secs: u64,
    /// Ask macOS for Screen Recording at native session start and fail loudly if
    /// it is not granted (macOS otherwise returns a wallpaper-only capture with
    /// every window stripped, silently). Set false if capture works in your
    /// setup but the TCC API under-reports (e.g. the worker runs as a child of
    /// an already-granted app). Ignored off macOS and for non-native drivers.
    pub screen_capture_preflight: bool,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            default_endpoint: String::new(),
            os: "linux".to_string(),
            max_sessions: 2,
            idle_stop_ms: 300_000,
            screencast_fps: 15,
            max_screenshot_dimension: 1280,
            screenshot_quality: 70,
            command_timeout_ms: 120_000,
            connect_timeout_ms: 15_000,
            sandbox_image: String::new(),
            sandbox_width: 1280,
            sandbox_height: 800,
            sandbox_network: true,
            sandbox_idle_timeout_secs: 86_400,
            screen_capture_preflight: true,
        }
    }
}

impl WorkerConfig {
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(WorkerConfig))
            .expect("WorkerConfig schema serializes")
    }

    /// Parse from a JSON object; missing keys fall back to defaults
    /// (`#[serde(default)]`). The configuration worker may store the value
    /// under a `computer` wrapper or flat: accept both.
    pub fn from_json(v: &serde_json::Value) -> Result<WorkerConfig, String> {
        let inner = v.get("computer").unwrap_or(v);
        serde_json::from_value(inner.clone()).map_err(|e| format!("invalid computer config: {e}"))
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    pub fn into_shared(self) -> SharedConfig {
        Arc::new(ArcSwap::from_pointee(self))
    }

    /// Minimum wall-clock interval between screencast frames (ms), derived
    /// from `screencast_fps` (floored at 1 fps so the divisor is never zero).
    pub fn screencast_interval_ms(&self) -> u64 {
        1_000 / self.screencast_fps.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = WorkerConfig::default();
        assert_eq!(c.default_endpoint, "");
        assert_eq!(c.os, "linux");
        assert_eq!(c.max_sessions, 2);
        assert_eq!(c.idle_stop_ms, 300_000);
        assert_eq!(c.screencast_fps, 15);
        assert_eq!(c.max_screenshot_dimension, 1280);
        assert_eq!(c.screenshot_quality, 70);
        assert_eq!(c.command_timeout_ms, 120_000);
        assert_eq!(c.connect_timeout_ms, 15_000);
        assert_eq!(c.sandbox_image, "");
        assert_eq!(c.sandbox_width, 1280);
        assert_eq!(c.sandbox_height, 800);
        assert!(c.sandbox_network);
        assert_eq!(c.sandbox_idle_timeout_secs, 86_400);
        assert!(c.screen_capture_preflight);
    }

    #[test]
    fn json_roundtrip() {
        let c = WorkerConfig {
            os: "macos".to_string(),
            max_sessions: 1,
            ..WorkerConfig::default()
        };
        let back = WorkerConfig::from_json(&c.to_json()).unwrap();
        assert_eq!(back.os, "macos");
        assert_eq!(back.max_sessions, 1);
    }

    #[test]
    fn from_json_fills_missing_with_defaults() {
        let v = serde_json::json!({ "max_sessions": 4 });
        let c = WorkerConfig::from_json(&v).unwrap();
        assert_eq!(c.max_sessions, 4);
        assert_eq!(c.screencast_fps, 15);
    }

    #[test]
    fn from_json_accepts_wrapped_value() {
        let v = serde_json::json!({ "computer": { "os": "windows" } });
        let c = WorkerConfig::from_json(&v).unwrap();
        assert_eq!(c.os, "windows");
    }

    #[test]
    fn screencast_interval_floors_fps_at_one() {
        let mut c = WorkerConfig::default();
        assert_eq!(c.screencast_interval_ms(), 66);
        c.screencast_fps = 0;
        assert_eq!(c.screencast_interval_ms(), 1_000);
    }

    #[test]
    fn schema_lists_fields() {
        let s = WorkerConfig::json_schema();
        let props = &s["properties"];
        assert!(props.get("default_endpoint").is_some());
        assert!(props.get("os").is_some());
        assert!(props.get("screencast_fps").is_some());
    }
}
