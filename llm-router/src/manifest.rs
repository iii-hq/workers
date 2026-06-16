//! Registry-publish manifest emitted by `llm-router --manifest`.
//!
//! The publish pipeline runs the binary with `--manifest`, parses the JSON on
//! stdout, and writes it to the registry (binary-worker.md § manifest).
use serde::Serialize;

const DESCRIPTION: &str = "One front door + provider protocol in front of every LLM provider.";

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

/// Build the manifest for the currently-compiled binary.
///
/// `default_config` is intentionally empty: the router's operator-facing
/// configuration lives in the engine's `llm-router` configuration entry
/// (spec § Configuration), not in a config file.
pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: DESCRIPTION.to_string(),
        default_config: serde_json::json!({}),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let m = build_manifest();
        let json = serde_json::to_string_pretty(&m).expect("serialize manifest");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed["name"], "llm-router");
        assert!(!parsed["version"].as_str().unwrap().is_empty());
        assert!(!parsed["description"].as_str().unwrap().is_empty());
        assert!(parsed["default_config"].is_object());
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }
}
