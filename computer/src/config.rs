//! Config for the computer worker. Numeric caps are HARD ceilings: a caller
//! may ask for less per call, never more. Timeouts and the screencast rate are
//! read per call / per pump tick, so they hot-reload. `default_endpoint` and
//! `os` are read at session start, so a change applies to sessions started
//! after it; running sessions keep the backend they were launched against.

use std::sync::Arc;

use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type SharedConfig = Arc<ArcSwap<WorkerConfig>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WorkerConfig {
    /// computer-server endpoint new sessions connect to when
    /// `sessions::start` omits one. Accepts `ws://host:port`,
    /// `http://host:port`, or a bare `host:port` (defaults to ws, `/ws`
    /// appended). Empty means every `sessions::start` must pass its own
    /// `endpoint`.
    pub default_endpoint: String,
    /// Informational OS label recorded on started sessions and surfaced in
    /// `session-started` events (`linux`, `macos`, `windows`, `android`).
    /// Does not change the backend; it labels which guest you connected to.
    pub os: String,
    /// Maximum concurrently running sessions; `sessions::start` beyond this
    /// fails until one stops.
    pub max_sessions: u64,
    /// Stop sessions idle longer than this (ms). 0 disables the sweep.
    pub idle_stop_ms: u64,
    /// Live-view frame rate cap for the screencast (frames/sec). Clamped to
    /// at least 1. The pump polls the backend screenshot at most this often.
    pub screencast_fps: u64,
    /// Timeout used when a caller omits `timeout_ms` on an action.
    pub default_timeout_ms: u64,
    /// Hard ceiling on per-call timeout; caller `timeout_ms` is clamped DOWN
    /// to this.
    pub max_timeout_ms: u64,
    /// Timeout for establishing the backend connection at session start (ms).
    pub connect_timeout_ms: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            default_endpoint: String::new(),
            os: "linux".to_string(),
            max_sessions: 2,
            idle_stop_ms: 300_000,
            screencast_fps: 15,
            default_timeout_ms: 30_000,
            max_timeout_ms: 120_000,
            connect_timeout_ms: 15_000,
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

    /// Clamp a caller-supplied timeout to the configured ceiling, defaulting
    /// when omitted.
    pub fn clamp_timeout(&self, requested: Option<u64>) -> u64 {
        requested
            .unwrap_or(self.default_timeout_ms)
            .min(self.max_timeout_ms)
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
        assert_eq!(c.default_timeout_ms, 30_000);
        assert_eq!(c.max_timeout_ms, 120_000);
        assert_eq!(c.connect_timeout_ms, 15_000);
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
    fn clamp_timeout_defaults_and_ceils() {
        let c = WorkerConfig::default();
        assert_eq!(c.clamp_timeout(None), 30_000);
        assert_eq!(c.clamp_timeout(Some(5_000)), 5_000);
        assert_eq!(c.clamp_timeout(Some(600_000)), 120_000);
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
