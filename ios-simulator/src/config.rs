//! Config for the ios-simulator worker. Every field is read per call, so a
//! change applies to the next operation without a restart: a new `data_dir`
//! points tenants at a different store (simulators already running from the
//! old one keep running), caps apply to the next boot/create, and the stream
//! knobs apply the next time a bridge starts streaming.

use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type SharedConfig = Arc<ArcSwap<WorkerConfig>>;

/// The tenant a call without `tenant` belongs to.
pub const DEFAULT_TENANT: &str = "default";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WorkerConfig {
    /// Where the worker keeps tenant device sets, screenshots and recordings
    /// (`<data_dir>/tenants/<tenant>/{devices,media}`). Relative paths resolve
    /// against the project directory.
    pub data_dir: String,
    /// Xcode developer directory. Empty uses `xcode-select -p`.
    pub developer_dir: String,
    /// The `default` tenant manages this Mac's own Xcode simulators (the set
    /// Simulator.app and `xcrun simctl` use). Turn off on a shared Mac so every
    /// tenant, `default` included, only sees simulators the worker created
    /// for it.
    pub share_system_devices: bool,
    /// Refuse calls that do not name a tenant. Turn on when the worker is an
    /// API for several consumers, so a missing tenant can never fall through
    /// to `default`.
    pub require_tenant: bool,
    /// Simulators booted at once across every tenant: the Mac's capacity.
    #[schemars(range(min = 1, max = 64))]
    pub max_booted: u32,
    /// Simulators one tenant may have booted at once.
    #[schemars(range(min = 1, max = 64))]
    pub max_booted_per_tenant: u32,
    /// Simulators one tenant may own (created through the worker).
    #[schemars(range(min = 1, max = 500))]
    pub max_devices_per_tenant: u32,
    /// Live-view frame rate cap while someone watches a simulator.
    #[schemars(range(min = 1, max = 60))]
    pub stream_fps: u32,
    /// Longest edge (px) of a live-view frame; the device keeps its native
    /// resolution for input, screenshots and recordings.
    #[schemars(range(min = 240, max = 4096))]
    pub stream_max_dimension: u32,
    /// JPEG quality (1-100) of live-view frames.
    #[schemars(range(min = 1, max = 100))]
    pub stream_quality: u32,
    /// Recordings stop by themselves after this many seconds.
    #[schemars(range(min = 5, max = 7200))]
    pub max_recording_seconds: u32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            data_dir: iii_worker_paths::default_path("data/ios-simulator"),
            developer_dir: String::new(),
            share_system_devices: true,
            require_tenant: false,
            max_booted: 3,
            max_booted_per_tenant: 2,
            max_devices_per_tenant: 10,
            stream_fps: 30,
            stream_max_dimension: 1000,
            stream_quality: 70,
            max_recording_seconds: 600,
        }
    }
}

impl WorkerConfig {
    pub fn json_schema() -> serde_json::Value {
        let mut schema = serde_json::to_value(schemars::schema_for!(WorkerConfig))
            .expect("WorkerConfig schema serializes");
        schema["example"] = WorkerConfig::default().to_json();
        schema
    }

    /// Missing keys fall back to defaults; unknown keys are refused.
    pub fn from_json(v: &serde_json::Value) -> Result<WorkerConfig, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("invalid ios-simulator config: {e}"))
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    pub fn into_shared(self) -> SharedConfig {
        Arc::new(ArcSwap::from_pointee(self))
    }

    pub fn data_root(&self) -> PathBuf {
        iii_worker_paths::resolve_path(&self.data_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_local_and_bounded() {
        let c = WorkerConfig::default();
        assert_eq!(c.data_dir, "data/ios-simulator");
        assert!(c.share_system_devices);
        assert!(!c.require_tenant);
        assert!(c.max_booted_per_tenant <= c.max_booted);
    }

    #[test]
    fn missing_keys_use_defaults() {
        let c = WorkerConfig::from_json(&serde_json::json!({ "stream_fps": 12 })).unwrap();
        assert_eq!(c.stream_fps, 12);
        assert_eq!(c.max_booted, 3);
    }

    #[test]
    fn unknown_keys_are_refused() {
        let err = WorkerConfig::from_json(&serde_json::json!({ "stream_fsp": 12 })).unwrap_err();
        assert!(err.contains("stream_fsp"), "{err}");
    }

    #[test]
    fn schema_carries_defaults_as_example() {
        let s = WorkerConfig::json_schema();
        assert_eq!(s["example"]["data_dir"], "data/ios-simulator");
        assert!(s["properties"]["require_tenant"].is_object());
    }
}
