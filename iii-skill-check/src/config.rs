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
pub fn load(_path: &Path) -> anyhow::Result<Config> {
    anyhow::bail!("config::load not yet implemented")
}
