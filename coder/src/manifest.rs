//! Manifest emitted by `coder --manifest`. The registry publish pipeline
//! runs the binary with `--manifest`, parses the JSON on stdout, and
//! writes it to the registry. The five required fields (`name`,
//! `version`, `description`, `default_config` object, non-empty
//! `supported_targets`) are validated by `POST /publish`.

use serde::Serialize;

const DESCRIPTION: &str = "Path-jailed code worker — read/search/update/create/delete files plus paginated list-folder and tree, with non-accessible glob protection.";

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
        description: DESCRIPTION.to_string(),
        default_config: serde_json::json!({
            "base_path": "./",
            "non_accessible_globs": [
                "**/.env",
                "**/.env.*",
                "**/*.pem",
                "**/*.key",
                "**/secrets/**"
            ],
            "max_read_bytes": 10_485_760u64,
            "max_write_bytes": 10_485_760u64,
            "tree_default_depth": 4,
            "tree_per_folder_limit": 50,
            "list_default_page_size": 100,
            "list_max_page_size": 1_000,
            "search_default_max_matches": 1_000,
            "search_default_max_line_bytes": 4_096,
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
        assert!(parsed["description"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false));
        assert!(parsed["default_config"].is_object());
        assert!(!parsed["supported_targets"]
            .as_array()
            .expect("array")
            .is_empty());
    }
}
