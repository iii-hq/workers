use std::{path::PathBuf, sync::OnceLock};

use anyhow::{Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_CONFIG_ID: &str = "whatsapp";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CloudConfig {
    pub enabled: bool,
    pub api_version: String,
    pub business_account_id: String,
    pub phone_number_id: String,
    pub access_token: String,
    pub app_secret: String,
    pub verify_token: String,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_version: "v26.0".into(),
            business_account_id: String::new(),
            phone_number_id: String::new(),
            access_token: String::new(),
            app_secret: String::new(),
            verify_token: String::new(),
        }
    }
}

impl std::fmt::Debug for CloudConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudConfig")
            .field("enabled", &self.enabled)
            .field("api_version", &self.api_version)
            .field("business_account_id", &self.business_account_id)
            .field("phone_number_id", &self.phone_number_id)
            .field("access_token_set", &!self.access_token.is_empty())
            .field("app_secret_set", &!self.app_secret.is_empty())
            .field("verify_token_set", &!self.verify_token.is_empty())
            .finish()
    }
}

impl CloudConfig {
    fn validate(&self) -> Result<()> {
        let version = self
            .api_version
            .strip_prefix('v')
            .and_then(|v| v.split_once('.'));
        ensure!(
            version.is_some_and(|(major, minor)| !major.is_empty()
                && !minor.is_empty()
                && major.len() <= 3
                && minor.len() <= 2
                && major
                    .bytes()
                    .chain(minor.bytes())
                    .all(|b| b.is_ascii_digit())),
            "cloud.api_version must use a version such as v26.0"
        );
        for (name, value) in [
            ("business_account_id", &self.business_account_id),
            ("phone_number_id", &self.phone_number_id),
        ] {
            ensure!(
                value.is_empty()
                    || (value.len() <= 32 && value.bytes().all(|b| b.is_ascii_digit())),
                "cloud.{name} must be a numeric Meta identifier"
            );
            ensure!(
                !self.enabled || !value.is_empty(),
                "cloud.{name} is required when Cloud API is enabled"
            );
        }
        ensure!(
            !self.enabled || !self.access_token.trim().is_empty(),
            "cloud.access_token is required when Cloud API is enabled"
        );
        for (name, value) in [
            ("access_token", &self.access_token),
            ("app_secret", &self.app_secret),
            ("verify_token", &self.verify_token),
        ] {
            ensure!(
                value.len() <= 16384 && !value.contains(['\r', '\n']),
                "cloud.{name} must be a single line of at most 16384 bytes"
            );
        }
        Ok(())
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Optional official Meta integration; independent of linked-device sessions.
    pub cloud: CloudConfig,
    /// Persistent native session storage. Changing this requires a worker restart.
    #[schemars(length(min = 1))]
    pub data_dir: PathBuf,
    /// Maximum number of sessions; existing sessions survive a lower limit.
    #[schemars(range(min = 1, max = 1024))]
    pub max_sessions: usize,
    #[schemars(range(min = 1, max = 67108864))]
    pub max_media_bytes: usize,
    /// Event retention and broadcast capacity. Changing this requires a worker restart.
    #[schemars(range(min = 1, max = 100000))]
    pub max_events: usize,
    /// Maximum retained inbox messages per session; lowering this trims oldest rows live.
    #[schemars(range(min = 1, max = 100000))]
    pub max_inbox_messages: usize,
    #[schemars(range(min = 1, max = 600))]
    pub operation_timeout_secs: u64,
    #[schemars(range(min = 1, max = 120))]
    pub shutdown_timeout_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cloud: CloudConfig::default(),
            data_dir: "data/whatsapp".into(),
            max_sessions: 32,
            max_media_bytes: 16 * 1024 * 1024,
            max_events: 1000,
            max_inbox_messages: 10_000,
            operation_timeout_secs: 30,
            shutdown_timeout_secs: 15,
        }
    }
}

impl Config {
    pub fn json_schema() -> serde_json::Value {
        let mut schema = serde_json::to_value(schemars::schema_for!(Self))
            .expect("configuration schema is serializable");
        schema["example"] =
            serde_json::to_value(Self::default()).expect("default configuration is serializable");
        for field in ["access_token", "app_secret", "verify_token"] {
            schema["definitions"]["CloudConfig"]["properties"][field]["writeOnly"] = true.into();
            schema["definitions"]["CloudConfig"]["properties"][field]["format"] = "password".into();
        }
        schema
    }

    pub fn from_json(value: &serde_json::Value) -> Result<Self> {
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|_| anyhow::anyhow!("invalid WhatsApp configuration structure"))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        self.cloud.validate()?;
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
            (1..=100_000).contains(&self.max_inbox_messages),
            "max_inbox_messages must be 1..100000"
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
        let config: Self = serde_yaml::from_str(input)
            .map_err(|_| anyhow::anyhow!("invalid WhatsApp configuration structure"))?;
        config.validate()?;
        Ok(config)
    }

    pub(crate) fn redacted(mut self) -> Self {
        self.cloud.access_token.clear();
        self.cloud.app_secret.clear();
        self.cloud.verify_token.clear();
        self
    }
}
