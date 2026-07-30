//! Operator-facing limits, owned by the `configuration` worker.
//!
//! Every field is a bound, not a behaviour switch: this worker has no jail, no
//! roots and no denylist of its own because it opens no files — reads, writes
//! and process spawns all go through `shell`, so `shell`'s config is the
//! security surface. What is left to tune here is how much work a single call
//! may cost, and all of it hot-reloads.
//!
//! Nothing in this shape is committed: the configuration worker is the source of
//! truth after first boot, and these defaults are what gets seeded when nothing
//! is stored yet. The `config.yaml` next to this crate is an *engine* config for
//! local runs, not a seed — `deny_unknown_fields` below rejects it as one, which
//! `tests::an_engine_config_is_not_a_worker_seed` pins.

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[schemars(example = "example_config")]
pub struct WorkerConfig {
    /// Largest text, in bytes, either side of a diff may be before
    /// `editor::diff` refuses it and answers `truncated: true`.
    #[serde(default = "default_max_diff_bytes")]
    pub max_diff_bytes: usize,

    /// Unchanged lines `editor::diff` keeps around each hunk when the caller
    /// does not say. `editor::git::hunks` is not covered: it defaults to 0, so
    /// its ranges stay exactly the lines that changed.
    #[serde(default = "default_diff_context_lines")]
    pub diff_context_lines: usize,

    /// Default number of rows `editor::find` returns.
    #[serde(default = "default_find_limit")]
    pub find_limit: usize,

    /// Upper bound on paths scanned per `editor::find` call. A workspace larger
    /// than this is ranked on the first N paths and the response says so,
    /// rather than the call growing unboundedly.
    #[serde(default = "default_max_find_candidates")]
    pub max_find_candidates: usize,

    /// Largest file `editor::open` will pull through the read channel.
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: usize,

    /// Matching lines `editor::search` collects before it stops.
    #[serde(default = "default_search_max_matches")]
    pub search_max_matches: u64,

    /// Timeout handed to `shell::exec` for each git invocation.
    #[serde(default = "default_git_timeout_ms")]
    pub git_timeout_ms: u64,
}

fn default_max_diff_bytes() -> usize {
    2_000_000
}
fn default_diff_context_lines() -> usize {
    3
}
fn default_find_limit() -> usize {
    50
}
fn default_max_find_candidates() -> usize {
    50_000
}
fn default_max_file_bytes() -> usize {
    2_000_000
}
fn default_search_max_matches() -> u64 {
    2_000
}
fn default_git_timeout_ms() -> u64 {
    15_000
}

fn example_config() -> WorkerConfig {
    WorkerConfig::default()
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            max_diff_bytes: default_max_diff_bytes(),
            diff_context_lines: default_diff_context_lines(),
            find_limit: default_find_limit(),
            max_find_candidates: default_max_find_candidates(),
            max_file_bytes: default_max_file_bytes(),
            search_max_matches: default_search_max_matches(),
            git_timeout_ms: default_git_timeout_ms(),
        }
    }
}

impl WorkerConfig {
    /// Parse a `--config` seed. `${NAME}` is expanded against the process env
    /// here, because a seed file has not been through the configuration worker.
    pub fn from_yaml(text: &str) -> Result<Self> {
        let expanded = expand_env(text);
        Ok(serde_yml::from_str(&expanded)?)
    }

    pub fn from_file(path: &str) -> Result<Self> {
        Self::from_yaml(&std::fs::read_to_string(path)?)
    }

    /// Parse a value the configuration worker already env-expanded. Do **not**
    /// re-expand: a literal `${…}` in a stored value is intentional by then.
    pub fn from_json(value: &Value) -> std::result::Result<Self, String> {
        serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid editor configuration: {e}"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// JSON Schema published to the configuration worker, with the shipped
    /// defaults attached as the example the console renders.
    pub fn json_schema() -> Value {
        serde_json::to_value(schemars::schema_for!(WorkerConfig)).unwrap_or(Value::Null)
    }
}

/// `${NAME}` → env value, empty when unset. Deliberately minimal: the seed path
/// is a convenience, and the configuration worker owns the richer
/// `${VAR:default}` form.
fn expand_env(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                out.push_str(&std::env::var(name).unwrap_or_default());
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_from_empty_yaml() {
        assert_eq!(
            WorkerConfig::from_yaml("{}").unwrap(),
            WorkerConfig::default()
        );
    }

    #[test]
    fn custom_yaml_overrides_one_field_and_defaults_the_rest() {
        let cfg = WorkerConfig::from_yaml("find_limit: 5").unwrap();
        assert_eq!(cfg.find_limit, 5);
        assert_eq!(cfg.diff_context_lines, default_diff_context_lines());
    }

    /// A typo in a seed file must fail loudly rather than silently defaulting
    /// the field the operator thought they were setting.
    #[test]
    fn unknown_fields_are_rejected() {
        assert!(WorkerConfig::from_yaml("find_limt: 5").is_err());
    }

    /// The committed `config.yaml` is an engine config, not a seed for this
    /// worker, and handing it to `--config` is a mistake the README used to
    /// teach. It fails loudly, which is the behaviour worth keeping.
    #[test]
    fn an_engine_config_is_not_a_worker_seed() {
        assert!(WorkerConfig::from_yaml("workers: []").is_err());
    }

    #[test]
    fn json_round_trips_through_the_configuration_shape() {
        let cfg = WorkerConfig {
            find_limit: 7,
            ..WorkerConfig::default()
        };
        assert_eq!(WorkerConfig::from_json(&cfg.to_json()).unwrap(), cfg);
    }

    #[test]
    fn from_json_reports_a_readable_error() {
        let err = WorkerConfig::from_json(&serde_json::json!({ "find_limit": "lots" }))
            .expect_err("a string is not a limit");
        assert!(err.contains("invalid editor configuration"));
    }

    #[test]
    fn schema_carries_the_defaults_as_an_example() {
        let schema = WorkerConfig::json_schema();
        let rendered = serde_json::to_string(&schema).unwrap();
        assert!(rendered.contains("max_diff_bytes"));
        assert!(rendered.contains("examples"), "console renders the example");
    }

    #[test]
    fn seed_expands_environment_variables() {
        std::env::set_var("EDITOR_TEST_LIMIT", "9");
        let cfg = WorkerConfig::from_yaml("find_limit: ${EDITOR_TEST_LIMIT}").unwrap();
        assert_eq!(cfg.find_limit, 9);
    }

    #[test]
    fn several_placeholders_expand_in_one_pass() {
        std::env::set_var("EDITOR_TEST_FIND_LIMIT", "11");
        std::env::set_var("EDITOR_TEST_CONTEXT", "22");
        let cfg = WorkerConfig::from_yaml(
            "find_limit: ${EDITOR_TEST_FIND_LIMIT}\ndiff_context_lines: ${EDITOR_TEST_CONTEXT}\n",
        )
        .expect("both placeholders resolve");
        assert_eq!((cfg.find_limit, cfg.diff_context_lines), (11, 22));
    }

    /// An unset variable expands to nothing, which leaves the key with no
    /// value at all. That must fail the parse rather than land as a zero — a
    /// `find_limit` of 0 returns nothing and a `max_file_bytes` of 0 truncates
    /// every open, and both would look like the worker was working.
    #[test]
    fn an_unset_variable_fails_the_parse_rather_than_zeroing_a_limit() {
        assert!(
            std::env::var("EDITOR_TEST_DEFINITELY_UNSET").is_err(),
            "the fixture depends on this name being unset"
        );
        assert!(WorkerConfig::from_yaml("find_limit: ${EDITOR_TEST_DEFINITELY_UNSET}").is_err());
    }

    #[test]
    fn an_unterminated_placeholder_is_left_alone() {
        assert_eq!(expand_env("a ${UNCLOSED"), "a ${UNCLOSED");
    }

    /// Every shipped default has to be usable as it stands. A bound of zero
    /// does not mean "unbounded" anywhere in this worker — it means the feature
    /// it bounds returns nothing.
    #[test]
    fn no_shipped_default_is_zero() {
        let d = WorkerConfig::default();
        for (name, value) in [
            ("max_diff_bytes", d.max_diff_bytes as u64),
            ("diff_context_lines", d.diff_context_lines as u64),
            ("find_limit", d.find_limit as u64),
            ("max_find_candidates", d.max_find_candidates as u64),
            ("max_file_bytes", d.max_file_bytes as u64),
            ("search_max_matches", d.search_max_matches),
            ("git_timeout_ms", d.git_timeout_ms),
        ] {
            assert!(
                value > 0,
                "{name} ships as zero, which disables what it bounds"
            );
        }
    }
}
