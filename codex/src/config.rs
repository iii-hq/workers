//! Runtime config managed by the `configuration` worker. `config.yaml` is the
//! seed installed as `initial_value` on first registration; the live value from
//! the configuration worker is authoritative thereafter and hot-reloads.
//!
//! `engine_url` is intentionally NOT here — it is bootstrap (you need it to
//! reach the configuration worker), so it stays on the `--url` CLI flag.
//!
//! The event feeds are the fixed trigger types `codex::agent-event` and
//! `codex::raw-event` (see `agent_feed`), so there are no stream-name keys.
//! A stored config that still carries the retired `events_stream` /
//! `raw_events_stream` keys keeps loading: unknown fields are ignored.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Defaults {
    /// Model id used when a run omits one. Empty = the Codex CLI's own default.
    pub model: String,
    /// Codex sandbox mode: read-only | workspace-write | danger-full-access.
    pub sandbox_mode: String,
    /// Codex approval policy: never | on-request | on-failure | untrusted.
    /// Headless callers leave it at never.
    pub approval_policy: String,
    /// Model reasoning effort: minimal | low | medium | high | xhigh.
    /// Empty = the Codex default.
    pub reasoning_effort: String,
    /// Default working directory a turn runs in when a run omits `cwd`.
    /// Empty = the worker's process directory.
    pub cwd: String,
    /// Allow running outside a git repository (skip the repo check).
    pub skip_git_repo_check: bool,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            model: String::new(),
            sandbox_mode: "workspace-write".to_string(),
            approval_policy: "never".to_string(),
            reasoning_effort: String::new(),
            cwd: String::new(),
            skip_git_repo_check: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Config {
    /// Per-turn defaults applied when a `codex::run` payload omits a field.
    pub defaults: Defaults,
    /// Path to the Codex CLI binary. Empty = resolve `codex` on PATH.
    pub codex_executable: String,
    /// Override the API base URL (passed to the SDK as baseUrl). Empty =
    /// default.
    pub base_url: String,
    /// Prepend the iii runtime context to the turn so the agent discovers and
    /// calls engine functions through the `iii` CLI.
    pub iii_context: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            defaults: Defaults::default(),
            codex_executable: String::new(),
            base_url: String::new(),
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
