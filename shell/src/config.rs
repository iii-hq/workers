use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Shell worker config.
///
/// Argv-level policy (`allowlist` + `denylist_patterns`) is **optional** and
/// defaults to empty. In the standard playground/harness deployment the
/// approval-gate sits in front of shell and is the sole authority on which
/// commands run — leave both empty and the gate decides. Operators running
/// the shell worker standalone (no gate upstream) can populate either list
/// to get a defense-in-depth floor evaluated inside `shell::exec` /
/// `shell::exec_bg` before the command is spawned. Semantics:
///
/// | allowlist | denylist_patterns | behavior                                 |
/// | --------- | ----------------- | ---------------------------------------- |
/// | empty     | empty             | pass-through (default; gate decides)     |
/// | empty     | set               | denylist-only mode                       |
/// | set       | empty             | allowlist upper bound (pre-T13 semantics)|
/// | set       | set               | denylist wins, then allowlist            |
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

    /// Optional argv-prefix allowlist. Empty (default) = no upper bound from
    /// this layer (the approval-gate, if present, is authoritative). Non-empty
    /// = only matching argv are permitted at the shell boundary. Matching is
    /// arity-aware via [`crate::arity::prefix_matches`], so `"git"` matches
    /// `git <subcommand>` but not `git-lfs`. Full-path argv heads
    /// (`/usr/bin/ls`) are normalized to basenames before matching.
    #[serde(default)]
    pub allowlist: Vec<String>,

    /// Optional regex denylist evaluated against the joined argv. Empty
    /// (default) = nothing rejected at this layer. Hits short-circuit before
    /// the allowlist check, so a denylisted argv is rejected even if it
    /// otherwise satisfies an allowlist entry.
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

    /// Returns `true` if the arity-aware prefix of `argv` matches any
    /// entry in `allowlist`. Entries can be single tokens (`"ls"`) or
    /// multi-token prefixes (`"git checkout"`, `"npm run dev"`); the
    /// match is token-aligned via [`crate::arity::prefix_matches`]
    /// so `"git"` matches argv beginning with `git <subcommand>` but
    /// not `git-lfs`. Full-path argv heads (e.g. `/usr/bin/ls`) are
    /// normalized to their basename before matching. Empty allowlist
    /// returns `false` (caller decides what to do with that).
    pub fn allowlist_contains(&self, argv: &[String]) -> bool {
        if argv.is_empty() || self.allowlist.is_empty() {
            return false;
        }
        self.allowlist
            .iter()
            .any(|entry| crate::arity::prefix_matches(argv, entry))
    }

    /// Optional shell-side argv policy check. Both lists empty (default) =
    /// pass-through, gate is sole authority. Denylist wins over allowlist.
    /// Empty allowlist with a non-empty denylist = denylist-only mode.
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
        assert!(c.allowlist.is_empty());
        assert!(c.denylist_patterns.is_empty());
        assert!(c.compiled_denylist.is_empty());
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
    fn allowlist_arity_single_token_entry_matches_subcommand_argv() {
        let c = cfg_with(vec!["git"], vec![]);
        assert!(c
            .is_command_allowed(&["git".into(), "checkout".into(), "main".into()])
            .is_ok());
    }

    #[test]
    fn allowlist_arity_multi_token_entry_matches() {
        let c = cfg_with(vec!["git checkout"], vec![]);
        assert!(c
            .is_command_allowed(&["git".into(), "checkout".into(), "main".into()])
            .is_ok());
        let err = c
            .is_command_allowed(&["git".into(), "push".into()])
            .expect_err("git push must be rejected when only git checkout is allowed");
        assert!(err.contains("allowlist"));
    }

    #[test]
    fn allowlist_arity_npm_run_dev_three_token_entry() {
        let c = cfg_with(vec!["npm run dev"], vec![]);
        assert!(c
            .is_command_allowed(&["npm".into(), "run".into(), "dev".into(), "--watch".into()])
            .is_ok());
        let err = c
            .is_command_allowed(&["npm".into(), "run".into(), "build".into()])
            .expect_err("npm run build must be rejected when only npm run dev is allowed");
        assert!(err.contains("allowlist"));
    }

    #[test]
    fn allowlist_arity_does_not_collide_on_hyphenated_token() {
        let c = cfg_with(vec!["git"], vec![]);
        let err = c
            .is_command_allowed(&["git-lfs".into(), "push".into()])
            .expect_err("git-lfs must not match an allowlist entry of 'git'");
        assert!(err.contains("allowlist"));
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
    fn denylist_wins_over_allowlist() {
        let c = cfg_with(vec!["rm"], vec![r"rm\s+-rf\s+/"]);
        let err = c
            .is_command_allowed(&["rm".into(), "-rf".into(), "/".into()])
            .expect_err("denylist must take precedence over allowlist hit");
        assert!(err.contains("denylist"));
    }

    #[test]
    fn test_empty_argv_rejected() {
        let c = ShellConfig::default();
        assert!(c.is_command_allowed(&[]).is_err());
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
    fn yaml_with_allow_deny_section_parses() {
        let yaml = r#"
allowlist:
  - ls
  - "git checkout"
denylist_patterns:
  - "rm\\s+-rf\\s+/"
fs:
  allow_unjailed: true
"#;
        let mut c: ShellConfig = serde_yaml::from_str(yaml).unwrap();
        c.compile_denylist().unwrap();
        assert_eq!(c.allowlist.len(), 2);
        assert_eq!(c.denylist_patterns.len(), 1);
        assert_eq!(c.compiled_denylist.len(), 1);
        assert!(c
            .is_command_allowed(&["git".into(), "checkout".into(), "main".into()])
            .is_ok());
        assert!(c.is_command_allowed(&["curl".into()]).is_err());
    }

    /// Regression pin on the shipped `shell/config.yaml`: ships with empty
    /// allow/deny lists so the harness/playground flow stays pass-through
    /// (gate is sole authority). Standalone operators opt in by populating.
    #[test]
    fn shipped_config_is_passthrough_by_default() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml");
        let content = fs::read_to_string(path).expect("read config.yaml");
        let mut c: ShellConfig = serde_yaml::from_str(&content).expect("config.yaml parses");
        c.compile_denylist().expect("denylist compiles");
        assert!(c.allowlist.is_empty(), "shipped config must ship empty allowlist");
        assert!(
            c.denylist_patterns.is_empty(),
            "shipped config must ship empty denylist_patterns"
        );
        // Pass-through behavior: any non-empty argv runs at the shell layer.
        assert!(c.is_command_allowed(&["curl".into()]).is_ok());
        assert!(c
            .is_command_allowed(&["env".into(), "nmap".into(), "host".into()])
            .is_ok());
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
