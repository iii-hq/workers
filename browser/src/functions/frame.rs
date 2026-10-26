//! `browser::screencast::start` / `stop` / `browser::frame` — the live-view
//! pipeline behind the console viewport. Chromium pushes encoded frames
//! continuously (`Page.startScreencast`); the worker fans each one out on the
//! `browser::frame-event` trigger (bind with a `session_id` filter) and keeps
//! the newest in memory, which `browser::frame` hands out for a viewer's
//! first paint. All three are internal console-UI plumbing, not agent
//! surface — agents read pages with `browser::snapshot` and
//! `browser::screenshot`.
//!
//! A viewer that names itself (`viewer_id`) holds the screencast through a
//! lease instead of a bare count: repeating the start renews it, and a
//! viewer that disappears without a stop (an app killed or sent to the
//! background) is dropped once the lease lapses, so it cannot keep the tab
//! awake and streaming forever.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Longest `viewer_id` accepted.
pub const MAX_VIEWER_ID_LEN: usize = 128;
/// Lease a named viewer gets when it passes no `lease_ms`, and the bounds a
/// requested one is held to.
pub const DEFAULT_LEASE_MS: u64 = 20_000;
pub const MIN_LEASE_MS: u64 = 5_000;
pub const MAX_LEASE_MS: u64 = 120_000;
/// Bounds of a viewer's `scale`.
pub const MIN_SCALE: f64 = 0.5;
pub const MAX_SCALE: f64 = 3.0;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreencastStartInput {
    pub session_id: String,
    /// A corner thumbnail rather than a pane: streams frames but does not
    /// count as a viewer for `browser::resize` fit arbitration.
    #[serde(default)]
    pub preview: Option<bool>,
    /// Names this viewer (up to 128 characters) and makes the start
    /// idempotent: the first call with an id joins the screencast, every
    /// later one only renews its lease (and `scale`). Without it the start
    /// counts one more anonymous viewer, released by one stop.
    #[serde(default)]
    pub viewer_id: Option<String>,
    /// How long the viewer stays without another start (ms, with
    /// `viewer_id`; default 20000, held to 5000..120000). Renew well inside
    /// it; a lapsed viewer is dropped within about two seconds.
    #[serde(default)]
    pub lease_ms: Option<u64>,
    /// Frame pixels this viewer wants per CSS pixel (with `viewer_id`;
    /// 0.5..3, default 1). The stream runs at the largest scale among the
    /// named viewers, never above the `frame_density` setting; the frame's
    /// `width`/`height` stay the CSS viewport either way.
    #[serde(default)]
    pub scale: Option<f64>,
}

/// A named viewer's validated lease request.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerLease {
    pub viewer_id: String,
    pub lease_ms: u64,
    pub scale: f64,
}

impl ScreencastStartInput {
    /// The lease this start asks for: None for an anonymous viewer, an
    /// error for an unusable `viewer_id`.
    pub fn lease(&self) -> Result<Option<ViewerLease>, String> {
        let Some(viewer_id) = self.viewer_id.as_deref() else {
            return Ok(None);
        };
        let viewer_id = viewer_id.trim();
        if viewer_id.is_empty() {
            return Err("viewer_id must not be empty".to_string());
        }
        if viewer_id.chars().count() > MAX_VIEWER_ID_LEN {
            return Err(format!(
                "viewer_id is longer than {MAX_VIEWER_ID_LEN} characters"
            ));
        }
        let lease_ms = self
            .lease_ms
            .unwrap_or(DEFAULT_LEASE_MS)
            .clamp(MIN_LEASE_MS, MAX_LEASE_MS);
        let scale = match self.scale {
            Some(scale) if scale.is_finite() => scale.clamp(MIN_SCALE, MAX_SCALE),
            _ => 1.0,
        };
        Ok(Some(ViewerLease {
            viewer_id: viewer_id.to_string(),
            lease_ms,
            scale,
        }))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreencastStopInput {
    /// Stopping the screencast on an unknown session succeeds.
    pub session_id: String,
    /// Must match the `preview` the start was made with (anonymous viewers
    /// only; a named viewer remembers its own).
    #[serde(default)]
    pub preview: Option<bool>,
    /// The viewer to release, as passed to `browser::screencast::start`.
    /// An unknown or already-lapsed id succeeds and releases nothing.
    #[serde(default)]
    pub viewer_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FrameInput {
    pub session_id: String,
    /// Frame cursor from the previous read; the response omits `frame` while
    /// the newest frame still has this seq.
    #[serde(default)]
    pub since_frame: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FrameOutput {
    /// Base64 JPEG of the newest frame; absent when `since_frame` is still
    /// current or no frame has arrived yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    /// Page-viewport width the frame maps to (input coordinate space).
    pub width: u32,
    /// Page-viewport height the frame maps to.
    pub height: u32,
    pub frame_seq: u64,
    pub timestamp: i64,
    /// False when no screencast is running (call screencast::start first).
    pub active: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(
        viewer_id: Option<&str>,
        lease_ms: Option<u64>,
        scale: Option<f64>,
    ) -> ScreencastStartInput {
        ScreencastStartInput {
            session_id: "b1".to_string(),
            preview: None,
            viewer_id: viewer_id.map(str::to_string),
            lease_ms,
            scale,
        }
    }

    #[test]
    fn an_anonymous_start_has_no_lease() {
        assert_eq!(start(None, Some(9_000), Some(2.0)).lease(), Ok(None));
    }

    #[test]
    fn a_named_start_defaults_and_clamps_its_lease_and_scale() {
        let lease = start(Some("android-1-7-1"), None, None)
            .lease()
            .unwrap()
            .unwrap();
        assert_eq!(lease.viewer_id, "android-1-7-1");
        assert_eq!(lease.lease_ms, DEFAULT_LEASE_MS);
        assert_eq!(lease.scale, 1.0);

        let lease = start(Some("v"), Some(1), Some(0.1))
            .lease()
            .unwrap()
            .unwrap();
        assert_eq!((lease.lease_ms, lease.scale), (MIN_LEASE_MS, MIN_SCALE));
        let lease = start(Some("v"), Some(u64::MAX), Some(9.0))
            .lease()
            .unwrap()
            .unwrap();
        assert_eq!((lease.lease_ms, lease.scale), (MAX_LEASE_MS, MAX_SCALE));
        let lease = start(Some("v"), Some(30_000), Some(f64::NAN))
            .lease()
            .unwrap()
            .unwrap();
        assert_eq!((lease.lease_ms, lease.scale), (30_000, 1.0));
    }

    #[test]
    fn an_empty_or_overlong_viewer_id_is_refused() {
        assert!(start(Some("  "), None, None).lease().is_err());
        let long = "v".repeat(MAX_VIEWER_ID_LEN + 1);
        assert!(start(Some(&long), None, None).lease().is_err());
        let longest = "v".repeat(MAX_VIEWER_ID_LEN);
        assert!(start(Some(&longest), None, None).lease().unwrap().is_some());
    }
}
