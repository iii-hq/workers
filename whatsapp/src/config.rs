use std::{path::PathBuf, sync::OnceLock};

use anyhow::{Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_CONFIG_ID: &str = "whatsapp";

pub fn config_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_owned())
            .unwrap_or_else(|| DEFAULT_CONFIG_ID.to_owned())
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub data_dir: PathBuf,
    pub max_sessions: usize,
    pub max_media_bytes: usize,
    pub max_events: usize,
    pub operation_timeout_secs: u64,
    pub shutdown_timeout_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: "data/whatsapp".into(),
            max_sessions: 32,
            max_media_bytes: 16 * 1024 * 1024,
            max_events: 1000,
            operation_timeout_secs: 30,
            shutdown_timeout_secs: 15,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.data_dir.as_os_str().is_empty(),
            "data_dir must not be empty"
        );
        ensure!(
            (1..=1024).contains(&self.max_sessions),
            "max_sessions must be 1..1024"
        );
        ensure!(
            (1..=64 * 1024 * 1024).contains(&self.max_media_bytes),
            "max_media_bytes must be 1..67108864"
        );
        ensure!(
            (1..=100_000).contains(&self.max_events),
            "max_events must be 1..100000"
        );
        ensure!(
            (1..=600).contains(&self.operation_timeout_secs),
            "operation_timeout_secs must be 1..600"
        );
        ensure!(
            (1..=120).contains(&self.shutdown_timeout_secs),
            "shutdown_timeout_secs must be 1..120"
        );
        Ok(())
    }

    pub fn from_yaml(input: &str) -> Result<Self> {
        let config: Self = serde_yaml::from_str(input)?;
        config.validate()?;
        Ok(config)
    }
}
