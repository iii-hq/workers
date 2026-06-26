//! Operator-facing runtime configuration — hot-reloadable via the
//! `configuration` worker (see [`crate::configuration`]).
//!
//! M1 covers the Slack **API surface** only: the bot/user tokens, optional
//! scoping, and RPC timeout. The harness-bridge fields (`public_base_url`,
//! `signing_secret`, `default_model`, streaming, …) land in later milestones;
//! re-registering the schema then is safe (it replaces the schema, preserves
//! the stored value).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Required. Slack bot token (`xoxb-`). Env-expandable: `"${SLACK_BOT_TOKEN}"`.
    /// Used for every `slack::*` call except `search.messages`.
    #[serde(default)]
    pub bot_token: String,

    /// Optional Slack user token (`xoxp-`). Required only by `slack::search::messages`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_token: Option<String>,

    /// Optional default channel id (e.g. `C0123ABC`) used as the target for
    /// proactive/broadcast helpers when a call omits one. Reserved for the bridge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_channel: Option<String>,

    /// Optional allowlist of channel ids the inbound bridge will respond in
    /// (empty = all). Reserved for the bridge.
    #[serde(default)]
    pub allowed_channels: Vec<String>,

    /// Optional allowlist of team ids accepted by the inbound bridge
    /// (empty = all). Reserved for the bridge.
    #[serde(default)]
    pub allowed_teams: Vec<String>,

    /// Timeout for Slack Web API and engine RPCs (ms).
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    10_000
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            user_token: None,
            default_channel: None,
            allowed_channels: Vec::new(),
            allowed_teams: Vec::new(),
            timeout_ms: default_timeout_ms(),
        }
    }
}

impl WorkerConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Parse a value already env-expanded by the configuration worker.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(WorkerConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("WorkerConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
            obj.insert("example".into(), WorkerConfig::default().to_json());
        }
        schema
    }

    /// Returns an error when `bot_token` is empty — used at boot and on hot-reload.
    pub fn validate(&self) -> Result<(), String> {
        if self.bot_token.trim().is_empty() {
            return Err("bot_token must be a non-empty string".into());
        }
        Ok(())
    }
}

/// Expand `${NAME}` against the process env (empty when unset). Mirrors the
/// configuration worker's `${VAR}` read-time expansion for the `--config` seed
/// path; the configuration worker handles expansion for stored values.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        if let Some(end) = rest.find('}') {
            let name = &rest[..end];
            out.push_str(&std::env::var(name).unwrap_or_default());
            rest = &rest[end + 1..];
        } else {
            out.push_str("${");
            break;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_from_empty_json() {
        let cfg = WorkerConfig::from_json(&json!({})).unwrap();
        assert_eq!(cfg.timeout_ms, 10_000);
        assert!(cfg.user_token.is_none());
        assert!(cfg.allowed_channels.is_empty());
    }

    #[test]
    fn validate_rejects_empty_token() {
        assert!(WorkerConfig::default().validate().is_err());
    }

    #[test]
    fn validate_accepts_nonempty_token() {
        let cfg = WorkerConfig {
            bot_token: "xoxb-test".into(),
            ..WorkerConfig::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn parses_tokens_and_scoping() {
        let yaml = "bot_token: xoxb-a\nuser_token: xoxp-b\nallowed_channels: [C1, C2]\n";
        let cfg = WorkerConfig::from_yaml(yaml).unwrap();
        assert_eq!(cfg.bot_token, "xoxb-a");
        assert_eq!(cfg.user_token.as_deref(), Some("xoxp-b"));
        assert_eq!(cfg.allowed_channels, vec!["C1", "C2"]);
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(WorkerConfig::from_yaml("bogus_field: 1\n").is_err());
    }

    #[test]
    fn shipped_collect_config_boots_for_interface_collection() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.collect.yaml");
        let cfg = WorkerConfig::from_file(path).expect("config.collect.yaml parses");
        cfg.validate()
            .expect("config.collect.yaml must boot for CI interface collection");
    }
}
