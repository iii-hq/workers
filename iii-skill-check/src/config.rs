use anyhow::Context;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub rules: PathSource,
    pub styles: PathSource,
    pub ai_check: AiCheck,
}

#[derive(Debug, Deserialize)]
pub struct PathSource {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct AiCheck {
    pub provider: String,
    pub model: String,
    pub api_key_env_var: String,
    pub max_tokens: u32,
}

/// Load `.skill-check.yaml` from a path.
pub fn load(path: &Path) -> anyhow::Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let config: Config = serde_yaml::from_str(&content)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(config)
}
