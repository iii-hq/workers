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
        name: "tmux".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "tmux as an iii worker: sessions and windows, agent / compose / port status per pane, and a Console page with a live tmux terminal.".to_string(),
        default_config: WorkerConfig::default().to_json(),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_carries_the_defaults() {
        let manifest = build_manifest();
        assert_eq!(manifest.name, "tmux");
        assert_eq!(manifest.default_config["tmux_binary"], "tmux");
        assert!(!manifest.supported_targets.is_empty());
    }
}
