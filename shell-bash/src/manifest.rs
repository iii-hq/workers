use serde::Serialize;
use shell_bash::exec::{DEFAULT_TIMEOUT_MS, MAX_OUTPUT_BYTES, TRIGGER_TIMEOUT_MS};

#[derive(Serialize)]
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
        description: "Sandboxed shell execution under shell::bash::* — wraps the engine sandbox::exec primitive with no host fallback."
            .to_string(),
        default_config: serde_json::json!({
            "default_timeout_ms": DEFAULT_TIMEOUT_MS,
            "trigger_timeout_ms": TRIGGER_TIMEOUT_MS,
            "max_output_bytes": MAX_OUTPUT_BYTES,
        }),
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
        assert!(parsed["default_config"].is_object());
        let arr = parsed["supported_targets"].as_array().expect("array");
        assert!(!arr.is_empty());
    }
}
