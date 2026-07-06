//! Runtime config managed by the `configuration` worker. `config.yaml` is the
//! seed installed as `initial_value` on first registration; the live value from
//! the configuration worker is authoritative thereafter and hot-reloads.
//!
//! `engine_url` is intentionally NOT here — it is bootstrap (you need it to
//! reach the configuration worker), so it stays on the `--url` CLI flag.
//!
//! The Devin API key is referenced in the seed as `${DEVIN_API_KEY}`; the
//! configuration worker env-expands it before this worker ever sees the value,
//! so the secret never lives in the repo or on the wire in plaintext form.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Config {
    /// Devin API bearer token. Seed as `${DEVIN_API_KEY}` so the configuration
    /// worker expands it from the environment; empty disables the API surface
    /// (`devin::session::*` and `devin::api`) while the CLI surface still works
    /// if the local `devin` binary is already authenticated.
    pub api_key: String,
    /// Devin organization id. Required for the organization-scoped v3 endpoints
    /// (sessions, pr-reviews, code-scan remediation) because it is a path
    /// segment, e.g. `/v3/organizations/{org_id}/sessions`. Empty leaves those
    /// functions returning a clear "org_id not configured" error while the
    /// passthrough and CLI surface still work.
    pub org_id: String,
    /// Base URL for the Devin REST API. Default is the v3 endpoint.
    pub base_url: String,
    /// HTTP request timeout for Devin API calls, in seconds.
    pub request_timeout_secs: u64,
    /// Path to the `devin` CLI binary. Empty = resolve `devin` on PATH.
    pub devin_executable: String,
    /// Extra arguments inserted before the `-- <prompt>` separator on every
    /// `devin::run` invocation (e.g. a workspace flag). The prompt is always
    /// passed after `--`.
    pub cli_extra_args: Vec<String>,
    /// Stream that carries the AgentEvent frames the console and acp worker
    /// render. Grouped by session_id.
    pub events_stream: String,
    /// Stream that carries the raw `devin` CLI stdout lines, verbatim. Grouped
    /// by session_id.
    pub raw_events_stream: String,
    /// Prepend the iii runtime context to the first prompt of a `devin::run`
    /// session so the agent discovers and calls engine functions through the
    /// `iii` CLI. Devin normally runs in its own cloud VM without the `iii` CLI
    /// on PATH, so this defaults off; enable it only when the CLI runs locally
    /// against a reachable engine.
    pub iii_context: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            org_id: String::new(),
            base_url: "https://api.devin.ai/v3".to_string(),
            request_timeout_secs: 120,
            devin_executable: String::new(),
            cli_extra_args: Vec::new(),
            events_stream: "agent::events".to_string(),
            raw_events_stream: "devin::events".to_string(),
            iii_context: false,
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

    /// Resolve the CLI binary: the configured path, or `devin` on PATH.
    pub fn devin_bin(&self) -> String {
        if self.devin_executable.is_empty() {
            "devin".to_string()
        } else {
            self.devin_executable.clone()
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
