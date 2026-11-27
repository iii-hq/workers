//! Runtime config managed by the `configuration` worker. `config.yaml` is the
//! seed installed as `initial_value` on first registration; the live value is
//! authoritative thereafter and hot-reloads. The engine URL is bootstrap and
//! stays on the `--url` flag.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    /// The `gh pr merge` flag.
    pub fn flag(self) -> &'static str {
        match self {
            Self::Merge => "--merge",
            Self::Squash => "--squash",
            Self::Rebase => "--rebase",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Config {
    /// The console. Calls go to its functions over `<cloud_url>/_iii/ws`
    /// (`<cloud_url>/api/v1` while that socket is down), the device page is
    /// `<cloud_url>/device`. A change signs out.
    pub cloud_url: String,
    /// The `iii` binary the run here uses. It must report the cloud's engine version.
    pub iii_bin: String,
    /// `merge`, `squash` or `rebase`: the `gh pr merge` flag.
    pub merge_method: MergeMethod,
    /// The local Postgres image for a database resource. pgvector is in the
    /// default so `CREATE EXTENSION vector` works here as it does in the cloud.
    pub postgres_image: String,
    /// The whole run here, from install to healthcheck, in seconds.
    pub run_timeout_s: u64,
    /// Records and run logs, relative to `III_COMPOSE_DIR`.
    pub data_dir: String,
    /// A check the person starts on the page hands the agent the problems that
    /// are its to fix, when it ends not ready.
    pub auto_fix: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cloud_url: "https://cloud.iii.dev".into(),
            iii_bin: "iii".into(),
            merge_method: MergeMethod::Squash,
            postgres_image: "pgvector/pgvector:pg17".into(),
            run_timeout_s: 900,
            data_dir: "data/iii-cloud".into(),
            auto_fix: true,
        }
    }
}

impl Config {
    /// Load the seed from a YAML file. A missing file yields defaults; a parse
    /// error propagates so a typo fails the worker fast.
    pub fn load(path: &str) -> anyhow::Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_yaml_ng::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// `<cloud_url>/api/v1` without a doubled slash.
    pub fn api_base(&self) -> String {
        format!("{}/api/v1", self.cloud_url.trim_end_matches('/'))
    }

    /// Where the records and run logs live: absolute, or under `III_COMPOSE_DIR`.
    pub fn data_path(&self) -> PathBuf {
        let dir = PathBuf::from(&self.data_dir);
        if dir.is_absolute() {
            return dir;
        }
        std::env::var_os("III_COMPOSE_DIR")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(dir)
    }

    pub fn json_schema() -> Value {
        let root = schemars::r#gen::SchemaGenerator::default().into_root_schema_for::<Config>();
        serde_json::to_value(root).expect("config schema serializes")
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("config serializes")
    }

    pub fn from_json(value: &Value) -> anyhow::Result<Config> {
        let config: Self = serde_json::from_value(value.clone())?;
        if config.cloud_url.trim().is_empty() {
            anyhow::bail!("cloud_url must not be empty");
        }
        if config.run_timeout_s == 0 {
            anyhow::bail!("run_timeout_s must be above 0");
        }
        Ok(config)
    }
}
