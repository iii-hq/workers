use serde::Serialize;

use crate::config::WorkerConfig;

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    // Serialized from the same struct `--config` parses into, so the published
    // default_config can never drift from what the worker actually boots with.
    let defaults = WorkerConfig::default();
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "A shared code workspace — open buffers, a file tree, unified diffs, \
                      fuzzy find and conflict-safe saves that an agent and a person see the \
                      same view of, plus a console editor page."
            .to_string(),
        // Serialized from the same struct the configuration worker
        // validates, so the published default_config cannot drift.
        default_config: defaults.to_json(),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let m = build_manifest();
        let json = serde_json::to_string_pretty(&m).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(!parsed["description"].as_str().unwrap().is_empty());
        assert!(parsed["default_config"].is_object());
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn default_config_mirrors_the_config_struct() {
        let m = build_manifest();
        let defaults = WorkerConfig::default();
        assert_eq!(m.default_config["find_limit"], defaults.find_limit);
        assert_eq!(m.default_config["git_timeout_ms"], defaults.git_timeout_ms);
    }
}
