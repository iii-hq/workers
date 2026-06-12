//! Operator-facing runtime configuration loaded from `config.yaml`.
//!
//! The storage backend is selected by a `backend` discriminator plus a
//! nested, backend-specific `backend_config` object:
//!
//! ```yaml
//! backend: fs
//! backend_config:
//!   data_dir: ~/.iii/data/session-manager
//! ```
//!
//! or
//!
//! ```yaml
//! backend: bridge
//! backend_config:
//!   url: ws://127.0.0.1:49134
//!   timeout_ms: 5000
//! ```
//!
//! `backend_config` is held raw and resolved into a typed per-backend
//! struct by [`WorkerConfig::resolve_backend`]; resolution failures are
//! fatal at boot (a misconfigured bridge must never silently fall back
//! to writing a local fs store).

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// One append-only JSONL file per session under `data_dir`.
    Fs,
    /// Defer raw storage (and event fan-out) to a main session-manager
    /// on another iii instance.
    Bridge,
}

/// Root config shape. Unknown keys are rejected so a typo'd field
/// (e.g. `backnd: bridge`) fails loudly instead of silently running
/// the default backend.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Storage backend selector. Default `fs`.
    #[serde(default = "default_backend")]
    pub backend: BackendKind,

    /// Backend-specific settings; shape depends on `backend`. See
    /// [`FsBackendConfig`] / [`BridgeBackendConfig`].
    #[serde(default = "default_backend_config")]
    pub backend_config: Value,

    /// Default page size for `session::list` / `session::messages` when
    /// the request omits `limit`.
    #[serde(default = "default_default_list_limit")]
    pub default_list_limit: usize,

    /// Hard cap applied to any requested `limit`.
    #[serde(default = "default_max_list_limit")]
    pub max_list_limit: usize,
}

/// Typed settings for `backend: fs`.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct FsBackendConfig {
    /// Directory holding one `<session_id>.jsonl` per session. A
    /// leading `~/` expands to the home directory.
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
}

impl FsBackendConfig {
    pub fn resolved_data_dir(&self) -> PathBuf {
        expand_tilde(&self.data_dir)
    }
}

impl Default for FsBackendConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
        }
    }
}

/// Typed settings for `backend: bridge`.
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct BridgeBackendConfig {
    /// WebSocket URL of the main instance's engine.
    pub url: String,

    /// Per store/publish call timeout (milliseconds).
    #[serde(default = "default_bridge_timeout_ms")]
    pub timeout_ms: u64,
}

/// A resolved, validated backend selection.
#[derive(Debug, Clone)]
pub enum Backend {
    Fs(FsBackendConfig),
    Bridge(BridgeBackendConfig),
}

impl WorkerConfig {
    /// Parse `backend_config` into the typed struct for `backend`.
    /// Unknown keys are rejected so a typo'd field fails loudly.
    pub fn resolve_backend(&self) -> Result<Backend> {
        let raw = if self.backend_config.is_null() {
            Value::Object(serde_json::Map::new())
        } else {
            self.backend_config.clone()
        };
        match self.backend {
            BackendKind::Fs => {
                let cfg: FsBackendConfig = serde_json::from_value(raw)
                    .context("invalid backend_config for backend `fs`")?;
                Ok(Backend::Fs(cfg))
            }
            BackendKind::Bridge => {
                let cfg: BridgeBackendConfig = serde_json::from_value(raw)
                    .context("invalid backend_config for backend `bridge`")?;
                Ok(Backend::Bridge(cfg))
            }
        }
    }
}

fn default_backend() -> BackendKind {
    BackendKind::Fs
}

fn default_backend_config() -> Value {
    Value::Object(serde_json::Map::new())
}

pub fn default_data_dir() -> String {
    "~/.iii/data/session-manager".to_string()
}

fn default_bridge_timeout_ms() -> u64 {
    5000
}

fn default_default_list_limit() -> usize {
    50
}

fn default_max_list_limit() -> usize {
    500
}

fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            backend_config: default_backend_config(),
            default_list_limit: default_default_list_limit(),
            max_list_limit: default_max_list_limit(),
        }
    }
}

pub fn load_config(path: &str) -> Result<WorkerConfig> {
    let contents = std::fs::read_to_string(path)?;
    let cfg: WorkerConfig = serde_yaml::from_str(&contents)?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml_resolve_to_fs() {
        let cfg: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.backend, BackendKind::Fs);
        assert_eq!(cfg.default_list_limit, 50);
        assert_eq!(cfg.max_list_limit, 500);
        let Backend::Fs(fs) = cfg.resolve_backend().unwrap() else {
            panic!("expected fs backend");
        };
        assert_eq!(fs.data_dir, "~/.iii/data/session-manager");
    }

    #[test]
    fn fs_yaml_overrides_data_dir() {
        let yaml = "backend: fs\nbackend_config:\n  data_dir: /tmp/sessions";
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        let Backend::Fs(fs) = cfg.resolve_backend().unwrap() else {
            panic!("expected fs backend");
        };
        assert_eq!(fs.data_dir, "/tmp/sessions");
        assert_eq!(fs.resolved_data_dir(), PathBuf::from("/tmp/sessions"));
    }

    #[test]
    fn tilde_data_dir_expands_to_home() {
        let fs = FsBackendConfig::default();
        let resolved = fs.resolved_data_dir();
        if let Some(home) = dirs::home_dir() {
            assert!(resolved.starts_with(home));
            assert!(resolved.ends_with(".iii/data/session-manager"));
        }
    }

    #[test]
    fn bridge_yaml_parses_with_defaulted_timeout() {
        let yaml = "backend: bridge\nbackend_config:\n  url: ws://main:49134";
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        let Backend::Bridge(bridge) = cfg.resolve_backend().unwrap() else {
            panic!("expected bridge backend");
        };
        assert_eq!(bridge.url, "ws://main:49134");
        assert_eq!(bridge.timeout_ms, 5000);
    }

    #[test]
    fn bridge_without_url_fails_resolution() {
        let yaml = "backend: bridge\nbackend_config:\n  timeout_ms: 100";
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        let err = cfg.resolve_backend().unwrap_err();
        assert!(format!("{err:#}").contains("backend `bridge`"));
    }

    #[test]
    fn unknown_backend_config_key_fails_resolution() {
        let yaml = "backend: fs\nbackend_config:\n  datadir: /tmp/x";
        let cfg: WorkerConfig = serde_yaml::from_str(yaml).unwrap();
        let err = format!("{:#}", cfg.resolve_backend().unwrap_err());
        assert!(err.contains("unknown field"), "got: {err}");
    }

    #[test]
    fn unknown_backend_kind_is_rejected_at_parse() {
        assert!(serde_yaml::from_str::<WorkerConfig>("backend: cloud").is_err());
    }

    #[test]
    fn unknown_root_key_is_rejected_at_parse() {
        let err = serde_yaml::from_str::<WorkerConfig>("backnd: bridge").unwrap_err();
        assert!(err.to_string().contains("unknown field"), "got: {err}");
    }

    #[test]
    fn impl_default_matches_yaml_defaults() {
        let from_yaml: WorkerConfig = serde_yaml::from_str("{}").unwrap();
        let from_default = WorkerConfig::default();
        assert_eq!(from_yaml.backend, from_default.backend);
        assert_eq!(from_yaml.backend_config, from_default.backend_config);
        assert_eq!(
            from_yaml.default_list_limit,
            from_default.default_list_limit
        );
        assert_eq!(from_yaml.max_list_limit, from_default.max_list_limit);
    }

    #[test]
    fn committed_config_yaml_parses_and_resolves() {
        let cfg = load_config(concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml")).unwrap();
        assert_eq!(cfg.backend, BackendKind::Fs);
        assert!(matches!(cfg.resolve_backend().unwrap(), Backend::Fs(_)));
    }
}
