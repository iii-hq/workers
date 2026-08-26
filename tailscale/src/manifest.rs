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
        name: "tailscale".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "Share the local iii Console through Tailscale Serve or explicitly enabled Funnel, with safe typed controls and QR links.".to_string(),
        default_config: WorkerConfig::default().to_json(),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_safe_defaults() {
        let manifest = build_manifest();
        assert_eq!(manifest.name, "tailscale");
        assert_eq!(manifest.default_config["allow_funnel"], false);
        assert!(!manifest.supported_targets.is_empty());
    }
}
