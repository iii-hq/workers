use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_registry")]
    pub registry_url: String,
    #[serde(default = "default_timeout")]
    pub default_timeout_ms: u64,
}

fn default_registry() -> String {
    "https://workers.iii.dev".to_string()
}

fn default_timeout() -> u64 {
    5000
}

impl Default for Config {
    fn default() -> Self {
        Self {
            registry_url: default_registry(),
            default_timeout_ms: default_timeout(),
        }
    }
}

pub fn load(path: &str) -> anyhow::Result<Config> {
    let p = Path::new(path);
    if !p.exists() {
        anyhow::bail!("config not found: {}", path);
    }
    let text = std::fs::read_to_string(p)?;
    let cfg: Config = serde_yaml::from_str(&text)
        .map_err(|e| anyhow::anyhow!("invalid yaml in {}: {e}", path))?;
    Ok(cfg)
}
