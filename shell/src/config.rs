use anyhow::{Context, Result};
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Root configuration for the shell worker: exec policy (timeouts, output
/// caps, allow/denylist, env forwarding), the fs jail, the sandbox toggle,
/// and the folded `coder::*` code surface. Stored in the `configuration`
/// worker under id `shell` and hot-reloaded on change.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ShellConfig {
    /// Hard cap, in milliseconds, on a foreground `shell::exec` call. A
    /// per-call `timeout_ms` above this is clamped down to it. Default 30000
    /// (30s); the shipped dev seed raises it to 120000 so real builds/tests
    /// are not reaped.
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

    /// Timeout, in milliseconds, applied to a foreground `shell::exec` call
    /// when the caller omits `timeout_ms`. Always clamped to `max_timeout_ms`.
    /// Default 10000 (10s).
    #[serde(default = "default_default_timeout_ms")]
    pub default_timeout_ms: u64,

    /// Per-stream cap, in bytes, on captured stdout and stderr. Output beyond
    /// the cap is dropped and the response flags `stdout_truncated` /
    /// `stderr_truncated`. Default 1048576 (1 MiB).
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,

    /// Default working directory for spawned commands. `null` (the default)
    /// runs children in the worker's own cwd. A per-call `cwd` or a
    /// harness-stamped session `base_dir` overrides it for that one call.
    #[serde(default)]
    pub working_dir: Option<PathBuf>,

    /// Environment policy for spawned commands (host target): whether the
    /// worker's env is forwarded to children, and which keys are
    /// forwardable/settable. See the field docs on `EnvConfig`.
    #[serde(default)]
    pub env: EnvConfig,

    /// Command allowlist by argv[0] basename. EMPTY (the default) means every
    /// command is allowed — deliberate for coding agents that need arbitrary
    /// build/test/VCS tooling; the security boundary is the fs jail and the
    /// sandbox backend, not this list. A non-empty list flips exec to
    /// deny-by-default: only the listed basenames run.
    #[serde(default)]
    pub allowlist: Vec<String>,

    /// ADVISORY ONLY. Regular expressions matched against the whole command
    /// line (`argv.join(" ")`). A match rejects the exec, but this is a
    /// best-effort guardrail, NOT the security boundary — the sandbox backend
    /// is. Do not rely on it to contain untrusted input: regexes over a joined
    /// argv are trivially evadable (quoting, env indirection, alternate paths).
    #[serde(default)]
    pub denylist_patterns: Vec<String>,

    /// Maximum number of live background jobs (`shell::exec_bg`). A spawn past
    /// the cap is rejected until a job finishes or is killed. Default 16.
    #[serde(default = "default_max_concurrent_jobs")]
    pub max_concurrent_jobs: usize,

    /// How long, in seconds, a FINISHED job record (status, exit code,
    /// captured output) stays queryable via `shell::status` before a
    /// background reaper evicts it. Default 3600 (1h).
    #[serde(default = "default_job_retention_secs")]
    pub job_retention_secs: u64,

    /// The filesystem jail shared by `shell::fs::*`, `coder::*`, and per-call
    /// exec `cwd` confinement. See the field docs on `FsConfig`.
    #[serde(default)]
    pub fs: FsConfig,

    /// The `iii-sandbox` microVM backend toggle for sandbox-targeted calls.
    #[serde(default)]
    pub sandbox: SandboxConfig,

    /// The folded `code` surface (`coder::*`) config: glob protection
    /// (`non_accessible_globs`), noise excludes (`default_exclude_globs`), and
    /// per-file/response budgets. The code resolver's ROOTS are NOT taken from
    /// here — it uses `fs.host_roots` so there is a single jail config
    /// (`code.base_path`/`base_paths` were removed from the schema in 0.7.0;
    /// stored values still carrying them are ignored — they never had an
    /// effect).
    #[serde(default)]
    pub code: crate::code::config::CoderConfig,

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
fn default_max_concurrent_jobs() -> usize {
    16
}
fn default_job_retention_secs() -> u64 {
    3600
}

/// Top-level keys removed in 0.7.0 and where they moved. serde ignores
/// unknown fields, so a 0.6.x config carrying `inherit_env: true` would
/// otherwise parse into `env.inherit = false` — silently disabling env
/// forwarding. Fail closed with a migration hint instead.
const REMOVED_TOP_LEVEL_KEYS: &[(&str, &str)] =
    &[("inherit_env", "env.inherit"), ("allowed_env", "env.allow")];

fn check_removed_keys<'a>(keys: impl Iterator<Item = &'a str>) -> Result<(), String> {
    let hits: Vec<String> = keys
        .filter_map(|k| {
            REMOVED_TOP_LEVEL_KEYS
                .iter()
                .find(|(old, _)| *old == k)
                .map(|(old, new)| format!("`{old}` -> `{new}`"))
        })
        .collect();
    if hits.is_empty() {
        return Ok(());
    }
    Err(format!(
        "config keys removed in 0.7.0: {}. Nest them under `env:` (e.g. env: {{ inherit: true, \
         allow: [PATH, HOME] }}). If this is the stored value, rewrite it via \
         configuration::set (id: shell).",
        hits.join(", ")
    ))
}

/// Nested `fs` key removed in 0.7.0: `host_root`, the 0.6.x single-root
/// alias. serde ignores unknown fields, so a config still carrying it would
/// otherwise parse with NO jail configured — and either fail the jail check
/// with a message that never names the stale key, or (with `allow_unjailed`)
/// silently boot unjailed. Same fail-closed treatment as the top-level keys.
fn check_removed_fs_keys<'a>(mut keys: impl Iterator<Item = &'a str>) -> Result<(), String> {
    if keys.any(|k| k == "host_root") {
        return Err(
            "config key removed in 0.7.0: `fs.host_root` -> `fs.host_roots` (one-entry list). \
             Set fs: { host_roots: [<path>] }. If this is the stored value, rewrite it via \
             configuration::set (id: shell)."
                .to_string(),
        );
    }
    Ok(())
}

/// Environment policy for spawned commands (host target). Replaces the
/// 0.6.x top-level `inherit_env` / `allowed_env` keys (renamed in 0.7.0;
/// the old keys are rejected at parse with a migration hint).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EnvConfig {
    /// Forward the worker's ENTIRE environment to child processes. Toolchains
    /// (cargo, rustup, git, node) need this to find PATH/HOME/CARGO_HOME.
    /// WARNING: it also forwards any secrets in the worker's env to every
    /// command — run the worker with a clean environment if that matters.
    /// When false (the default), children start from a clean env containing
    /// only the keys listed in `allow`.
    #[serde(default)]
    pub inherit: bool,
    /// Env keys with a dual role. (1) Forwarding allowlist: when `inherit` is
    /// false, ONLY these keys are copied from the worker's env into the child.
    /// (2) Per-call gate: a `shell::exec`/`shell::exec_bg` request may set an
    /// `env` value only for a key listed here — MINUS the hardcoded dangerous
    /// keys (PATH, IFS, HOME, LD_*/DYLD_*, GCONV_PATH, BASH_ENV,
    /// PYTHONSTARTUP, NODE_OPTIONS, ...), which are never settable per call
    /// even if listed. Default: [PATH, HOME, LANG, LC_ALL, TERM].
    #[serde(default = "default_env_allow")]
    pub allow: Vec<String>,
}

fn default_env_allow() -> Vec<String> {
    vec!["PATH", "HOME", "LANG", "LC_ALL", "TERM"]
        .into_iter()
        .map(String::from)
        .collect()
}

impl Default for EnvConfig {
    fn default() -> Self {
        Self {
            inherit: false,
            allow: default_env_allow(),
        }
    }
}

/// The filesystem jail: which host roots are reachable through
/// `shell::fs::*`, `coder::*`, and per-call exec `cwd`, plus read/write
/// budgets and hard-denied paths.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FsConfig {
    /// Allowed jail roots. The FIRST entry is the PRIMARY root: relative wire
    /// paths and a relative per-call `cwd`/`base_dir` resolve against it.
    /// Absolute paths are accepted when they canonicalize inside ANY listed
    /// root. Empty means unjailed — refused at boot unless `allow_unjailed`
    /// is true. (The 0.6.x single-root `host_root` alias was removed in
    /// 0.7.0 and is rejected at parse with a migration hint.)
    #[serde(default)]
    pub host_roots: Vec<PathBuf>,
    /// Operator opt-in for running with an empty `host_roots`. When false
    /// (the default) the worker refuses to start unjailed — the entire host
    /// filesystem is reachable through `shell::fs::*` aside from the small
    /// denylist, which is rarely what the operator actually wants. Setting
    /// this to true is equivalent to acknowledging that fact (test
    /// harnesses, sandbox-only deployments).
    #[serde(default)]
    pub allow_unjailed: bool,
    /// Cap, in bytes, on a single `shell::fs::read` (S218 when exceeded).
    /// `0` (the default) means unlimited — safe because reads stream over a
    /// channel in 64 KiB chunks rather than buffering the file in memory; the
    /// cap exists to bound CALLER cost, not worker memory. The shipped seed
    /// sets 16777216 (16 MiB).
    #[serde(default = "default_max_read_bytes")]
    pub max_read_bytes: usize,
    /// Cap, in bytes, on a single `shell::fs::write` (S218 mid-stream when
    /// exceeded). `0` (the default) means unlimited — writes stream like
    /// reads, so the cap bounds caller cost, not worker memory. The shipped
    /// seed sets 16777216 (16 MiB).
    #[serde(default = "default_max_write_bytes")]
    pub max_write_bytes: usize,
    /// Absolute path prefixes that are hard-rejected (S215) by every fs
    /// operation and per-call exec `cwd`, even inside a jail root. A separate
    /// layer from `code.non_accessible_globs` (glob-based, show-but-lock).
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

/// Toggle for the `iii-sandbox` microVM exec backend.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SandboxConfig {
    /// Accept `target: { kind: "sandbox", sandbox_id }` calls and forward
    /// them to the `iii-sandbox` worker. When false, sandbox-targeted calls
    /// are rejected with S210. Default true.
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
    /// Effective jail roots, in priority order (index 0 = primary). Empty
    /// means unjailed. Does NOT canonicalize — that happens once at backend
    /// construction.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.host_roots.clone()
    }

    /// True when a jail boundary is configured (at least one root).
    pub fn is_jailed(&self) -> bool {
        !self.host_roots.is_empty()
    }
}

impl Default for FsConfig {
    fn default() -> Self {
        Self {
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
            env: EnvConfig::default(),
            allowlist: Vec::new(),
            denylist_patterns: Vec::new(),
            max_concurrent_jobs: default_max_concurrent_jobs(),
            job_retention_secs: default_job_retention_secs(),
            fs: FsConfig::default(),
            sandbox: SandboxConfig::default(),
            code: crate::code::config::CoderConfig::default(),
            compiled_denylist: Vec::new(),
        }
    }
}

impl ShellConfig {
    /// The bootable, zero-config default: seeded as `initial_value` on first
    /// registration and used as the runtime fallback when the stored value is
    /// null, so the worker boots with no config file at all (database-style
    /// zero-config). This is deliberately NOT `Default::default()` — that is
    /// unjailed (empty `host_roots`) so an operator config that omits the jail
    /// fails closed. This seed is the shipped permissive dev default: jailed to
    /// `/tmp`, env forwarded, open exec with a catastrophic-only denylist. It is
    /// kept in sync with `config.yaml` by a unit test.
    pub fn seed_default() -> Self {
        Self {
            max_timeout_ms: 120_000,
            // 30s (not the 10s code default): the dev seed raises max_timeout_ms
            // to 120s so real builds survive — a 10s default for callers that
            // omit timeout_ms would undercut that on the first `cargo build`.
            default_timeout_ms: 30_000,
            env: EnvConfig {
                inherit: true,
                ..EnvConfig::default()
            },
            // Command-SHAPED patterns (mkfs/shutdown/reboot/dd) are anchored to
            // argv[0] — `^(\S*/)?name` fires when the tool IS the command, not
            // when the word appears in an argument, so `grep -rn shutdown src/`
            // or `rg "dd if=" docs/` are not rejected. Argument-shaped patterns
            // (rm -rf /, the fork bomb, /etc/shadow) stay full-line: their
            // dangerous form lives in the arguments.
            denylist_patterns: vec![
                r"rm\s+-rf\s+/".into(),
                r":\(\)\s*\{\s*:\|".into(),
                r"^(\S*/)?mkfs".into(),
                r"^(\S*/)?dd\s+if=".into(),
                r"^(\S*/)?shutdown\b".into(),
                r"^(\S*/)?reboot\b".into(),
                "/etc/shadow".into(),
            ],
            fs: FsConfig {
                host_roots: vec![PathBuf::from("/tmp")],
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
    /// glob/budget settings from the `code` block, with ROOTS taken from
    /// `fs.host_roots` (the unified jail) — `code.base_paths` is runtime
    /// plumbing filled here, never read from config. This is what keeps the
    /// merge's promise that the operator sets the root once.
    pub fn code_resolver_config(&self) -> crate::code::config::CoderConfig {
        let mut c = self.code.clone();
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
    /// behind only the (advisory) denylist — the operator must either pin
    /// `fs.host_roots` or explicitly opt in via `fs.allow_unjailed: true`.
    pub fn validate_fs_jail(&self) -> Result<()> {
        if !self.fs.is_jailed() && !self.fs.allow_unjailed {
            anyhow::bail!(
                "fs.host_roots is empty and fs.allow_unjailed is false — refusing \
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
                return Err(format!(
                    "command matches denylist pattern '{}' — an advisory tripwire for \
                     catastrophic mistakes, not a security boundary; rephrase the command \
                     to avoid the pattern",
                    re.as_str()
                ));
            }
        }

        // Confinement guard: a command given as a PATH (contains a '/') that
        // canonicalizes to a location INSIDE the writable fs jail is rejected.
        // `shell::fs::write` can plant an executable (0755) under a jail root,
        // and the basename allowlist check above matches by file_name — so
        // `command: "<jail root>/ls"` would otherwise pass the allowlist and be
        // executed verbatim, a host RCE that bypasses the read-only allowlist.
        // Bare program names (no '/') are PATH-resolved by the OS and stay
        // allowed; legitimate absolute paths OUTSIDE the jail (e.g. /usr/bin/ls)
        // are not writable via shell::fs::write and stay allowed. A path that
        // fails to canonicalize (does not exist) is NOT rejected here — the
        // normal exec spawn surfaces its own not-found error.
        if cmd.contains('/') {
            // Unjailed mode (empty host_roots) has NO writable boundary — the
            // whole host filesystem is reachable via shell::fs::write, so an
            // agent can plant `/tmp/ls` and run `command: "/tmp/ls"` (basename
            // `ls` is allowlisted), bypassing the read-only allowlist entirely.
            // There is no path that distinguishes "agent-planted" from "system
            // binary" here, so reject ALL command paths and require a bare,
            // PATH-resolved name. (In jailed mode the check below is precise:
            // only paths inside the jail roots are rejected.)
            if !self.fs.is_jailed() {
                return Err(format!(
                    "command path '{}' is not allowed when fs is unjailed \
                     (fs.host_roots is empty): any host path is writable via \
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
    ///
    /// The removed-key check parses to a `Value` first, but the config itself
    /// deserializes from the TEXT again: `serde_yaml::from_value` self-tags
    /// plain scalars (an unquoted `false` in `allowlist` becomes `Bool` and can
    /// no longer deserialize into `String`), while `from_str` drives parsing by
    /// the target type. Double-parsing a config-sized string is free.
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let raw: serde_yaml::Value =
            serde_yaml::from_str(yaml).map_err(|e| format!("yaml parse: {e}"))?;
        if let Some(map) = raw.as_mapping() {
            check_removed_keys(map.keys().filter_map(|k| k.as_str()))?;
            if let Some(fs) = map.get("fs").and_then(|v| v.as_mapping()) {
                check_removed_fs_keys(fs.keys().filter_map(|k| k.as_str()))?;
            }
        }
        serde_yaml::from_str(yaml).map_err(|e| format!("yaml parse: {e}"))
    }

    /// Load a YAML seed file. Used only for the optional `--config` seed.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    /// Deserialize the live value fetched from the configuration worker.
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        if let Some(obj) = value.as_object() {
            check_removed_keys(obj.keys().map(String::as_str))?;
            if let Some(fs) = obj.get("fs").and_then(serde_json::Value::as_object) {
                check_removed_fs_keys(fs.keys().map(String::as_str))?;
            }
        }
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
        assert!(!c.env.inherit);
        assert_eq!(c.env.allow, default_env_allow());
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
        // (see exec_command_path_rejected_when_unjailed), so set a jail root
        // that does NOT contain /usr/bin/ls to exercise the basename contract.
        let mut c = cfg_with(vec!["ls"], vec![]);
        c.fs.host_roots =
            vec![std::env::temp_dir().join(format!("shell-basename-{}", uuid::Uuid::new_v4()))];
        std::fs::create_dir_all(&c.fs.host_roots[0]).unwrap();
        assert!(c
            .is_command_allowed(&["/usr/bin/ls".into(), "-la".into()])
            .is_ok());
        std::fs::remove_dir_all(&c.fs.host_roots[0]).ok();
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

        // Command-shaped patterns are anchored to argv[0]: they fire when the
        // tool IS the command (bare or path-qualified)...
        for argv in [
            vec!["shutdown".to_string(), "-h".into(), "now".into()],
            vec!["/sbin/shutdown".to_string(), "-r".into()],
            vec!["reboot".to_string()],
            vec!["mkfs.ext4".to_string(), "/dev/sda1".into()],
            vec!["dd".to_string(), "if=/dev/zero".into(), "of=/dev/sda".into()],
        ] {
            assert!(
                c.is_command_allowed(&argv).is_err(),
                "{argv:?} must trip the anchored denylist"
            );
        }
        // ...but NOT when the word merely appears in an argument — a coding
        // agent grepping a codebase for "shutdown" is not a mistake.
        for argv in [
            vec!["grep".to_string(), "-rn".into(), "shutdown".into(), "src/".into()],
            vec!["cargo".to_string(), "test".into(), "reboot".into()],
            vec!["rg".to_string(), "dd if=".into(), "docs/".into()],
            vec!["git".to_string(), "log".into(), "--grep".into(), "mkfs".into()],
        ] {
            assert!(
                c.is_command_allowed(&argv).is_ok(),
                "{argv:?} must NOT trip the denylist (argument, not command)"
            );
        }
    }

    /// `seed_default()` is the in-code twin of the shipped `config.yaml` — the
    /// file the registry publishes and that `cargo run` loads. If they drift, a
    /// zero-config boot and a `config.yaml` boot would diverge silently.
    /// Routed through `from_yaml` so the shipped seed also passes the
    /// removed-key check production uses.
    #[test]
    fn seed_default_matches_shipped_config_yaml() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml");
        let content = std::fs::read_to_string(path).expect("read config.yaml");
        let from_file = ShellConfig::from_yaml(&content).expect("config.yaml parses");
        assert_eq!(
            from_file.to_json(),
            ShellConfig::seed_default().to_json(),
            "config.yaml and ShellConfig::seed_default() must stay in sync"
        );
    }

    /// A 0.6.x seed carrying the removed top-level `inherit_env` must be
    /// rejected with a hint naming the new key — serde would otherwise ignore
    /// it and silently boot with env forwarding OFF.
    #[test]
    fn from_yaml_rejects_removed_inherit_env_with_hint() {
        let err = ShellConfig::from_yaml("inherit_env: true\n").expect_err("removed key rejects");
        assert!(err.contains("removed in 0.7.0"), "{err}");
        assert!(err.contains("`inherit_env` -> `env.inherit`"), "{err}");
    }

    /// Same for `allowed_env` through the live-value (JSON) funnel, which is
    /// what an un-migrated stored configuration hits at boot and hot-reload.
    #[test]
    fn from_json_rejects_removed_allowed_env_with_hint() {
        let v = serde_json::json!({"allowed_env": ["PATH"], "fs": {"allow_unjailed": true}});
        let err = ShellConfig::from_json(&v).expect_err("removed key rejects");
        assert!(err.contains("`allowed_env` -> `env.allow`"), "{err}");
        assert!(err.contains("configuration::set"), "{err}");
    }

    /// Both removed keys present → both mappings named, so an operator fixes
    /// the config in one pass instead of playing whack-a-mole.
    #[test]
    fn removed_keys_error_names_both_mappings() {
        let err = ShellConfig::from_yaml("inherit_env: true\nallowed_env: [PATH]\n")
            .expect_err("removed keys reject");
        assert!(err.contains("`inherit_env` -> `env.inherit`"), "{err}");
        assert!(err.contains("`allowed_env` -> `env.allow`"), "{err}");
    }

    /// Round-trip realism: exactly what a live 0.6.x STORED value looks like —
    /// the old seed serialized with top-level `inherit_env`/`allowed_env` and
    /// no `env` block — must fail closed through `from_json`.
    #[test]
    fn stored_060_shape_fails_closed_with_hint() {
        let mut v = ShellConfig::seed_default().to_json();
        let obj = v.as_object_mut().unwrap();
        obj.remove("env");
        obj.insert("inherit_env".into(), serde_json::Value::Bool(true));
        obj.insert("allowed_env".into(), serde_json::json!(["PATH", "HOME"]));
        let err = ShellConfig::from_json(&v).expect_err("0.6.x shape fails closed");
        assert!(err.contains("removed in 0.7.0"), "{err}");
    }

    /// Regression: the removed-key pre-parse must NOT change how scalars
    /// deserialize. An unquoted `false` in a string list (the e2e fixture
    /// allowlists the `false` binary) self-tags as Bool through
    /// `serde_yaml::from_value`, so `from_yaml` must re-deserialize from the
    /// text, where the target type drives parsing.
    #[test]
    fn from_yaml_keeps_unquoted_boolean_like_strings() {
        let c = ShellConfig::from_yaml("allowlist: [echo, false, \"true\"]\n")
            .expect("boolean-looking allowlist entries parse as strings");
        assert_eq!(c.allowlist, vec!["echo", "false", "true"]);
    }

    /// The nested block parses, and every omitted field takes the EnvConfig
    /// default (inherit false, standard allow list).
    #[test]
    fn env_block_parses_and_defaults() {
        let c = ShellConfig::from_yaml("env:\n  inherit: true\n").expect("nested env parses");
        assert!(c.env.inherit);
        assert_eq!(c.env.allow, default_env_allow());

        let d = ShellConfig::from_yaml("{}").expect("empty config parses");
        assert!(!d.env.inherit);
        assert_eq!(d.env.allow, default_env_allow());
    }

    /// Every operator-visible config field must carry a schema description —
    /// the console configuration UI renders them, and a bare field name is
    /// exactly the DX gap this schema exists to close. Also pins that the
    /// nested `EnvConfig` definition documents the dual role of `allow`.
    #[test]
    fn json_schema_every_field_has_description() {
        let schema = ShellConfig::json_schema();
        let props = schema["properties"].as_object().expect("top-level properties");
        assert!(!props.is_empty());
        for (name, prop) in props {
            assert!(
                prop.get("description")
                    .and_then(|d| d.as_str())
                    .is_some_and(|s| !s.is_empty()),
                "config field `{name}` has no schema description (console UI shows it bare)"
            );
        }
        // Same rule for every nested definition (EnvConfig, FsConfig,
        // SandboxConfig, CoderConfig, and anything added later): the console
        // renders their fields too.
        let defs = schema["definitions"]
            .as_object()
            .expect("nested definitions");
        for (def_name, def) in defs {
            let Some(props) = def["properties"].as_object() else {
                continue; // non-object definitions (enums etc.) have no fields
            };
            for (name, prop) in props {
                assert!(
                    prop.get("description")
                        .and_then(|d| d.as_str())
                        .is_some_and(|s| !s.is_empty()),
                    "{def_name}.{name} has no schema description (console UI shows it bare)"
                );
            }
        }
        let allow_desc = schema["definitions"]["EnvConfig"]["properties"]["allow"]["description"]
            .as_str()
            .expect("env.allow described");
        assert!(
            allow_desc.contains("Forwarding") && allow_desc.contains("Per-call"),
            "env.allow description must explain both roles: {allow_desc}"
        );
    }

    #[test]
    fn exec_command_path_inside_jail_is_rejected() {
        // An agent can plant `<jail root>/ls` (0755) via shell::fs::write; the
        // basename allowlist matches "ls", so without the confinement guard
        // `command: "<jail root>/ls"` would execute that jail-planted file —
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
        c.fs.host_roots = vec![root.clone()];
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
                c2.fs.host_roots = vec![root.clone()];
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
        // Unjailed mode (empty host_roots) has no writable boundary: the whole
        // host FS is reachable via shell::fs::write, so ANY command path could
        // execute agent-planted bytes and bypass the allowlist. Reject every
        // path; only bare PATH-resolved names are permitted.
        let mut c = ShellConfig {
            allowlist: vec!["ls".into()],
            ..Default::default()
        };
        c.fs.host_roots = Vec::new();
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
        assert!(c.fs.host_roots.is_empty());
    }

    #[test]
    fn yaml_with_fs_section_parses() {
        let yaml = r#"
allowlist: []
fs:
  host_roots: [/tmp/shell]
  max_read_bytes: 1024
  denylist_paths:
    - /etc
sandbox:
  enabled: false
"#;
        let c: ShellConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            c.fs.host_roots,
            vec![std::path::PathBuf::from("/tmp/shell")]
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
        assert!(msg.contains("host_roots"));
        assert!(msg.contains("allow_unjailed"));
    }

    #[test]
    fn validate_fs_jail_accepts_explicit_opt_in() {
        let mut c = ShellConfig::default();
        c.fs.allow_unjailed = true;
        c.validate_fs_jail().expect("explicit opt-in is valid");
    }

    #[test]
    fn validate_fs_jail_accepts_single_host_root() {
        let mut c = ShellConfig::default();
        c.fs.host_roots = vec![std::path::PathBuf::from("/tmp/something")];
        c.validate_fs_jail().expect("a one-entry host_roots is valid");
    }

    #[test]
    fn validate_fs_jail_accepts_host_roots_list() {
        let mut c = ShellConfig::default();
        c.fs.host_roots = vec!["/tmp/a".into(), "/tmp/b".into()];
        c.validate_fs_jail().expect("host_roots list is valid");
    }

    #[test]
    fn roots_returns_host_roots_and_empty_means_unjailed() {
        let mut c = FsConfig::default();
        assert!(c.roots().is_empty(), "unset = unjailed");
        assert!(!c.is_jailed());
        c.host_roots = vec!["/tmp/a".into(), "/tmp/b".into()];
        assert_eq!(
            c.roots(),
            vec![
                std::path::PathBuf::from("/tmp/a"),
                std::path::PathBuf::from("/tmp/b")
            ]
        );
        assert!(c.is_jailed());
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

    /// The resolver-roots wiring hop: `code_resolver_config` must copy
    /// `fs.roots()` into `code.base_paths` while preserving the rest of the
    /// `code` block. Since 0.7.0 `code.base_paths` is serde-skipped, this
    /// copy is the ONLY source of coder roots — a regression dropping it
    /// would hand `PathResolver` an empty root set, which falls back to the
    /// default `["./", "/tmp"]` jail: coder::* silently jailed WIDER than
    /// the operator's fs jail. (Same wiring class as the D4 glob test in
    /// tests/code_unified_protection.rs.)
    #[test]
    fn code_resolver_config_fills_roots_from_fs_jail() {
        let mut c = ShellConfig::default();
        c.fs.host_roots = vec!["/tmp/a".into(), "/tmp/b".into()];
        c.code.non_accessible_globs = vec!["**/.env".into()];
        let resolved = c.code_resolver_config();
        assert_eq!(
            resolved.base_paths,
            c.fs.roots(),
            "coder roots must come from the unified fs jail"
        );
        assert_eq!(
            resolved.non_accessible_globs,
            vec!["**/.env".to_string()],
            "the rest of the code block is preserved"
        );
    }

    #[test]
    fn to_json_from_json_round_trips() {
        let mut c = ShellConfig::default();
        c.fs.host_roots = vec![std::path::PathBuf::from("/tmp/shell")];
        c.allowlist = vec!["ls".into(), "cat".into()];
        let v = c.to_json();
        let back = ShellConfig::from_json(&v).expect("from_json round-trips");
        assert_eq!(back.allowlist, c.allowlist);
        assert_eq!(back.fs.host_roots, c.fs.host_roots);
    }

    #[test]
    fn from_yaml_parses_seed() {
        let c = ShellConfig::from_yaml("allowlist: [ls]\nfs:\n  host_roots: [/tmp/x]\n")
            .expect("seed yaml parses");
        assert_eq!(c.allowlist, vec!["ls".to_string()]);
    }

    /// A 0.6.x seed still carrying the removed single-root alias must be
    /// rejected with a hint naming the list form — serde would otherwise
    /// ignore it and parse a config with NO jail configured.
    #[test]
    fn from_yaml_rejects_removed_fs_host_root_with_hint() {
        let err = ShellConfig::from_yaml("fs:\n  host_root: /tmp\n").expect_err("removed key rejects");
        assert!(err.contains("removed in 0.7.0"), "{err}");
        assert!(err.contains("fs.host_roots"), "{err}");
    }

    /// Same through the live-value (JSON) funnel — what an un-migrated stored
    /// configuration hits at boot and hot-reload.
    #[test]
    fn from_json_rejects_removed_fs_host_root_with_hint() {
        let v = serde_json::json!({"fs": {"host_root": "/tmp"}});
        let err = ShellConfig::from_json(&v).expect_err("removed key rejects");
        assert!(err.contains("removed in 0.7.0"), "{err}");
        assert!(err.contains("fs.host_roots"), "{err}");
        assert!(err.contains("configuration::set"), "{err}");
    }
}
