//! Runtime config managed by the `configuration` worker. `config.yaml` is the
//! seed installed as `initial_value` on first registration; the live value from
//! the configuration worker is authoritative thereafter and hot-reloads.
//!
//! `engine_url` is intentionally NOT here — it is bootstrap (you need it to
//! reach the configuration worker), so it stays on the `--url` CLI flag.
//!
//! The GitHub token is referenced in the seed as `${GH_TOKEN}`; the
//! configuration worker env-expands it before this worker ever sees the value,
//! so the secret never lives in the repo or on the wire in plaintext form.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Config {
    /// Path to the `gh` binary. Empty = resolve `gh` on PATH.
    pub gh_executable: String,
    /// GitHub token set as GH_TOKEN on every child gh process. Seed as
    /// `${GH_TOKEN}` so the configuration worker env-expands it; empty = the
    /// child inherits the worker's environment (ambient `gh auth login` state
    /// or an already-exported GH_TOKEN).
    pub token: String,
    /// Timeout applied when a call does not pass `timeout_ms`, in ms.
    pub default_timeout_ms: u64,
    /// Upper clamp for any per-call `timeout_ms`, in ms.
    pub max_timeout_ms: u64,
    /// Per-stream (stdout/stderr) capture cap in bytes; output beyond it is
    /// dropped and flagged `stdout_truncated` / `stderr_truncated`.
    pub max_output_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gh_executable: String::new(),
            token: String::new(),
            default_timeout_ms: 30_000,
            max_timeout_ms: 120_000,
            max_output_bytes: 1_048_576,
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

    /// Resolve the gh binary: the configured path, or `gh` on PATH.
    pub fn gh_bin(&self) -> String {
        if self.gh_executable.is_empty() {
            "gh".to_string()
        } else {
            self.gh_executable.clone()
        }
    }

    /// Per-call timeout: the requested value (or the default), clamped to
    /// `max_timeout_ms`.
    pub fn resolve_timeout(&self, requested: Option<u64>) -> u64 {
        requested
            .unwrap_or(self.default_timeout_ms)
            .min(self.max_timeout_ms)
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
