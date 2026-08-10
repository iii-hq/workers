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
        // sees the same surface `config.yaml` documents. node-engine's
        // manifest drifted by omitting `external_mb`; this one is checked
        // against the struct field-for-field below.
        default_config: serde_json::json!({
            "max_runtimes": d.max_runtimes,
            "default_timeout_ms": d.default_timeout_ms,
            "max_timeout_ms": d.max_timeout_ms,
            "idle_ttl_secs": d.idle_ttl_secs,
            "heap_mb": d.heap_mb,
            "external_mb": d.external_mb,
            "scratch_mb": d.scratch_mb,
            "scratch_files": d.scratch_files,
        }),
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

    /// Every operator-facing key must be advertised. Mutation: drop any line
    /// from `default_config` — node-engine's manifest silently omitted
    /// `external_mb` for exactly as long as nobody checked.
    #[test]
    fn default_config_advertises_every_operator_key() {
        let m = build_manifest();
        let d = CodeRunnerConfig::default();
        for (k, v) in [
            ("max_runtimes", serde_json::json!(d.max_runtimes)),
            (
                "default_timeout_ms",
                serde_json::json!(d.default_timeout_ms),
            ),
            ("max_timeout_ms", serde_json::json!(d.max_timeout_ms)),
            ("idle_ttl_secs", serde_json::json!(d.idle_ttl_secs)),
            ("heap_mb", serde_json::json!(d.heap_mb)),
            ("external_mb", serde_json::json!(d.external_mb)),
            ("scratch_mb", serde_json::json!(d.scratch_mb)),
            ("scratch_files", serde_json::json!(d.scratch_files)),
        ] {
            assert_eq!(
                m.default_config[k], v,
                "manifest is missing or wrong for {k}"
            );
        }
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
