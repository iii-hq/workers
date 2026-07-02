//! `--manifest` output for the registry publish pipeline. Fast and
//! side-effect-free; `default_config` mirrors `WorkerConfig::default()`.

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
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "Git worktree lifecycle for parallel agents: isolated worktrees, \
                      a cross-agent registry with lifecycle trigger types, and a \
                      per-repo FIFO land queue (rebase, test gate, ff-only merge)."
            .to_string(),
        default_config: WorkerConfig::default().to_json(),
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
    fn default_config_mirrors_worker_config_default() {
        let m = build_manifest();
        assert_eq!(m.default_config, WorkerConfig::default().to_json());
        assert_eq!(m.default_config["land_queue"], "worktree-land");
        assert_eq!(m.default_config["prune_expire_hours"], 72);
    }
}
