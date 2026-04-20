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

    #[serde(default)]
    pub denylist_patterns: Vec<String>,

    #[serde(default = "default_max_concurrent_jobs")]
    pub max_concurrent_jobs: usize,

    #[serde(default = "default_job_retention_secs")]
    pub job_retention_secs: u64,

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
            compiled_denylist: Vec::new(),
        }
    }
}

pub fn load_config(path: &str) -> Result<ShellConfig> {
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path))?;
    let mut cfg: ShellConfig =
        serde_yaml::from_str(&content).with_context(|| format!("parse {}", path))?;
    cfg.compile_denylist()?;
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

    pub fn is_command_allowed(&self, argv: &[String]) -> Result<(), String> {
        let cmd = argv
            .first()
            .ok_or_else(|| "empty command".to_string())?
            .clone();

        if !self.allowlist.is_empty() {
            let base = std::path::Path::new(&cmd)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&cmd);
            if !self.allowlist.iter().any(|a| a == base || a == &cmd) {
                return Err(format!("command '{}' not in allowlist", base));
            }
        }

        let joined = argv.join(" ");
        for re in &self.compiled_denylist {
            if re.is_match(&joined) {
                return Err(format!("command matches denylist: {}", re.as_str()));
            }
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

    #[test]
    fn test_resolve_timeout_caps_at_max() {
        let c = ShellConfig::default();
        assert_eq!(c.resolve_timeout(Some(60_000)), 30_000);
        assert_eq!(c.resolve_timeout(Some(5_000)), 5_000);
        assert_eq!(c.resolve_timeout(None), 10_000);
    }
}
