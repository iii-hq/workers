//! The `--manifest` payload the registry publish pipeline reads.
//!
//! Printed without connecting to the engine, so it stays fast and
//! side-effect-free.

use serde::Serialize;

use crate::config::WorkerConfig;

#[derive(Debug, Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub const DESCRIPTION: &str =
    "Create, edit and render diagrams in the console: canvases store editable source (mermaid \
     text or excalidraw scenes), drawn live in chat and on a canvas page.";

pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: DESCRIPTION.to_string(),
        default_config: WorkerConfig::default().to_json(),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `POST /publish` rejects a manifest missing any of the five fields.
    #[test]
    fn manifest_carries_every_required_field() {
        let json = serde_json::to_value(build_manifest()).expect("manifest serializes");
        assert_eq!(json["name"], "canvas");
        assert!(json["version"].as_str().is_some_and(|v| !v.is_empty()));
        assert!(json["description"].as_str().is_some_and(|d| d.len() > 20));
        assert!(json["default_config"].is_object());
        assert!(json["supported_targets"]
            .as_array()
            .is_some_and(|t| !t.is_empty()));
    }

    /// The manifest name is the folder name, the binary name, and the registry
    /// key. A drift here breaks the release, not the build.
    #[test]
    fn manifest_name_matches_the_worker_name() {
        assert_eq!(build_manifest().name, "canvas");
    }

    #[test]
    fn default_config_mirrors_the_shipped_defaults() {
        assert_eq!(
            build_manifest().default_config,
            WorkerConfig::default().to_json()
        );
    }
}
