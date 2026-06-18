//! Operator-facing runtime configuration — fully hot-reloadable via the
//! `configuration` worker (see [`crate::configuration`]).
//!
//! Telegram update ingress is selected by an `updates` block: a `name`
//! discriminator plus a nested, adapter-specific `config` object (the same
//! shape session-manager uses for its storage `adapter`):
//!
//! ```yaml
//! updates:
//!   name: polling
//!   config:
//!     timeout_seconds: 50
//! ```
//!
//! or
//!
//! ```yaml
//! updates:
//!   name: webhook
//!   config:
//!     url: https://your-engine.example/telegram-bot/webhook
//!     secret: optional-secret
//! ```

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// How much of the agent transcript is mirrored to Telegram.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Verbosity {
    #[default]
    None,
    Minimal,
    High,
    Debug,
}

/// Whether mid-turn messages merge into the running turn or queue locally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SteeringMode {
    #[default]
    Steering,
    Fifo,
}

/// Model reasoning depth forwarded to `harness::send` `options.thinking_level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

/// Streaming transport for assistant output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StreamTransport {
    #[default]
    Auto,
    Draft,
    Edit,
}

#[derive(Serialize, Debug, Clone, PartialEq, JsonSchema)]
pub struct StreamingConfig {
    #[serde(default)]
    pub transport: StreamTransport,
    #[serde(default = "default_draft_id_seed")]
    pub draft_id_seed: i32,
    #[serde(default = "default_draft_throttle_ms")]
    pub draft_throttle_ms: u64,
    /// Settle window (ms) a new-message creation waits before claiming its
    /// ordering slot, so near-simultaneous sibling entries register first
    /// and post in append order.
    #[serde(default = "default_create_settle_ms")]
    pub create_settle_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct StreamingConfigRaw {
    #[serde(default)]
    transport: StreamTransport,
    #[serde(default = "default_draft_id_seed")]
    draft_id_seed: i32,
    #[serde(default = "default_draft_throttle_ms")]
    draft_throttle_ms: u64,
    #[serde(default = "default_create_settle_ms")]
    create_settle_ms: u64,
    #[serde(default, deserialize_with = "deserialize_ignored")]
    use_rich: IgnoredField,
}

impl<'de> Deserialize<'de> for StreamingConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = StreamingConfigRaw::deserialize(deserializer)?;
        Ok(Self {
            transport: raw.transport,
            draft_id_seed: raw.draft_id_seed,
            draft_throttle_ms: raw.draft_throttle_ms,
            create_settle_ms: raw.create_settle_ms,
        })
    }
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            transport: StreamTransport::default(),
            draft_id_seed: default_draft_id_seed(),
            draft_throttle_ms: default_draft_throttle_ms(),
            create_settle_ms: default_create_settle_ms(),
        }
    }
}

/// Provider + model id pair used in harness sends and session metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelRef {
    /// Provider id (e.g. `anthropic`).
    pub provider: String,
    /// Model id (e.g. `claude-sonnet-4`).
    pub id: String,
}

/// Update ingress selection. Adjacently tagged (`name` + `config`) so the
/// wire shape mirrors session-manager and `schemars` emits a `oneOf` the
/// console renders as a variant picker.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(tag = "name", content = "config", rename_all = "snake_case")]
pub enum UpdatesAdapter {
    /// Long-poll `getUpdates` in a background task (default; no public URL needed).
    Polling(PollingConfig),
    /// Telegram POSTs updates to the engine HTTP trigger.
    Webhook(WebhookConfig),
}

impl Default for UpdatesAdapter {
    fn default() -> Self {
        UpdatesAdapter::Polling(PollingConfig::default())
    }
}

/// Settings for the `polling` adapter.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PollingConfig {
    /// `getUpdates` long-poll timeout in seconds (Telegram max: 50).
    #[serde(default = "default_poll_timeout_seconds")]
    pub timeout_seconds: u64,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: default_poll_timeout_seconds(),
        }
    }
}

/// Settings for the `webhook` adapter.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WebhookConfig {
    /// Public engine URL Telegram should POST updates to.
    pub url: String,

    /// Optional `X-Telegram-Bot-Api-Secret-Token` header value for validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// Required. Telegram Bot API token. Env-expandable: `"${TELEGRAM_BOT_TOKEN:}"`.
    #[serde(default)]
    pub bot_token: String,

    /// Update ingress adapter (`polling` or `webhook`) plus its settings.
    #[serde(default)]
    pub updates: UpdatesAdapter,

    /// When set, `/start` skips the model picker and uses this model for new chats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<ModelRef>,

    /// How much transcript detail is sent to Telegram.
    #[serde(default)]
    pub verbosity: Verbosity,

    /// Default model reasoning depth for harness sends (omit = provider default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_thinking_level: Option<ThinkingLevel>,

    /// Assistant output streaming settings.
    #[serde(default)]
    pub streaming: StreamingConfig,

    /// `steering` folds mid-turn messages into the running turn; `fifo` queues locally.
    #[serde(default)]
    pub steering_mode: SteeringMode,

    /// Globs passed to `harness::send` `options.functions.allow`.
    #[serde(default)]
    pub functions_allow: Vec<String>,

    /// Optional system prompt for every harness send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,

    /// Timeout for harness, approval, state, and configuration RPCs (ms).
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

/// Signature of the adapter-defining config fields (see [`WorkerConfig::boot_signature`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BootSignature {
    pub updates: UpdatesAdapter,
}

impl WorkerConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        let raw: WorkerConfigRaw =
            serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))?;
        Ok(raw.into())
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        let raw: WorkerConfigRaw =
            serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))?;
        Ok(raw.into())
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

    /// The updates-adapter-defining fields. The reload path compares this to
    /// decide whether ingress must be restarted (adapter swap) or only the
    /// shared config snapshot changes (list limits, token, etc.).
    pub fn boot_signature(&self) -> BootSignature {
        BootSignature {
            updates: self.updates.clone(),
        }
    }

    pub fn resolve_updates(&self) -> UpdatesAdapter {
        self.updates.clone()
    }

    /// Returns an error when `bot_token` is empty — used at boot and on hot-reload.
    pub fn validate(&self) -> Result<(), String> {
        if self.bot_token.trim().is_empty() {
            return Err("bot_token must be a non-empty string".into());
        }
        Ok(())
    }
}

/// Wire shape accepting deprecated timeout and display fields for migration.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct WorkerConfigRaw {
    #[serde(default)]
    bot_token: String,
    #[serde(default)]
    updates: UpdatesAdapter,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_model: Option<ModelRef>,
    #[serde(default)]
    verbosity: Verbosity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_thinking_level: Option<ThinkingLevel>,
    #[serde(default)]
    streaming: StreamingConfig,
    #[serde(default)]
    steering_mode: SteeringMode,
    #[serde(default)]
    functions_allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    timeout_ms: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    harness_send_timeout_ms: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    approval_timeout_ms: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    state_timeout_ms: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_ignored")]
    thinking_display: IgnoredField,
    #[serde(default, deserialize_with = "deserialize_ignored")]
    use_rich: IgnoredField,
    #[serde(default, deserialize_with = "deserialize_ignored")]
    edit_throttle_ms: IgnoredField,
}

#[derive(Default)]
struct IgnoredField;

fn deserialize_ignored<'de, D>(deserializer: D) -> Result<IgnoredField, D::Error>
where
    D: Deserializer<'de>,
{
    let _ = Value::deserialize(deserializer)?;
    Ok(IgnoredField)
}

fn deserialize_optional_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<u64>::deserialize(deserializer)?)
}

impl From<WorkerConfigRaw> for WorkerConfig {
    fn from(raw: WorkerConfigRaw) -> Self {
        let timeout_ms = raw
            .timeout_ms
            .or(raw.harness_send_timeout_ms)
            .or(raw.approval_timeout_ms)
            .or(raw.state_timeout_ms)
            .unwrap_or_else(default_timeout_ms);
        Self {
            bot_token: raw.bot_token,
            updates: raw.updates,
            default_model: raw.default_model,
            verbosity: raw.verbosity,
            default_thinking_level: raw.default_thinking_level,
            streaming: raw.streaming,
            steering_mode: raw.steering_mode,
            functions_allow: raw.functions_allow,
            system_prompt: raw.system_prompt,
            timeout_ms,
        }
    }
}

fn default_poll_timeout_seconds() -> u64 {
    50
}

fn default_timeout_ms() -> u64 {
    10_000
}

fn default_draft_id_seed() -> i32 {
    1
}

fn default_draft_throttle_ms() -> u64 {
    300
}

fn default_create_settle_ms() -> u64 {
    50
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            updates: UpdatesAdapter::default(),
            default_model: None,
            verbosity: Verbosity::default(),
            default_thinking_level: None,
            streaming: StreamingConfig::default(),
            steering_mode: SteeringMode::default(),
            functions_allow: Vec::new(),
            system_prompt: None,
            timeout_ms: default_timeout_ms(),
        }
    }
}

fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        rest = &rest[start + 2..];
        if let Some(end) = rest.find('}') {
            let name = &rest[..end];
            let val = std::env::var(name).unwrap_or_default();
            out.push_str(&val);
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
        assert_eq!(cfg.verbosity, Verbosity::None);
        assert_eq!(cfg.steering_mode, SteeringMode::Steering);
        assert_eq!(cfg.timeout_ms, 10_000);
        assert!(matches!(cfg.updates, UpdatesAdapter::Polling(_)));
    }

    #[test]
    fn parses_polling_adapter() {
        let yaml = "updates:\n  name: polling\n  config:\n    timeout_seconds: 30";
        let cfg = WorkerConfig::from_yaml(yaml).unwrap();
        let UpdatesAdapter::Polling(p) = cfg.updates else {
            panic!("expected polling");
        };
        assert_eq!(p.timeout_seconds, 30);
    }

    #[test]
    fn parses_webhook_adapter() {
        let yaml =
            "updates:\n  name: webhook\n  config:\n    url: https://x/webhook\n    secret: s";
        let cfg = WorkerConfig::from_yaml(yaml).unwrap();
        let UpdatesAdapter::Webhook(w) = cfg.updates else {
            panic!("expected webhook");
        };
        assert_eq!(w.url, "https://x/webhook");
        assert_eq!(w.secret.as_deref(), Some("s"));
    }

    #[test]
    fn boot_signature_detects_adapter_swap() {
        let polling = WorkerConfig {
            bot_token: "t".into(),
            updates: UpdatesAdapter::Polling(PollingConfig::default()),
            ..WorkerConfig::default()
        };
        let webhook = WorkerConfig {
            updates: UpdatesAdapter::Webhook(WebhookConfig {
                url: "https://x".into(),
                secret: None,
            }),
            ..polling.clone()
        };
        assert_ne!(polling.boot_signature(), webhook.boot_signature());
        assert_eq!(polling.boot_signature(), polling.boot_signature());
    }

    #[test]
    fn validate_rejects_empty_token() {
        assert!(WorkerConfig::default().validate().is_err());
    }

    #[test]
    fn validate_accepts_nonempty_token() {
        let cfg = WorkerConfig {
            bot_token: "123:ABC".into(),
            ..WorkerConfig::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn parses_streaming_config() {
        let yaml = r#"
default_thinking_level: medium
streaming:
  transport: draft
"#;
        let cfg = WorkerConfig::from_yaml(yaml).unwrap();
        assert_eq!(cfg.default_thinking_level, Some(ThinkingLevel::Medium));
        assert_eq!(cfg.streaming.transport, StreamTransport::Draft);
    }

    #[test]
    fn timeout_ms_accepts_deprecated_field_names() {
        let from_harness =
            WorkerConfig::from_json(&json!({ "harness_send_timeout_ms": 8000 })).unwrap();
        assert_eq!(from_harness.timeout_ms, 8000);

        let from_approval =
            WorkerConfig::from_json(&json!({ "approval_timeout_ms": 7000 })).unwrap();
        assert_eq!(from_approval.timeout_ms, 7000);

        let from_state = WorkerConfig::from_json(&json!({ "state_timeout_ms": 6000 })).unwrap();
        assert_eq!(from_state.timeout_ms, 6000);

        let explicit = WorkerConfig::from_json(&json!({ "timeout_ms": 5000 })).unwrap();
        assert_eq!(explicit.timeout_ms, 5000);
    }

    #[test]
    fn ignores_removed_display_fields() {
        let cfg = WorkerConfig::from_json(&json!({
            "thinking_display": "separate",
            "use_rich": true,
            "edit_throttle_ms": 500,
            "streaming": { "use_rich": true }
        }))
        .unwrap();
        assert_eq!(cfg.timeout_ms, 10_000);
    }
}
