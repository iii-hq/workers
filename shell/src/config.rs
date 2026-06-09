use anyhow::{Context, Result};
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

    /// ADVISORY ONLY. Regular expressions matched against the whole command
    /// line (`argv.join(" ")`). A match rejects the exec, but this is a
    /// best-effort guardrail, NOT the security boundary — the sandbox backend
    /// is. Do not rely on it to contain untrusted input: regexes over a joined
    /// argv are trivially evadable (quoting, env indirection, alternate paths).
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
    #[schemars(skip)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
    /// Permit setuid/setgid/sticky bits (the top octal digit, `mode & 0o7000`)
    /// in mkdir/chmod/write modes. Default false: a chmod to e.g. `4755`
    /// (setuid) is a privilege-escalation primitive when the worker runs as
    /// root, so the top bits are rejected with S210 unless an operator
    /// explicitly opts in here.
    #[serde(default)]
    pub allow_special_bits: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
            allow_special_bits: false,
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
                // Append the permitted commands so an agent can self-correct
                // (mirrors COMMAND_ARRAY_HINT in functions/types.rs). The
                // allowlist is the policy the caller must comply with, not a
                // secret, so list it in full.
                return Err(format!(
                    "command '{}' not in allowlist; allowed: [{}]",
                    base,
                    self.allowlist.join(", ")
                ));
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

    /// Parse a YAML seed (no denylist compile, no jail validation — those run
    /// in `configuration::build_runtime`).
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        serde_yaml::from_str(yaml).map_err(|e| format!("yaml parse: {e}"))
    }

    /// Load a YAML seed file. Used only for the optional `--config` seed.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Deserialize the live value fetched from the configuration worker.
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))
    }

    /// Serialize for `initial_value` when registering with the configuration worker.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("ShellConfig serializes")
    }

    /// JSON Schema registered with the configuration worker so operators get
    /// a typed editing surface.
    pub fn json_schema() -> serde_json::Value {
        let root = schemars::gen::SchemaGenerator::default().into_root_schema_for::<ShellConfig>();
        serde_json::to_value(root).expect("ShellConfig JSON Schema serializes")
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
    fn test_allowlist_rejection_lists_allowed_commands() {
        // The rejection must name the permitted commands so an agent can
        // self-correct without trial-and-error against the policy.
        let c = cfg_with(vec!["ls", "cat", "grep"], vec![]);
        let err = c
            .is_command_allowed(&["nmap".into()])
            .expect_err("must reject");
        assert!(err.contains("ls"), "got: {err}");
        assert!(err.contains("cat"), "got: {err}");
        assert!(err.contains("grep"), "got: {err}");
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
    fn test_allowlisted_command_still_blocked_by_denylist() {
        // Both lists non-empty: `tar` is allowlisted (passes argv[0] check)
        // but the argv string matches a denylist pattern, so it is rejected.
        // The denylist is the second gate and must apply even to allowlisted
        // commands.
        let c = cfg_with(vec!["tar", "ls"], vec![r"--checkpoint-action"]);
        // Sanity: a benign allowlisted invocation passes.
        assert!(c
            .is_command_allowed(&["tar".into(), "-czf".into(), "out.tgz".into()])
            .is_ok());
        // The dangerous allowlisted invocation is blocked by the denylist.
        let err = c
            .is_command_allowed(&[
                "tar".into(),
                "--checkpoint-action=exec=sh".into(),
                "x".into(),
            ])
            .expect_err("denylist must override allowlist");
        assert!(err.contains("denylist"), "got: {err}");
    }

    /// Loads the shipped `config.yaml` and asserts the default allowlist
    /// preserves read-only env inspection (`printenv`) while rejecting the
    /// `env <cmd>` exec-escape. `env` was removed from the default allowlist
    /// because `is_command_allowed` only checks argv[0]; with `env`
    /// allowlisted, `env nmap target` would have argv[0]=="env" and pass.
    /// Parses the YAML directly (skipping the fs-jail check,
    /// which is unrelated to the allowlist policy under test).
    #[test]
    fn shipped_config_blocks_env_exec_escape() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml");
        let content = std::fs::read_to_string(path).expect("read config.yaml");
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

    #[test]
    fn json_schema_has_top_level_keys() {
        let schema = ShellConfig::json_schema();
        let obj = schema.as_object().expect("schema is an object");
        assert!(obj.contains_key("properties"));
        let props = obj["properties"].as_object().expect("properties object");
        assert!(props.contains_key("allowlist"));
        assert!(props.contains_key("fs"));
        // compiled_denylist must NOT leak into the published schema.
        assert!(!props.contains_key("compiled_denylist"));
    }

    #[test]
    fn to_json_from_json_round_trips() {
        let mut c = ShellConfig::default();
        c.fs.host_root = Some(std::path::PathBuf::from("/tmp/shell"));
        c.allowlist = vec!["ls".into(), "cat".into()];
        let v = c.to_json();
        let back = ShellConfig::from_json(&v).expect("from_json round-trips");
        assert_eq!(back.allowlist, c.allowlist);
        assert_eq!(back.fs.host_root, c.fs.host_root);
    }

    #[test]
    fn from_yaml_parses_seed() {
        let c = ShellConfig::from_yaml("allowlist: [ls]\nfs:\n  host_root: /tmp/x\n")
            .expect("seed yaml parses");
        assert_eq!(c.allowlist, vec!["ls".to_string()]);
    }
}
