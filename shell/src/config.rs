use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

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

    #[serde(default)]
    pub allowlist: Vec<String>,

    /// Bypass the allowlist-miss-prompts-user behavior. When `true`, every
    /// non-denylisted command is auto-approved on the classifier path.
    /// Denylist still wins. Default `false` (fail-closed).
    ///
    /// Spec: docs/superpowers/specs/2026-05-15-shell-allowlist-approval-design.md § 6.5
    #[serde(default)]
    pub allow_any: bool,

    #[serde(default)]
    pub denylist_patterns: Vec<String>,

    #[serde(default = "default_max_concurrent_jobs")]
    pub max_concurrent_jobs: usize,

    #[serde(default = "default_job_retention_secs")]
    pub job_retention_secs: u64,

    #[serde(default)]
    pub fs: FsConfig,

    #[serde(default)]
    pub sandbox: SandboxConfig,

    #[serde(default, skip)]
    pub compiled_denylist: Vec<Regex>,
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
            allowlist: Vec::new(),
            allow_any: false,
            denylist_patterns: Vec::new(),
            max_concurrent_jobs: default_max_concurrent_jobs(),
            job_retention_secs: default_job_retention_secs(),
            fs: FsConfig::default(),
            sandbox: SandboxConfig::default(),
            compiled_denylist: Vec::new(),
        }
    }
}

pub fn load_config(path: &str) -> Result<ShellConfig> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path))?;
    let mut cfg: ShellConfig =
        serde_yaml::from_str(&content).with_context(|| format!("parse {}", path))?;
    cfg.compile_denylist()?;
    cfg.validate_fs_jail()?;
    Ok(cfg)
}

impl ShellConfig {
    pub fn compile_denylist(&mut self) -> Result<()> {
        self.compiled_denylist = self
            .denylist_patterns
            .iter()
            .map(|p| Regex::new(p).with_context(|| format!("bad denylist pattern: {}", p)))
            .collect::<Result<Vec<_>>>()?;
        Ok(())
    }

    /// Refuse to start with the host backend exposing the entire filesystem
    /// behind only the (advisory) denylist — the operator must either pin a
    /// host_root jail or explicitly opt in via `fs.allow_unjailed: true`.
    pub fn validate_fs_jail(&self) -> Result<()> {
        if self.fs.host_root.is_none() && !self.fs.allow_unjailed {
            anyhow::bail!(
                "fs.host_root is unset and fs.allow_unjailed is false — refusing to start \
                 unjailed. Set fs.host_root to a directory you intend to expose, or set \
                 fs.allow_unjailed: true to accept that the entire host filesystem is \
                 reachable through shell::fs::* (subject only to the advisory denylist)."
            );
        }
        Ok(())
    }

    /// Returns `Some(reason)` if joined argv matches any compiled denylist regex.
    /// Pure predicate; no allowlist consultation.
    pub fn denylist_hit_reason(&self, argv: &[String]) -> Option<String> {
        let joined = argv.join(" ");
        for re in &self.compiled_denylist {
            if re.is_match(&joined) {
                return Some(format!("command matches denylist: {}", re.as_str()));
            }
        }
        None
    }

    /// Returns `true` if argv[0] (basename or exact path) appears in `allowlist`.
    /// Empty allowlist returns `false` (caller decides what to do with that).
    pub fn allowlist_contains(&self, argv: &[String]) -> bool {
        let Some(cmd) = argv.first() else {
            return false;
        };
        if self.allowlist.is_empty() {
            return false;
        }
        let base = std::path::Path::new(cmd)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(cmd);
        self.allowlist.iter().any(|a| a == base || a == cmd)
    }

    /// Today's combined check, preserved unchanged on the wire for direct
    /// (non-agent) callers. Empty allowlist = open. Denylist always wins.
    /// Agent calls bypass this via the approval-gate classifier path
    /// (see docs/superpowers/specs/2026-05-15-shell-allowlist-approval-design.md § 6.5).
    pub fn is_command_allowed(&self, argv: &[String]) -> Result<(), String> {
        let cmd = argv.first().ok_or_else(|| "empty command".to_string())?;
        if let Some(reason) = self.denylist_hit_reason(argv) {
            return Err(reason);
        }
        if !self.allowlist.is_empty() && !self.allowlist_contains(argv) {
            let base = std::path::Path::new(cmd)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(cmd);
            return Err(format!("command '{}' not in allowlist", base));
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

    fn cfg_with(allow: Vec<&str>, deny: Vec<&str>) -> ShellConfig {
        let mut c = ShellConfig {
            allowlist: allow.into_iter().map(String::from).collect(),
            denylist_patterns: deny.into_iter().map(String::from).collect(),
            ..Default::default()
        };
        c.compile_denylist().unwrap();
        c
    }

    #[test]
    fn test_defaults() {
        let c = ShellConfig::default();
        assert_eq!(c.max_timeout_ms, 30_000);
        assert_eq!(c.default_timeout_ms, 10_000);
        assert!(!c.inherit_env);
        assert_eq!(c.max_concurrent_jobs, 16);
    }

    #[test]
    fn test_allowlist_permits() {
        let c = cfg_with(vec!["ls", "cat"], vec![]);
        assert!(c.is_command_allowed(&["ls".into(), "-la".into()]).is_ok());
    }

    #[test]
    fn test_allowlist_rejects() {
        let c = cfg_with(vec!["ls"], vec![]);
        let err = c
            .is_command_allowed(&["nmap".into()])
            .expect_err("must reject");
        assert!(err.contains("not in allowlist"));
    }

    #[test]
    fn test_allowlist_empty_means_open() {
        let c = cfg_with(vec![], vec![]);
        assert!(c.is_command_allowed(&["anything".into()]).is_ok());
    }

    #[test]
    fn test_allowlist_basename_match() {
        let c = cfg_with(vec!["ls"], vec![]);
        assert!(c
            .is_command_allowed(&["/usr/bin/ls".into(), "-la".into()])
            .is_ok());
    }

    #[test]
    fn test_denylist_blocks() {
        let c = cfg_with(vec![], vec![r"rm\s+-rf\s+/"]);
        let err = c
            .is_command_allowed(&["rm".into(), "-rf".into(), "/".into()])
            .expect_err("must reject");
        assert!(err.contains("denylist"));
    }

    #[test]
    fn test_empty_argv_rejected() {
        let c = ShellConfig::default();
        assert!(c.is_command_allowed(&[]).is_err());
    }

    /// Loads the shipped `config.yaml` and asserts the default allowlist
    /// preserves read-only env inspection (`printenv`) while rejecting the
    /// `env <cmd>` exec-escape. `env` was removed from the default allowlist
    /// because `is_command_allowed` only checks argv[0]; with `env`
    /// allowlisted, `env nmap target` would have argv[0]=="env" and pass.
    /// Loads the shipped `config.yaml` and asserts the default allowlist
    /// preserves read-only env inspection (`printenv`) while rejecting the
    /// `env <cmd>` exec-escape. `env` was removed from the default allowlist
    /// because `is_command_allowed` only checks argv[0]; with `env`
    /// allowlisted, `env nmap target` would have argv[0]=="env" and pass.
    /// Parses the YAML directly (skipping `load_config`'s fs-jail check,
    /// which is unrelated to the allowlist policy under test).
    #[test]
    fn shipped_config_blocks_env_exec_escape() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml");
        let content = fs::read_to_string(path).expect("read config.yaml");
        let mut c: ShellConfig = serde_yaml::from_str(&content).expect("config.yaml parses");
        c.compile_denylist().expect("denylist compiles");
        assert!(c.is_command_allowed(&["printenv".into()]).is_ok());
        let err = c
            .is_command_allowed(&["env".into(), "nmap".into(), "host".into()])
            .expect_err("env <cmd> must be rejected");
        assert!(err.contains("not in allowlist"));
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
allowlist: []
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
    fn missing_fs_section_uses_defaults() {
        let yaml = "allowlist: []\n";
        let c: ShellConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(c.fs.max_read_bytes, 0);
        assert_eq!(c.fs.max_write_bytes, 0);
        assert!(c.sandbox.enabled);
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
