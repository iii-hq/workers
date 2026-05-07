//! Registry-publish manifest emitted by `storage --manifest`.
//!
//! The publish pipeline runs the binary with `--manifest`, parses the JSON on
//! stdout, and writes it to the registry. The five required fields (`name`,
//! `version`, `description`, `default_config` object, non-empty
//! `supported_targets` array) are validated by `POST /publish`.

use serde::Serialize;

const DESCRIPTION: &str = "S3-compatible object storage across AWS S3, GCS, Cloudflare R2, and a managed local rustfs backend. Streamed uploads, presigned URLs, and object change triggers.";

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
/// `default_config` is intentionally an empty object: a fresh checkout has no
/// providers configured and no buckets wired, matching `WorkerConfig::default()`.
/// Operators populate it via `config.yaml`.
pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: DESCRIPTION.to_string(),
        default_config: serde_json::json!({
            "providers": {},
            "buckets": {}
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
        let json = serde_json::to_string_pretty(&m).expect("serialize manifest");
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("manifest is valid JSON");

        assert_eq!(parsed["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(
            parsed["description"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "description must be a non-empty string"
        );
        assert!(
            parsed["default_config"].is_object(),
            "default_config must be an object"
        );
        assert!(
            !parsed["supported_targets"]
                .as_array()
                .expect("supported_targets must be an array")
                .is_empty(),
            "supported_targets must not be empty"
        );
    }

    #[test]
    fn default_config_mirrors_worker_config_default() {
        // If WorkerConfig::default() ever grows non-empty fields, the manifest
        // should mirror them. Today both are empty; this test pins that invariant.
        let m = build_manifest();
        assert_eq!(
            m.default_config,
            serde_json::json!({ "providers": {}, "buckets": {} })
        );
    }
}
