//! Operator-facing runtime configuration — hot-reloadable via the
//! `configuration` worker (see [`crate::configuration`]).
//!
//! The Slack **API surface** needs only `bot_token`. The **bridge** (inbound
//! events → `harness::send`, streamed replies) additionally needs
//! `signing_secret` + `public_base_url`; when either is missing the worker runs
//! API-surface-only and registers no HTTP routes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Engine HTTP `api_path`s for the bridge ingress routes, and the suffixes
/// appended to `public_base_url` to build the URLs Slack posts to.
pub const EVENTS_API_PATH: &str = "slack/events";
pub const INTERACTIONS_API_PATH: &str = "slack/interactions";

/// Provider + model id pair forwarded to `harness::send`. Provider-agnostic:
/// validated against `router::models::list`, never a baked-in default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelRef {
    pub provider: String,
    pub id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Required. Slack bot token (`xoxb-`). Env-expandable: `"${SLACK_BOT_TOKEN}"`.
    #[serde(default)]
    pub bot_token: String,

    /// Optional Slack user token (`xoxp-`). Required only by `slack::search::messages`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_token: Option<String>,

    /// Optional Slack app-level token (`xapp-`). When set, the inbound bridge runs
    /// in **Socket Mode** (the worker dials out to Slack over a WebSocket; no
    /// public URL, signing secret, or Event Subscriptions URL needed). Takes
    /// precedence over the HTTP/events ingress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_token: Option<String>,

    /// Slack signing secret. Required to enable the inbound bridge (HMAC verify).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_secret: Option<String>,

    /// Public root of the iii engine (e.g. `https://engine.example`). Slack posts
    /// events to `{public_base_url}/slack/events`. Required to enable the bridge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_base_url: Option<String>,

    /// Optional default channel id used as a fallback target for proactive sends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_channel: Option<String>,

    /// Optional allowlist of channel ids the bridge responds in (empty = all).
    #[serde(default)]
    pub allowed_channels: Vec<String>,

    /// Optional allowlist of team ids the bridge accepts (empty = all).
    #[serde(default)]
    pub allowed_teams: Vec<String>,

    /// Bridge: fire a harness turn in channels only on an explicit @bot mention.
    /// DMs always trigger. Default true.
    #[serde(default = "default_true")]
    pub require_mention: bool,

    /// Bridge: on the first mention in a thread, pull prior replies into the
    /// session for context. Default true.
    #[serde(default = "default_true")]
    pub backfill_thread: bool,

    /// Bridge: cap on backfilled/pending context messages (budget guard).
    #[serde(default = "default_backfill_max")]
    pub backfill_max_messages: u32,

    /// Bridge: model for harness sends. OPTIONAL and provider-agnostic — if unset
    /// the first model from `router::models::list` is used; if set it is validated
    /// against that list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<ModelRef>,

    /// Bridge: optional system prompt appended to every harness send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Bridge: globs passed to `harness::send` `options.functions.allow`.
    #[serde(default = "default_functions_allow")]
    pub functions_allow: Vec<String>,

    /// Timeout for Slack Web API and engine RPCs (ms).
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_true() -> bool {
    true
}
fn default_backfill_max() -> u32 {
    50
}
fn default_timeout_ms() -> u64 {
    10_000
}
fn default_functions_allow() -> Vec<String> {
    vec!["slack::*".to_string()]
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            user_token: None,
            app_token: None,
            signing_secret: None,
            public_base_url: None,
            default_channel: None,
            allowed_channels: Vec::new(),
            allowed_teams: Vec::new(),
            require_mention: true,
            backfill_thread: true,
            backfill_max_messages: default_backfill_max(),
            default_model: None,
            system_prompt: None,
            functions_allow: default_functions_allow(),
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

    /// Socket Mode is active when an app-level token (`xapp-`) is configured.
    /// It dials out to Slack, so no public URL or signing secret is needed.
    pub fn socket_enabled(&self) -> bool {
        self.app_token
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
    }

    /// The HTTP/events inbound bridge is enabled when a signing secret and a
    /// public engine URL are configured. Socket Mode takes precedence.
    pub fn http_bridge_enabled(&self) -> bool {
        !self.socket_enabled()
            && self
                .signing_secret
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            && self.public_base_url().is_some()
    }

    /// Any inbound bridge (socket or http) is active.
    pub fn bridge_enabled(&self) -> bool {
        self.socket_enabled() || self.http_bridge_enabled()
    }

    /// Normalized public engine root (no trailing slash), if set.
    pub fn public_base_url(&self) -> Option<String> {
        let raw = self
            .public_base_url
            .as_deref()?
            .trim()
            .trim_end_matches('/');
        (!raw.is_empty()).then(|| raw.to_string())
    }

    pub fn events_url(&self) -> Option<String> {
        self.public_base_url()
            .map(|b| format!("{b}/{EVENTS_API_PATH}"))
    }
}

/// Expand `${NAME}` against the process env (empty when unset).
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
        assert!(cfg.require_mention);
        assert!(cfg.backfill_thread);
        assert_eq!(cfg.backfill_max_messages, 50);
        assert!(!cfg.bridge_enabled());
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
    fn bridge_enabled_requires_secret_and_url() {
        let cfg = WorkerConfig {
            bot_token: "xoxb".into(),
            signing_secret: Some("s".into()),
            public_base_url: Some("https://engine.example/".into()),
            ..WorkerConfig::default()
        };
        assert!(cfg.bridge_enabled());
        assert_eq!(
            cfg.events_url().as_deref(),
            Some("https://engine.example/slack/events")
        );

        let no_url = WorkerConfig {
            public_base_url: None,
            ..cfg.clone()
        };
        assert!(!no_url.bridge_enabled());
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
