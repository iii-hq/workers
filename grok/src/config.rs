//! Runtime config managed by the `configuration` worker. `config.yaml` is the
//! seed installed as `initial_value` on first registration; the live value from
//! the configuration worker is authoritative thereafter and hot-reloads.
//!
//! `engine_url` is intentionally NOT here — it is bootstrap (you need it to
//! reach the configuration worker), so it stays on the `--url` CLI flag.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Defaults {
    /// Model id used when a run omits one. Empty = the Grok CLI's own default
    /// (e.g. grok-build-0.1).
    pub model: String,
    /// Default working directory a turn runs in when a run omits `cwd`.
    /// Empty = the worker's process directory.
    pub cwd: String,
    /// Auto-approve tool/command execution on headless turns so they never
    /// block on an interactive approval prompt.
    pub always_approve: bool,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            model: String::new(),
            cwd: String::new(),
            always_approve: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Config {
    /// Per-turn defaults applied when a `grok::run` payload omits a field.
    pub defaults: Defaults,
    /// Stream that carries the translated AgentEvent frames (what the console
    /// and acp worker render). Grouped by session_id.
    pub events_stream: String,
    /// Stream that carries the raw Grok streaming-json events, verbatim.
    /// Grouped by session_id.
    pub raw_events_stream: String,
    /// Path to the Grok CLI binary. Empty = resolve `grok` on PATH.
    pub grok_executable: String,
    /// Prepend the iii runtime context to the first prompt of a session so the
    /// agent discovers and calls engine functions through the `iii` CLI.
    pub iii_context: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            defaults: Defaults::default(),
            events_stream: "agent::events".to_string(),
            raw_events_stream: "grok::events".to_string(),
            grok_executable: String::new(),
            iii_context: true,
        }
    }
}

impl Config {
    /// Load the seed from a YAML file. Missing file yields defaults; a parse
    /// error propagates so a typo fails the worker fast.
    pub fn load(path: &str) -> anyhow::Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_yaml::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn json_schema() -> Value {
        let root = schemars::gen::SchemaGenerator::default().into_root_schema_for::<Config>();
        serde_json::to_value(root).expect("config schema serializes")
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("config serializes")
    }

    /// Parse a value fetched from the configuration worker (already env-expanded
    /// by the worker; this does not re-expand).
    pub fn from_json(value: &Value) -> anyhow::Result<Config> {
        Ok(serde_json::from_value(value.clone())?)
    }
}
