//! The `--manifest` payload the registry publish pipeline reads. Printed
//! without connecting to the engine.

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

pub const DESCRIPTION: &str = "Store versioned single-file artifacts (HTML designs and prototypes, \
     markdown RFCs and docs, SVG drawings) and share a frozen version through an expiring, \
     token-guarded link, locally or through a quick-tunnel.";

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

    #[test]
    fn manifest_carries_every_required_field() {
        let json = serde_json::to_value(build_manifest()).expect("manifest serializes");
        assert_eq!(json["name"], "artifact");
        assert!(json["version"].as_str().is_some_and(|v| !v.is_empty()));
        assert!(json["description"].as_str().is_some_and(|d| d.len() > 20));
        assert_eq!(json["default_config"], WorkerConfig::default().to_json());
        assert!(
            json["supported_targets"]
                .as_array()
                .is_some_and(|t| !t.is_empty())
        );
    }
}
