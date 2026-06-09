//! Per-call `cwd` + `env` gating for `shell::exec` / `shell::exec_bg`.
//!
//! These two optional request fields let an agent scope a single command to a
//! directory and set specific environment values WITHOUT wrapping everything in
//! `sh -lc` (which would defeat the argv allowlist). The whole point of this
//! module is the gating: untrusted LLM input must never escape the fs jail via
//! `cwd`, nor hijack the child's execution environment via `env`.
//!
//! Gating rules (mandatory, not best-effort):
//! - `cwd` is confined to the SAME jail the fs backend enforces — it is
//!   canonicalized and must `starts_with(host_root)` and miss the denylist,
//!   exactly like `shell::fs::*` paths. A `cwd` resolving outside the jail is
//!   rejected `S215`. When `fs.host_root` is unset (operator opted into
//!   `allow_unjailed`), the same code path runs with no root to confine to,
//!   matching the fs backend's unjailed behaviour.
//! - `env` may set a VALUE only for a key the operator already put in
//!   `cfg.allowed_env`, and NEVER for an exec-hijacking key (see
//!   [`DANGEROUS_ENV_KEYS`]) — those are rejected even if an operator
//!   mistakenly lists them in `allowed_env`. Any offending key rejects the
//!   WHOLE call `S210` (we never silently drop a key — the agent must learn its
//!   env was not applied), naming the offending key and listing the permitted
//!   ones so the agent can self-correct.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::config::ShellConfig;
use crate::exec::error::ExecError;

/// Environment keys that an agent may NEVER set per-call, regardless of
/// `allowed_env`. Setting any of these can hijack which binary the child
/// actually executes or which shared libraries it loads — turning a benign
/// allowlisted `command` into arbitrary code execution. The denylist is a
/// HARD boundary: it wins over `allowed_env` so an operator's typo can't open
/// a privesc hole.
///
/// - `PATH` / `IFS`: change which binary an allowlisted name resolves to / how
///   the shell re-tokenizes the command line.
/// - `LD_*` (glibc) and `DYLD_*` (macOS dyld): preload / library-path
///   injection — load attacker-controlled code into the child at startup.
/// - `GCONV_PATH` and other glibc lookup paths: code-load vectors (e.g. the
///   `GCONV_PATH` chain in CVE-2021-4034) that point libc at attacker files.
/// - `HOME`: not a loader vector, but redirecting it lets an agent point a
///   config-reading allowlisted program (git/ssh/curl/python) at a jail-planted
///   config; the worker still forwards its own `HOME` when allowlisted, callers
///   just cannot override it per call.
pub const DANGEROUS_ENV_KEYS: &[&str] = &[
    "PATH",
    "IFS",
    "HOME",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "LD_PROFILE",
    "LD_DEBUG",
    "LD_CONFIG",
    "LD_ORIGIN_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    "DYLD_FALLBACK_FRAMEWORK_PATH",
    "GCONV_PATH",
    "GETCONF_DIR",
    "HOSTALIASES",
    "NIS_PATH",
    "MALLOC_TRACE",
    "RES_OPTIONS",
    "LOCALDOMAIN",
];

/// True if `key` is in the always-rejected denylist (case-sensitive: env var
/// names are case-sensitive on Unix, and the dangerous names are upper-case).
fn is_dangerous_env_key(key: &str) -> bool {
    DANGEROUS_ENV_KEYS.contains(&key)
}

/// Validated per-call exec overrides, ready to apply in `build_command`.
/// `None` fields mean "unchanged from the config-derived default", so the
/// default behaviour (both request fields omitted) is byte-for-byte the prior
/// behaviour.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOverrides {
    /// Canonical, jail-confined working directory. `None` falls back to
    /// `cfg.working_dir` in `build_command`.
    pub cwd: Option<PathBuf>,
    /// Per-call env values, already gated against `allowed_env` +
    /// [`DANGEROUS_ENV_KEYS`]. Applied on top of the config-forwarded env.
    pub env: Option<BTreeMap<String, String>>,
}

impl ExecOverrides {
    /// True when there is nothing to apply — used by the sandbox backend to
    /// fast-path the common (omitted) case while still rejecting a populated
    /// override (cwd/env are host-only for now).
    pub fn is_empty(&self) -> bool {
        self.cwd.is_none() && self.env.is_none()
    }
}

/// Confine a raw `cwd` string against the fs jail and verify it is an existing
/// directory. Reuses the fs backend's [`crate::fs::host::confine_path`] so the
/// rule is literally the same one `shell::fs::*` enforces:
/// canonicalize → `starts_with(host_root)` → denylist. Per-call (not cached
/// like the fs backend) because the exec handler reads the live config
/// snapshot; canonicalizing the root + denylist here is one extra `stat` per
/// call with a `cwd`, which is negligible.
fn confine_cwd(cwd: &str, cfg: &ShellConfig) -> Result<PathBuf, ExecError> {
    // Canonicalize host_root + denylist the same way HostFsBackend::try_new
    // does, so confine_path sees the identical inputs. An unreachable root /
    // denylist entry is an operator config error; surface it rather than
    // silently degrading the jail.
    let host_root_canon = match &cfg.fs.host_root {
        Some(root) => Some(std::fs::canonicalize(root).map_err(|e| {
            ExecError::new(
                "S216",
                format!("host_root unreachable ({}): {e}", root.display()),
            )
        })?),
        None => None,
    };
    let mut denylist_canon = Vec::with_capacity(cfg.fs.denylist_paths.len());
    for deny in &cfg.fs.denylist_paths {
        match std::fs::canonicalize(deny) {
            Ok(c) => denylist_canon.push(c),
            // A denylist entry that doesn't exist can't be escaped through;
            // skip it rather than failing the whole exec (matches the spirit
            // of the fs backend, which canonicalizes with a fallback). The
            // jail root check below is the primary boundary.
            Err(_) => denylist_canon.push(deny.clone()),
        }
    }

    let canon = crate::fs::host::confine_path(cwd, host_root_canon.as_deref(), &denylist_canon)
        // FsError and ExecError carry the same { code, message } shape; the
        // confinement codes (S210 bad/empty path, S215 jail escape/denylist)
        // are exactly what exec wants to surface, so re-wrap verbatim.
        .map_err(|e| ExecError::new(static_code(e.code), e.message))?;

    // The directory must exist and BE a directory — a missing or
    // wrong-type cwd is a clear request error, not a spawn-time surprise.
    let md = std::fs::symlink_metadata(&canon)
        .map_err(|e| ExecError::new("S211", format!("cwd not found: {cwd}: {e}")))?;
    // Follow one level for symlinks-to-dir: confine_path already resolved the
    // path canonically, so this metadata stat is on the real target.
    let md = if md.file_type().is_symlink() {
        std::fs::metadata(&canon)
            .map_err(|e| ExecError::new("S211", format!("cwd not found: {cwd}: {e}")))?
    } else {
        md
    };
    if !md.is_dir() {
        return Err(ExecError::new(
            "S210",
            format!("cwd is not a directory: {cwd}"),
        ));
    }
    Ok(canon)
}

/// Map an `FsError`'s runtime `String` code onto the `&'static str` codes
/// `ExecError` uses, preserving the confinement taxonomy (S210/S211/S215) and
/// collapsing anything else to the generic S216.
fn static_code(code: &str) -> &'static str {
    match code {
        "S210" => "S210",
        "S211" => "S211",
        "S215" => "S215",
        "S216" => "S216",
        _ => "S216",
    }
}

/// Validate a per-call `env` map: every key must be present in
/// `cfg.allowed_env` AND absent from [`DANGEROUS_ENV_KEYS`]. The dangerous
/// check runs FIRST so a key that is both dangerous and (mistakenly)
/// allowlisted is still rejected. On any violation the WHOLE call fails S210 —
/// we never partially apply env, so the agent always knows whether its env took
/// effect.
fn validate_env(
    env: &BTreeMap<String, String>,
    cfg: &ShellConfig,
) -> Result<BTreeMap<String, String>, ExecError> {
    for key in env.keys() {
        if is_dangerous_env_key(key) {
            return Err(ExecError::new(
                "S210",
                format!(
                    "env key '{key}' is never settable per-call (exec-hijacking key); \
                     remove it. Settable keys (must also be in allowed_env): [{}]",
                    cfg.allowed_env.join(", ")
                ),
            ));
        }
        if !cfg.allowed_env.iter().any(|a| a == key) {
            return Err(ExecError::new(
                "S210",
                format!(
                    "env key '{key}' is not in allowed_env; the operator must permit it. \
                     Permitted keys: [{}]",
                    cfg.allowed_env.join(", ")
                ),
            ));
        }
    }
    Ok(env.clone())
}

/// Build the validated [`ExecOverrides`] from the raw request fields, applying
/// all gating. Returns an `ExecError` (with the S-code the caller surfaces on
/// the wire) on any violation. Both inputs `None` ⇒ an empty `ExecOverrides`,
/// i.e. unchanged default behaviour.
pub fn build_overrides(
    cwd: Option<&str>,
    env: Option<&BTreeMap<String, String>>,
    cfg: &ShellConfig,
) -> Result<ExecOverrides, ExecError> {
    let cwd = match cwd {
        Some(c) => Some(confine_cwd(c, cfg)?),
        None => None,
    };
    let env = match env {
        Some(e) => Some(validate_env(e, cfg)?),
        None => None,
    };
    Ok(ExecOverrides { cwd, env })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_jailed(root: &std::path::Path) -> ShellConfig {
        let mut c = ShellConfig {
            allowed_env: vec!["NODE_ENV".into(), "MY_VAR".into()],
            ..Default::default()
        };
        c.fs.host_root = Some(root.to_path_buf());
        c
    }

    #[test]
    fn dangerous_keys_cover_ld_and_dyld_and_path() {
        for k in [
            "PATH",
            "IFS",
            "HOME",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "GCONV_PATH",
            "HOSTALIASES",
        ] {
            assert!(is_dangerous_env_key(k), "{k} must be dangerous");
        }
        assert!(!is_dangerous_env_key("NODE_ENV"));
    }

    #[test]
    fn env_in_allowed_is_accepted() {
        let c = ShellConfig {
            allowed_env: vec!["NODE_ENV".into()],
            ..Default::default()
        };
        let mut env = BTreeMap::new();
        env.insert("NODE_ENV".to_string(), "test".to_string());
        let out = validate_env(&env, &c).expect("allowed key accepted");
        assert_eq!(out.get("NODE_ENV").map(String::as_str), Some("test"));
    }

    #[test]
    fn env_not_in_allowed_is_rejected_naming_key_and_listing_permitted() {
        let c = ShellConfig {
            allowed_env: vec!["NODE_ENV".into(), "MY_VAR".into()],
            ..Default::default()
        };
        let mut env = BTreeMap::new();
        env.insert("SECRET".to_string(), "x".to_string());
        let err = validate_env(&env, &c).expect_err("must reject");
        assert_eq!(err.code, "S210");
        assert!(
            err.message.contains("SECRET"),
            "names the key: {}",
            err.message
        );
        assert!(
            err.message.contains("NODE_ENV"),
            "lists permitted: {}",
            err.message
        );
        assert!(
            err.message.contains("MY_VAR"),
            "lists permitted: {}",
            err.message
        );
    }

    #[test]
    fn dangerous_key_rejected_even_when_allowlisted() {
        // The denylist must WIN over allowed_env: an operator typo listing
        // LD_PRELOAD must not open a code-injection hole.
        let c = ShellConfig {
            allowed_env: vec!["LD_PRELOAD".into()],
            ..Default::default()
        };
        let mut env = BTreeMap::new();
        env.insert("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string());
        let err = validate_env(&env, &c).expect_err("dangerous key must reject");
        assert_eq!(err.code, "S210");
        assert!(err.message.contains("LD_PRELOAD"));
        assert!(err.message.contains("exec-hijacking"));
    }

    #[test]
    fn cwd_inside_jail_resolves() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let c = cfg_jailed(&root);
        let canon = confine_cwd("sub", &c).expect("cwd inside jail");
        assert_eq!(canon, root.join("sub").canonicalize().unwrap());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cwd_escaping_jail_is_rejected_s215() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let c = cfg_jailed(&root);
        let err = confine_cwd("../../etc", &c).expect_err("escape rejected");
        assert_eq!(err.code, "S215");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cwd_missing_directory_is_rejected() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let c = cfg_jailed(&root);
        // `nope` is inside the jail lexically but does not exist — confine_path
        // resolves the longest existing ancestor (the root) then the stat
        // fails, surfacing S211.
        let err = confine_cwd("nope", &c).expect_err("missing cwd rejected");
        assert!(err.code == "S211" || err.code == "S210", "got {}", err.code);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cwd_pointing_at_a_file_is_rejected_s210() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("f.txt"), "x").unwrap();
        let c = cfg_jailed(&root);
        let err = confine_cwd("f.txt", &c).expect_err("file-as-cwd rejected");
        assert_eq!(err.code, "S210");
        assert!(err.message.contains("not a directory"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn build_overrides_both_none_is_empty() {
        let c = ShellConfig::default();
        let ov = build_overrides(None, None, &c).expect("ok");
        assert!(ov.is_empty());
    }
}
