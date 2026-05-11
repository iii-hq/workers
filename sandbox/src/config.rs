use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Fallback provider when callers omit the `provider` field. Defaults to
    /// `iii_sandbox_abi::DEFAULT_PROVIDER` ("local").
    #[serde(default = "default_provider")]
    pub default_provider: String,
}

fn default_provider() -> String {
    sandbox_abi::DEFAULT_PROVIDER.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_provider: default_provider(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Self = serde_yaml::from_str(&raw)?;
        Ok(cfg)
    }
}
