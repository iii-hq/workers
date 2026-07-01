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
    /// Stream that carries the translated AgentEvent frames (what the console
    /// and acp worker render). Grouped by session_id.
    pub events_stream: String,
    /// Stream that carries the raw Codex thread events, verbatim. Grouped by
    /// session_id.
    pub raw_events_stream: String,
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
            events_stream: "agent::events".to_string(),
            raw_events_stream: "codex::events".to_string(),
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
