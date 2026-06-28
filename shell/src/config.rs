use anyhow::{Context, Result};
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ShellConfig {
    #[serde(default = "default_max_timeout_ms")]
    pub max_timeout_ms: u64,

    /// Hard cap, in milliseconds, on a HOST background job (`shell::exec_bg`).
    /// `0` (the default) means UNBOUNDED — a host bg job runs until it exits or
    /// `shell::kill` terminates it. This is deliberately separate from
    /// `max_timeout_ms` (which bounds foreground `shell::exec`): background jobs
    /// are how callers run long work (installs, builds, dev servers), so binding
    /// them to the short foreground cap would kill legitimate jobs. Set a
    /// positive value to force-kill runaway bg jobs after that long.
    #[serde(default = "default_max_bg_timeout_ms")]
    pub max_bg_timeout_ms: u64,

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

    /// The folded `code` surface (`coder::*`) config: glob protection
    /// (`non_accessible_globs`), noise excludes (`default_exclude_globs`), and
    /// per-file/response budgets. The code resolver's ROOTS are NOT taken from
    /// here — it uses `fs.host_roots` so there is a single jail config; any
    /// `base_path`/`base_paths` set under `code` is ignored.
    #[serde(default)]
    pub code: crate::code::config::CoderConfig,

    /// One-shot migration marker (D4/T5): set true once the legacy `coder`
    /// config entry has been folded into this value at boot. Persisted in the
    /// stored value so the fold runs exactly once, but hidden from the operator
    /// schema (not a knob anyone edits). PERSISTS (no `skip`) — that is the
    /// whole point of an idempotency marker.
    #[serde(default)]
    #[schemars(skip)]
    pub migrated_from_coder: bool,

    #[serde(default, skip)]
    #[schemars(skip)]
    pub compiled_denylist: Vec<Regex>,
}

fn default_max_timeout_ms() -> u64 {
    30_000
}
fn default_max_bg_timeout_ms() -> u64 {
    // 0 == unbounded: a host bg job is not force-killed by time, only by
    // shell::kill or natural exit. Operators set a positive cap to bound
    // runaway jobs.
    0
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
    /// Legacy single jail root. Honored as a one-entry `host_roots` list.
    /// Setting BOTH `host_root` and `host_roots` is a config error
    /// (`validate_fs_jail`). Prefer `host_roots`.
    #[serde(default)]
    pub host_root: Option<PathBuf>,
    /// Allowed jail roots. The FIRST entry is the PRIMARY root: relative wire
    /// paths and a relative per-call `cwd`/`base_dir` resolve against it.
    /// Absolute paths are accepted when they canonicalize inside ANY listed
    /// root. Empty (and `host_root` unset) means unjailed — refused at boot
    /// unless `allow_unjailed` is true.
    #[serde(default)]
    pub host_roots: Vec<PathBuf>,
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

impl FsConfig {
    /// Effective jail roots, in priority order (index 0 = primary). Returns
    /// `host_roots` when set, else the legacy `host_root` as a one-entry list,
    /// else empty (unjailed). Does NOT canonicalize — that happens once at
    /// backend construction. `validate_fs_jail` rejects setting both keys.
    pub fn roots(&self) -> Vec<PathBuf> {
        if !self.host_roots.is_empty() {
            self.host_roots.clone()
        } else if let Some(r) = &self.host_root {
            vec![r.clone()]
        } else {
            Vec::new()
        }
    }

    /// True when a jail boundary is configured (at least one root).
    pub fn is_jailed(&self) -> bool {
        !self.host_roots.is_empty() || self.host_root.is_some()
    }
}

impl Default for FsConfig {
    fn default() -> Self {
        Self {
            host_root: None,
            host_roots: Vec::new(),
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
            max_bg_timeout_ms: default_max_bg_timeout_ms(),
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
            code: crate::code::config::CoderConfig::default(),
            migrated_from_coder: false,
            compiled_denylist: Vec::new(),
        }
    }
}

impl ShellConfig {
    /// The bootable, zero-config default: seeded as `initial_value` on first
    /// registration and used as the runtime fallback when the stored value is
    /// null, so the worker boots with no config file at all (database-style
    /// zero-config). This is deliberately NOT `Default::default()` — that is
    /// unjailed (`host_root: None`) so an operator config that omits the jail
    /// fails closed. This seed is the shipped permissive dev default: jailed to
    /// `/tmp`, env forwarded, open exec with a catastrophic-only denylist. It is
    /// kept in sync with `config.yaml` by a unit test.
    pub fn seed_default() -> Self {
        Self {
            max_timeout_ms: 120_000,
            inherit_env: true,
            denylist_patterns: vec![
                r"rm\s+-rf\s+/".into(),
                r":\(\)\s*\{\s*:\|".into(),
                "mkfs".into(),
                r"dd\s+if=".into(),
                "shutdown".into(),
                "reboot".into(),
                "/etc/shadow".into(),
            ],
            fs: FsConfig {
                host_root: Some(PathBuf::from("/tmp")),
                max_read_bytes: 16_777_216,
                max_write_bytes: 16_777_216,
                denylist_paths: vec![PathBuf::from("/etc/passwd"), PathBuf::from("/etc/shadow")],
                ..FsConfig::default()
            },
            // D4: secrets are protected out of the box, so a zero-config boot
            // matches the shipped config.yaml's `code.non_accessible_globs`.
            // Both surfaces (coder::* C211, shell::fs::* S215) honor these.
            code: crate::code::config::CoderConfig {
                non_accessible_globs: vec![
                    "**/.env".into(),
                    "**/.env.*".into(),
                    "**/*.pem".into(),
                    "**/*.key".into(),
                    "**/secrets/**".into(),
                ],
                ..Default::default()
            },
            ..Self::default()
        }
    }

    /// Assemble the `CoderConfig` the code `PathResolver` is built from: the
    /// glob/budget settings from the `code` block, but with ROOTS taken from
    /// `fs.host_roots` (the unified jail) — never from `code.base_paths`. This
    /// is what keeps the merge's promise that the operator sets the root once.
    pub fn code_resolver_config(&self) -> crate::code::config::CoderConfig {
        let mut c = self.code.clone();
        c.base_path = None;
        c.base_paths = self.fs.roots();
        c
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
        if self.fs.host_root.is_some() && !self.fs.host_roots.is_empty() {
            anyhow::bail!(
                "both fs.host_root and fs.host_roots are set — set either fs.host_root (legacy \
                 single root) or fs.host_roots (the list form), not both. Keep only fs.host_roots."
            );
        }
        if !self.fs.is_jailed() && !self.fs.allow_unjailed {
            anyhow::bail!(
                "fs.host_root/fs.host_roots are unset and fs.allow_unjailed is false — refusing \
                 to start unjailed. Set fs.host_roots to the directories you intend to expose, or \
                 set fs.allow_unjailed: true to accept that the entire host filesystem is \
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

        // Confinement guard: a command given as a PATH (contains a '/') that
        // canonicalizes to a location INSIDE the writable fs jail is rejected.
        // `shell::fs::write` can plant an executable (0755) under `fs.host_root`,
        // and the basename allowlist check above matches by file_name — so
        // `command: "<host_root>/ls"` would otherwise pass the allowlist and be
        // executed verbatim, a host RCE that bypasses the read-only allowlist.
        // Bare program names (no '/') are PATH-resolved by the OS and stay
        // allowed; legitimate absolute paths OUTSIDE the jail (e.g. /usr/bin/ls)
        // are not writable via shell::fs::write and stay allowed. A path that
        // fails to canonicalize (does not exist) is NOT rejected here — the
        // normal exec spawn surfaces its own not-found error.
        if cmd.contains('/') {
            // Unjailed mode (host_root: null) has NO writable boundary — the
            // whole host filesystem is reachable via shell::fs::write, so an
            // agent can plant `/tmp/ls` and run `command: "/tmp/ls"` (basename
            // `ls` is allowlisted), bypassing the read-only allowlist entirely.
            // There is no path that distinguishes "agent-planted" from "system
            // binary" here, so reject ALL command paths and require a bare,
            // PATH-resolved name. (In jailed mode the check below is precise:
            // only paths inside host_root are rejected.)
            if !self.fs.is_jailed() {
                return Err(format!(
                    "command path '{}' is not allowed when fs is unjailed \
                     (no fs.host_root/fs.host_roots): any host path is writable via \
                     shell::fs::write, so a command path could execute \
                     agent-planted bytes and bypass the allowlist. Use a bare \
                     command name (PATH-resolved).",
                    cmd
                ));
            }
            // Jailed: reject a command path that canonicalizes inside ANY
            // writable root. A binary planted via shell::fs::write under any
            // root would otherwise pass the basename allowlist and execute —
            // host RCE. All roots are writable, so all must be checked.
            if let Ok(canon_cmd) = std::fs::canonicalize(&cmd) {
                for root in self.fs.roots() {
                    if let Ok(canon_root) = std::fs::canonicalize(&root) {
                        if canon_cmd.starts_with(&canon_root) {
                            return Err(format!(
                                "command path '{}' resolves inside the writable fs jail ({}); \
                                 executing files written via shell::fs::write is not allowed. \
                                 Use a bare command name (PATH-resolved) or a path outside the jail.",
                                cmd,
                                root.display()
                            ));
                        }
                    }
                }
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
        // Basename matching for an absolute command path is only meaningful in
        // JAILED mode: an out-of-jail path (not writable via shell::fs::write)
        // is permitted by basename. Unjailed mode rejects all paths outright
        // (see exec_command_path_rejected_when_unjailed), so set a host_root
        // that does NOT contain /usr/bin/ls to exercise the basename contract.
        let mut c = cfg_with(vec!["ls"], vec![]);
        c.fs.host_root =
            Some(std::env::temp_dir().join(format!("shell-basename-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir_all(c.fs.host_root.as_ref().unwrap()).unwrap();
        assert!(c
            .is_command_allowed(&["/usr/bin/ls".into(), "-la".into()])
            .is_ok());
        std::fs::remove_dir_all(c.fs.host_root.as_ref().unwrap()).ok();
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

    /// Loads the shipped `config.yaml` and pins the permissive standard: an
    /// empty allowlist means arbitrary commands are allowed (cargo/git/bash/…),
    /// while the catastrophic-only denylist still trips on host-wrecking
    /// patterns. Parses the YAML directly (skipping the fs-jail check, which is
    /// unrelated to the exec policy under test).
    #[test]
    fn shipped_config_is_open_exec_with_catastrophic_denylist() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml");
        let content = std::fs::read_to_string(path).expect("read config.yaml");
        let mut c: ShellConfig = serde_yaml::from_str(&content).expect("config.yaml parses");
        c.compile_denylist().expect("denylist compiles");
        // Empty allowlist == open: every command a coding agent needs is permitted.
        assert!(
            c.allowlist.is_empty(),
            "shipped allowlist must be open (empty)"
        );
        for cmd in ["cargo", "git", "bash", "make", "node", "python3"] {
            assert!(
                c.is_command_allowed(&[cmd.into()]).is_ok(),
                "open allowlist must permit {cmd}"
            );
        }
        // The previously-blocked exec-escape (`env <cmd>`) is now permitted —
        // the open allowlist is the point of this standard.
        assert!(c.is_command_allowed(&["env".into(), "nmap".into()]).is_ok());
        // The catastrophic denylist is still a live tripwire.
        let err = c
            .is_command_allowed(&["rm".into(), "-rf".into(), "/".into()])
            .expect_err("rm -rf / must still trip the denylist");
        assert!(err.contains("denylist"), "got: {err}");
    }

    /// `seed_default()` is the in-code twin of the shipped `config.yaml` — the
    /// file the registry publishes and that `cargo run` loads. If they drift, a
    /// zero-config boot and a `config.yaml` boot would diverge silently.
    #[test]
    fn seed_default_matches_shipped_config_yaml() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml");
        let content = std::fs::read_to_string(path).expect("read config.yaml");
        let from_file: ShellConfig = serde_yaml::from_str(&content).expect("config.yaml parses");
        assert_eq!(
            from_file.to_json(),
            ShellConfig::seed_default().to_json(),
            "config.yaml and ShellConfig::seed_default() must stay in sync"
        );
    }

    #[test]
    fn exec_command_path_inside_jail_is_rejected() {
        // An agent can plant `<host_root>/ls` (0755) via shell::fs::write; the
        // basename allowlist matches "ls", so without the confinement guard
        // `command: "<host_root>/ls"` would execute that jail-planted file —
        // host RCE. The guard must reject a command path that canonicalizes
        // inside the jail while still permitting bare PATH-resolved names and
        // out-of-jail absolute paths.
        let root = std::env::temp_dir().join(format!("shell-cfg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("ls"), "#!/bin/sh\necho pwned\n").unwrap();

        let mut c = ShellConfig {
            allowlist: vec!["ls".into()],
            ..Default::default()
        };
        c.fs.host_root = Some(root.clone());
        c.compile_denylist().unwrap();

        // The jail-planted path is rejected with a jail-mentioning error.
        let jailed_cmd = root.join("ls").to_string_lossy().to_string();
        let err = c
            .is_command_allowed(&[jailed_cmd])
            .expect_err("jail-planted command must be rejected");
        assert!(err.contains("jail"), "got: {err}");

        // A bare command name stays allowed (PATH-resolved by the OS).
        assert!(c.is_command_allowed(&["ls".into()]).is_ok());

        // An out-of-jail absolute path whose basename is allowlisted stays
        // allowed — it is not writable via shell::fs::write.
        for candidate in ["/bin/cat", "/usr/bin/true"] {
            if std::path::Path::new(candidate).exists() {
                let mut c2 = ShellConfig {
                    allowlist: vec![std::path::Path::new(candidate)
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_string()],
                    ..Default::default()
                };
                c2.fs.host_root = Some(root.clone());
                c2.compile_denylist().unwrap();
                assert!(
                    c2.is_command_allowed(&[candidate.to_string()]).is_ok(),
                    "out-of-jail absolute path {candidate} must be allowed"
                );
            }
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn exec_command_path_rejected_when_unjailed() {
        // Unjailed mode (host_root: null) has no writable boundary: the whole
        // host FS is reachable via shell::fs::write, so ANY command path could
        // execute agent-planted bytes and bypass the allowlist. Reject every
        // path; only bare PATH-resolved names are permitted.
        let mut c = ShellConfig {
            allowlist: vec!["ls".into()],
            ..Default::default()
        };
        c.fs.host_root = None;
        c.fs.allow_unjailed = true;
        c.compile_denylist().unwrap();

        // Even a real system binary is rejected — there is no way to tell it
        // apart from an agent-planted file when nothing confines writes.
        let err = c
            .is_command_allowed(&["/bin/ls".into()])
            .expect_err("command path must be rejected when unjailed");
        assert!(err.contains("unjailed"), "got: {err}");

        // The bare name still resolves via PATH and is allowed.
        assert!(c.is_command_allowed(&["ls".into()]).is_ok());
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
    fn validate_fs_jail_accepts_host_roots_list() {
        let mut c = ShellConfig::default();
        c.fs.host_roots = vec!["/tmp/a".into(), "/tmp/b".into()];
        c.validate_fs_jail().expect("host_roots list is valid");
    }

    #[test]
    fn validate_fs_jail_rejects_both_host_root_and_host_roots() {
        let mut c = ShellConfig::default();
        c.fs.host_root = Some("/tmp/a".into());
        c.fs.host_roots = vec!["/tmp/b".into()];
        let err = c.validate_fs_jail().expect_err("both set must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("both fs.host_root and fs.host_roots"),
            "got: {msg}"
        );
    }

    #[test]
    fn roots_prefers_host_roots_then_legacy_host_root() {
        let mut c = FsConfig::default();
        assert!(c.roots().is_empty(), "unset = unjailed");
        assert!(!c.is_jailed());
        c.host_root = Some("/tmp/legacy".into());
        assert_eq!(c.roots(), vec![std::path::PathBuf::from("/tmp/legacy")]);
        assert!(c.is_jailed());
        c.host_roots = vec!["/tmp/a".into(), "/tmp/b".into()];
        // host_roots wins when both are present (validate_fs_jail rejects that
        // combo at boot, but roots() stays deterministic).
        assert_eq!(
            c.roots(),
            vec![
                std::path::PathBuf::from("/tmp/a"),
                std::path::PathBuf::from("/tmp/b")
            ]
        );
    }

    #[test]
    fn exec_guard_rejects_command_path_inside_any_root() {
        // SECURITY: the planted-binary guard must check EVERY writable root,
        // not just the first. A binary planted via shell::fs::write under the
        // SECOND root must be rejected exactly like one under the primary.
        let root_a = std::env::temp_dir().join(format!("shell-mr-a-{}", uuid::Uuid::new_v4()));
        let root_b = std::env::temp_dir().join(format!("shell-mr-b-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();
        std::fs::write(root_b.join("ls"), "#!/bin/sh\necho pwned\n").unwrap();

        let mut c = ShellConfig {
            allowlist: vec!["ls".into()],
            ..Default::default()
        };
        c.fs.host_roots = vec![root_a.clone(), root_b.clone()];
        c.compile_denylist().unwrap();

        // Planted under the SECOND root → rejected with a jail-mentioning error.
        let planted = root_b.join("ls").to_string_lossy().to_string();
        let err = c
            .is_command_allowed(&[planted])
            .expect_err("a command path inside the 2nd root must be rejected");
        assert!(err.contains("jail"), "got: {err}");

        // A bare name still PATH-resolves and is allowed.
        assert!(c.is_command_allowed(&["ls".into()]).is_ok());

        std::fs::remove_dir_all(&root_a).ok();
        std::fs::remove_dir_all(&root_b).ok();
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
