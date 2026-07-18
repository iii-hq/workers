//! Path resolution and access control.
//!
//! The worker is jailed to a set of allowed roots (`base_paths`, filled at
//! runtime from `fs.host_roots`). Relative wire
//! paths resolve against the FIRST root (the "primary"); absolute wire
//! paths are accepted when they canonicalise inside ANY allowed root.
//! When the operator opts into the unjailed mode (`fs.allow_unjailed: true`
//! with empty `fs.host_roots`) the containment gate is skipped — matching
//! `shell::fs::*`'s deny-only policy — and the roots become relative-path
//! ANCHORS only; `fs.denylist_paths` and `non_accessible_globs` still
//! apply everywhere.
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
    grant_roots_canon: Vec<PathBuf>,
    /// Deny-only permissive mode (`fs.allow_unjailed: true`, empty
    /// `fs.host_roots`): the containment gate is skipped and `roots_canon`
    /// only anchors relative wire paths, mirroring `shell::fs::*`.
    unjailed: bool,
    /// Canonical `fs.denylist_paths`. A resolved path under any entry is
    /// rejected with the redacted C211 in every mode — the same operator
    /// denylist `shell::fs::*` enforces.
    denylist_canon: Vec<PathBuf>,
}

/// Effective roots when `base_paths` is empty: the engine workspace cwd
/// plus `/tmp` (a deliberate, user-approved default).
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
        let configured: Vec<PathBuf> = if cfg.base_paths.is_empty() {
            default_roots()
        } else {
            cfg.base_paths.clone()
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
            // C210: an operator config error detected at construction time,
            // not a runtime I/O failure.
            return Err(CoderError::BadInput(format!(
                "no reachable roots: none of [{}] could be canonicalized. \
                 Ensure the directories exist and are accessible, then set \
                 `fs.host_roots` to at least one reachable path.",
                display_paths(&configured)
            )));
        }

        let non_accessible = compile_globset(&cfg.non_accessible_globs, "non_accessible_glob")?;
        let default_exclude = compile_globset(&cfg.default_exclude_globs, "default_exclude_glob")?;
        let default_exclude_dirs = compile_globset(
            &with_dir_companions(&cfg.default_exclude_globs),
            "default_exclude_glob",
        )?;

        let mut denylist_canon = Vec::with_capacity(cfg.denylist_paths.len());
        for deny in &cfg.denylist_paths {
            match canonicalize_with_fallback(deny) {
                Ok(canon) => {
                    if !denylist_canon.contains(&canon) {
                        denylist_canon.push(canon);
                    }
                }
                Err(e) => tracing::warn!(
                    path = %deny.display(),
                    error = %e,
                    "skipping denylist path: cannot canonicalize"
                ),
            }
        }

        tracing::info!(roots = ?roots_canon, unjailed = cfg.unjailed, "path resolver roots");
        Ok(Self {
            roots_canon,
            non_accessible,
            default_exclude,
            default_exclude_dirs,
            grant_roots_canon: Vec::new(),
            unjailed: cfg.unjailed,
            denylist_canon,
        })
    }

    /// True when the operator opted into the deny-only permissive mode
    /// (`fs.allow_unjailed: true` with empty `fs.host_roots`).
    pub fn unjailed(&self) -> bool {
        self.unjailed
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

    /// Return a resolver that also treats a trusted, existing session
    /// directory as an allowed root. The configured roots stay intact; the
    /// session root is additive and only used for calls whose request still
    /// carries `scope_root`, so [`resolve_in`] continues to enforce the stricter
    /// "all paths stay under scope_root" containment check.
    pub fn session_scoped(
        self: &Arc<Self>,
        scope_root: Option<&str>,
        scope_grants: Option<&[String]>,
    ) -> Arc<Self> {
        let mut roots_canon = self.roots_canon.clone();
        let mut grant_roots_canon = self.grant_roots_canon.clone();
        let mut changed = false;

        if let Some(scope_root) = scope_root {
            let base_path = Path::new(scope_root);
            if base_path.is_absolute() {
                if let Ok(base_canon) = self.canonicalize_wire(scope_root, base_path) {
                    if base_canon.is_dir() && !roots_canon.contains(&base_canon) {
                        roots_canon.push(base_canon);
                        changed = true;
                    }
                }
            }
        }

        for extra in confine_scope_grants(scope_grants) {
            if !roots_canon.contains(&extra) {
                roots_canon.push(extra.clone());
                changed = true;
            }
            if !grant_roots_canon.contains(&extra) {
                grant_roots_canon.push(extra);
                changed = true;
            }
        }

        if !changed {
            return self.clone();
        }
        Arc::new(Self {
            roots_canon,
            non_accessible: self.non_accessible.clone(),
            default_exclude: self.default_exclude.clone(),
            default_exclude_dirs: self.default_exclude_dirs.clone(),
            grant_roots_canon,
            unjailed: self.unjailed,
            denylist_canon: self.denylist_canon.clone(),
        })
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

    /// Canonical form of a session `scope_root` (the per-call working directory
    /// the harness scopes a call to), using the SAME canonicalisation as
    /// [`resolve_in`]. `None` when `scope_root` cannot be canonicalised or sits
    /// outside every allowed root — exactly the conditions under which
    /// `resolve_in` rejects, so any caller that already resolved a path through
    /// this `scope_root` gets `Some`.
    ///
    /// Used to refuse operations that target the session directory itself (which
    /// is a SUBDIR of an allowed root, so [`is_root`] does not catch it).
    ///
    /// [`resolve_in`]: Self::resolve_in
    /// [`is_root`]: Self::is_root
    pub fn session_root(&self, scope_root: &str) -> Option<PathBuf> {
        let base_path = Path::new(scope_root);
        if !base_path.is_absolute() {
            return None;
        }
        let base_canon = self.canonicalize_wire(scope_root, base_path).ok()?;
        (self.unjailed || self.containing_root(&base_canon).is_some()).then_some(base_canon)
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
        // Unjailed (deny-only) mode: no containment — the roots only anchored
        // the relative join above. The denylist gate below still applies.
        let inside = self.unjailed
            || if is_absolute {
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
                     {C215_ROOTS_PREFIX}{roots}. {SHELL_FS_HINT}{hint}",
                    roots = self.roots_list(),
                    hint = crate::filesystem_access::request_suffix("C215", path, &canon),
                )));
            } else {
                // Relative path that escaped the primary root (e.g. via `..`).
                let primary = self.base_root().display();
                return Err(CoderError::OutsideBase(format!(
                    "path escapes the primary allowed root {primary}: {path}. \
                     Relative paths resolve against {primary}; \
                     use an absolute path inside an allowed root instead.{hint}",
                    hint = crate::filesystem_access::request_suffix("C215", path, &canon),
                )));
            }
        }
        self.deny_check(path, &canon)?;
        Ok(canon)
    }

    /// Reject a canonical path under any `fs.denylist_paths` entry with the
    /// redacted C211 — the REDACTION INVARIANT applies to the operator
    /// denylist exactly as it does to `non_accessible_globs`: a denylisted
    /// path must be indistinguishable from a missing one.
    fn deny_check(&self, path: &str, canon: &Path) -> Result<(), CoderError> {
        if self.denylist_canon.iter().any(|d| canon.starts_with(d)) {
            return Err(CoderError::not_found_or_denied(path));
        }
        Ok(())
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
        if let Some(rel) = self.relative(abs) {
            return !rel.is_empty() && set.is_match(&rel);
        }
        // Unjailed mode resolves paths outside every anchor root, where no
        // root-relative form exists. Match against the root-stripped absolute
        // form instead so `**/`-style patterns keep protecting secrets
        // anywhere on the host. Jailed resolutions never reach this branch.
        if self.unjailed {
            let abs_str = abs.to_string_lossy().replace('\\', "/");
            let stripped = abs_str.trim_start_matches('/');
            return !stripped.is_empty() && set.is_match(stripped);
        }
        false
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
                    "{path}: {msg}. {C215_ROOTS_PREFIX}{roots}. {SHELL_FS_HINT}{hint}",
                    roots = self.roots_list(),
                    hint = crate::filesystem_access::request_suffix("C215", path, joined),
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

    /// Resolve a wire `path` for a session scoped to `scope_root` — the
    /// per-call working directory the harness sets. A containment check and
    /// relative-anchor LAYERED on top of the existing resolver, reusing
    /// `containing_root` and the shared `canonicalize_wire` algorithm; the
    /// MIRROR-INVARIANT jail core is untouched.
    ///
    /// Semantics (all three checks, in order):
    /// 1. `scope_root` itself must be absolute and canonicalise inside one of the
    ///    configured allowed roots — else `C215` (an operator/route error: the session
    ///    was scoped to a directory the worker is not allowed to serve).
    /// 2. relative `path` anchors at `scope_root` (not the primary root);
    ///    absolute `path` is taken as-is. Either form runs through the SAME
    ///    `canonicalize_wire` as `resolve`.
    /// 3. the canonical result must stay inside the canonical `scope_root` —
    ///    else `C218` (DX-1), which NAMES the session directory rather than
    ///    reusing the generic "outside every allowed root" wording.
    ///
    /// This is strictly NARROWER than `resolve`: it can only reject paths
    /// `resolve` would accept (those inside an allowed root but outside the
    /// session), never widen access. `scope_root = None` callers use
    /// `resolve`/`require_writable`.
    pub fn resolve_in(&self, scope_root: &str, path: &str) -> Result<PathBuf, CoderError> {
        let base_path = Path::new(scope_root);
        if !base_path.is_absolute() {
            return Err(CoderError::BadInput(format!(
                "scope_root must be an absolute path: {scope_root}"
            )));
        }
        // (c) scope_root must canonicalise inside an EXISTING allowed root.
        // Reuse the shared canonicalisation so a `..`/symlink escape in the
        // session dir itself fails closed exactly like a wire path would.
        let base_canon = self.canonicalize_wire(scope_root, base_path)?;
        if self.containing_root(&base_canon).is_none() {
            return Err(CoderError::OutsideBase(format!(
                "scope_root is outside every allowed root: {scope_root}. \
                 {C215_ROOTS_PREFIX}{roots}. The session working directory \
                 must canonicalize inside one of the allowed roots; {SHELL_FS_HINT}",
                roots = self.roots_list()
            )));
        }

        // (a) relative anchors at scope_root; absolute is taken as-is.
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
        if !canon.starts_with(&base_canon)
            && !self.grant_roots_canon.iter().any(|r| canon.starts_with(r))
        {
            let base = base_canon.display();
            if self.containing_root(&canon).is_some() {
                return Err(CoderError::OutsideSession(format!(
                    "this session is scoped to {base}; {path} is inside an \
                     allowed root but outside the session directory — use a \
                     path under {base}.{hint}",
                    hint = crate::filesystem_access::request_suffix("C218", path, &canon),
                )));
            }
            // Outside the session AND outside every allowed root: the
            // generic C215 wording is correct and most actionable here.
            return Err(CoderError::OutsideBase(format!(
                "path is outside the session directory {base} and outside \
                 every allowed root: {path}. {C215_ROOTS_PREFIX}{roots}. {SHELL_FS_HINT}{hint}",
                roots = self.roots_list(),
                hint = crate::filesystem_access::request_suffix("C215", path, &canon),
            )));
        }
        self.deny_check(path, &canon)?;
        Ok(canon)
    }

    fn resolve_from(&self, anchor: &str, path: &str) -> Result<PathBuf, CoderError> {
        let anchor_path = Path::new(anchor);
        if !anchor_path.is_absolute() {
            return Err(CoderError::BadInput(format!(
                "scope_root must be an absolute path: {anchor}"
            )));
        }
        let anchor_canon = self.canonicalize_wire(anchor, anchor_path)?;
        // Unjailed mode trusts the harness-stamped working directory anywhere
        // on the host — the same contract shell::exec's cwd already honors.
        if !self.unjailed && self.containing_root(&anchor_canon).is_none() {
            return Err(CoderError::OutsideBase(format!(
                "scope_root is outside every allowed root: {anchor}. {C215_ROOTS_PREFIX}{roots}",
                roots = self.roots_list()
            )));
        }
        if Path::new(path).is_absolute() {
            self.resolve(path)
        } else {
            self.resolve(&anchor_canon.join(path).to_string_lossy())
        }
    }

    pub fn resolve_scope(
        &self,
        scope: Option<&crate::fs::FsScope>,
        path: &str,
    ) -> Result<PathBuf, CoderError> {
        match scope {
            Some(scope) if scope.boundary == crate::fs::FsBoundary::ConfiguredRoots => {
                self.resolve_from(scope.root(), path)
            }
            Some(scope) => self.resolve_in(scope.root(), path),
            None => self.resolve(path),
        }
    }

    pub fn require_writable_scope(
        &self,
        scope: Option<&crate::fs::FsScope>,
        path: &str,
    ) -> Result<PathBuf, CoderError> {
        let abs = self.resolve_scope(scope, path)?;
        if self.is_non_accessible(&abs) {
            return Err(CoderError::not_found_or_denied(path));
        }
        Ok(abs)
    }
}

fn confine_scope_grants(scope_grants: Option<&[String]>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(scope_grants) = scope_grants else {
        return out;
    };
    for raw in scope_grants {
        if raw.is_empty() {
            continue;
        }
        let p = Path::new(raw);
        if !p.is_absolute() {
            continue;
        }
        let Ok(canon) = canonicalize_with_fallback(p) else {
            continue;
        };
        if !canon.is_dir() {
            continue;
        }
        if !out.iter().any(|r| r == &canon) {
            out.push(canon);
        }
    }
    out
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
    use std::sync::Arc;
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
    fn zero_reachable_roots_is_construction_error() {
        let cfg = cfg_roots(
            vec![
                PathBuf::from("/this/does/not/exist/a-xyz123"),
                PathBuf::from("/this/does/not/exist/b-xyz123"),
            ],
            vec![],
        );
        let err = PathResolver::new(&cfg).unwrap_err();
        // C210: operator config error detected at construction time.
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn single_unreachable_root_is_construction_error() {
        // One-entry form of the case above (the shape a single-root
        // fs.host_roots produces): still an operator config error.
        let cfg = cfg_roots(
            vec![PathBuf::from("/this/does/not/exist/probably/xyz123")],
            vec![],
        );
        let err = PathResolver::new(&cfg).unwrap_err();
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
    fn single_base_path_entry_jails_to_that_root() {
        let tmp = tempdir().unwrap();
        let cfg = cfg_roots(vec![tmp.path().to_path_buf()], vec![]);
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
        let scope = crate::fs::FsScope {
            root: tmp.path().display().to_string(),
            grants: Vec::new(),
            boundary: crate::fs::FsBoundary::ConfiguredRoots,
        };
        let err = r.require_writable_scope(Some(&scope), ".env").unwrap_err();
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
        assert!(r.require_writable_scope(None, "node_modules/x.txt").is_ok());
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
    // Per-call scope_root (resolve_in / require_writable_scope): a containment
    // check + relative-anchor LAYERED on the existing jail core. These
    // exercise the new C218 DX-1 error and prove scope_root=None is unchanged.
    // -----------------------------------------------------------------------

    /// Relative wire paths anchor at scope_root, NOT the primary root — even
    /// when scope_root is a nested subdirectory of the single allowed root.
    #[test]
    fn resolve_in_relative_anchors_at_scope_root_not_primary_root() {
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

    /// `.` resolves to scope_root itself (the anchor), not the primary root.
    #[test]
    fn resolve_in_dot_returns_scope_root() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("session")).unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let base = tmp.path().join("session").display().to_string();
        let got = r.resolve_in(&base, ".").unwrap();
        assert_eq!(got, canon(&tmp.path().join("session")));
    }

    /// An absolute wire path INSIDE scope_root is accepted as-is.
    #[test]
    fn resolve_in_absolute_inside_scope_root_ok() {
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
    /// scope_root is rejected with the new C218 code, and the message NAMES
    /// the session directory (not the generic "outside every allowed root"
    /// wording, which would contradict coder::info's allowed-roots list).
    #[test]
    fn resolve_in_absolute_inside_root_but_outside_scope_root_rejected_c218_naming_session() {
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
    /// outside the scope_root (which lives in the FIRST root) is still the
    /// DX-1 C218 case — it is inside *an* allowed root, just not the session.
    #[test]
    fn resolve_in_absolute_in_other_root_outside_scope_root_is_c218() {
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

    /// scope_root that canonicalizes OUTSIDE every allowed root is a C215
    /// (operator/route error): the session was scoped to a directory the
    /// worker may not serve.
    #[test]
    fn resolve_in_scope_root_outside_all_roots_rejected_c215() {
        let tmp = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let err = r
            .resolve_in(&outside.path().display().to_string(), "f.txt")
            .unwrap_err();
        assert_eq!(err.code(), "C215");
        let msg = err.to_string();
        assert!(
            msg.contains("scope_root is outside every allowed root"),
            "C215 must explain the scope_root is unserveable; got: {msg}"
        );
        assert!(msg.contains(C215_ROOTS_PREFIX));
    }

    /// The live registration layer can trust a harness-provided `scope_root` by
    /// adding that directory as an ephemeral session root. This lets console
    /// sessions work in a selected host directory that is outside the configured
    /// coder roots while keeping every operation confined below that directory.
    #[test]
    fn session_scoped_scope_root_outside_roots_becomes_effective_root() {
        let jail = tempdir().unwrap();
        let selected = tempdir().unwrap();
        std::fs::create_dir(selected.path().join("sub")).unwrap();
        std::fs::write(selected.path().join("sub/a.txt"), b"hi").unwrap();
        let r = Arc::new(PathResolver::new(&cfg_with(jail.path().to_path_buf(), vec![])).unwrap());
        let base = selected.path().display().to_string();

        assert!(
            r.resolve_in(&base, "sub/a.txt").is_err(),
            "unscoped resolver must still reject a scope_root outside configured roots"
        );

        let scoped = r.session_scoped(Some(&base), None);
        let got = scoped.resolve_in(&base, "sub/a.txt").unwrap();
        assert_eq!(got, canon(&selected.path().join("sub/a.txt")));
    }

    #[test]
    fn session_scoped_scope_root_outside_roots_applies_non_accessible_globs() {
        let jail = tempdir().unwrap();
        let selected = tempdir().unwrap();
        std::fs::write(selected.path().join(".env"), b"secret").unwrap();
        let r = Arc::new(
            PathResolver::new(&cfg_with(jail.path().to_path_buf(), vec!["**/.env"])).unwrap(),
        );
        let base = selected.path().display().to_string();

        let scoped = r.session_scoped(Some(&base), None);
        let scope = crate::fs::FsScope {
            root: base,
            grants: Vec::new(),
            boundary: crate::fs::FsBoundary::Workspace,
        };
        let err = scoped
            .require_writable_scope(Some(&scope), ".env")
            .unwrap_err();
        assert_eq!(err.code(), "C211");
    }

    #[test]
    fn session_scoped_scope_grants_allow_absolute_grants_without_moving_relative_anchor() {
        let jail = tempdir().unwrap();
        let session = jail.path().join("session");
        std::fs::create_dir(&session).unwrap();
        std::fs::write(session.join("same-name.txt"), b"session").unwrap();
        let granted = tempdir().unwrap();
        std::fs::write(granted.path().join("same-name.txt"), b"grant").unwrap();
        let r = Arc::new(PathResolver::new(&cfg_with(jail.path().to_path_buf(), vec![])).unwrap());
        let base = session.display().to_string();
        let extras = vec![granted.path().display().to_string()];

        let scoped = r.session_scoped(Some(&base), Some(&extras));
        let absolute_grant = granted.path().join("same-name.txt").display().to_string();
        let got = scoped.resolve_in(&base, &absolute_grant).unwrap();
        assert_eq!(got, canon(&granted.path().join("same-name.txt")));

        let relative = scoped.resolve_in(&base, "same-name.txt").unwrap();
        assert_eq!(
            relative,
            canon(&session.join("same-name.txt")),
            "relative paths must keep anchoring at scope_root, not an extra root"
        );
    }

    /// A `..` escape from scope_root that lands OUTSIDE every allowed root is
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
    /// scope_root is the DX-1 C218 case (inside a root, outside the session).
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

    /// A symlink inside scope_root pointing OUTSIDE the allowed root cannot be
    /// used to escape — same symlink-safety guarantee the jail core gives
    /// resolve().
    #[test]
    fn resolve_in_symlink_escape_from_scope_root_rejected() {
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

    /// require_writable_scope applies the non-accessible glob gate on top of
    /// the session containment (C211, identical to require_writable).
    #[test]
    fn require_writable_scope_rejects_non_accessible_with_c211() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("session")).unwrap();
        std::fs::write(tmp.path().join("session/.env"), b"x").unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec!["**/.env"])).unwrap();
        let base = tmp.path().join("session").display().to_string();
        let scope = crate::fs::FsScope {
            root: base,
            grants: Vec::new(),
            boundary: crate::fs::FsBoundary::Workspace,
        };
        let err = r.require_writable_scope(Some(&scope), ".env").unwrap_err();
        assert_eq!(err.code(), "C211");
    }

    #[test]
    fn configured_roots_scope_anchors_relative_paths_and_allows_siblings() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("session")).unwrap();
        std::fs::create_dir_all(tmp.path().join("sibling")).unwrap();
        std::fs::write(tmp.path().join("session/local.txt"), b"local").unwrap();
        std::fs::write(tmp.path().join("sibling/shared.txt"), b"shared").unwrap();
        let resolver = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let scope = crate::fs::FsScope {
            root: tmp.path().join("session").display().to_string(),
            grants: Vec::new(),
            boundary: crate::fs::FsBoundary::ConfiguredRoots,
        };

        assert_eq!(
            resolver.resolve_scope(Some(&scope), "local.txt").unwrap(),
            canon(&tmp.path().join("session/local.txt"))
        );
        assert_eq!(
            resolver
                .resolve_scope(
                    Some(&scope),
                    &tmp.path().join("sibling/shared.txt").display().to_string(),
                )
                .unwrap(),
            canon(&tmp.path().join("sibling/shared.txt"))
        );
    }

    // ------------------------------------------------------------------
    // Unjailed (deny-only) mode: fs.allow_unjailed + empty fs.host_roots.
    // The roots become relative-path anchors; containment is skipped;
    // denylist_paths and non_accessible_globs still protect everywhere.
    // ------------------------------------------------------------------

    fn cfg_unjailed(anchor: PathBuf, globs: Vec<&str>) -> CoderConfig {
        CoderConfig {
            unjailed: true,
            ..cfg_with(anchor, globs)
        }
    }

    #[test]
    fn unjailed_accepts_absolute_path_outside_all_roots() {
        let anchor = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::fs::write(outside.path().join("free.txt"), b"free").unwrap();
        let r = PathResolver::new(&cfg_unjailed(anchor.path().to_path_buf(), vec![])).unwrap();
        let got = r
            .resolve(&outside.path().join("free.txt").display().to_string())
            .unwrap();
        assert_eq!(got, canon(&outside.path().join("free.txt")));
    }

    #[test]
    fn unjailed_relative_still_anchors_at_primary_root() {
        let anchor = tempdir().unwrap();
        std::fs::write(anchor.path().join("here.txt"), b"here").unwrap();
        let r = PathResolver::new(&cfg_unjailed(anchor.path().to_path_buf(), vec![])).unwrap();
        let got = r.resolve("here.txt").unwrap();
        assert_eq!(got, canon(&anchor.path().join("here.txt")));
    }

    #[test]
    fn unjailed_configured_roots_scope_anchors_outside_all_roots() {
        // Anthony's exact scenario (MOT-4099): the harness stamps a working
        // directory OUTSIDE every anchor root under the degraded
        // configured_roots boundary. Unjailed must trust it as the anchor.
        let anchor = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::write(workspace.path().join("doc.md"), b"doc").unwrap();
        let r = PathResolver::new(&cfg_unjailed(anchor.path().to_path_buf(), vec![])).unwrap();
        let scope = crate::fs::FsScope {
            root: workspace.path().display().to_string(),
            grants: Vec::new(),
            boundary: crate::fs::FsBoundary::ConfiguredRoots,
        };
        assert_eq!(
            r.resolve_scope(Some(&scope), "doc.md").unwrap(),
            canon(&workspace.path().join("doc.md"))
        );
    }

    #[test]
    fn jailed_configured_roots_scope_outside_roots_still_rejected() {
        // Without the opt-in the degraded boundary keeps failing closed —
        // the pre-MOT-4099 behavior for explicitly jailed deployments.
        let root = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(root.path().to_path_buf(), vec![])).unwrap();
        let scope = crate::fs::FsScope {
            root: workspace.path().display().to_string(),
            grants: Vec::new(),
            boundary: crate::fs::FsBoundary::ConfiguredRoots,
        };
        let err = r.resolve_scope(Some(&scope), "doc.md").unwrap_err();
        assert_eq!(err.code(), "C215");
        assert!(err.to_string().contains("scope_root is outside"), "{err}");
    }

    #[test]
    fn unjailed_non_accessible_globs_protect_outside_roots() {
        // The REDACTION INVARIANT must hold anywhere on the host: a
        // protected file outside every anchor root still returns C211.
        let anchor = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::fs::write(outside.path().join(".env"), b"SECRET=1").unwrap();
        let r =
            PathResolver::new(&cfg_unjailed(anchor.path().to_path_buf(), vec!["**/.env"])).unwrap();
        let abs = r
            .resolve(&outside.path().join(".env").display().to_string())
            .unwrap();
        assert!(r.is_non_accessible(&abs), "glob must match outside roots");
        let err = r
            .require_writable_scope(None, &outside.path().join(".env").display().to_string())
            .unwrap_err();
        assert_eq!(err.code(), "C211");
    }

    #[test]
    fn denylist_paths_reject_with_redacted_c211_in_both_modes() {
        let anchor = tempdir().unwrap();
        let denied = tempdir().unwrap();
        std::fs::write(denied.path().join("passwd"), b"x").unwrap();
        // Unjailed: the denylist is the only confinement — must hold.
        let cfg = CoderConfig {
            denylist_paths: vec![denied.path().to_path_buf()],
            ..cfg_unjailed(anchor.path().to_path_buf(), vec![])
        };
        let r = PathResolver::new(&cfg).unwrap();
        let err = r
            .resolve(&denied.path().join("passwd").display().to_string())
            .unwrap_err();
        assert_eq!(err.code(), "C211", "denylisted must be redacted, not C215");

        // Jailed with the denied dir INSIDE a root: the denylist still wins.
        let jailed_cfg = CoderConfig {
            base_paths: vec![denied.path().to_path_buf()],
            denylist_paths: vec![denied.path().to_path_buf()],
            ..CoderConfig::default()
        };
        let r = PathResolver::new(&jailed_cfg).unwrap();
        let err = r.resolve("passwd").unwrap_err();
        assert_eq!(err.code(), "C211");
    }

    #[test]
    fn unjailed_session_root_resolves_outside_all_roots() {
        // move/delete use session_root to refuse operating on the session
        // dir itself; that protection must survive the unjailed mode where
        // the stamped root sits outside every anchor root.
        let anchor = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let r = PathResolver::new(&cfg_unjailed(anchor.path().to_path_buf(), vec![])).unwrap();
        assert_eq!(
            r.session_root(&workspace.path().display().to_string()),
            Some(canon(workspace.path()))
        );
    }

    #[test]
    fn unjailed_workspace_boundary_still_scopes_to_session() {
        // With the approval hook live (workspace boundary), unjailed does
        // NOT bypass the session scope: escapes keep raising C218 so the
        // grant flow still triggers.
        let anchor = tempdir().unwrap();
        std::fs::create_dir_all(anchor.path().join("session")).unwrap();
        std::fs::create_dir_all(anchor.path().join("elsewhere")).unwrap();
        std::fs::write(anchor.path().join("elsewhere/f.txt"), b"f").unwrap();
        let r = Arc::new(
            PathResolver::new(&cfg_unjailed(anchor.path().to_path_buf(), vec![])).unwrap(),
        );
        let session = anchor.path().join("session").display().to_string();
        let scoped = r.session_scoped(Some(&session), None);
        let err = scoped
            .resolve_in(
                &session,
                &anchor.path().join("elsewhere/f.txt").display().to_string(),
            )
            .unwrap_err();
        assert_eq!(err.code(), "C218");
    }
}
