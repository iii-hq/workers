use serde::Serialize;

use crate::config::CodeRunnerConfig;

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    let d = CodeRunnerConfig::default();
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "Run untrusted Node.js and Python in-process — V8 isolates and \
             CPython-on-WebAssembly behind one run/register_function/teardown API, with no \
             microVM and no /dev/kvm. Code gets a global `iii` and a private scratch directory."
            .to_string(),
        // Every operator-facing key, so an operator reading the registry
        // sees the same surface the configuration entry holds. Serialized
        // from the struct rather than hand-listed: node-engine's manifest
        // drifted by omitting `external_mb`, and the hand-listed version of
        // THIS one drifted by four keys within one branch.
        default_config: serde_json::to_value(&d).expect("CodeRunnerConfig serializes"),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let json = serde_json::to_string_pretty(&build_manifest()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(!parsed["description"].as_str().unwrap().is_empty());
        assert!(parsed["default_config"].is_object());
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }

    /// Every operator-facing key must be advertised: the manifest's keys and
    /// the configuration schema's properties are the same surface. Compared
    /// as key SETS so neither side can drift — the hand-listed predecessor
    /// of this test blessed a four-key omission.
    #[test]
    fn default_config_advertises_every_operator_key() {
        let m = build_manifest();
        let advertised: std::collections::BTreeSet<String> = m
            .default_config
            .as_object()
            .expect("default_config is an object")
            .keys()
            .cloned()
            .collect();
        let schema = CodeRunnerConfig::json_schema();
        let configurable: std::collections::BTreeSet<String> = schema["properties"]
            .as_object()
            .expect("schema has properties")
            .keys()
            .cloned()
            .collect();
        assert_eq!(advertised, configurable);
    }

    /// A registry entry that describes the wrong worker is worse than none.
    #[test]
    fn the_description_names_what_this_worker_actually_is() {
        let m = build_manifest();
        assert!(m.description.contains("Python"), "{}", m.description);
        assert!(
            !m.description.contains("Evaluate JavaScript in V8 isolates"),
            "node-engine's description leaked through: {}",
            m.description
        );
    }
}
