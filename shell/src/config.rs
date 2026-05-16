use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Shell worker config. Post-T13 this is just execution-runtime tunables —
/// allowlist / denylist / allow_any / compiled regex live in the
/// approval-gate's rules layer, not here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellConfig {
    #[serde(default = "default_max_timeout_ms")]
    pub max_timeout_ms: u64,

    #[serde(default = "default_default_timeout_ms")]
    pub default_timeout_ms: u64,

    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,

    #[serde(default)]
    pub working_dir: Option<PathBuf>,

    #[serde(default)]
    pub inherit_env: bool,

    #[serde(default = "default_allowed_env")]
    pub allowed_env: Vec<String>,

    #[serde(default = "default_max_concurrent_jobs")]
    pub max_concurrent_jobs: usize,

    #[serde(default = "default_job_retention_secs")]
    pub job_retention_secs: u64,

    #[serde(default)]
    pub fs: FsConfig,

    #[serde(default)]
    pub sandbox: SandboxConfig,
}

fn default_max_timeout_ms() -> u64 {
    30_000
}
fn default_default_timeout_ms() -> u64 {
    10_000
}
fn default_max_output_bytes() -> usize {
    1_048_576
}
fn default_allowed_env() -> Vec<String> {
    vec!["PATH", "HOME", "LANG", "LC_ALL", "TERM"]
        .into_iter()
        .map(String::from)
        .collect()
}
fn default_max_concurrent_jobs() -> usize {
    16
}
fn default_job_retention_secs() -> u64 {
    3600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsConfig {
    #[serde(default)]
    pub host_root: Option<PathBuf>,
    /// Operator opt-in for running with `host_root: null`. When false (the
    /// default) the worker refuses to start unjailed — the entire host
    /// filesystem is reachable through `shell::fs::*` aside from the small
    /// denylist, which is rarely what the operator actually wants. Setting
    /// this to true is equivalent to acknowledging that fact (test
    /// harnesses, sandbox-only deployments).
    #[serde(default)]
    pub allow_unjailed: bool,
    #[serde(default = "default_max_read_bytes")]
    pub max_read_bytes: usize,
    #[serde(default = "default_max_write_bytes")]
    pub max_write_bytes: usize,
    #[serde(default)]
    pub denylist_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default = "default_sandbox_enabled")]
    pub enabled: bool,
}

fn default_max_read_bytes() -> usize {
    0
}
fn default_max_write_bytes() -> usize {
    0
}
fn default_sandbox_enabled() -> bool {
    true
}

impl Default for FsConfig {
    fn default() -> Self {
        Self {
            host_root: None,
            allow_unjailed: false,
            max_read_bytes: default_max_read_bytes(),
            max_write_bytes: default_max_write_bytes(),
            denylist_paths: Vec::new(),
        }
    }
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: default_sandbox_enabled(),
        }
    }
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            max_timeout_ms: default_max_timeout_ms(),
            default_timeout_ms: default_default_timeout_ms(),
            max_output_bytes: default_max_output_bytes(),
            working_dir: None,
            inherit_env: false,
            allowed_env: default_allowed_env(),
            max_concurrent_jobs: default_max_concurrent_jobs(),
            job_retention_secs: default_job_retention_secs(),
            fs: FsConfig::default(),
            sandbox: SandboxConfig::default(),
        }
    }
}

pub fn load_config(path: &str) -> Result<ShellConfig> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path))?;
    let cfg: ShellConfig =
        serde_yaml::from_str(&content).with_context(|| format!("parse {}", path))?;
    cfg.validate_fs_jail()?;
    Ok(cfg)
}

impl ShellConfig {
    /// Refuse to start with the host backend exposing the entire filesystem
    /// unjailed — the operator must either pin a host_root jail or
    /// explicitly opt in via `fs.allow_unjailed: true`.
    pub fn validate_fs_jail(&self) -> Result<()> {
        if self.fs.host_root.is_none() && !self.fs.allow_unjailed {
            anyhow::bail!(
                "fs.host_root is unset and fs.allow_unjailed is false — refusing to start \
                 unjailed. Set fs.host_root to a directory you intend to expose, or set \
                 fs.allow_unjailed: true to accept that the entire host filesystem is \
                 reachable through shell::fs::*."
            );
        }
        Ok(())
    }

    pub fn resolve_timeout(&self, requested: Option<u64>) -> u64 {
        let t = requested.unwrap_or(self.default_timeout_ms);
        t.min(self.max_timeout_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let c = ShellConfig::default();
        assert_eq!(c.max_timeout_ms, 30_000);
        assert_eq!(c.default_timeout_ms, 10_000);
        assert!(!c.inherit_env);
        assert_eq!(c.max_concurrent_jobs, 16);
    }

    #[test]
    fn test_resolve_timeout_caps_at_max() {
        let c = ShellConfig::default();
        assert_eq!(c.resolve_timeout(Some(60_000)), 30_000);
        assert_eq!(c.resolve_timeout(Some(5_000)), 5_000);
        assert_eq!(c.resolve_timeout(None), 10_000);
    }

    #[test]
    fn defaults_include_fs_and_sandbox_sections() {
        let c = ShellConfig::default();
        assert_eq!(c.fs.max_read_bytes, 0);
        assert_eq!(c.fs.max_write_bytes, 0);
        assert!(c.sandbox.enabled);
        assert!(c.fs.host_root.is_none());
    }

    #[test]
    fn yaml_with_fs_section_parses() {
        let yaml = r#"
fs:
  host_root: /tmp/shell
  max_read_bytes: 1024
  denylist_paths:
    - /etc
sandbox:
  enabled: false
"#;
        let c: ShellConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            c.fs.host_root.as_deref(),
            Some(std::path::Path::new("/tmp/shell"))
        );
        assert_eq!(c.fs.max_read_bytes, 1024);
        assert!(!c.sandbox.enabled);
        assert_eq!(c.fs.denylist_paths.len(), 1);
    }

    #[test]
    fn validate_fs_jail_rejects_default_unjailed_config() {
        let c = ShellConfig::default();
        let err = c.validate_fs_jail().expect_err("must reject default");
        let msg = format!("{err}");
        assert!(msg.contains("host_root"));
        assert!(msg.contains("allow_unjailed"));
    }

    #[test]
    fn validate_fs_jail_accepts_explicit_opt_in() {
        let mut c = ShellConfig::default();
        c.fs.allow_unjailed = true;
        c.validate_fs_jail().expect("explicit opt-in is valid");
    }

    #[test]
    fn validate_fs_jail_accepts_pinned_host_root() {
        let mut c = ShellConfig::default();
        c.fs.host_root = Some(std::path::PathBuf::from("/tmp/something"));
        c.validate_fs_jail().expect("pinned host_root is valid");
    }
}
