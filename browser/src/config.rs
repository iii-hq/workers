//! Config for the browser worker. Numeric caps are HARD ceilings — a caller
//! may ask for less per call, never more. Buffer sizes, timeouts, and the
//! scheme allowlist are read per call, so they hot-reload. `executable`,
//! `headless`, and the viewport are read at session start, so a change
//! applies to sessions started after it — running sessions keep the browser
//! they were launched with.

use std::sync::Arc;

use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type SharedConfig = Arc<ArcSwap<WorkerConfig>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WorkerConfig {
    /// Absolute path to a Chromium/Chrome executable. Empty string
    /// auto-detects a system install (Chrome, Chromium, Edge).
    pub executable: String,
    /// Chromium profile directory. Empty string is ephemeral (fresh profile
    /// per session); set a path to persist cookies, localStorage, and logins
    /// across sessions. All sessions share the one profile.
    pub user_data_dir: String,
    /// Launch sessions headless. Set false to watch a real window locally.
    pub headless: bool,
    /// Maximum concurrently running sessions; `sessions::start` beyond this
    /// fails until one stops.
    pub max_sessions: u64,
    /// Per-session console ring-buffer capacity (entries).
    pub console_buffer: u64,
    /// Per-session network ring-buffer capacity (entries).
    pub network_buffer: u64,
    /// Viewport width for new sessions (pixels).
    pub viewport_width: u32,
    /// Viewport height for new sessions (pixels).
    pub viewport_height: u32,
    /// Timeout used when the caller omits `timeout_ms` (navigation, act,
    /// evaluate).
    pub default_timeout_ms: u64,
    /// Hard ceiling on per-call timeout; caller `timeout_ms` is clamped DOWN
    /// to this.
    pub max_timeout_ms: u64,
    /// Stop sessions idle longer than this (ms). 0 disables the sweep.
    pub idle_stop_ms: u64,
    /// JPEG quality for `browser::screenshot` (1-100).
    pub screenshot_quality: u64,
    /// URL schemes `browser::navigate` accepts.
    pub allowed_schemes: Vec<String>,
    /// Maximum nodes serialized by `browser::snapshot` before truncation.
    pub max_snapshot_nodes: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            executable: String::new(),
            user_data_dir: String::new(),
            headless: true,
            max_sessions: 4,
            console_buffer: 500,
            network_buffer: 500,
            viewport_width: 1280,
            viewport_height: 800,
            default_timeout_ms: 30_000,
            max_timeout_ms: 120_000,
            idle_stop_ms: 300_000,
            screenshot_quality: 60,
            allowed_schemes: vec!["http".to_string(), "https".to_string()],
            max_snapshot_nodes: 2_000,
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
    /// under a `browser` wrapper or flat — accept both.
    pub fn from_json(v: &serde_json::Value) -> Result<WorkerConfig, String> {
        let inner = v.get("browser").unwrap_or(v);
        serde_json::from_value(inner.clone()).map_err(|e| format!("invalid browser config: {e}"))
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = WorkerConfig::default();
        assert_eq!(c.executable, "");
        assert_eq!(c.user_data_dir, "");
        assert!(c.headless);
        assert_eq!(c.max_sessions, 4);
        assert_eq!(c.console_buffer, 500);
        assert_eq!(c.network_buffer, 500);
        assert_eq!(c.viewport_width, 1280);
        assert_eq!(c.viewport_height, 800);
        assert_eq!(c.default_timeout_ms, 30_000);
        assert_eq!(c.max_timeout_ms, 120_000);
        assert_eq!(c.idle_stop_ms, 300_000);
        assert_eq!(c.screenshot_quality, 60);
        assert_eq!(c.allowed_schemes, vec!["http", "https"]);
        assert_eq!(c.max_snapshot_nodes, 2_000);
    }

    #[test]
    fn json_roundtrip() {
        let c = WorkerConfig {
            headless: false,
            max_sessions: 2,
            ..WorkerConfig::default()
        };
        let back = WorkerConfig::from_json(&c.to_json()).unwrap();
        assert!(!back.headless);
        assert_eq!(back.max_sessions, 2);
    }

    #[test]
    fn from_json_fills_missing_with_defaults() {
        let v = serde_json::json!({ "max_sessions": 8 });
        let c = WorkerConfig::from_json(&v).unwrap();
        assert_eq!(c.max_sessions, 8);
        assert_eq!(c.console_buffer, 500);
    }

    #[test]
    fn from_json_accepts_wrapped_value() {
        let v = serde_json::json!({ "browser": { "headless": false } });
        let c = WorkerConfig::from_json(&v).unwrap();
        assert!(!c.headless);
    }

    #[test]
    fn clamp_timeout_defaults_and_ceils() {
        let c = WorkerConfig::default();
        assert_eq!(c.clamp_timeout(None), 30_000);
        assert_eq!(c.clamp_timeout(Some(5_000)), 5_000);
        assert_eq!(c.clamp_timeout(Some(600_000)), 120_000);
    }

    #[test]
    fn schema_lists_fields() {
        let s = WorkerConfig::json_schema();
        let props = &s["properties"];
        assert!(props.get("executable").is_some());
        assert!(props.get("headless").is_some());
        assert!(props.get("allowed_schemes").is_some());
    }
}
