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
    /// Devin API bearer token for the cloud surface. Accepts a personal token
    /// (`apk_...`, which uses the flat v1 API) or an organization service key
    /// (`cog_...`, which uses the v3 API with `org_id` set and `base_url`
    /// pointed at v3). This is separate from the devin CLI's own login
    /// (`devin auth login`), which the CLI surface uses. Seed as
    /// `${DEVIN_API_KEY}` so the configuration worker expands it from the
    /// environment; empty disables the API surface (`devin::session::*` and
    /// `devin::api`) while the CLI surface still works if the CLI is logged in.
    pub api_key: String,
    /// Devin organization id. Leave empty for a personal token: the session
    /// functions then use the flat v1 API (`sessions`, `session/{id}`). Set it
    /// for a service key: the session functions switch to the v3 org-scoped
    /// shape (`organizations/{org_id}/sessions`), and it becomes the required
    /// path segment for pr-review. Set `base_url` to match (v1 vs v3).
    pub org_id: String,
    /// Base URL for the Devin REST API. Default is the v1 endpoint, which
    /// personal tokens use; set it to `https://api.devin.ai/v3` alongside
    /// `org_id` for a service key.
    pub base_url: String,
    /// HTTP request timeout for Devin API calls, in seconds.
    pub request_timeout_secs: u64,
    /// Path to the `devin` CLI binary. Empty = resolve `devin` on PATH.
    pub devin_executable: String,
    /// Extra arguments inserted before `--print --` on every `devin::run`
    /// invocation. The devin CLI permission modes are `auto` (read-only tools),
    /// `accept-edits` (also workspace edits), and `dangerous` (all tools).
    /// Defaults to `dangerous` because only it auto-approves command execution,
    /// so a headless iii-context run can actually run `iii trigger` against the
    /// engine (the point of the CLI surface). Drop to `accept-edits` or `auto`
    /// to restrict the agent. The prompt is always passed after `--`.
    pub cli_extra_args: Vec<String>,
    /// Stream that carries the AgentEvent frames the console and acp worker
    /// render. Grouped by session_id.
    pub events_stream: String,
    /// Stream that carries the raw `devin` CLI stdout lines, verbatim. Grouped
    /// by session_id.
    pub raw_events_stream: String,
    /// Prepend the iii runtime context to the first prompt of a `devin::run`
    /// session so the agent discovers and calls engine functions through the
    /// `iii` CLI, the same as the grok and codex workers. Defaults on for
    /// parity; the context is most useful when the CLI can reach the engine.
    /// Turn it off per turn with `iii_context: false` or globally here.
    pub iii_context: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            org_id: String::new(),
            base_url: "https://api.devin.ai/v1".to_string(),
            request_timeout_secs: 120,
            devin_executable: String::new(),
            cli_extra_args: vec!["--permission-mode".to_string(), "dangerous".to_string()],
            events_stream: "agent::events".to_string(),
            raw_events_stream: "devin::events".to_string(),
            iii_context: true,
        }
    }
}

impl Config {
    /// Load the seed from a YAML file, expanding `${NAME}` against the process
    /// environment first (an unset var expands to empty). Missing file yields
    /// defaults; a parse error propagates so a typo fails the worker fast.
    pub fn load(path: &str) -> anyhow::Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_yaml::from_str(&expand_env(&text))?),
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

/// Expand `${NAME}` occurrences against the process environment. An unset
/// variable expands to empty (with a warning); a `${` without a closing `}`
/// is left verbatim.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => tracing::warn!(var = %name, "config references undefined env var"),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}
