//! The `secrets` configuration entry: where the vault lives and, optionally,
//! where the key file lives. Both are paths, never key material, so the
//! entry is safe in the committed `./config` folder.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CONFIG_ID: &str = "secrets";
pub const CONFIG_NAME: &str = "Secrets";
pub const CONFIG_DESCRIPTION: &str =
    "Where the encrypted vault lives and, optionally, the master key file (which must live outside the project).";
pub const DEFAULT_DATA_DIR: &str = "data/secrets";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SecretsConfig {
    /// Folder holding vault.json. Relative paths resolve against the Compose
    /// project directory (III_COMPOSE_DIR), whose data/ tree is gitignored.
    pub data_dir: String,
    /// Master key file. Null keeps ~/.config/iii/secrets/<vault_id>.key
    /// (XDG_CONFIG_HOME respected). Must be absolute or ~/ and outside the
    /// project. Ignored when III_SECRETS_KEY is set.
    pub key_file: Option<String>,
}

impl Default for SecretsConfig {
    fn default() -> Self {
        Self {
            data_dir: iii_worker_paths::default_path(DEFAULT_DATA_DIR),
            key_file: None,
        }
    }
}

impl SecretsConfig {
    /// Parse a stored value. Blank strings fall back to the defaults; a value
    /// of the wrong shape is an error (the vault location must never change
    /// silently).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let mut config: Self = serde_json::from_value(value.clone())
            .map_err(|_| "secrets configuration does not match its schema".to_owned())?;
        if config.data_dir.trim().is_empty() {
            config.data_dir = Self::default().data_dir;
        }
        config.key_file = config
            .key_file
            .map(|path| path.trim().to_owned())
            .filter(|path| !path.is_empty());
        Ok(config)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("secrets config serializes")
    }

    pub fn schema() -> Value {
        serde_json::to_value(schemars::schema_for!(Self)).expect("secrets schema serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_match_the_manifest() {
        assert_eq!(
            SecretsConfig::default().to_json(),
            json!({"data_dir":"data/secrets","key_file":null})
        );
    }

    #[test]
    fn blanks_fall_back_and_bad_shapes_fail() {
        let config = SecretsConfig::from_json(&json!({"data_dir":" ","key_file":"  "})).unwrap();
        assert_eq!(config, SecretsConfig::default());
        let config = SecretsConfig::from_json(&json!({"key_file":"/keys/v.key"})).unwrap();
        assert_eq!(config.key_file.as_deref(), Some("/keys/v.key"));
        assert_eq!(config.data_dir, DEFAULT_DATA_DIR);
        assert!(SecretsConfig::from_json(&json!({"data_dir": 3})).is_err());
    }

    #[test]
    fn schema_describes_both_fields() {
        let schema = SecretsConfig::schema();
        assert!(schema["properties"]["data_dir"].is_object());
        assert!(schema["properties"]["key_file"].is_object());
    }
}
