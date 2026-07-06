//! Per-call `cwd` + `env` gating for `shell::exec` / `shell::exec_bg`.
//!
//! These two optional request fields let an agent scope a single command to a
//! directory and set specific environment values WITHOUT wrapping everything in
//! `sh -lc` (which would blur what the argv actually was). The whole point of this
//! module is the gating: untrusted LLM input must never escape the fs jail via
//! `cwd`, nor hijack the child's execution environment via `env`.
//!
//! Gating rules (mandatory, not best-effort):
//! - `cwd` is confined to the SAME jail the fs backend enforces — it is
//!   canonicalized and must resolve inside a jail root (`fs.host_roots`) and
//!   miss the denylist, exactly like `shell::fs::*` paths. A `cwd` resolving
//!   outside the jail is rejected `S215`. When `fs.host_roots` is empty
//!   (operator opted into `allow_unjailed`), the same code path runs with no
//!   root to confine to, matching the fs backend's unjailed behaviour.
//! - `env` may set a VALUE only for a key the operator already put in
//!   `cfg.env.allow`, and NEVER for an exec-hijacking key (see
//!   [`DANGEROUS_ENV_KEYS`]) — those are rejected even if an operator
//!   mistakenly lists them in `env.allow`. Any offending key rejects the
//!   WHOLE call `S210` (we never silently drop a key — the agent must learn its
//!   env was not applied), naming the offending key and listing the permitted
//!   ones so the agent can self-correct.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::ShellConfig;
use crate::exec::error::ExecError;
use crate::target::Target;

/// Environment keys that an agent may NEVER set per-call, regardless of
/// `env.allow`. Setting any of these can hijack which binary the child
/// actually executes or which shared libraries it loads — turning a benign
/// `command` into arbitrary code execution. The denylist is a
/// HARD boundary: it wins over `env.allow` so an operator's typo can't open
/// a privesc hole.
///
/// - `PATH` / `IFS`: change which binary a bare name resolves to / how
///   the shell re-tokenizes the command line.
/// - `LD_*` (glibc) and `DYLD_*` (macOS dyld): preload / library-path
///   injection — load attacker-controlled code into the child at startup.
/// - `GCONV_PATH` and other glibc lookup paths: code-load vectors (e.g. the
///   `GCONV_PATH` chain in CVE-2021-4034) that point libc at attacker files.
/// - `HOME`: not a loader vector, but redirecting it lets an agent point a
///   config-reading program (git/ssh/curl/python) at a jail-planted
///   config; the worker still forwards its own `HOME` when `env.allow` lists it, callers
///   just cannot override it per call.
/// - `BASH_ENV` / `ENV` / `PYTHONSTARTUP` / `PERL5OPT` / `RUBYOPT` /
///   `NODE_OPTIONS`: startup-file / option-injection vectors honored by
///   sh/bash/python/perl/ruby/node at process start — a non-interactive
///   interpreter sources or applies them before running anything, so pointing
///   them at a jail-planted file/option turns an interpreter into
///   arbitrary code execution (same exec-hijack class as the loader vars).
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
    "BASH_ENV",
    "ENV",
    "PYTHONSTARTUP",
    "PERL5OPT",
    "RUBYOPT",
    "NODE_OPTIONS",
];

/// True if `key` is always rejected. Two layers: (1) family-prefix checks for
/// the dynamic-loader variables (`LD_*` on glibc, `DYLD_*` on macOS) — the set
/// of these is open-ended across libc/OS versions, so an exact-name list would
/// silently let a future `LD_SOMETHING` through if an operator widened
/// `env.allow`; (2) the explicit [`DANGEROUS_ENV_KEYS`] denylist for the
/// non-family names (PATH/IFS/HOME, glibc lookup paths, interpreter startup
/// keys). Case-sensitive: env var names are case-sensitive on Unix and the
/// dangerous names are upper-case.
fn is_dangerous_env_key(key: &str) -> bool {
    key.starts_with("LD_") || key.starts_with("DYLD_") || DANGEROUS_ENV_KEYS.contains(&key)
}

/// Validated per-call exec overrides, ready to apply in `build_command`.
/// `None` fields mean "use the config-derived default"; when both request
/// fields are omitted, execution follows the configured command policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOverrides {
    /// Canonical, jail-confined working directory. `None` falls back to
    /// `cfg.working_dir` in `build_command`.
    pub cwd: Option<PathBuf>,
    /// Per-call env values, already gated against `env.allow` +
    /// [`DANGEROUS_ENV_KEYS`]. Applied on top of the config-forwarded env.
    pub env: Option<BTreeMap<String, String>>,
    /// Bytes fed to the child's stdin (then EOF). `None` leaves stdin closed
    /// (`/dev/null`). Needs no jail/allowlist gating — it is opaque input data,
    /// not a path or env key — but like cwd/env it is HOST-only (the sandbox
    /// exec protocol does not forward stdin), enforced via [`is_empty`].
    pub stdin: Option<String>,
}

impl ExecOverrides {
    /// True when there is nothing to apply — used by the sandbox backend to
    /// fast-path the common (omitted) case while still rejecting a populated
    /// override (cwd/env/stdin are host-only for now).
    pub fn is_empty(&self) -> bool {
        self.cwd.is_none() && self.env.is_none() && self.stdin.is_none()
    }
}

/// Canonicalize every jail root + every `denylist_paths` entry the same way
/// `HostFsBackend::try_new` does, so the confinement helpers see the identical
/// inputs. An unreachable root is an operator config error (surfaced S216); a
/// non-existent denylist entry can't be escaped through, so it is kept as-is.
fn jail_inputs(cfg: &ShellConfig) -> Result<(Vec<PathBuf>, Vec<PathBuf>), ExecError> {
    let mut host_roots_canon = Vec::new();
    for root in cfg.fs.roots() {
        let canon = std::fs::canonicalize(&root).map_err(|e| {
            ExecError::new(
                "S216",
                format!("jail root unreachable ({}): {e}", root.display()),
            )
        })?;
        if !host_roots_canon.contains(&canon) {
            host_roots_canon.push(canon);
        }
    }
    let mut denylist_canon = Vec::with_capacity(cfg.fs.denylist_paths.len());
    for deny in &cfg.fs.denylist_paths {
        match std::fs::canonicalize(deny) {
            Ok(c) => denylist_canon.push(c),
            Err(_) => denylist_canon.push(deny.clone()),
        }
    }
    Ok((host_roots_canon, denylist_canon))
}

/// Verify a directory operand exists and is a directory (following one symlink
/// level). Shared by the `cwd` check and the `scope_root` check — a missing or
/// wrong-type directory is a clear request error, not a spawn-time surprise.
fn require_existing_dir(canon: &PathBuf, raw: &str, kind: &str) -> Result<(), ExecError> {
    let md = std::fs::symlink_metadata(canon)
        .map_err(|e| ExecError::new("S211", format!("{kind} not found: {raw}: {e}")))?;
    let md = if md.file_type().is_symlink() {
        std::fs::metadata(canon)
            .map_err(|e| ExecError::new("S211", format!("{kind} not found: {raw}: {e}")))?
    } else {
        md
    };
    if !md.is_dir() {
        return Err(ExecError::new(
            "S210",
            format!("{kind} is not a directory: {raw}"),
        ));
    }
    Ok(())
}

/// Resolve and validate an OPTIONAL trusted per-call `scope_root` for exec.
/// `None` ⇒ `Ok(None)` (unchanged behaviour). An absolute `scope_root` may point
/// outside configured host roots and becomes the effective root for this call.
/// Relative values are rejected; the harness-controlled workspace selection is
/// always an absolute path.
fn confine_scope_root(
    scope_root: Option<&str>,
    denylist_canon: &[PathBuf],
) -> Result<Option<PathBuf>, ExecError> {
    let Some(scope_root) = scope_root else {
        return Ok(None);
    };
    if scope_root.is_empty() {
        return Err(ExecError::new("S210", "scope_root must not be empty"));
    }
    let p = Path::new(scope_root);
    if !p.is_absolute() {
        return Err(ExecError::new(
            "S210",
            format!("scope_root must be an absolute path: {scope_root}"),
        ));
    }
    let canon = crate::path::canonicalize_with_fallback(p)
        .map_err(|e| ExecError::new("S210", format!("{scope_root}: {e}")))?;
    for deny in denylist_canon {
        if canon.starts_with(deny) {
            return Err(ExecError::new(
                "S215",
                format!("scope_root is denylisted: {scope_root}"),
            ));
        }
    }
    require_existing_dir(&canon, scope_root, "scope_root")?;
    Ok(Some(canon))
}

/// Confine a raw `cwd` string against the fs jail and verify it is an existing
/// directory. Reuses the fs backend's
/// [`crate::fs::host::confine_path_with_scope_root`] so the rule is literally the
/// same one `shell::fs::*` enforces: canonicalize → `starts_with(root)` →
/// denylist. When `scope_root_canon` is set the confinement root is the session
/// directory (relative `cwd` anchors there; an absolute `cwd` outside it is
/// S220); otherwise it is the jail roots (unchanged). Per-call (not cached
/// like the fs backend) because the exec handler reads the live config
/// snapshot.
fn confine_cwd(
    cwd: &str,
    host_roots_canon: &[PathBuf],
    scope_root_canon: Option<&std::path::Path>,
    scope_grants_canon: &[PathBuf],
    denylist_canon: &[PathBuf],
) -> Result<PathBuf, ExecError> {
    let canon = crate::fs::host::confine_path_with_scope_root(
        cwd,
        host_roots_canon,
        scope_root_canon,
        scope_grants_canon,
        denylist_canon,
    )
    // FsError and ExecError carry the same { code, message } shape; the
    // confinement codes (S210 bad/empty path, S215 jail escape/denylist, S220
    // outside-session-dir) are exactly what exec wants to surface, so re-wrap
    // verbatim.
    .map_err(|e| ExecError::new(static_code(e.code), e.message))?;

    require_existing_dir(&canon, cwd, "cwd")?;
    Ok(canon)
}

/// Map an `FsError`'s runtime `String` code onto the `&'static str` codes
/// `ExecError` uses, preserving the confinement taxonomy
/// (S210/S211/S215/S220) and collapsing anything else to the generic S216.
fn static_code(code: &str) -> &'static str {
    match code {
        "S210" => "S210",
        "S211" => "S211",
        "S215" => "S215",
        "S220" => "S220",
        _ => "S216",
    }
}

/// Validate a per-call `env` map: every key must be present in
/// `cfg.env.allow` AND absent from [`DANGEROUS_ENV_KEYS`]. The dangerous
/// check runs FIRST so a key that is both dangerous and (mistakenly)
/// allowlisted is still rejected. On any violation the WHOLE call fails S210 —
/// we never partially apply env, so the agent always knows whether its env took
/// effect.
fn validate_env(
    env: &BTreeMap<String, String>,
    cfg: &ShellConfig,
) -> Result<BTreeMap<String, String>, ExecError> {
    // The keys an agent can actually set per call: in env.allow AND not in the
    // dangerous denylist. Listing the raw env.allow would name HOME/PATH (both
    // default-allowed but always-rejected) as "settable", contradicting the very
    // error that rejected them and sending the agent into a retry loop.
    let settable = cfg
        .env
        .allow
        .iter()
        .filter(|k| !is_dangerous_env_key(k))
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    for key in env.keys() {
        if is_dangerous_env_key(key) {
            return Err(ExecError::new(
                "S210",
                format!(
                    "env key '{key}' is never settable per-call (exec-hijacking key); \
                     remove it. Settable keys (env.allow minus exec-hijacking keys): [{settable}]"
                ),
            ));
        }
        if !cfg.env.allow.iter().any(|a| a == key) {
            return Err(ExecError::new(
                "S210",
                format!(
                    "env key '{key}' is not in the operator's env.allow list; the operator must \
                     permit it. Settable keys: [{settable}]"
                ),
            ));
        }
    }
    Ok(env.clone())
}

/// Build the validated [`ExecOverrides`] from the raw request fields, applying
/// all gating. Returns an `ExecError` (with the S-code the caller surfaces on
/// the wire) on any violation. All inputs `None` ⇒ an empty `ExecOverrides`,
/// i.e. unchanged default behaviour.
///
/// `scope_root` (when set) scopes the call to a session directory: it becomes
/// BOTH the confinement root for `cwd` AND the effective working directory when
/// no `cwd` is given.
///
/// - `scope_root=None`: today's behaviour — `cwd` is confined to the jail roots
///   and an omitted `cwd` leaves the working dir to fall back to
///   `cfg.working_dir`.
/// - `scope_root=Some`, `cwd=None`: the child runs in `scope_root`.
/// - `scope_root=Some`, `cwd=Some`: the child runs in `cwd`, which must resolve
///   inside `scope_root` (relative anchors there; an absolute cwd outside it is
///   S220).
pub fn build_overrides(
    cwd: Option<&str>,
    env: Option<&BTreeMap<String, String>>,
    scope_root: Option<&str>,
    scope_grants: Option<&[String]>,
    cfg: &ShellConfig,
) -> Result<ExecOverrides, ExecError> {
    let (host_roots_canon, denylist_canon) = jail_inputs(cfg)?;
    // Resolve the session directory first: it gates the cwd confinement below
    // and, when no cwd is supplied, becomes the working directory itself.
    let scope_root_canon = confine_scope_root(scope_root, &denylist_canon)?;
    let scope_grants_canon = crate::fs::host::confine_scope_grants(scope_grants, &denylist_canon);

    let cwd = match cwd {
        Some(c) => Some(confine_cwd(
            c,
            &host_roots_canon,
            scope_root_canon.as_deref(),
            &scope_grants_canon,
            &denylist_canon,
        )?),
        // No explicit cwd: a session scope_root (already validated as an existing
        // directory) becomes the working directory, so the child runs in the
        // session root instead of `cfg.working_dir`. Without scope_root this stays
        // None and build_command falls back to `cfg.working_dir` (unchanged).
        None => scope_root_canon,
    };
    let env = match env {
        Some(e) => Some(validate_env(e, cfg)?),
        None => None,
    };
    // `stdin` is set by the handler after this call (it needs no gating); leave
    // it None here so build_overrides stays the cwd/env confinement entry point.
    Ok(ExecOverrides {
        cwd,
        env,
        stdin: None,
    })
}

/// The `scope_root` that actually applies to a given exec `target`.
///
/// `scope_root` scopes a HOST working directory. A sandbox target runs against the
/// VM's own filesystem, so an injected `scope_root` is meaningless there and must
/// be dropped: left in place it materializes (via [`build_overrides`]) as a
/// populated host `cwd` override, which both exec sandbox paths reject as
/// host-only (S210). Since the harness stamps `scope_root` onto EVERY `shell::*`
/// call, that would break every sandbox exec issued from a working-dir-scoped
/// chat. Genuine user-supplied `cwd`/`env`/`stdin` on a sandbox call are still
/// rejected — only the harness `scope_root` is dropped here.
pub fn scope_root_for_target<'a>(target: &Target, scope_root: Option<&'a str>) -> Option<&'a str> {
    match target {
        Target::Host => scope_root,
        Target::Sandbox { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_jailed(root: &std::path::Path) -> ShellConfig {
        let mut c = ShellConfig {
            env: crate::config::EnvConfig {
                allow: vec!["NODE_ENV".into(), "MY_VAR".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        c.fs.host_roots = vec![root.to_path_buf()];
        c
    }

    // scope_root applies to a HOST target verbatim, but must be dropped for a
    // sandbox target — otherwise the harness-injected session dir becomes a
    // populated host cwd override and the sandbox exec paths reject it (S210),
    // breaking sandbox exec for every working-dir-scoped chat.
    #[test]
    fn scope_root_dropped_for_sandbox_target_kept_for_host() {
        let sandbox = Target::Sandbox {
            sandbox_id: uuid::Uuid::nil(),
        };
        assert_eq!(
            scope_root_for_target(&sandbox, Some("/work/session-7")),
            None
        );
        assert_eq!(scope_root_for_target(&sandbox, None), None);
        assert_eq!(
            scope_root_for_target(&Target::Host, Some("/work/session-7")),
            Some("/work/session-7")
        );
        assert_eq!(scope_root_for_target(&Target::Host, None), None);
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
            "BASH_ENV",
            "ENV",
        ] {
            assert!(is_dangerous_env_key(k), "{k} must be dangerous");
        }
        assert!(!is_dangerous_env_key("NODE_ENV"));
        // Family-prefix coverage: loader vars NOT in the explicit list (future
        // / less-common names) are still rejected by the LD_/DYLD_ prefix.
        for k in ["LD_BIND_NOW", "LD_SOMETHING_NEW", "DYLD_PRINT_LIBRARIES"] {
            assert!(
                is_dangerous_env_key(k),
                "{k} must be rejected by family prefix"
            );
            assert!(!DANGEROUS_ENV_KEYS.contains(&k), "{k} is prefix-only");
        }
    }

    #[test]
    fn env_in_allowed_is_accepted() {
        let c = ShellConfig {
            env: crate::config::EnvConfig {
                allow: vec!["NODE_ENV".into()],
                ..Default::default()
            },
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
            env: crate::config::EnvConfig {
                allow: vec!["NODE_ENV".into(), "MY_VAR".into()],
                ..Default::default()
            },
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
        // The denylist must WIN over env.allow: an operator typo listing
        // LD_PRELOAD must not open a code-injection hole.
        let c = ShellConfig {
            env: crate::config::EnvConfig {
                allow: vec!["LD_PRELOAD".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let mut env = BTreeMap::new();
        env.insert("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string());
        let err = validate_env(&env, &c).expect_err("dangerous key must reject");
        assert_eq!(err.code, "S210");
        assert!(err.message.contains("LD_PRELOAD"));
        assert!(err.message.contains("exec-hijacking"));
    }

    /// Test shim: derive the canonical jail inputs from `cfg` and confine `cwd`
    /// against the jail roots with NO session scope_root, so the existing
    /// no-scope_root cwd tests keep asserting the same contract against the new
    /// four-arg `confine_cwd`.
    fn confine_cwd_via_cfg(cwd: &str, cfg: &ShellConfig) -> Result<PathBuf, ExecError> {
        let (host_roots_canon, denylist_canon) = jail_inputs(cfg)?;
        confine_cwd(cwd, &host_roots_canon, None, &[], &denylist_canon)
    }

    #[test]
    fn cwd_inside_jail_resolves() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let c = cfg_jailed(&root);
        let canon = confine_cwd_via_cfg("sub", &c).expect("cwd inside jail");
        assert_eq!(canon, root.join("sub").canonicalize().unwrap());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cwd_escaping_jail_is_rejected_s215() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let c = cfg_jailed(&root);
        let err = confine_cwd_via_cfg("../../etc", &c).expect_err("escape rejected");
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
        let err = confine_cwd_via_cfg("nope", &c).expect_err("missing cwd rejected");
        assert!(err.code == "S211" || err.code == "S210", "got {}", err.code);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cwd_pointing_at_a_file_is_rejected_s210() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("f.txt"), "x").unwrap();
        let c = cfg_jailed(&root);
        let err = confine_cwd_via_cfg("f.txt", &c).expect_err("file-as-cwd rejected");
        assert_eq!(err.code, "S210");
        assert!(err.message.contains("not a directory"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn build_overrides_both_none_is_empty() {
        let c = ShellConfig::default();
        let ov = build_overrides(None, None, None, None, &c).expect("ok");
        assert!(ov.is_empty());
    }

    // --- per-call scope_root (session scope) ---

    /// With scope_root set, a relative cwd anchors at scope_root (not the jail root),
    /// and an OMITTED cwd makes scope_root itself the working directory.
    #[test]
    fn scope_root_anchors_relative_cwd_and_becomes_default_cwd() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("session/inner")).unwrap();
        let c = cfg_jailed(&root);
        let base = root.join("session").to_string_lossy().into_owned();

        // cwd omitted ⇒ working dir is scope_root.
        let ov = build_overrides(None, None, Some(&base), None, &c).expect("scope_root is valid");
        assert_eq!(
            ov.cwd.as_deref(),
            Some(root.join("session").canonicalize().unwrap().as_path()),
            "omitted cwd defaults to scope_root"
        );

        // relative cwd anchors at scope_root, not the jail root.
        let ov = build_overrides(Some("inner"), None, Some(&base), None, &c)
            .expect("inner is under scope_root");
        assert_eq!(
            ov.cwd.as_deref(),
            Some(root.join("session/inner").canonicalize().unwrap().as_path()),
            "relative cwd anchors at scope_root"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// DX-1: an ABSOLUTE cwd that is inside the jail but OUTSIDE scope_root is
    /// rejected with the new S220 code, and the message names the session dir.
    #[test]
    fn abs_cwd_inside_jail_but_outside_scope_root_is_s220_naming_session() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("session")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        let c = cfg_jailed(&root);
        let outside = root.join("other").canonicalize().unwrap();
        let base = root.join("session").to_string_lossy().into_owned();
        let err = build_overrides(Some(outside.to_str().unwrap()), None, Some(&base), None, &c)
            .expect_err("abs cwd outside scope_root must reject");
        assert_eq!(err.code, "S220", "must be the distinct scope_root code");
        let session_canon = root.join("session").canonicalize().unwrap();
        assert!(
            err.message.contains(&session_canon.display().to_string()),
            "S220 message must name the session dir, got: {}",
            err.message
        );
        // It must NOT reuse the generic 'outside every allowed root' S215 wording.
        assert!(
            !err.message.contains("escapes the fs jail"),
            "must not contradict the tool's own roots, got: {}",
            err.message
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// An absolute selected scope_root outside the configured jail roots is
    /// trusted as the working directory root for exec.
    #[test]
    fn absolute_scope_root_outside_jail_roots_is_honored() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        let selected =
            std::env::temp_dir().join(format!("shell-policy-selected-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(selected.join("inner")).unwrap();
        let c = cfg_jailed(&root);
        let selected_raw = selected.to_string_lossy().into_owned();
        let ov = build_overrides(None, None, Some(&selected_raw), None, &c)
            .expect("absolute selected scope_root is valid");
        assert_eq!(
            ov.cwd.as_deref(),
            Some(selected.canonicalize().unwrap().as_path())
        );
        let ov = build_overrides(Some("inner"), None, Some(&selected_raw), None, &c)
            .expect("relative cwd anchors at selected scope_root");
        assert_eq!(
            ov.cwd.as_deref(),
            Some(selected.join("inner").canonicalize().unwrap().as_path())
        );
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&selected).ok();
    }

    /// `scope_root` is trusted harness metadata, so relative values are rejected
    /// instead of interpreted relative to the jail root.
    #[test]
    fn relative_scope_root_is_rejected() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let c = cfg_jailed(&root);
        let err = build_overrides(None, None, Some("../../etc"), None, &c)
            .expect_err("relative scope_root is not part of the trusted contract");
        assert_eq!(err.code, "S210");
        std::fs::remove_dir_all(&root).ok();
    }

    /// With no session scope, a relative cwd still anchors at the primary jail root and an
    /// omitted cwd stays None (build_command then falls back to cfg.working_dir).
    #[test]
    fn scope_root_none_is_unchanged_behaviour() {
        let root = std::env::temp_dir().join(format!("shell-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let c = cfg_jailed(&root);

        // Omitted cwd + omitted scope_root ⇒ no cwd override (cfg.working_dir wins).
        let ov = build_overrides(None, None, None, None, &c).expect("ok");
        assert!(
            ov.cwd.is_none(),
            "no scope_root, no cwd ⇒ None (prior behaviour)"
        );

        // Relative cwd still anchors at the primary jail root when scope_root is absent.
        let ov = build_overrides(Some("sub"), None, None, None, &c).expect("ok");
        assert_eq!(
            ov.cwd.as_deref(),
            Some(root.join("sub").canonicalize().unwrap().as_path()),
            "relative cwd anchors at the primary jail root when scope_root is absent"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
