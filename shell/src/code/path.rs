//! Path resolution and access control.
//!
//! The worker is jailed to a set of allowed roots (`base_paths`; the
//! legacy `base_path` is honored as a one-entry list). Relative wire
//! paths resolve against the FIRST root (the "primary"); absolute wire
//! paths are accepted when they canonicalise inside ANY allowed root.
//! `PathResolver` canonicalises inputs (symlink-aware) and verifies
//! containment so `..` and crafted symlinks cannot escape. A `GlobSet`
//! built from `non_accessible_globs` further blocks read/write/delete on
//! sensitive entries (`.env`, `*.pem`, …) while still allowing them to
//! appear in `list-folder`/`tree` listings; globs match the path
//! *relative to its containing root*. A second GlobSet compiled from
//! `default_exclude_globs` (same matching convention) is a hide-only
//! noise filter applied by `coder::tree` — opt-out per call, never
//! access control.
//!
//! Every `coder::*` response carries canonical ABSOLUTE paths (decision
//! D2-eng) so multi-root results are unambiguous, and handlers operate
//! ONLY on resolver-returned paths — never on operands re-derived from
//! the raw request after validation.
//!
//! The symlink-safe canonicalisation walks to the longest existing
//! ancestor, canonicalises that, then lexically collapses the tail. That
//! leaf (`canonicalize_with_fallback` + `normalize_lexical`) is no longer
//! duplicated: it lives once in [`crate::path`] and is imported here and by
//! `shell::fs::host`, so there is a single jail-safety implementation (the
//! old MIRROR-INVARIANT between two copies is gone).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::code::config::CoderConfig;
use crate::code::error::CoderError;
use crate::path::canonicalize_with_fallback;

#[derive(Debug)]
pub struct PathResolver {
    /// Canonical allowed roots, in configuration order. Index 0 is the
    /// primary root that relative wire paths resolve against.
    roots_canon: Vec<PathBuf>,
    non_accessible: GlobSet,
    /// Compiled `default_exclude_globs` exactly as configured. Used to
    /// omit matching NON-directory entries from `coder::tree`. Hide-only
    /// noise filter — never an access-control surface; that is
    /// `non_accessible`'s job.
    default_exclude: GlobSet,
    /// `default_exclude_globs` plus `/**`-stripped dir-boundary
    /// companions, so `**/node_modules/**` also catches the
    /// `node_modules` directory itself. Checked against DIRECTORIES only
    /// — on other entry kinds the companions would wrongly drop a file
    /// or symlink merely NAMED like an excluded directory.
    default_exclude_dirs: GlobSet,
}

/// Effective roots when neither `base_paths` nor legacy `base_path` is
/// configured: the engine workspace cwd plus `/tmp` (a deliberate,
/// user-approved default).
fn default_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("./"), PathBuf::from("/tmp")]
}

/// Prefix of the roots listing inside C215 messages. The recovery-pair
/// test (`c215_error_text_alone_enables_successful_second_call`) parses
/// the allowed roots back out of the error text using this marker — keep
/// the format! sites and this const in lockstep.
const C215_ROOTS_PREFIX: &str = "Allowed roots: ";

/// Standard re-route hint appended to C215 messages. The recovery-pair
/// test uses `". " + SHELL_FS_HINT` as the end-of-roots-list marker.
const SHELL_FS_HINT: &str =
    "Use a path inside an allowed root, or the shell worker's shell::fs::* for other host paths.";

/// `"<p1>, <p2>"` display form for path lists in error messages.
fn display_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

impl PathResolver {
    pub fn new(cfg: &CoderConfig) -> Result<Self, CoderError> {
        let configured: Vec<PathBuf> = match (&cfg.base_path, cfg.base_paths.as_slice()) {
            (Some(_), [_, ..]) => {
                return Err(CoderError::BadInput(
                    "both `base_path` and `base_paths` are set; set either \
                     `base_path` or `base_paths` in config.yaml, not both. \
                     Remove `base_path` and keep only `base_paths` \
                     (legacy `base_path` is honored as a one-entry list)."
                        .into(),
                ))
            }
            (Some(single), []) => vec![single.clone()],
            (None, []) => default_roots(),
            (None, many) => many.to_vec(),
        };

        let mut roots_canon: Vec<PathBuf> = Vec::with_capacity(configured.len());
        for root in &configured {
            match std::fs::canonicalize(root) {
                Ok(canon) if roots_canon.contains(&canon) => tracing::warn!(
                    root = %root.display(),
                    canonical = %canon.display(),
                    "dropping duplicate root: same canonical path already configured"
                ),
                Ok(canon) => roots_canon.push(canon),
                Err(e) => tracing::warn!(
                    root = %root.display(),
                    error = %e,
                    "skipping unreachable root: cannot canonicalize"
                ),
            }
        }
        if roots_canon.is_empty() {
            // C210 like the both-set case above: an operator config error
            // detected at construction time, not a runtime I/O failure.
            return Err(CoderError::BadInput(format!(
                "no reachable roots: none of [{}] could be canonicalized. \
                 Ensure the directories exist and are accessible, then set \
                 `base_paths` in config.yaml to at least one reachable path.",
                display_paths(&configured)
            )));
        }

        let non_accessible = compile_globset(&cfg.non_accessible_globs, "non_accessible_glob")?;
        let default_exclude = compile_globset(&cfg.default_exclude_globs, "default_exclude_glob")?;
        let default_exclude_dirs = compile_globset(
            &with_dir_companions(&cfg.default_exclude_globs),
            "default_exclude_glob",
        )?;

        tracing::info!(roots = ?roots_canon, "path resolver roots");
        Ok(Self {
            roots_canon,
            non_accessible,
            default_exclude,
            default_exclude_dirs,
        })
    }

    /// Primary root — the first configured (and reachable) root. Relative
    /// wire paths resolve against it.
    pub fn base_root(&self) -> &Path {
        &self.roots_canon[0]
    }

    /// All canonical allowed roots, in configuration order.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots_canon
    }

    /// True when `p` is exactly one of the allowed roots.
    pub fn is_root(&self, p: &Path) -> bool {
        self.roots_canon.iter().any(|r| r == p)
    }

    /// The allowed root containing `canon`, if any. First match in
    /// configuration order (only relevant when roots nest).
    pub fn containing_root(&self, canon: &Path) -> Option<&Path> {
        self.roots_canon
            .iter()
            .find(|r| canon.starts_with(r))
            .map(PathBuf::as_path)
    }

    /// Canonical form of a session `base_dir` (the per-call working directory
    /// the harness scopes a call to), using the SAME canonicalisation as
    /// [`resolve_in`]. `None` when `base_dir` cannot be canonicalised or sits
    /// outside every allowed root — exactly the conditions under which
    /// `resolve_in` rejects, so any caller that already resolved a path through
    /// this `base_dir` gets `Some`.
    ///
    /// Used to refuse operations that target the session directory itself (which
    /// is a SUBDIR of an allowed root, so [`is_root`] does not catch it).
    ///
    /// [`resolve_in`]: Self::resolve_in
    /// [`is_root`]: Self::is_root
    pub fn session_root(&self, base_dir: &str) -> Option<PathBuf> {
        let base_canon = self.canonicalize_wire(base_dir, Path::new(base_dir)).ok()?;
        self.containing_root(&base_canon)
            .is_some()
            .then_some(base_canon)
    }

    /// Comma-separated display of all allowed roots, for C215 messages.
    fn roots_list(&self) -> String {
        display_paths(&self.roots_canon)
    }

    /// Resolve a wire `path` to a canonical absolute path inside the
    /// jail. The path need not exist; we walk to the longest existing
    /// ancestor, canonicalise that, then lexically collapse the tail.
    ///
    /// - relative inputs resolve against the primary (first) root and
    ///   must stay inside it → escape is `C215`
    /// - absolute inputs are accepted when they canonicalise inside ANY
    ///   allowed root; outside all roots → `C215`
    /// - dangling symlinks in the tail → `C215`
    pub fn resolve(&self, path: &str) -> Result<PathBuf, CoderError> {
        let wire = Path::new(path);
        let is_absolute = wire.is_absolute();
        let joined = if is_absolute {
            wire.to_path_buf()
        } else {
            self.base_root().join(wire)
        };
        let canon = self.canonicalize_wire(path, &joined)?;
        let inside = if is_absolute {
            self.containing_root(&canon).is_some()
        } else {
            canon.starts_with(self.base_root())
        };
        if !inside {
            if is_absolute {
                // Absolute path outside every allowed root. The marker
                // consts are parsed back out by the recovery-pair test.
                return Err(CoderError::OutsideBase(format!(
                    "path is outside every allowed root: {path}. \
                     {C215_ROOTS_PREFIX}{roots}. {SHELL_FS_HINT}",
                    roots = self.roots_list()
                )));
            } else {
                // Relative path that escaped the primary root (e.g. via `..`).
                let primary = self.base_root().display();
                return Err(CoderError::OutsideBase(format!(
                    "path escapes the primary allowed root {primary}: {path}. \
                     Relative paths resolve against {primary}; \
                     use an absolute path inside an allowed root instead."
                )));
            }
        }
        Ok(canon)
    }

    /// Path's location relative to its CONTAINING root as a forward-slash
    /// string, suitable for glob matching. Returns `None` if `abs` isn't
    /// under any allowed root (should never happen for paths that came
    /// out of `resolve`).
    pub fn relative(&self, abs: &Path) -> Option<String> {
        let root = self.containing_root(abs)?;
        abs.strip_prefix(root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    /// True if `abs`'s root-relative form matches any non-accessible
    /// glob. `abs` is expected to be a path previously returned by
    /// `resolve`. Glob semantics are per containing root, so a pattern
    /// like `**/.env` blocks `.env` in every allowed root.
    pub fn is_non_accessible(&self, abs: &Path) -> bool {
        self.matches_rel(&self.non_accessible, abs)
    }

    /// True if `abs`'s root-relative form matches a
    /// `default_exclude_globs` entry exactly as configured. `coder::tree`
    /// omits matching NON-directory entries. Hide-only: it must never
    /// gate access — `is_non_accessible` does that.
    pub fn is_default_excluded(&self, abs: &Path) -> bool {
        self.matches_rel(&self.default_exclude, abs)
    }

    /// Directory-boundary form of [`is_default_excluded`]: additionally
    /// matches the `/**`-stripped companions, so `**/node_modules/**`
    /// catches the `node_modules` directory itself and descent can be
    /// suppressed. Call this for DIRECTORIES only — on other entry kinds
    /// the companions would match files/symlinks merely NAMED like an
    /// excluded directory.
    ///
    /// [`is_default_excluded`]: Self::is_default_excluded
    pub fn is_default_excluded_dir(&self, abs: &Path) -> bool {
        self.matches_rel(&self.default_exclude_dirs, abs)
    }

    fn matches_rel(&self, set: &GlobSet, abs: &Path) -> bool {
        let Some(rel) = self.relative(abs) else {
            return false;
        };
        if rel.is_empty() {
            return false;
        }
        set.is_match(&rel)
    }

    /// Resolve and reject if the result is on the non-accessible list.
    /// Used by every mutating operation and by `read-file` so the same
    /// glob hides both reads and writes.
    ///
    /// The C211 message is intentionally identical in wording to the
    /// not-found case (REDACTION INVARIANT: callers must not be able to
    /// distinguish "denied" from "missing" by observing the error text).
    pub fn require_writable(&self, rel: &str) -> Result<PathBuf, CoderError> {
        let abs = self.resolve(rel)?;
        if self.is_non_accessible(&abs) {
            return Err(CoderError::not_found_or_denied(rel));
        }
        Ok(abs)
    }

    /// Shared symlink-safe canonicalisation of a joined wire path, with the
    /// standardized C215/C211/C216 error mapping. Extracted verbatim from
    /// `resolve` so `resolve` and `resolve_in` map I/O failures identically
    /// — `path` is the caller-supplied wire string used in the messages.
    fn canonicalize_wire(&self, path: &str, joined: &Path) -> Result<PathBuf, CoderError> {
        canonicalize_with_fallback(joined).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("dangling symlink in path") {
                // Dangling symlink: name the containing root context so the
                // caller knows where they are allowed to work. The marker
                // consts are parsed back out by the recovery-pair test.
                CoderError::OutsideBase(format!(
                    "{path}: {msg}. {C215_ROOTS_PREFIX}{roots}. {SHELL_FS_HINT}",
                    roots = self.roots_list()
                ))
            } else if e.kind() == std::io::ErrorKind::InvalidInput
                || e.kind() == std::io::ErrorKind::NotFound
            {
                // Not-found / invalid during ancestor walk — treated as
                // C211 (not-found-or-denied) rather than C215. The single
                // constructor guarantees the standardized wording.
                CoderError::not_found_or_denied(path)
            } else {
                CoderError::Io(format!("canonicalize {path}: {e}"))
            }
        })
    }

    /// Resolve a wire `path` for a session scoped to `base_dir` — the
    /// per-call working directory the harness sets. A containment check and
    /// relative-anchor LAYERED on top of the existing resolver, reusing
    /// `containing_root` and the shared `canonicalize_wire` algorithm; the
    /// MIRROR-INVARIANT jail core is untouched.
    ///
    /// Semantics (all three checks, in order):
    /// 1. `base_dir` itself must canonicalise inside one of the configured
    ///    allowed roots — else `C215` (an operator/route error: the session
    ///    was scoped to a directory the worker is not allowed to serve).
    /// 2. relative `path` anchors at `base_dir` (not the primary root);
    ///    absolute `path` is taken as-is. Either form runs through the SAME
    ///    `canonicalize_wire` as `resolve`.
    /// 3. the canonical result must stay inside the canonical `base_dir` —
    ///    else `C218` (DX-1), which NAMES the session directory rather than
    ///    reusing the generic "outside every allowed root" wording.
    ///
    /// This is strictly NARROWER than `resolve`: it can only reject paths
    /// `resolve` would accept (those inside an allowed root but outside the
    /// session), never widen access. `base_dir = None` callers use
    /// `resolve`/`require_writable` and are byte-for-byte unchanged.
    pub fn resolve_in(&self, base_dir: &str, path: &str) -> Result<PathBuf, CoderError> {
        // (c) base_dir must canonicalise inside an EXISTING allowed root.
        // Reuse the shared canonicalisation so a `..`/symlink escape in the
        // session dir itself fails closed exactly like a wire path would.
        let base_canon = self.canonicalize_wire(base_dir, Path::new(base_dir))?;
        if self.containing_root(&base_canon).is_none() {
            return Err(CoderError::OutsideBase(format!(
                "base_dir is outside every allowed root: {base_dir}. \
                 {C215_ROOTS_PREFIX}{roots}. The session working directory \
                 must canonicalize inside one of the allowed roots; {SHELL_FS_HINT}",
                roots = self.roots_list()
            )));
        }

        // (a) relative anchors at base_dir; absolute is taken as-is.
        let wire = Path::new(path);
        let joined = if wire.is_absolute() {
            wire.to_path_buf()
        } else {
            base_canon.join(wire)
        };
        let canon = self.canonicalize_wire(path, &joined)?;

        // (b) the canonical result must stay inside the session directory.
        // DX-1: when the path IS inside an allowed root but escaped the
        // session, name the session dir — do NOT reuse the generic
        // "outside every allowed root" wording, which would contradict
        // coder::info's allowed-roots list for a path that genuinely lives
        // in a root.
        if !canon.starts_with(&base_canon) {
            let base = base_canon.display();
            if self.containing_root(&canon).is_some() {
                return Err(CoderError::OutsideSession(format!(
                    "this session is scoped to {base}; {path} is inside an \
                     allowed root but outside the session directory — use a \
                     path under {base}."
                )));
            }
            // Outside the session AND outside every allowed root: the
            // generic C215 wording is correct and most actionable here.
            return Err(CoderError::OutsideBase(format!(
                "path is outside the session directory {base} and outside \
                 every allowed root: {path}. {C215_ROOTS_PREFIX}{roots}. {SHELL_FS_HINT}",
                roots = self.roots_list()
            )));
        }
        Ok(canon)
    }

    /// `require_writable` for a session scoped to `base_dir`: resolve via
    /// `resolve_in`, then apply the same non-accessible glob gate. Mutating
    /// ops in `base_dir`-present mode call this instead of `require_writable`.
    pub fn require_writable_in(&self, base_dir: &str, rel: &str) -> Result<PathBuf, CoderError> {
        let abs = self.resolve_in(base_dir, rel)?;
        if self.is_non_accessible(&abs) {
            return Err(CoderError::not_found_or_denied(rel));
        }
        Ok(abs)
    }

    /// Dispatch helper: resolve `path` against `base_dir` when present,
    /// else exactly as `resolve` (the back-compat path is byte-for-byte
    /// unchanged). Handlers thread the per-call `base_dir` straight through.
    pub fn resolve_opt(&self, base_dir: Option<&str>, path: &str) -> Result<PathBuf, CoderError> {
        match base_dir {
            Some(b) => self.resolve_in(b, path),
            None => self.resolve(path),
        }
    }

    /// Dispatch helper mirroring [`resolve_opt`] for mutating/accessibility-
    /// gated reads: `require_writable_in` when `base_dir` is present, else
    /// the unchanged `require_writable`.
    ///
    /// [`resolve_opt`]: Self::resolve_opt
    pub fn require_writable_opt(
        &self,
        base_dir: Option<&str>,
        rel: &str,
    ) -> Result<PathBuf, CoderError> {
        match base_dir {
            Some(b) => self.require_writable_in(b, rel),
            None => self.require_writable(rel),
        }
    }

    /// Clone this resolver with `base_canon` ADDED to the allowed roots —
    /// the per-session working directory the harness scoped the call to.
    /// A *selected* directory is thereby folded into the jail, so every
    /// downstream check treats it as a first-class root: containment in
    /// [`resolve`]/[`resolve_in`], the non-accessible denylist (which
    /// relativises via [`containing_root`]), [`is_root`], and
    /// [`session_root`]. No-op (the root is not duplicated) when
    /// `base_canon` already sits inside a configured root — the existing
    /// in-jail behaviour is byte-for-byte unchanged.
    ///
    /// [`resolve`]: Self::resolve
    /// [`resolve_in`]: Self::resolve_in
    /// [`containing_root`]: Self::containing_root
    /// [`is_root`]: Self::is_root
    /// [`session_root`]: Self::session_root
    fn with_session_root(&self, base_canon: PathBuf) -> Self {
        let mut roots_canon = self.roots_canon.clone();
        if !roots_canon.iter().any(|r| base_canon.starts_with(r)) {
            roots_canon.push(base_canon);
        }
        Self {
            roots_canon,
            non_accessible: self.non_accessible.clone(),
            default_exclude: self.default_exclude.clone(),
            default_exclude_dirs: self.default_exclude_dirs.clone(),
        }
    }

    /// Per-call jail for a session scoped to `base_dir`: returns a resolver
    /// whose roots include the SELECTED working directory, so a directory
    /// the operator picked in the console (delivered as `base_dir`) is
    /// reachable instead of rejected with C215.
    ///
    /// Returns the shared resolver UNCHANGED when there is no `base_dir`,
    /// when `base_dir` already canonicalises inside a configured root (the
    /// common case — only an `Arc` bump), or when it cannot be
    /// canonicalised (the handler's own `resolve_in` then produces the
    /// precise error). Only when the selected directory sits OUTSIDE the
    /// configured jail is it added via [`with_session_root`].
    ///
    /// This is safe to widen on: `base_dir` is stamped by the harness
    /// control plane (workspace injection), never by the model, so only
    /// operator-chosen directories grow the jail — and `resolve_in` still
    /// scopes access to the session directory, while the non-accessible
    /// denylist still applies because the selected directory is now a real
    /// root.
    ///
    /// [`with_session_root`]: Self::with_session_root
    pub fn session_scoped(self: &Arc<Self>, base_dir: Option<&str>) -> Arc<Self> {
        let Some(bd) = base_dir else {
            return self.clone();
        };
        let Ok(base_canon) = self.canonicalize_wire(bd, Path::new(bd)) else {
            return self.clone();
        };
        if self.containing_root(&base_canon).is_some() {
            return self.clone();
        }
        Arc::new(self.with_session_root(base_canon))
    }
}

fn compile_globset(patterns: &[String], key: &str) -> Result<GlobSet, CoderError> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let g = Glob::new(pat)
            .map_err(|e| CoderError::BadInput(format!("invalid {key} {pat:?}: {e}")))?;
        builder.add(g);
    }
    builder
        .build()
        .map_err(|e| CoderError::BadInput(format!("globset build failed: {e}")))
}

/// A pattern like `**/node_modules/**` matches paths INSIDE the directory
/// but not the directory itself, so descent suppression at the dir
/// boundary would never trigger; compile a `/**`-stripped companion
/// (`**/node_modules`) alongside each such pattern so the boundary
/// matches too. The degenerate pattern `/**` would strip to an empty
/// companion, which is dropped.
fn with_dir_companions(patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .flat_map(|p| {
            let companion = p
                .strip_suffix("/**")
                .filter(|s| !s.is_empty())
                .map(String::from);
            std::iter::once(p.clone()).chain(companion)
        })
        .collect()
}

// `canonicalize_with_fallback` + `normalize_lexical` are the shared jail-safety
// leaf, imported from `crate::path` (one implementation for both the
// `shell::fs::*` jail and this folded `code` resolver). They used to live here
// byte-for-byte under the MIRROR-INVARIANT note; the merge removed that hazard.

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn cfg_roots(roots: Vec<PathBuf>, globs: Vec<&str>) -> CoderConfig {
        CoderConfig {
            base_paths: roots,
            non_accessible_globs: globs.into_iter().map(String::from).collect(),
            ..CoderConfig::default()
        }
    }

    fn cfg_with(base: PathBuf, globs: Vec<&str>) -> CoderConfig {
        cfg_roots(vec![base], globs)
    }

    fn canon(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).unwrap()
    }

    #[test]
    fn resolve_dot_returns_base_root() {
        let tmp = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let got = r.resolve(".").unwrap();
        assert_eq!(got, canon(tmp.path()));
    }

    #[test]
    fn resolve_existing_subpath() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/a.txt"), b"hi").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let got = r.resolve("sub/a.txt").unwrap();
        assert!(got.ends_with("sub/a.txt"));
        assert!(got.starts_with(r.base_root()));
    }

    #[test]
    fn resolve_nonexistent_inside_base_succeeds_via_fallback() {
        let tmp = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let got = r.resolve("does/not/exist.txt").unwrap();
        assert!(got.starts_with(r.base_root()));
    }

    #[test]
    fn relative_resolves_against_primary_root_only() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        // The file only exists in the SECOND root; a relative wire path
        // must still anchor at the primary (first) root.
        std::fs::write(b.path().join("f.txt"), b"secondary").unwrap();
        let r = PathResolver::new(&cfg_roots(
            vec![a.path().to_path_buf(), b.path().to_path_buf()],
            vec![],
        ))
        .unwrap();
        let got = r.resolve("f.txt").unwrap();
        assert!(got.starts_with(canon(a.path())));
        assert!(!got.starts_with(canon(b.path())));
    }

    #[test]
    fn absolute_inside_any_root_accepted() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        std::fs::write(a.path().join("x.txt"), b"x").unwrap();
        std::fs::write(b.path().join("y.txt"), b"y").unwrap();
        let r = PathResolver::new(&cfg_roots(
            vec![a.path().to_path_buf(), b.path().to_path_buf()],
            vec![],
        ))
        .unwrap();
        let in_a = r
            .resolve(&a.path().join("x.txt").display().to_string())
            .unwrap();
        assert!(in_a.starts_with(canon(a.path())));
        let in_b = r
            .resolve(&b.path().join("y.txt").display().to_string())
            .unwrap();
        assert!(in_b.starts_with(canon(b.path())));
    }

    #[test]
    fn absolute_outside_all_roots_rejected_with_c215() {
        let tmp = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let err = r.resolve("/etc/passwd").unwrap_err();
        assert_eq!(err.code(), "C215");
    }

    // REGRESSION PIN: `..` escapes must keep failing closed.
    #[test]
    fn resolve_dotdot_escape_rejected_as_outside_base() {
        let tmp = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let err = r.resolve("../etc/passwd").unwrap_err();
        assert_eq!(err.code(), "C215");
    }

    #[test]
    fn absolute_dotdot_escape_rejected_per_root() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let r = PathResolver::new(&cfg_roots(
            vec![a.path().to_path_buf(), b.path().to_path_buf()],
            vec![],
        ))
        .unwrap();
        // `<root b>/../escape.txt` collapses to b's PARENT — outside both.
        let input = format!("{}/../escape.txt", b.path().display());
        let err = r.resolve(&input).unwrap_err();
        assert_eq!(err.code(), "C215");
    }

    #[test]
    fn resolve_through_symlink_escape_rejected() {
        let tmp = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("escape")).unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let err = r.resolve("escape").unwrap_err();
        assert_eq!(err.code(), "C215");
    }

    #[test]
    fn dangling_symlink_in_tail_rejected_with_c215() {
        let tmp = tempdir().unwrap();
        std::os::unix::fs::symlink(tmp.path().join("missing-target"), tmp.path().join("dangle"))
            .unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let err = r.resolve("dangle/child.txt").unwrap_err();
        assert_eq!(err.code(), "C215");
    }

    #[test]
    fn root_and_input_both_canonicalize_before_containment_check() {
        // On macOS `/tmp` is a symlink to `/private/tmp`; both the
        // configured root and the wire input must be canonicalized before
        // the starts_with comparison, or every absolute `/tmp/...` input
        // would be rejected. The expectation is canonicalized too so this
        // passes on Linux (where /tmp is already canonical).
        let r = PathResolver::new(&cfg_roots(vec![PathBuf::from("/tmp")], vec![])).unwrap();
        let name = format!("coder-multiroot-test-{}", std::process::id());
        let got = r.resolve(&format!("/tmp/{name}")).unwrap();
        let expected = std::fs::canonicalize("/tmp").unwrap().join(&name);
        assert_eq!(got, expected);
    }

    #[test]
    fn same_filename_in_two_roots_resolves_to_each_own_root() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        std::fs::write(a.path().join("same.txt"), b"a").unwrap();
        std::fs::write(b.path().join("same.txt"), b"b").unwrap();
        let r = PathResolver::new(&cfg_roots(
            vec![a.path().to_path_buf(), b.path().to_path_buf()],
            vec![],
        ))
        .unwrap();
        let in_a = r
            .resolve(&a.path().join("same.txt").display().to_string())
            .unwrap();
        let in_b = r
            .resolve(&b.path().join("same.txt").display().to_string())
            .unwrap();
        assert_ne!(in_a, in_b, "absolute responses must disambiguate roots");
        assert!(in_a.starts_with(canon(a.path())));
        assert!(in_b.starts_with(canon(b.path())));
    }

    #[test]
    fn both_base_path_and_base_paths_set_is_construction_error() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let cfg = CoderConfig {
            base_path: Some(a.path().to_path_buf()),
            base_paths: vec![b.path().to_path_buf()],
            ..CoderConfig::default()
        };
        let err = PathResolver::new(&cfg).unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn zero_reachable_roots_is_construction_error() {
        let cfg = cfg_roots(
            vec![
                PathBuf::from("/this/does/not/exist/a-xyz123"),
                PathBuf::from("/this/does/not/exist/b-xyz123"),
            ],
            vec![],
        );
        let err = PathResolver::new(&cfg).unwrap_err();
        // C210: operator config error, same class as the both-set case.
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn unreachable_root_among_several_is_skipped() {
        let a = tempdir().unwrap();
        let cfg = cfg_roots(
            vec![
                a.path().to_path_buf(),
                PathBuf::from("/this/does/not/exist/xyz123"),
            ],
            vec![],
        );
        let r = PathResolver::new(&cfg).unwrap();
        assert_eq!(r.roots().len(), 1);
        assert_eq!(r.base_root(), canon(a.path()));
        assert!(r.resolve(".").is_ok());
    }

    #[test]
    fn duplicate_roots_deduped_after_canonicalization() {
        // The same directory configured twice — once verbatim, once in its
        // canonical form (on macOS tempdirs live under /var/folders, an
        // alias of /private/var/folders, so these strings can differ) —
        // must collapse to a single canonical root.
        let tmp = tempdir().unwrap();
        let cfg = cfg_roots(vec![tmp.path().to_path_buf(), canon(tmp.path())], vec![]);
        let r = PathResolver::new(&cfg).unwrap();
        assert_eq!(r.roots().len(), 1);
        assert_eq!(r.base_root(), canon(tmp.path()));
    }

    #[test]
    fn non_accessible_glob_matches_per_containing_root() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        std::fs::write(a.path().join(".env"), b"x").unwrap();
        std::fs::write(b.path().join(".env"), b"x").unwrap();
        let r = PathResolver::new(&cfg_roots(
            vec![a.path().to_path_buf(), b.path().to_path_buf()],
            vec!["**/.env"],
        ))
        .unwrap();
        let abs_a = r
            .resolve(&a.path().join(".env").display().to_string())
            .unwrap();
        let abs_b = r
            .resolve(&b.path().join(".env").display().to_string())
            .unwrap();
        assert!(r.is_non_accessible(&abs_a), ".env in root[0] must match");
        assert!(r.is_non_accessible(&abs_b), ".env in root[1] must match");
    }

    #[test]
    fn session_scoped_adds_selected_dir_outside_jail() {
        // A directory the operator selects (delivered as `base_dir`) that
        // lives OUTSIDE the configured jail must become reachable — added
        // into the effective roots — while the denylist still applies and
        // access stays scoped to that directory.
        let jail = tempdir().unwrap();
        let selected = tempdir().unwrap();
        std::fs::write(selected.path().join("a.txt"), b"x").unwrap();
        std::fs::write(selected.path().join(".env"), b"secret").unwrap();
        let r = std::sync::Arc::new(
            PathResolver::new(&cfg_roots(vec![jail.path().to_path_buf()], vec!["**/.env"]))
                .unwrap(),
        );
        let sel = selected.path().display().to_string();

        // Before scoping: the selected dir is outside every allowed root.
        assert!(
            r.resolve_in(&sel, "a.txt").is_err(),
            "selected dir outside the jail must reject until added"
        );

        // session_scoped folds the selected dir into the jail roots.
        let scoped = r.session_scoped(Some(&sel));
        let abs = scoped
            .resolve_in(&sel, "a.txt")
            .expect("selected dir reachable once added to host_roots");
        assert!(abs.starts_with(canon(selected.path())));

        // The non-accessible denylist still guards the added root.
        let env_abs = scoped.resolve_in(&sel, ".env").unwrap();
        assert!(
            scoped.is_non_accessible(&env_abs),
            "**/.env must still be blocked inside the selected dir"
        );

        // A base_dir already inside a configured root is a no-op.
        let inside = r.session_scoped(Some(&jail.path().display().to_string()));
        assert_eq!(inside.roots().len(), r.roots().len());

        // No base_dir is a no-op too.
        assert_eq!(r.session_scoped(None).roots().len(), r.roots().len());
    }

    #[test]
    fn default_config_constructs_resolver() {
        // CI interface collection boots the worker with zero config from a
        // scratch cwd; the defaults are ["./", "/tmp"] and "./" must
        // canonicalize from whatever cwd the process happens to have.
        let r = PathResolver::new(&CoderConfig::default()).unwrap();
        assert!(!r.roots().is_empty());
        assert_eq!(
            r.base_root(),
            std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap()
        );
    }

    #[test]
    fn legacy_base_path_honored_as_single_root() {
        let tmp = tempdir().unwrap();
        let cfg = CoderConfig {
            base_path: Some(tmp.path().to_path_buf()),
            ..CoderConfig::default()
        };
        let r = PathResolver::new(&cfg).unwrap();
        assert_eq!(r.roots().len(), 1);
        assert_eq!(r.resolve(".").unwrap(), canon(tmp.path()));
        let err = r.resolve("/etc/passwd").unwrap_err();
        assert_eq!(err.code(), "C215");
    }

    #[test]
    fn is_non_accessible_matches_root_dotenv() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), b"x").unwrap();
        let r = PathResolver::new(&cfg_with(
            tmp.path().to_path_buf(),
            vec!["**/.env", "**/.env.*"],
        ))
        .unwrap();
        let abs = r.resolve(".env").unwrap();
        assert!(r.is_non_accessible(&abs));
    }

    #[test]
    fn is_non_accessible_matches_nested_dotenv() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        std::fs::write(tmp.path().join("a/b/.env"), b"x").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec!["**/.env"])).unwrap();
        let abs = r.resolve("a/b/.env").unwrap();
        assert!(r.is_non_accessible(&abs));
    }

    #[test]
    fn is_non_accessible_false_for_unrelated_file() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("hello.txt"), b"x").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec!["**/.env"])).unwrap();
        let abs = r.resolve("hello.txt").unwrap();
        assert!(!r.is_non_accessible(&abs));
    }

    #[test]
    fn require_writable_rejects_non_accessible_with_c211() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), b"x").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec!["**/.env"])).unwrap();
        let err = r.require_writable(".env").unwrap_err();
        assert_eq!(err.code(), "C211");
    }

    #[test]
    fn new_with_invalid_glob_returns_bad_input() {
        let tmp = tempdir().unwrap();
        let err = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec!["["])).unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn new_with_invalid_default_exclude_glob_returns_bad_input() {
        let tmp = tempdir().unwrap();
        let cfg = CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            default_exclude_globs: vec!["[".to_string()],
            ..CoderConfig::default()
        };
        let err = PathResolver::new(&cfg).unwrap_err();
        assert_eq!(err.code(), "C210");
        assert!(
            err.to_string().contains("default_exclude_glob"),
            "message must name the config key: {err}"
        );
    }

    // DIR-BOUNDARY PIN: `**/node_modules/**` only matches paths INSIDE
    // the directory; the dir-set companion must catch the directory
    // itself so descent suppression triggers at the boundary — while the
    // plain set must NOT match the bare name, or files/symlinks merely
    // named like an excluded directory would be dropped.
    #[test]
    fn default_exclude_matches_dir_itself_not_just_children() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(tmp.path().join("sub/node_modules")).unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let dir = r.resolve("node_modules").unwrap();
        assert!(
            r.is_default_excluded_dir(&dir),
            "the directory itself must match, not only its children"
        );
        assert!(
            !r.is_default_excluded(&dir),
            "companions live only in the dir set: a non-directory entry \
             merely NAMED node_modules must not match the plain set"
        );
        let child = r.resolve("node_modules/pkg").unwrap();
        assert!(r.is_default_excluded(&child));
        assert!(r.is_default_excluded_dir(&child));
        let nested = r.resolve("sub/node_modules").unwrap();
        assert!(
            r.is_default_excluded_dir(&nested),
            "nested dir boundary must match"
        );
    }

    #[test]
    fn default_exclude_false_for_ordinary_paths() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/main.rs"), b"x").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let f = r.resolve("src/main.rs").unwrap();
        assert!(!r.is_default_excluded(&f));
        let d = r.resolve("src").unwrap();
        assert!(!r.is_default_excluded(&d));
        assert!(!r.is_default_excluded_dir(&d));
    }

    #[test]
    fn dir_companions_derived_only_for_slash_star_star_suffixes() {
        let patterns = vec![
            "**/node_modules/**".to_string(),
            "**/*.log".to_string(),
            "/**".to_string(),
        ];
        assert_eq!(
            with_dir_companions(&patterns),
            vec![
                "**/node_modules/**".to_string(),
                "**/node_modules".to_string(),
                "**/*.log".to_string(),
                "/**".to_string(),
            ],
            "no companion for non-/** patterns; empty companion of the \
             degenerate /** must be dropped"
        );
    }

    #[test]
    fn degenerate_slash_star_star_exclude_still_constructs() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        let cfg = CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            default_exclude_globs: vec!["/**".to_string()],
            ..CoderConfig::default()
        };
        let r = PathResolver::new(&cfg).expect("empty companion must be dropped, not compiled");
        let d = r.resolve("src").unwrap();
        assert!(!r.is_default_excluded_dir(&d));
    }

    // REDACTION INVARIANT separation: the hide-only exclude set must not
    // bleed into the access-control set.
    #[test]
    fn default_exclude_does_not_make_paths_non_accessible() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("node_modules")).unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let dir = r.resolve("node_modules").unwrap();
        assert!(!r.is_non_accessible(&dir));
        assert!(r.require_writable("node_modules/x.txt").is_ok());
    }

    #[test]
    fn legacy_missing_base_path_is_construction_error() {
        let cfg = CoderConfig {
            base_path: Some(PathBuf::from("/this/does/not/exist/probably/xyz123")),
            ..CoderConfig::default()
        };
        let err = PathResolver::new(&cfg).unwrap_err();
        // C210: operator config error, same class as the both-set case.
        assert_eq!(err.code(), "C210");
    }

    // RECOVERY-PAIR TEST: parse the first allowed root out of the C215 error
    // text, write a file there, then verify success. This proves the error
    // message alone contains enough information for a caller to make a
    // successful second call.
    //
    // The message format is:
    //   "... {C215_ROOTS_PREFIX}<root1>, <root2>. {SHELL_FS_HINT}"
    // We parse using the SAME consts the format! sites use, so the test
    // and the message can never drift apart.
    #[test]
    fn c215_error_text_alone_enables_successful_second_call() {
        let tmp = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();

        // Trigger C215 with an absolute path outside the root.
        let err = r.resolve("/etc/passwd").unwrap_err();
        assert_eq!(err.code(), "C215");
        let msg = err.to_string();

        // Parse the first allowed root from the error text using the
        // shared marker consts.
        let after_prefix = msg
            .split(C215_ROOTS_PREFIX)
            .nth(1)
            .expect("C215 message must contain C215_ROOTS_PREFIX");

        // The roots list ends where the shell::fs re-route hint begins.
        let hint_marker = format!(". {SHELL_FS_HINT}");
        let roots_list = after_prefix
            .split(hint_marker.as_str())
            .next()
            .expect("roots section must be followed by the shell::fs hint");

        // The first root is everything up to the first ", " (or the whole
        // string if there is only one root).
        let first_root = roots_list
            .split(", ")
            .next()
            .expect("at least one root")
            .trim()
            .to_string();

        // Now write a file inside that first root and verify it resolves.
        let target = format!("{first_root}/c215_recovery_test.txt");
        std::fs::write(&target, b"ok").unwrap();
        let resolved = r
            .resolve(&target)
            .expect("writing to a path parsed from the C215 error text must succeed");
        assert!(resolved.starts_with(canon(tmp.path())));
        // Cleanup.
        let _ = std::fs::remove_file(&target);
    }

    // NOTE: the C211 identical-wording invariant (missing vs glob-denied)
    // is pinned end-to-end through a real handler call in
    // functions::update_file::handler_tests::
    // c211_wording_identical_for_missing_and_glob_denied.

    // C215 absolute messages name the primary root for relative-escape
    // and name all roots for absolute-escape.
    #[test]
    fn c215_relative_escape_names_primary_root_in_message() {
        let tmp = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let err = r.resolve("../../../etc/passwd").unwrap_err();
        assert_eq!(err.code(), "C215");
        let msg = err.to_string();
        let primary = r.base_root().display().to_string();
        assert!(
            msg.contains(&primary),
            "C215 relative-escape message must name the primary root \
             ({primary}); got: {msg}"
        );
    }

    #[test]
    fn c215_absolute_outside_message_names_all_roots() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let r = PathResolver::new(&cfg_roots(
            vec![a.path().to_path_buf(), b.path().to_path_buf()],
            vec![],
        ))
        .unwrap();
        let err = r.resolve("/etc/passwd").unwrap_err();
        assert_eq!(err.code(), "C215");
        let msg = err.to_string();
        let root_a = canon(a.path()).display().to_string();
        let root_b = canon(b.path()).display().to_string();
        assert!(
            msg.contains(&root_a),
            "C215 absolute-outside message must name root_a ({root_a}); got: {msg}"
        );
        assert!(
            msg.contains(&root_b),
            "C215 absolute-outside message must name root_b ({root_b}); got: {msg}"
        );
        assert!(
            msg.contains(C215_ROOTS_PREFIX),
            "C215 message must contain C215_ROOTS_PREFIX; got: {msg}"
        );
    }

    // -----------------------------------------------------------------------
    // Per-call base_dir (resolve_in / require_writable_in): a containment
    // check + relative-anchor LAYERED on the existing jail core. These
    // exercise the new C218 DX-1 error and prove base_dir=None is unchanged.
    // -----------------------------------------------------------------------

    /// Relative wire paths anchor at base_dir, NOT the primary root — even
    /// when base_dir is a nested subdirectory of the single allowed root.
    #[test]
    fn resolve_in_relative_anchors_at_base_dir_not_primary_root() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("session/sub")).unwrap();
        std::fs::write(tmp.path().join("session/sub/a.txt"), b"hi").unwrap();
        // A same-named file directly under the root must NOT be the target.
        std::fs::write(tmp.path().join("a.txt"), b"root-level").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();

        let base = tmp.path().join("session").display().to_string();
        let got = r.resolve_in(&base, "sub/a.txt").unwrap();
        assert_eq!(got, canon(&tmp.path().join("session/sub/a.txt")));
        // Crucially it resolved under session/, not the primary root.
        assert!(got.starts_with(canon(&tmp.path().join("session"))));
        assert_ne!(got, canon(&tmp.path().join("a.txt")));
    }

    /// `.` resolves to base_dir itself (the anchor), not the primary root.
    #[test]
    fn resolve_in_dot_returns_base_dir() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("session")).unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let base = tmp.path().join("session").display().to_string();
        let got = r.resolve_in(&base, ".").unwrap();
        assert_eq!(got, canon(&tmp.path().join("session")));
    }

    /// An absolute wire path INSIDE base_dir is accepted as-is.
    #[test]
    fn resolve_in_absolute_inside_base_dir_ok() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("session")).unwrap();
        std::fs::write(tmp.path().join("session/f.txt"), b"x").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let base = tmp.path().join("session").display().to_string();
        let abs_in = tmp.path().join("session/f.txt").display().to_string();
        let got = r.resolve_in(&base, &abs_in).unwrap();
        assert_eq!(got, canon(&tmp.path().join("session/f.txt")));
    }

    /// DX-1: an absolute path that IS inside a configured root but OUTSIDE
    /// base_dir is rejected with the new C218 code, and the message NAMES
    /// the session directory (not the generic "outside every allowed root"
    /// wording, which would contradict coder::info's allowed-roots list).
    #[test]
    fn resolve_in_absolute_inside_root_but_outside_base_dir_rejected_c218_naming_session() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("session")).unwrap();
        std::fs::write(tmp.path().join("sibling.txt"), b"x").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();

        let base_canon = canon(&tmp.path().join("session"));
        let base = base_canon.display().to_string();
        // sibling.txt is inside the allowed root but OUTSIDE session/.
        let sibling = tmp.path().join("sibling.txt").display().to_string();
        let err = r.resolve_in(&base, &sibling).unwrap_err();
        assert_eq!(err.code(), "C218");
        let msg = err.to_string();
        // DX-1: the session directory must be named in the rejection.
        assert!(
            msg.contains(&base),
            "C218 must name the session dir ({base}); got: {msg}"
        );
        // DX-1: it must NOT reuse the generic "outside every allowed root"
        // wording — that path genuinely lives in an allowed root.
        assert!(
            !msg.contains("outside every allowed root"),
            "C218 must not contradict the allowed-roots list; got: {msg}"
        );
        // It should affirmatively explain the path is inside an allowed root.
        assert!(
            msg.contains("inside an allowed root"),
            "C218 should clarify the path is inside a root but outside the \
             session; got: {msg}"
        );
    }

    /// In a multi-root config, an absolute path inside the SECOND root but
    /// outside the base_dir (which lives in the FIRST root) is still the
    /// DX-1 C218 case — it is inside *an* allowed root, just not the session.
    #[test]
    fn resolve_in_absolute_in_other_root_outside_base_dir_is_c218() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        std::fs::write(b.path().join("y.txt"), b"y").unwrap();
        let r = PathResolver::new(&cfg_roots(
            vec![a.path().to_path_buf(), b.path().to_path_buf()],
            vec![],
        ))
        .unwrap();
        // Session is scoped to root A; the path lives in root B.
        let base = canon(a.path()).display().to_string();
        let in_b = b.path().join("y.txt").display().to_string();
        let err = r.resolve_in(&base, &in_b).unwrap_err();
        assert_eq!(err.code(), "C218");
        assert!(err.to_string().contains(&base));
    }

    /// base_dir that canonicalizes OUTSIDE every allowed root is a C215
    /// (operator/route error): the session was scoped to a directory the
    /// worker may not serve.
    #[test]
    fn resolve_in_base_dir_outside_all_roots_rejected_c215() {
        let tmp = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let err = r
            .resolve_in(&outside.path().display().to_string(), "f.txt")
            .unwrap_err();
        assert_eq!(err.code(), "C215");
        let msg = err.to_string();
        assert!(
            msg.contains("base_dir is outside every allowed root"),
            "C215 must explain the base_dir is unserveable; got: {msg}"
        );
        assert!(msg.contains(C215_ROOTS_PREFIX));
    }

    /// A `..` escape from base_dir that lands OUTSIDE every allowed root is
    /// rejected (fails closed — same guarantee as resolve()).
    #[test]
    fn resolve_in_dotdot_escape_outside_roots_rejected() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("session")).unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let base = tmp.path().join("session").display().to_string();
        // ../../ climbs above the allowed root entirely.
        let err = r.resolve_in(&base, "../../escape.txt").unwrap_err();
        assert!(
            err.code() == "C215" || err.code() == "C218",
            "escape must fail closed (C215/C218); got {}",
            err.code()
        );
    }

    /// A single `..` that stays inside the allowed root but climbs OUT of
    /// base_dir is the DX-1 C218 case (inside a root, outside the session).
    #[test]
    fn resolve_in_dotdot_within_root_but_outside_base_is_c218() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("session")).unwrap();
        std::fs::write(tmp.path().join("secret.txt"), b"x").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let base = tmp.path().join("session").display().to_string();
        // `../secret.txt` collapses to <root>/secret.txt — inside the root,
        // outside the session.
        let err = r.resolve_in(&base, "../secret.txt").unwrap_err();
        assert_eq!(err.code(), "C218");
        assert!(err.to_string().contains(&base));
    }

    /// A symlink inside base_dir pointing OUTSIDE the allowed root cannot be
    /// used to escape — same symlink-safety guarantee the jail core gives
    /// resolve().
    #[test]
    fn resolve_in_symlink_escape_from_base_dir_rejected() {
        let tmp = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("session")).unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("session/escape")).unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let base = tmp.path().join("session").display().to_string();
        let err = r.resolve_in(&base, "escape/child.txt").unwrap_err();
        assert!(
            err.code() == "C215" || err.code() == "C218",
            "symlink escape must fail closed (C215/C218); got {}",
            err.code()
        );
    }

    /// require_writable_in applies the non-accessible glob gate on top of
    /// the session containment (C211, identical to require_writable).
    #[test]
    fn require_writable_in_rejects_non_accessible_with_c211() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("session")).unwrap();
        std::fs::write(tmp.path().join("session/.env"), b"x").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec!["**/.env"])).unwrap();
        let base = tmp.path().join("session").display().to_string();
        let err = r.require_writable_in(&base, ".env").unwrap_err();
        assert_eq!(err.code(), "C211");
    }

    /// BACK-COMPAT: the dispatch helpers with base_dir=None must produce
    /// EXACTLY what resolve()/require_writable() produce — both the Ok path
    /// and the error path, byte-for-byte. This pins that the None branch is
    /// the unchanged legacy code.
    #[test]
    fn resolve_opt_none_is_identical_to_resolve() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/a.txt"), b"hi").unwrap();
        std::fs::write(tmp.path().join(".env"), b"secret").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec!["**/.env"])).unwrap();

        // Ok path: relative, existing nested, dot — all must match resolve().
        for p in [".", "sub/a.txt", "sub/does/not/exist.txt"] {
            assert_eq!(
                r.resolve_opt(None, p).unwrap(),
                r.resolve(p).unwrap(),
                "resolve_opt(None) diverged from resolve() for {p:?}"
            );
        }

        // Error path: identical code AND identical message text.
        for p in ["/etc/passwd", "../etc/passwd"] {
            let via_opt = r.resolve_opt(None, p).unwrap_err();
            let via_resolve = r.resolve(p).unwrap_err();
            assert_eq!(via_opt.code(), via_resolve.code());
            assert_eq!(
                via_opt.to_string(),
                via_resolve.to_string(),
                "resolve_opt(None) error text diverged for {p:?}"
            );
        }

        // require_writable_opt(None) must also mirror require_writable,
        // including the glob-denied C211.
        assert_eq!(
            r.require_writable_opt(None, ".env")
                .unwrap_err()
                .to_string(),
            r.require_writable(".env").unwrap_err().to_string(),
        );
        assert_eq!(
            r.require_writable_opt(None, "sub/a.txt").unwrap(),
            r.require_writable("sub/a.txt").unwrap(),
        );
    }
}
