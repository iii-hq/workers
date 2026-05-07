use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_api_base")]
    pub api_base: String,
    #[serde(default = "default_api_key_env")]
    pub api_key_env: String,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_sandboxes: usize,
    #[serde(default = "default_idle_timeout")]
    pub default_idle_timeout_secs: u64,
    #[serde(default)]
    pub image_allowlist: Vec<String>,
}

fn default_api_base() -> String {
    "https://cloud.morph.so/api".to_string()
}
fn default_api_key_env() -> String {
    "MORPH_API_KEY".to_string()
}
fn default_max_concurrent() -> usize {
    10
}
fn default_idle_timeout() -> u64 {
    300
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_base: default_api_base(),
            api_key_env: default_api_key_env(),
            max_concurrent_sandboxes: default_max_concurrent(),
            default_idle_timeout_secs: default_idle_timeout(),
            image_allowlist: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        // Workers in this repo ship config.yaml whose top-level keys are the
        // worker config fields directly OR wrapped under `config:`. Try both.
        let value: serde_yaml::Value = serde_yaml::from_str(&raw)?;
        if let Some(inner) = value.get("config") {
            return Ok(serde_yaml::from_value(inner.clone())?);
        }
        Ok(serde_yaml::from_value(value)?)
    }

    pub fn image_allowed(&self, image: &str) -> bool {
        self.image_allowlist.is_empty() || self.image_allowlist.iter().any(|i| i == image)
    }
}
