//! Side-effect-free worker metadata for the registry publish pipeline.
//!
//! `--manifest` prints this without connecting to an engine, so the publish
//! pipeline can read a worker's identity from a binary it just built.

use serde::Serialize;

use crate::WorkerConfig;

pub const DESCRIPTION: &str = "Error monitor for the iii engine: groups failures from trace and log telemetry by deterministic fingerprint, freezes the evidence before the in-memory store discards it, and opens assisted harness investigations that record a structured diagnosis.";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: DESCRIPTION.to_string(),
        default_config: serde_json::to_value(WorkerConfig::default())
            .expect("the shipped configuration must serialize"),
        supported_targets: vec![build_target()],
    }
}

fn build_target() -> String {
    if let Some(target) = option_env!("TARGET") {
        return target.to_string();
    }

    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin".to_string()
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin".to_string()
    } else if cfg!(all(
        target_os = "windows",
        target_env = "msvc",
        target_arch = "aarch64"
    )) {
        "aarch64-pc-windows-msvc".to_string()
    } else if cfg!(all(
        target_os = "windows",
        target_env = "msvc",
        target_arch = "x86_64"
    )) {
        "x86_64-pc-windows-msvc".to_string()
    } else if cfg!(all(
        target_os = "linux",
        target_env = "musl",
        target_arch = "x86_64"
    )) {
        "x86_64-unknown-linux-musl".to_string()
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "aarch64"
    )) {
        "aarch64-unknown-linux-gnu".to_string()
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "x86_64"
    )) {
        "x86_64-unknown-linux-gnu".to_string()
    } else if cfg!(all(
        target_os = "linux",
        target_env = "gnu",
        target_arch = "arm"
    )) {
        "armv7-unknown-linux-gnueabihf".to_string()
    } else {
        format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_default_config_round_trips_into_the_worker_config() {
        let manifest = build_manifest();
        let config: WorkerConfig = serde_json::from_value(manifest.default_config)
            .expect("default_config parses back into WorkerConfig");
        assert_eq!(config, WorkerConfig::default());
    }
}
