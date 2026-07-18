//! Host-filesystem backend.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use globset::{Glob, GlobSet, GlobSetBuilder};
use iii_sdk::channel::Channel;
use iii_sdk::errors::Error;

use crate::fs::error::FsError;
use crate::path::{canonicalize_with_fallback, normalize_lexical};

/// RAII guard that unlinks a temp path on drop unless `commit()` is called.
/// Sync `remove_file` in `Drop` is deliberate — best-effort cleanup on
/// panic/early-return without awaiting.
struct TempGuard {
    path: Option<PathBuf>,
}
impl TempGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }
    fn commit(mut self) {
        self.path = None;
    }
    fn path(&self) -> &std::path::Path {
        self.path.as_deref().expect("path always Some until commit")
    }
}
impl Drop for TempGuard {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[async_trait]
pub trait ChannelMaker: Send + Sync + std::fmt::Debug {
    async fn create_channel(&self, buffer: usize) -> Result<Channel, Error>;
    fn engine_address(&self) -> String;
}

pub struct IiiChannelMaker {
    iii: iii_sdk::IIIClient,
}

impl std::fmt::Debug for IiiChannelMaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IiiChannelMaker")
            .field("address", &self.iii.address())
            .finish()
    }
}

impl IiiChannelMaker {
    pub fn new(iii: iii_sdk::IIIClient) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl ChannelMaker for IiiChannelMaker {
    async fn create_channel(&self, buffer: usize) -> Result<Channel, Error> {
        iii_sdk::helpers::create_channel(&self.iii, Some(buffer)).await
    }
    fn engine_address(&self) -> String {
        self.iii.address().to_string()
    }
}

#[derive(Debug, Clone, Default)]
pub struct HostFsConfig {
    /// Effective jail roots (index 0 = primary; empty = unjailed). Built from
    /// `FsConfig::roots()` (the `fs.host_roots` list) as one canonical list.
    pub host_roots: Vec<PathBuf>,
    pub max_read_bytes: usize,
    pub max_write_bytes: usize,
    pub denylist_paths: Vec<PathBuf>,
    /// Unified protected-paths globs (D4): the SAME list the folded `code`
    /// surface reads (`code.non_accessible_globs`). A path whose form relative
    /// to its containing root matches is hard-rejected (S215) for read / write
    /// / delete / move. Matched files stay VISIBLE to `shell::fs::ls` (the
    /// directory listing is not gated) — visible but locked, declared once for
    /// both surfaces. Glob syntax + root-relative matching mirror the code
    /// resolver; the legacy absolute-prefix `denylist_paths` stays alongside.
    pub non_accessible_globs: Vec<String>,
    /// Permit setuid/setgid/sticky bits (`mode & 0o7000`) in mkdir/chmod/write.
    /// Default false rejects them with S210 — they are a privesc primitive
    /// when the worker runs as root inside the jail.
    pub allow_special_bits: bool,
}

/// Hard caps on attacker-controlled regex/sed patterns. An unbounded pattern
/// can stall compilation and pin memory; N concurrent calls = DoS.
const MAX_PATTERN_BYTES: usize = 4096;
const REGEX_SIZE_LIMIT: usize = 256 * 1024;

/// Hard ceiling on the size of a single file `sed` will read+rewrite when the
/// backend config sets no `max_read_bytes` (cap == 0). `sed` builds a
/// same-size output String in memory, so an unbounded file is an OOM vector;
/// N concurrent calls multiply it. 16 MiB mirrors a sane edit target.
const SED_MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Upper bound on bytes buffered while reading a SINGLE grep "line". A file
/// with no newlines would otherwise buffer the whole file before the
/// per-match `max_line_bytes` truncation ever ran. We stop reading a line at
/// this cap (discarding the remainder up to the next newline) so memory stays
/// bounded DURING the read, not just in the returned match.
const GREP_MAX_LINE_SCAN_BYTES: usize = 1024 * 1024;

/// Defaults reused when a caller passes 0 for the corresponding grep cap. 0
/// must mean "use the default", not "unlimited" — an unbounded match count or
/// line length is a memory/DoS vector. These mirror the schema defaults in
/// `fs/mod.rs` (`default_max_matches` / `default_max_line_bytes`).
const DEFAULT_GREP_MAX_MATCHES: usize = 10_000;
const DEFAULT_GREP_MAX_LINE_BYTES: usize = 4096;

/// Reject setuid/setgid/sticky bits unless the operator opted in. The policy
/// reads the backend's config flag at the call site; `parse_mode` stays a pure
/// octal parser. Centralized so mkdir/chmod/write stay consistent.
fn check_special_bits(bits: u32, allow_special_bits: bool) -> Result<(), FsError> {
    if !allow_special_bits && (bits & 0o7000) != 0 {
        return Err(FsError::new(
            "S210",
            format!(
                "setuid/setgid/sticky bits not allowed (mode {bits:04o}); \
                 set fs.allow_special_bits to permit"
            ),
        ));
    }
    Ok(())
}

/// Reject over-long patterns before compiling (or before literal use).
fn check_pattern_len(pattern: &str) -> Result<(), FsError> {
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err(FsError::new(
            "S210",
            format!(
                "pattern too long ({} bytes); max is {MAX_PATTERN_BYTES} bytes",
                pattern.len()
            ),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct HostFsBackend {
    cfg: Arc<HostFsConfig>,
    chan: Arc<dyn ChannelMaker>,
    /// Canonical form of each `cfg.host_roots` entry (index 0 = primary;
    /// empty = unjailed), computed once at construction. Pre-fix
    /// `validate_path` recanonicalized this on every fs op (and every denylist
    /// entry) — a real perf hit on hot paths and an operator-config-error
    /// vector that surfaced silently. Caching here also fails loudly at startup
    /// if any root is unreachable.
    host_roots_canon: Vec<PathBuf>,
    /// Canonical form of each `cfg.denylist_paths` entry. Same rationale
    /// as `host_roots_canon`; an entry that can't canonicalize is a config
    /// error and the worker refuses to start.
    denylist_canon: Vec<PathBuf>,
    /// Compiled `cfg.non_accessible_globs` (D4). Checked against a path's
    /// form relative to its containing root; a match hard-rejects access. A
    /// bad glob is a config error and the worker refuses to start (fail-closed).
    non_accessible: GlobSet,
}

impl HostFsBackend {
    #[allow(dead_code)]
    pub fn new(cfg: Arc<HostFsConfig>, chan: Arc<dyn ChannelMaker>) -> Self {
        match Self::try_new(cfg.clone(), chan.clone()) {
            Ok(b) => b,
            Err(e) => panic!(
                "HostFsBackend init failed: code={} message={}. Either fix \
                 the config or call try_new directly to handle the error.",
                e.code, e.message
            ),
        }
    }

    /// Resolve every jail root and `denylist_paths` entry to canonical form
    /// once at startup. Errors here are operator config bugs (path doesn't
    /// exist, can't be canonicalized, etc.) and the worker should refuse to
    /// start instead of degrading to lexical fallback per-call.
    pub fn try_new(cfg: Arc<HostFsConfig>, chan: Arc<dyn ChannelMaker>) -> Result<Self, FsError> {
        let mut host_roots_canon = Vec::with_capacity(cfg.host_roots.len());
        for root in &cfg.host_roots {
            let canon = std::fs::canonicalize(root).map_err(|e| {
                FsError::new(
                    "S216",
                    format!("jail root unreachable ({}): {e}", root.display()),
                )
            })?;
            // Dedup after canonicalization (a root listed twice, or once
            // verbatim and once canonical, collapses to one).
            if !host_roots_canon.contains(&canon) {
                host_roots_canon.push(canon);
            }
        }
        let mut denylist_canon = Vec::with_capacity(cfg.denylist_paths.len());
        for deny in &cfg.denylist_paths {
            let canon = canonicalize_with_fallback(Path::new(deny)).map_err(|e| {
                FsError::new(
                    "S216",
                    format!("denylist entry unreachable ({}): {e}", deny.display()),
                )
            })?;
            denylist_canon.push(canon);
        }
        let mut builder = GlobSetBuilder::new();
        for pat in &cfg.non_accessible_globs {
            let g = Glob::new(pat).map_err(|e| {
                FsError::new("S210", format!("invalid non_accessible glob {pat:?}: {e}"))
            })?;
            builder.add(g);
        }
        let non_accessible = builder.build().map_err(|e| {
            FsError::new("S210", format!("non_accessible globset build failed: {e}"))
        })?;
        Ok(Self {
            cfg,
            chan,
            host_roots_canon,
            denylist_canon,
            non_accessible,
        })
    }

    /// Primary jail root (index 0), if any. Relative paths and relative
    /// lexical operands anchor here.
    fn primary_root(&self) -> Option<&Path> {
        self.host_roots_canon.first().map(PathBuf::as_path)
    }

    /// True when `canon` (a path already confined inside a root) matches a
    /// `non_accessible` glob, matched against its form relative to the
    /// containing root (so `**/.env` blocks `.env` under every root). D4: the
    /// access gate shared with the code surface — visible to `ls`, locked here.
    fn is_non_accessible(&self, canon: &Path) -> bool {
        path_is_non_accessible(canon, &self.host_roots_canon, &self.non_accessible)
    }

    fn is_non_accessible_scoped(
        &self,
        canon: &Path,
        scope_root_canon: Option<&Path>,
        scope_grants_canon: &[PathBuf],
    ) -> bool {
        let roots = access_roots(&self.host_roots_canon, scope_root_canon, scope_grants_canon);
        path_is_non_accessible(canon, &roots, &self.non_accessible)
    }

    /// TOCTOU: check-then-use gate, open to a race against an attacker who
    /// can mutate the filesystem between validation and subsequent syscalls.
    /// Use the sandbox backend for untrusted input.
    pub(crate) fn validate_path(&self, path: &str) -> Result<PathBuf, FsError> {
        let canon = confine_path(path, &self.host_roots_canon, &self.denylist_canon)?;
        if self.is_non_accessible(&canon) {
            return Err(FsError::new(
                "S215",
                format!("path is protected (non_accessible): {path}"),
            ));
        }
        Ok(canon)
    }

    /// Resolve and validate an optional trusted per-call `scope_root`; see
    /// [`confine_scope_root`].
    fn confine_scope_root(&self, scope_root: Option<&str>) -> Result<Option<PathBuf>, FsError> {
        confine_scope_root(scope_root, &self.denylist_canon)
    }

    fn confine_scope_grants(&self, scope_grants: Option<&[String]>) -> Vec<PathBuf> {
        confine_scope_grants(scope_grants, &self.denylist_canon)
    }

    /// scope_root-aware form of [`Self::validate_path`]. When `scope_root_canon`
    /// is `Some`, the path is scoped to that session directory (relative
    /// anchors there; an absolute path outside it is S220). When `None`, it
    /// delegates to [`Self::validate_path`].
    fn validate_path_scoped(
        &self,
        path: &str,
        scope_root_canon: Option<&Path>,
        scope_grants_canon: &[PathBuf],
        boundary: crate::fs::FsBoundary,
    ) -> Result<PathBuf, FsError> {
        let restrict_to_workspace = boundary == crate::fs::FsBoundary::Workspace;
        let canon = match (scope_root_canon, restrict_to_workspace) {
            // validate_path already applies the non_accessible gate.
            (None, _) if scope_grants_canon.is_empty() => return self.validate_path(path),
            (None, _) => confine_path(
                path,
                &effective_roots(&self.host_roots_canon, None, scope_grants_canon),
                &self.denylist_canon,
            )?,
            (Some(base), false) => confine_path_with_anchor(
                path,
                &self.host_roots_canon,
                base,
                scope_grants_canon,
                &self.denylist_canon,
            )?,
            (Some(_), true) => confine_path_with_scope_root(
                path,
                &self.host_roots_canon,
                scope_root_canon,
                scope_grants_canon,
                &self.denylist_canon,
            )?,
        };
        let access_scope = restrict_to_workspace.then_some(scope_root_canon).flatten();
        if self.is_non_accessible_scoped(&canon, access_scope, scope_grants_canon) {
            return Err(FsError::new(
                "S215",
                format!("path is protected (non_accessible): {path}"),
            ));
        }
        Ok(canon)
    }

    /// Lexical operand for handlers whose semantics forbid canonicalizing
    /// (rm/chmod/mv/sed act on the link itself, not its target). Relative
    /// inputs anchor to the SAME jail root `validate_path` validated
    /// against, so the validated path and the operated-on path can never
    /// diverge (the worker's CWD is unrelated to the jail).
    fn lexical_operand(&self, path: &str) -> PathBuf {
        lexical_operand_with(path, self.primary_root())
    }

    /// scope_root-aware form of [`Self::lexical_operand`]. When a session
    /// `scope_root` is in effect, a relative operand must anchor at `scope_root`
    /// (the directory `validate_path_scoped` validated against) so the
    /// validated and operated-on paths cannot diverge. `None` ⇒ delegates to
    /// [`Self::lexical_operand`] (the unchanged primary-root-anchored operand).
    fn lexical_operand_scoped(
        &self,
        path: &str,
        scope_root_canon: Option<&Path>,
        _scope_grants_canon: &[PathBuf],
    ) -> PathBuf {
        match scope_root_canon {
            None => self.lexical_operand(path),
            Some(base) => lexical_operand_with(path, Some(base)),
        }
    }
}

fn confine_path_with_anchor(
    path: &str,
    host_roots_canon: &[PathBuf],
    anchor: &Path,
    scope_grants_canon: &[PathBuf],
    denylist_canon: &[PathBuf],
) -> Result<PathBuf, FsError> {
    let raw = Path::new(path);
    let anchored = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        anchor.join(raw)
    };
    let display = anchored.to_string_lossy();
    confine_path(
        &display,
        &effective_roots(host_roots_canon, None, scope_grants_canon),
        denylist_canon,
    )
}

/// Jail-confinement check, factored out of `HostFsBackend::validate_path` so
/// the same logic can run inside a `spawn_blocking` closure (which needs a
/// `'static + Send` body and so cannot borrow `&self`). Callers on the blocking
/// thread pass owned/cloned copies of the precomputed canonical root and
/// denylist. Behaviour, error codes (S210/S215) and the returned canonical
/// path are identical to the method form — this is a pure extraction, not a
/// weakening of any check.
///
/// `pub(crate)` so the exec backend can confine a per-call `cwd` against the
/// SAME jail the fs backend enforces (shell::exec/exec_bg `cwd`) instead of
/// duplicating the canonicalize / jail-root containment / denylist logic.
pub(crate) fn confine_path(
    path: &str,
    host_roots_canon: &[PathBuf],
    denylist_canon: &[PathBuf],
) -> Result<PathBuf, FsError> {
    let p = Path::new(path);
    let joined;
    let p = if p.is_absolute() {
        p
    } else if path.is_empty() {
        return Err(FsError::new("S210", "path must not be empty"));
    } else if let Some(primary) = host_roots_canon.first() {
        // Relative paths resolve against the PRIMARY (first) jail root. The
        // canonical containment check below still runs on the joined path, so
        // `..` in the relative form cannot escape the jail.
        joined = primary.join(p);
        joined.as_path()
    } else {
        return Err(FsError::new(
            "S210",
            format!("path must be absolute: {path}"),
        ));
    };
    let canon = canonicalize_with_fallback(p).map_err(|e| {
        // Dangling-symlink errors are structurally jail violations
        // (the path would otherwise resolve through a link that
        // pre-fix slipped past the lexical fallback). Map them to
        // S215 so wire telemetry treats them as such.
        let msg = format!("{e}");
        if msg.contains("dangling symlink in path") {
            FsError::new("S215", format!("{path}: {msg}"))
        } else {
            FsError::new("S210", format!("{path}: {msg}"))
        }
    })?;
    if !host_roots_canon.is_empty() {
        // Accept when the canonical path is inside ANY allowed root.
        if !host_roots_canon.iter().any(|r| canon.starts_with(r)) {
            // Name the jail roots so a caller (human or agent) can
            // self-correct in one step instead of guessing paths.
            let hint = crate::filesystem_access::request_suffix("S215", path, &canon);
            return Err(FsError::new(
                "S215",
                format!(
                    "path escapes the fs jail roots [{}]: {path}{hint}",
                    display_roots(host_roots_canon),
                ),
            ));
        }
    }
    for deny_canon in denylist_canon {
        if canon.starts_with(deny_canon) {
            return Err(FsError::new("S215", format!("path is denylisted: {path}")));
        }
    }
    Ok(canon)
}

/// D4 protected-paths gate, factored out so the handlers that confine paths
/// inside a `spawn_blocking` closure (sed, grep — which can't borrow `&self`)
/// apply the SAME check as `HostFsBackend::validate_path`. `canon` is a path
/// already confined inside a root.
///
/// A path is non-accessible when its form RELATIVE TO ANY containing root
/// matches a `non_accessible` glob. Checking EVERY containing root (not just
/// the first) closes a gap with nested roots: a root-anchored glob like
/// `secrets.json` matched against the inner root's relative form would be
/// missed if only the outer root's form were tested. Fail-closed: more roots
/// checked can only ADD matches, never drop one.
fn path_is_non_accessible(
    canon: &Path,
    host_roots_canon: &[PathBuf],
    non_accessible: &GlobSet,
) -> bool {
    if non_accessible.is_empty() {
        return false;
    }
    let mut contained = false;
    for root in host_roots_canon {
        if let Ok(rel) = canon.strip_prefix(root) {
            contained = true;
            let rel = rel.to_string_lossy().replace('\\', "/");
            if !rel.is_empty() && non_accessible.is_match(&rel) {
                return true;
            }
        }
    }
    // No containing root — the unjailed mode (empty roots, no scope). The
    // globs must still protect secrets there: match against the
    // root-stripped absolute form, the same fallback the coder resolver
    // uses. Contained paths never reach this, so jailed matching is
    // byte-for-byte unchanged.
    if !contained {
        let abs = canon.to_string_lossy().replace('\\', "/");
        let stripped = abs.trim_start_matches('/');
        return !stripped.is_empty() && non_accessible.is_match(stripped);
    }
    false
}

fn access_roots(
    host_roots_canon: &[PathBuf],
    scope_root_canon: Option<&Path>,
    scope_grants_canon: &[PathBuf],
) -> Vec<PathBuf> {
    effective_roots(host_roots_canon, scope_root_canon, scope_grants_canon)
}

fn effective_roots(
    host_roots_canon: &[PathBuf],
    scope_root_canon: Option<&Path>,
    scope_grants_canon: &[PathBuf],
) -> Vec<PathBuf> {
    let mut roots = if let Some(base) = scope_root_canon {
        vec![base.to_path_buf()]
    } else {
        host_roots_canon.to_vec()
    };
    if let Some(base) = scope_root_canon {
        if !roots.iter().any(|r| r == base) {
            roots.push(base.to_path_buf());
        }
    }
    for extra in scope_grants_canon {
        if !roots.iter().any(|r| r == extra) {
            roots.push(extra.clone());
        }
    }
    roots
}

/// Comma-separated display of the jail roots, for S215 messages.
fn display_roots(roots: &[PathBuf]) -> String {
    roots
        .iter()
        .map(|r| r.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Free-function form of `HostFsBackend::lexical_operand`, for use inside a
/// `spawn_blocking` closure that can't borrow `&self`. Anchors relative inputs
/// to the given anchor (the primary jail root, or a session `scope_root`),
/// identical to the method form.
fn lexical_operand_with(path: &str, anchor_canon: Option<&Path>) -> PathBuf {
    let p = Path::new(path);
    if p.is_relative() {
        if let Some(anchor) = anchor_canon {
            return normalize_lexical(&anchor.join(p));
        }
    }
    normalize_lexical(p)
}

/// Resolve and validate an OPTIONAL trusted per-call `scope_root`. Returns:
/// - `Ok(None)` when no `scope_root` was supplied.
/// - `Ok(Some(canon))` when `scope_root` canonicalizes to an existing directory
///   that misses the absolute denylist. That directory becomes the effective
///   root for this call, even if it is outside the configured host roots.
/// - `Err(_)` when `scope_root` is empty, relative, not a directory, denylisted,
///   or otherwise cannot be canonicalized.
fn confine_scope_root(
    scope_root: Option<&str>,
    denylist_canon: &[PathBuf],
) -> Result<Option<PathBuf>, FsError> {
    let Some(scope_root) = scope_root else {
        return Ok(None);
    };
    if scope_root.is_empty() {
        return Err(FsError::new("S210", "scope_root must not be empty"));
    }
    let p = Path::new(scope_root);
    if !p.is_absolute() {
        return Err(FsError::new(
            "S210",
            format!("scope_root must be an absolute path: {scope_root}"),
        ));
    }
    let canon = canonicalize_with_fallback(p).map_err(|e| {
        let msg = format!("{e}");
        if msg.contains("dangling symlink in path") {
            FsError::new("S215", format!("{scope_root}: {msg}"))
        } else {
            FsError::new("S210", format!("{scope_root}: {msg}"))
        }
    })?;
    if !canon.is_dir() {
        return Err(FsError::new(
            "S212",
            format!("scope_root is not a directory: {scope_root}"),
        ));
    }
    for deny_canon in denylist_canon {
        if canon.starts_with(deny_canon) {
            return Err(FsError::new(
                "S215",
                format!("scope_root is denylisted: {scope_root}"),
            ));
        }
    }
    Ok(Some(canon))
}

pub(crate) fn confine_scope_grants(
    scope_grants: Option<&[String]>,
    denylist_canon: &[PathBuf],
) -> Vec<PathBuf> {
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
        if denylist_canon.iter().any(|d| canon.starts_with(d)) {
            continue;
        }
        if !out.iter().any(|r| r == &canon) {
            out.push(canon);
        }
    }
    out
}

/// scope_root-aware jail confinement, LAYERED on top of [`confine_path`]. When
/// `scope_root_canon` is `Some`, the call is scoped to that session directory:
/// relative paths anchor at `scope_root` (not the primary jail root) and the resolved path
/// must land INSIDE `scope_root`. When it is `None`, this is exactly
/// [`confine_path`] against the jail roots.
///
/// The core jail algorithm (`confine_path` →
/// `canonicalize_with_fallback`/`normalize_lexical`) is reused verbatim, not
/// re-implemented: passing `scope_root` as the effective root gives both the
/// relative-anchor and the containment check. The only addition is the DX-1
/// error refinement: an ABSOLUTE path that is inside a configured host root but
/// outside `scope_root` is rejected with S220 naming the session directory,
/// instead of the generic "escapes the fs jail roots" S215.
///
/// `pub(crate)` so the exec backend can confine a per-call `cwd` against the
/// SAME session-scoped jail (shell::exec/exec_bg `scope_root`) instead of
/// duplicating the layering.
pub(crate) fn confine_path_with_scope_root(
    path: &str,
    host_roots_canon: &[PathBuf],
    scope_root_canon: Option<&Path>,
    scope_grants_canon: &[PathBuf],
    denylist_canon: &[PathBuf],
) -> Result<PathBuf, FsError> {
    let Some(base) = scope_root_canon else {
        // No session scope: identical to the unscoped jail check unless the
        // harness stamped grants, in which case those become additional roots.
        if scope_grants_canon.is_empty() {
            return confine_path(path, host_roots_canon, denylist_canon);
        }
        return confine_path(
            path,
            &effective_roots(host_roots_canon, None, scope_grants_canon),
            denylist_canon,
        );
    };
    // Scope the confinement to scope_root: relative paths anchor at scope_root and
    // the canonical result must start_with(scope_root). Since scope_root ⊆ some
    // allowed root (validated by confine_scope_root), this is strictly tighter
    // than the host-roots check. scope_root becomes the sole effective root.
    let scoped_roots = effective_roots(host_roots_canon, Some(base), scope_grants_canon);
    match confine_path(path, &scoped_roots, denylist_canon) {
        Ok(canon) => Ok(canon),
        Err(e) => {
            // DX-1: an absolute path that is inside an allowed root but outside
            // scope_root would otherwise surface the generic "escapes the fs jail
            // roots" S215, which is confusing — the path IS inside an allowed
            // root, just not this session's. Detect that case and emit a
            // session-scoped S220 naming scope_root so the agent corrects to a
            // path under the session directory rather than guessing roots.
            if e.code == "S215" && Path::new(path).is_absolute() {
                if let Ok(canon) = canonicalize_with_fallback(Path::new(path)) {
                    let inside_jail_root = host_roots_canon.iter().any(|hr| canon.starts_with(hr));
                    let inside_extra_root = scope_grants_canon.iter().any(|r| canon.starts_with(r));
                    let denied = denylist_canon.iter().any(|d| canon.starts_with(d));
                    if inside_jail_root && !canon.starts_with(base) && !inside_extra_root && !denied
                    {
                        let hint = crate::filesystem_access::request_suffix("S220", path, &canon);
                        return Err(FsError::new(
                            "S220",
                            format!(
                                "this session is scoped to {}; {path} is inside an allowed \
                                 root but outside the session directory — use a path under {}",
                                base.display(),
                                base.display()
                            ) + &hint,
                        ));
                    }
                }
            }
            Err(e)
        }
    }
}

// The jail-safety LEAF (`canonicalize_with_fallback` + `normalize_lexical`)
// now lives in `crate::path` and is shared by every jail surface in this
// crate (this `fs::host` backend and the folded `code` `PathResolver`). It
// used to be duplicated byte-for-byte here and in coder; the merge removed
// that MIRROR-INVARIANT hazard. See `crate::path` for the algorithm + the
// canonicalization parity vectors.

/// Mirrors `iii-init/fs_handler/ops.rs::temp_sibling`.
fn temp_sibling(target: &Path) -> PathBuf {
    let mut t = target.as_os_str().to_os_string();
    t.push(".iii-tmp-");
    t.push(uuid::Uuid::new_v4().to_string());
    t.into()
}

/// Mirrors `iii-init/fs_handler/ops.rs::glob_matches_path`.
fn glob_matches_path(pattern: &str, relpath: &str) -> bool {
    if pattern.contains('/') {
        glob_match(pattern, relpath)
    } else {
        let base = relpath.rsplit('/').next().unwrap_or(relpath);
        glob_match(pattern, base)
    }
}

fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern == "**" {
        return true;
    }
    if let Some(rest) = pattern.strip_prefix("**/") {
        let base = path.rsplit('/').next().unwrap_or(path);
        if glob_match_simple(rest, base) {
            return true;
        }
        return glob_match_simple(rest, path);
    }
    glob_match_simple(pattern, path)
}

fn glob_match_simple(pattern: &str, path: &str) -> bool {
    let p = pattern.as_bytes();
    let t = path.as_bytes();
    let mut pi = 0usize;
    let mut ti = 0usize;
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < t.len() {
        let pc = p.get(pi).copied();
        let tc = t[ti];
        match pc {
            Some(b'*') => {
                star_pi = Some(pi);
                star_ti = ti;
                pi += 1;
            }
            Some(b'?') if tc != b'/' => {
                pi += 1;
                ti += 1;
            }
            Some(c) if c == tc => {
                pi += 1;
                ti += 1;
            }
            _ => {
                if let Some(sp) = star_pi {
                    if t[star_ti] == b'/' {
                        return false;
                    }
                    pi = sp + 1;
                    star_ti += 1;
                    ti = star_ti;
                } else {
                    return false;
                }
            }
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

fn looks_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 8192];
    let n = f.read(&mut buf).unwrap_or(0);
    buf[..n].contains(&0)
}

fn expand_regex_replacement(caps: &regex::Captures, template: &str) -> String {
    let mut out = String::new();
    caps.expand(template, &mut out);
    out
}

/// When case-insensitive literal replace is requested, the caller passes a
/// precompiled `(?i)`-escaped matcher in `ci_matcher` (built ONCE per sed
/// call, not per line — a 10k-line file otherwise does 10k regex compiles).
/// Hand-rolling case-fold over UTF-8 is unsound (e.g. `'İ'` U+0130 folds to a
/// length-changing sequence), so we delegate to regex for the fold.
fn literal_replace_line(
    line: &str,
    needle: &str,
    ci_matcher: Option<&regex::Regex>,
    replacement: &str,
    first_only: bool,
) -> (String, u64) {
    if line.is_empty() || needle.is_empty() {
        return (line.to_string(), 0);
    }
    if let Some(re) = ci_matcher {
        // Closures returning `String` to `Regex::replacen`/`replace_all`
        // satisfy `Replacer` via the FnMut blanket impl, which inserts the
        // returned string verbatim — no `$N` capture substitution. So the
        // literal replacement must NOT be pre-escaped; doing so would
        // double user-supplied `$` characters.
        let owned = replacement.to_string();
        let mut count = 0u64;
        let out = if first_only {
            re.replacen(line, 1, |_caps: &regex::Captures| {
                count += 1;
                owned.clone()
            })
            .into_owned()
        } else {
            re.replace_all(line, |_caps: &regex::Captures| {
                count += 1;
                owned.clone()
            })
            .into_owned()
        };
        return (out, count);
    }
    let mut out = String::with_capacity(line.len());
    let mut n = 0u64;
    let mut i = 0usize;
    let bytes = line.as_bytes();
    let needle_bytes = needle.as_bytes();
    while i < line.len() {
        if i + needle_bytes.len() <= bytes.len()
            && &bytes[i..i + needle_bytes.len()] == needle_bytes
        {
            out.push_str(replacement);
            i += needle_bytes.len();
            n += 1;
            if first_only {
                out.push_str(&line[i..]);
                return (out, n);
            }
        } else {
            let next = line[i..]
                .char_indices()
                .nth(1)
                .map(|(b, _)| i + b)
                .unwrap_or(line.len());
            out.push_str(&line[i..next]);
            i = next;
        }
    }
    (out, n)
}

fn collect_files_to_sed(
    root: &Path,
    recursive: bool,
    include_glob: &[String],
    exclude_glob: &[String],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let lmd = match root.symlink_metadata() {
        Ok(m) => m,
        Err(_) => return out,
    };
    let passes = |rel: &str| -> bool {
        if !include_glob.is_empty() && !include_glob.iter().any(|g| glob_matches_path(g, rel)) {
            return false;
        }
        if exclude_glob.iter().any(|g| glob_matches_path(g, rel)) {
            return false;
        }
        true
    };
    let target_is_dir = if lmd.file_type().is_symlink() {
        match std::fs::metadata(root) {
            Ok(m) => m.is_dir(),
            Err(_) => return out,
        }
    } else {
        lmd.is_dir()
    };
    if !target_is_dir {
        let rel = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if passes(&rel) {
            out.push(root.to_path_buf());
        }
        return out;
    }
    let resolved_root: PathBuf = if lmd.file_type().is_symlink() {
        match std::fs::canonicalize(root) {
            Ok(p) => p,
            Err(_) => return out,
        }
    } else {
        root.to_path_buf()
    };
    let walker = walkdir::WalkDir::new(&resolved_root).follow_links(false);
    let walker = if recursive {
        walker
    } else {
        walker.max_depth(1)
    };
    for entry in walker
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let rel = entry
            .path()
            .strip_prefix(&resolved_root)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .into_owned();
        if passes(&rel) {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

use crate::fs::wire::FsEntry;
use crate::fs::{FsBackend, FsCallResult, LsArgs, LsResponse, StatArgs, StatResponse};
use std::os::unix::fs::PermissionsExt;

/// Mode formatted as 4-octal-digit zero-padded, matching engine daemon's
/// `entry_from_metadata`.
fn fs_entry_from_metadata(name: String, md: &std::fs::Metadata) -> FsEntry {
    FsEntry {
        name,
        is_dir: md.is_dir(),
        size: md.len(),
        mode: format!("{:04o}", md.permissions().mode() & 0o7777),
        mtime: md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        is_symlink: md.file_type().is_symlink(),
    }
}

async fn pump_file_to_channel(
    path: &std::path::Path,
    writer: &iii_sdk::channels::ChannelWriter,
) -> Result<(), String> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("open failed: {e}"))?;
    let mut buf = vec![0u8; 64 * 1024];
    // Open the WS connection eagerly: for a zero-byte file the loop exits
    // without write(), leaving the sender never dropped and blocking the
    // reader side forever. This empty write forces the connect.
    writer
        .write(&[])
        .await
        .map_err(|e| format!("channel connect failed: {e}"))?;
    loop {
        let n = file
            .read(&mut buf)
            .await
            .map_err(|e| format!("file read failed: {e}"))?;
        if n == 0 {
            return Ok(());
        }
        writer
            .write(&buf[..n])
            .await
            .map_err(|e| format!("channel write failed: {e}"))?;
    }
}

#[async_trait]
impl FsBackend for HostFsBackend {
    async fn ls(&self, req: LsArgs) -> FsCallResult<LsResponse> {
        // Jail validation runs here, on the async fn, BEFORE the blocking work.
        // A per-call scope_root (when set) scopes both the relative anchor and the
        // containment ceiling to the session directory; None ⇒ the unchanged
        // configured jail.
        let base = self.confine_scope_root(crate::fs::scope_anchor(req.fs_scope.as_ref()))?;
        let extra = self.confine_scope_grants(crate::fs::scope_grants(req.fs_scope.as_ref()));
        let p = self.validate_path_scoped(
            &req.path,
            base.as_deref(),
            &extra,
            crate::fs::scope_boundary(req.fs_scope.as_ref()),
        )?;
        // The symlink_metadata stat, read_dir, and the per-entry
        // symlink_metadata loop are all blocking std::fs work that scales with
        // directory size; move it off the executor (mirrors grep/sed).
        let req_path = req.path;
        let join = tokio::task::spawn_blocking(move || -> Result<_, FsError> {
            let md = std::fs::symlink_metadata(&p).map_err(|e| FsError::from_io(&req_path, e))?;
            if !md.is_dir() {
                return Err(FsError::new("S212", format!("not a directory: {req_path}")));
            }
            let rd = std::fs::read_dir(&p).map_err(|e| FsError::from_io(&req_path, e))?;
            let mut entries = Vec::new();
            for ent in rd {
                let ent = match ent {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let md = match ent.path().symlink_metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let name = ent.file_name().to_string_lossy().into_owned();
                entries.push(fs_entry_from_metadata(name, &md));
            }
            Ok(entries)
        });
        let entries = join
            .await
            .map_err(|e| FsError::new("S216", format!("ls task join failed: {e}")))??;
        Ok(LsResponse { entries })
    }

    async fn stat(&self, req: StatArgs) -> FsCallResult<StatResponse> {
        let base = self.confine_scope_root(crate::fs::scope_anchor(req.fs_scope.as_ref()))?;
        let extra = self.confine_scope_grants(crate::fs::scope_grants(req.fs_scope.as_ref()));
        let p = self.validate_path_scoped(
            &req.path,
            base.as_deref(),
            &extra,
            crate::fs::scope_boundary(req.fs_scope.as_ref()),
        )?;
        let md = std::fs::symlink_metadata(&p).map_err(|e| FsError::from_io(&req.path, e))?;
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| req.path.clone());
        Ok(StatResponse(fs_entry_from_metadata(name, &md)))
    }

    async fn mkdir(&self, req: crate::fs::MkdirArgs) -> FsCallResult<crate::fs::MkdirResponse> {
        let base = self.confine_scope_root(crate::fs::scope_anchor(req.fs_scope.as_ref()))?;
        let extra = self.confine_scope_grants(crate::fs::scope_grants(req.fs_scope.as_ref()));
        let p = self.validate_path_scoped(
            &req.path,
            base.as_deref(),
            &extra,
            crate::fs::scope_boundary(req.fs_scope.as_ref()),
        )?;
        let bits = crate::fs::error::parse_mode(&req.mode)?;
        check_special_bits(bits, self.cfg.allow_special_bits)?;
        if p.exists() {
            if req.parents {
                // mkdir -p is idempotent only over an existing DIRECTORY. A
                // regular file (or symlink to one) at the path is a hard error:
                // reporting success here would silently mask a misconfigured
                // path and make later fs ops fail far from the real cause.
                if !p.is_dir() {
                    return Err(FsError::new(
                        "S213",
                        format!("path exists and is not a directory: {}", req.path),
                    ));
                }
                return Ok(crate::fs::MkdirResponse {
                    created: false,
                    path: req.path.clone(),
                    already_existed: true,
                });
            }
            return Err(FsError::new(
                "S213",
                format!(
                    "path already exists: {}; pass parents: true for an idempotent create",
                    req.path
                ),
            ));
        }
        let res = if req.parents {
            std::fs::create_dir_all(&p)
        } else {
            std::fs::create_dir(&p)
        };
        res.map_err(|e| FsError::from_io(&req.path, e))?;
        let perms = std::fs::Permissions::from_mode(bits);
        std::fs::set_permissions(&p, perms).map_err(|e| FsError::from_io(&req.path, e))?;
        Ok(crate::fs::MkdirResponse {
            created: true,
            path: req.path.clone(),
            already_existed: false,
        })
    }

    async fn rm(&self, req: crate::fs::RmArgs) -> FsCallResult<crate::fs::RmResponse> {
        // Lexical form required: rm of a symlink must remove the link, not
        // the target. validate_path canonicalizes for jail confinement; we
        // operate on the lexical path to preserve unlink semantics. Both the
        // validation and the operand anchor at the session scope_root when set,
        // so they cannot diverge.
        let base = self.confine_scope_root(crate::fs::scope_anchor(req.fs_scope.as_ref()))?;
        let extra = self.confine_scope_grants(crate::fs::scope_grants(req.fs_scope.as_ref()));
        self.validate_path_scoped(
            &req.path,
            base.as_deref(),
            &extra,
            crate::fs::scope_boundary(req.fs_scope.as_ref()),
        )?;
        let p = self.lexical_operand_scoped(&req.path, base.as_deref(), &extra);

        // The symlink_metadata stat, recursive remove_dir_all, the non-recursive
        // read_dir emptiness probe, and the unlink are all blocking std::fs work
        // that scales with subtree size; move it off the executor (mirrors
        // grep/sed). Jail validation already ran above on the async fn.
        let recursive = req.recursive;
        let req_path = req.path.clone();
        let join = tokio::task::spawn_blocking(move || -> Result<(), FsError> {
            let md = std::fs::symlink_metadata(&p).map_err(|e| FsError::from_io(&req_path, e))?;
            if md.is_dir() && !md.file_type().is_symlink() {
                if recursive {
                    std::fs::remove_dir_all(&p).map_err(|e| FsError::from_io(&req_path, e))?;
                } else {
                    let mut rd =
                        std::fs::read_dir(&p).map_err(|e| FsError::from_io(&req_path, e))?;
                    if rd.next().is_some() {
                        return Err(FsError::new(
                            "S214",
                            format!(
                                "directory not empty: {req_path}; pass recursive: true to remove it"
                            ),
                        ));
                    }
                    std::fs::remove_dir(&p).map_err(|e| FsError::from_io(&req_path, e))?;
                }
            } else {
                std::fs::remove_file(&p).map_err(|e| FsError::from_io(&req_path, e))?;
            }
            Ok(())
        });
        join.await
            .map_err(|e| FsError::new("S216", format!("rm task join failed: {e}")))??;
        Ok(crate::fs::RmResponse {
            removed: true,
            path: req.path.clone(),
            was_present: true,
        })
    }
    async fn chmod(&self, req: crate::fs::ChmodArgs) -> FsCallResult<crate::fs::ChmodResponse> {
        // Jail validation + mode parsing run here, on the async fn, BEFORE the
        // blocking work. scope_root (when set) scopes the validation + operand.
        let base = self.confine_scope_root(crate::fs::scope_anchor(req.fs_scope.as_ref()))?;
        let extra = self.confine_scope_grants(crate::fs::scope_grants(req.fs_scope.as_ref()));
        self.validate_path_scoped(
            &req.path,
            base.as_deref(),
            &extra,
            crate::fs::scope_boundary(req.fs_scope.as_ref()),
        )?;
        let p = self.lexical_operand_scoped(&req.path, base.as_deref(), &extra);
        let bits = crate::fs::error::parse_mode(&req.mode)?;
        check_special_bits(bits, self.cfg.allow_special_bits)?;

        // The exists probe, the recursive WalkDir traversal, and the per-entry
        // set_permissions/chown loop are all blocking std::fs work that scales
        // with subtree size; move it off the executor (mirrors grep/sed).
        // Owned values (canonical-anchored PathBuf, mode bits, uid/gid, flags)
        // move into the closure.
        let uid = req.uid;
        let gid = req.gid;
        let recursive = req.recursive;
        let req_path = req.path.clone();
        let join = tokio::task::spawn_blocking(move || -> Result<u64, FsError> {
            if !p.exists() {
                return Err(FsError::new("S211", format!("path not found: {req_path}")));
            }
            let apply = |target: &Path| -> Result<(), FsError> {
                let perms = std::fs::Permissions::from_mode(bits);
                std::fs::set_permissions(target, perms)
                    .map_err(|e| FsError::from_io(&target.to_string_lossy(), e))?;
                if uid.is_some() || gid.is_some() {
                    std::os::unix::fs::chown(target, uid, gid)
                        .map_err(|e| FsError::from_io(&target.to_string_lossy(), e))?;
                }
                Ok(())
            };
            let mut updated: u64 = 0;
            if recursive {
                // Reject if the walk root itself is a symlink: descending into
                // a symlink target would change perms outside the recursive
                // root, and skipping the root entry silently (which is what
                // the per-entry skip below would do) is a quiet no-op that
                // looks like success to the caller. S212 = wrong file type.
                let root_md =
                    std::fs::symlink_metadata(&p).map_err(|e| FsError::from_io(&req_path, e))?;
                if root_md.file_type().is_symlink() {
                    return Err(FsError::new(
                        "S212",
                        format!("recursive chmod refuses to follow symlink at root: {req_path}"),
                    ));
                }
                for entry in walkdir::WalkDir::new(&p)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    // Skip symlink entries inside the walk: chmod(2)/chown(2)
                    // follow symlinks and would rewrite the target's
                    // mode/owner — possibly outside the recursive root or the
                    // jail. lchmod isn't portable.
                    if entry.file_type().is_symlink() {
                        continue;
                    }
                    apply(entry.path())?;
                    updated += 1;
                }
            } else {
                apply(&p)?;
                updated = 1;
            }
            Ok(updated)
        });
        let updated = join
            .await
            .map_err(|e| FsError::new("S216", format!("chmod task join failed: {e}")))??;
        Ok(crate::fs::ChmodResponse {
            entries_changed: updated,
            path: req.path.clone(),
            recursive: req.recursive,
        })
    }

    async fn mv(&self, req: crate::fs::MvArgs) -> FsCallResult<crate::fs::MvResponse> {
        // A single session scope_root scopes BOTH operands.
        let base = self.confine_scope_root(crate::fs::scope_anchor(req.fs_scope.as_ref()))?;
        let extra = self.confine_scope_grants(crate::fs::scope_grants(req.fs_scope.as_ref()));
        self.validate_path_scoped(
            &req.src,
            base.as_deref(),
            &extra,
            crate::fs::scope_boundary(req.fs_scope.as_ref()),
        )?;
        self.validate_path_scoped(
            &req.dst,
            base.as_deref(),
            &extra,
            crate::fs::scope_boundary(req.fs_scope.as_ref()),
        )?;
        let src_p = self.lexical_operand_scoped(&req.src, base.as_deref(), &extra);
        let dst_p = self.lexical_operand_scoped(&req.dst, base.as_deref(), &extra);
        if !src_p.exists() {
            return Err(FsError::new("S211", format!("src not found: {}", req.src)));
        }
        // Best-effort overwrite guard: `dst_existed` is a pre-rename check, so
        // `overwrite:false` is race-able and `overwrote` may under-report if a
        // concurrent writer creates dst in the check→rename window (POSIX rename
        // replaces atomically). A race-free guard needs renameat2(RENAME_NOREPLACE)
        // (Linux-only) — tracked as a follow-up; the field doc notes this.
        let dst_existed = dst_p.exists();
        if dst_existed && !req.overwrite {
            return Err(FsError::new(
                "S213",
                format!(
                    "dst already exists: {}; pass overwrite: true to replace it",
                    req.dst
                ),
            ));
        }
        // The rename and the EXDEV copy+rename+unlink fallback are blocking
        // std::fs work; the cross-filesystem copy in particular is O(file size)
        // and would pin a worker thread on a large file. Move the whole
        // rename-or-fallback unit off the executor (mirrors grep/sed). Jail
        // validation already ran above on the async fn; we move owned copies of
        // the anchored src/dst paths and the src/dst strings (for error
        // messages) into the closure.
        let src_str = req.src.clone();
        let dst_str = req.dst.clone();
        let join = tokio::task::spawn_blocking(move || -> Result<(), FsError> {
            match std::fs::rename(&src_p, &dst_p) {
                Ok(()) => Ok(()),
                // EXDEV: cross-fs move — fall back to copy+rename+unlink.
                // File-only; directories are unsupported (matches engine daemon).
                Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                    let tmp = temp_sibling(&dst_p);
                    std::fs::copy(&src_p, &tmp).map_err(|e| FsError::from_io(&dst_str, e))?;
                    if let Err(e) = std::fs::rename(&tmp, &dst_p) {
                        let _ = std::fs::remove_file(&tmp);
                        return Err(FsError::from_io(&dst_str, e));
                    }
                    std::fs::remove_file(&src_p).map_err(|e| FsError::from_io(&src_str, e))?;
                    Ok(())
                }
                Err(e) => Err(FsError::from_io(&dst_str, e)),
            }
        });
        join.await
            .map_err(|e| FsError::new("S216", format!("mv task join failed: {e}")))??;
        Ok(crate::fs::MvResponse {
            moved: true,
            src: req.src.clone(),
            dst: req.dst.clone(),
            overwrote: dst_existed,
        })
    }
    async fn grep(&self, req: crate::fs::GrepArgs) -> FsCallResult<crate::fs::GrepResponse> {
        let base = self.confine_scope_root(crate::fs::scope_anchor(req.fs_scope.as_ref()))?;
        let extra = self.confine_scope_grants(crate::fs::scope_grants(req.fs_scope.as_ref()));
        let root = self.validate_path_scoped(
            &req.path,
            base.as_deref(),
            &extra,
            crate::fs::scope_boundary(req.fs_scope.as_ref()),
        )?;
        // Cap before compiling: an unbounded pattern stalls compilation and
        // pins memory.
        check_pattern_len(&req.pattern)?;
        let re = regex::RegexBuilder::new(&req.pattern)
            .case_insensitive(req.ignore_case)
            .size_limit(REGEX_SIZE_LIMIT)
            .dfa_size_limit(REGEX_SIZE_LIMIT)
            .build()
            .map_err(|e| FsError::new("S217", format!("bad regex: {e}")))?;

        // The symlink_metadata stat, the walk, and the per-file scan are all
        // blocking std::fs / BufReader work. Move ALL of it off the Tokio
        // worker thread — the residual stat at the top used to run on the
        // executor — so a grep over a large tree can't stall it. Capture owned
        // data; the compiled Regex is Send+Sync so it moves into the closure.
        //
        // A 0 cap means "use the default", NOT "unlimited": an unbounded match
        // count or line length is a memory/DoS vector. Clamp both before the
        // closure so the scan always runs with a positive bound.
        let max_matches_usize = if req.max_matches == 0 {
            DEFAULT_GREP_MAX_MATCHES
        } else {
            req.max_matches as usize
        };
        let max_line_usize = if req.max_line_bytes == 0 {
            DEFAULT_GREP_MAX_LINE_BYTES
        } else {
            req.max_line_bytes as usize
        };
        let include_glob = req.include_glob;
        let exclude_glob = req.exclude_glob;
        let recursive = req.recursive;
        let req_path = req.path;
        // D4: skip protected files during a directory walk (the gate that
        // validate_path_scoped applies to single-file/other ops).
        let host_roots_canon = self.host_roots_canon.clone();
        let access_roots = access_roots(&host_roots_canon, base.as_deref(), &extra);
        let non_accessible = self.non_accessible.clone();

        let join = tokio::task::spawn_blocking(move || -> Result<_, FsError> {
            let md =
                std::fs::symlink_metadata(&root).map_err(|e| FsError::from_io(&req_path, e))?;
            let is_dir = md.is_dir();
            if is_dir && !recursive {
                return Err(FsError::new(
                    "S210",
                    "recursive=false on a directory is unsupported; \
                     pass a file path or set recursive=true",
                ));
            }

            let should_scan = |rel: &str| -> bool {
                if !include_glob.is_empty()
                    && !include_glob.iter().any(|g| glob_matches_path(g, rel))
                {
                    return false;
                }
                if exclude_glob.iter().any(|g| glob_matches_path(g, rel)) {
                    return false;
                }
                true
            };

            let mut out: Vec<crate::fs::wire::FsMatch> = Vec::new();
            let mut truncated = false;

            let mut scan = |file_path: &Path| -> Result<bool, FsError> {
                if looks_binary(file_path) {
                    return Ok(false);
                }
                let f = std::fs::File::open(file_path)
                    .map_err(|e| FsError::from_io(&file_path.to_string_lossy(), e))?;
                let mut reader = std::io::BufReader::new(f);
                use std::io::{BufRead, Read};
                // Byte-bounded line read: `reader.lines()` materializes each
                // full line before `max_line_bytes` truncation ever runs, so a
                // file with no newlines buffers the WHOLE file. Read with
                // `read_until(b'\n')` capped at GREP_MAX_LINE_SCAN_BYTES; once a
                // single line hits the cap we stop buffering and discard the
                // rest of the line up to the next newline, so memory is bounded
                // during the read, not just in the returned match.
                let mut idx = 0usize;
                let mut buf: Vec<u8> = Vec::new();
                loop {
                    buf.clear();
                    // Read at most GREP_MAX_LINE_SCAN_BYTES of this line.
                    // `take(cap)` bounds the buffer; if the line is longer the
                    // read stops without a trailing newline and we drain the
                    // remainder below.
                    let n = (&mut reader)
                        .take(GREP_MAX_LINE_SCAN_BYTES as u64)
                        .read_until(b'\n', &mut buf)
                        .map_err(|e| FsError::from_io(&file_path.to_string_lossy(), e))?;
                    if n == 0 {
                        break;
                    }
                    let ended_with_nl = buf.last() == Some(&b'\n');
                    // A line longer than the scan cap: no newline yet AND we
                    // filled the cap. Discard the remainder of this line (up to
                    // and including the next newline) one bounded read at a
                    // time so nothing further is buffered.
                    let mut truncated_line = false;
                    if !ended_with_nl && buf.len() >= GREP_MAX_LINE_SCAN_BYTES {
                        truncated_line = true;
                        let mut scratch: Vec<u8> = Vec::new();
                        loop {
                            scratch.clear();
                            let m = (&mut reader)
                                .take(GREP_MAX_LINE_SCAN_BYTES as u64)
                                .read_until(b'\n', &mut scratch)
                                .map_err(|e| FsError::from_io(&file_path.to_string_lossy(), e))?;
                            // Stop at EOF (m==0) or once we consumed the newline
                            // that ends this over-long line. Inspect the bytes
                            // directly rather than comparing lengths — a chunk
                            // can fill the cap AND end in '\n' simultaneously.
                            if m == 0 || scratch.last() == Some(&b'\n') {
                                break;
                            }
                        }
                    }
                    idx += 1;
                    // Strip the trailing newline (and a preceding CR) so the
                    // match content matches the previous `lines()` behavior.
                    if buf.last() == Some(&b'\n') {
                        buf.pop();
                        if buf.last() == Some(&b'\r') {
                            buf.pop();
                        }
                    }
                    // Lossy is fine: grep operated on `String` lines before too
                    // (invalid-UTF8 lines were dropped by `lines()`); lossy
                    // keeps a best-effort match rather than silently skipping.
                    let mut line = String::from_utf8_lossy(&buf).into_owned();
                    if re.is_match(&line) {
                        if line.len() > max_line_usize {
                            // Floor to nearest char boundary so a multi-byte
                            // codepoint straddling the cut doesn't panic.
                            let cut = (0..=max_line_usize)
                                .rev()
                                .find(|&i| line.is_char_boundary(i))
                                .unwrap_or(0);
                            line.truncate(cut);
                            line.push('…');
                        } else if truncated_line {
                            // Cut at the scan cap before matching; mark partial.
                            line.push('…');
                        }
                        out.push(crate::fs::wire::FsMatch {
                            path: file_path.to_string_lossy().into_owned(),
                            line: idx as u64,
                            content: line,
                        });
                        if out.len() >= max_matches_usize {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            };

            if is_dir {
                for entry in walkdir::WalkDir::new(&root)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_file())
                {
                    let rel = entry
                        .path()
                        .strip_prefix(&root)
                        .unwrap_or(entry.path())
                        .to_string_lossy()
                        .into_owned();
                    if !should_scan(&rel) {
                        continue;
                    }
                    // D4: a directory grep must not LEAK the content of a
                    // protected file under it (the dir itself isn't protected,
                    // but files matching non_accessible are). Skip them — the
                    // file stays "visible but locked", same as the other ops.
                    if path_is_non_accessible(entry.path(), &access_roots, &non_accessible) {
                        continue;
                    }
                    if scan(entry.path())? {
                        truncated = true;
                        break;
                    }
                }
            } else {
                let rel = root
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if should_scan(&rel) && scan(&root)? {
                    truncated = true;
                }
            }

            Ok((out, truncated))
        });

        let (matches, truncated) = join
            .await
            .map_err(|e| FsError::new("S216", format!("grep task join failed: {e}")))??;

        Ok(crate::fs::GrepResponse { matches, truncated })
    }
    async fn sed(&self, req: crate::fs::SedArgs) -> FsCallResult<crate::fs::SedResponse> {
        // Decide the file-source shape on the executor (pure, no fs). The
        // actual directory walk + per-file jail validation are deferred into
        // the spawn_blocking closure below so NO blocking fs syscall (walkdir
        // traversal, per-entry symlink_metadata/canonicalize, per-file
        // validate_path) runs on the async worker thread.
        enum SedSource {
            Files(Vec<String>),
            Dir(String),
        }
        let source = match (req.files.is_empty(), req.path.as_ref()) {
            (false, None) => SedSource::Files(req.files.clone()),
            (true, Some(root)) => SedSource::Dir(root.clone()),
            (false, Some(_)) => {
                return Err(FsError::new(
                    "S210",
                    "sed: provide exactly one of `files` or `path`, not both",
                ));
            }
            (true, None) => {
                return Err(FsError::new(
                    "S210",
                    "sed: must provide exactly one of `files` or `path`",
                ));
            }
        };

        // Cap before compiling (regex) or using (literal) — an unbounded
        // pattern stalls compilation and pins memory.
        check_pattern_len(&req.pattern)?;

        let matcher: Option<regex::Regex> = if req.regex {
            Some(
                regex::RegexBuilder::new(&req.pattern)
                    .case_insensitive(req.ignore_case)
                    .size_limit(REGEX_SIZE_LIMIT)
                    .dfa_size_limit(REGEX_SIZE_LIMIT)
                    .build()
                    .map_err(|e| FsError::new("S217", format!("bad regex: {e}")))?,
            )
        } else if req.pattern.is_empty() {
            return Err(FsError::new("S210", "pattern is empty"));
        } else {
            None
        };
        // Hoist the case-insensitive literal matcher: build it ONCE per sed
        // call (`literal_replace_line` previously compiled a fresh `(?i)`
        // regex on every line). Built only for the literal + ignore_case
        // path; the regex path uses `matcher` above.
        let ci_matcher: Option<regex::Regex> = if req.ignore_case && !req.regex {
            let pattern = format!("(?i){}", regex::escape(&req.pattern));
            Some(
                regex::RegexBuilder::new(&pattern)
                    .size_limit(REGEX_SIZE_LIMIT)
                    .dfa_size_limit(REGEX_SIZE_LIMIT)
                    .build()
                    .map_err(|e| FsError::new("S217", format!("bad regex: {e}")))?,
            )
        } else {
            None
        };

        // ALL remaining work is blocking fs: the directory walk
        // (collect_files_to_sed — walkdir + per-entry symlink_metadata +
        // canonicalize), the per-file jail validation (confine_path), the
        // jail-relative anchoring, and the read_to_string + per-line replace
        // + temp-write + rename loop. Pre-fix the walk and the per-file
        // validation ran on the async worker thread BEFORE spawn_blocking, so
        // a `sed --path=large-dir --recursive` stalled the executor for the
        // entire traversal — exactly what spawn_blocking was meant to prevent.
        // We move it all into the closure. The per-file jail confinement
        // (confine_path: jail-root containment + denylist) still runs for
        // EVERY file — it just runs on the blocking thread now, with owned
        // copies of the precomputed canonical root + denylist (compiled
        // regexes are Send+Sync and move in too). Streaming read/write paths
        // are untouched — those already use channels.
        let pattern = req.pattern;
        let replacement = req.replacement;
        let first_only = req.first_only;
        let recursive = req.recursive;
        let include_glob = req.include_glob;
        let exclude_glob = req.exclude_glob;
        let host_roots_canon = self.host_roots_canon.clone();
        let denylist_canon = self.denylist_canon.clone();
        // D4: the protected-paths gate must run on the blocking thread too —
        // sed validates via the free confine fns, not validate_path_scoped.
        let non_accessible = self.non_accessible.clone();
        // Resolve the optional session scope_root up front (on the async fn) so the
        // blocking closure confines + anchors every operand to it instead of the
        // global jail roots. None ⇒ unchanged jail behaviour.
        let scope_root_canon =
            self.confine_scope_root(crate::fs::scope_anchor(req.fs_scope.as_ref()))?;
        let scope_grants_canon =
            self.confine_scope_grants(crate::fs::scope_grants(req.fs_scope.as_ref()));
        let access_roots = access_roots(
            &host_roots_canon,
            scope_root_canon.as_deref(),
            &scope_grants_canon,
        );
        // Per-file read cap: sed builds a same-size output String in memory, so
        // an unbounded file is an OOM vector (grep skips binary + bounds its
        // line read; sed did neither). Honor the backend's max_read_bytes; a 0
        // cap (no limit configured) falls back to a hard ceiling.
        let read_cap: u64 = if self.cfg.max_read_bytes > 0 {
            self.cfg.max_read_bytes as u64
        } else {
            SED_MAX_FILE_BYTES
        };

        let join = tokio::task::spawn_blocking(move || -> Result<_, FsError> {
            use std::os::unix::fs::PermissionsExt;

            // Resolve the concrete file list on the blocking thread. For the
            // directory form this is the walk that previously stalled the
            // executor; the recursive=false-on-dir guard and the io errors
            // keep the same S-codes they had on the executor.
            let files: Vec<String> = match source {
                SedSource::Files(fs) => fs,
                SedSource::Dir(root) => {
                    confine_path_with_scope_root(
                        &root,
                        &host_roots_canon,
                        scope_root_canon.as_deref(),
                        &scope_grants_canon,
                        &denylist_canon,
                    )?;
                    let anchor = scope_root_canon
                        .as_deref()
                        .or(host_roots_canon.first().map(|p| p.as_path()));
                    let root_anchored = lexical_operand_with(&root, anchor);
                    let root_path = root_anchored.as_path();
                    let _ = root_path
                        .symlink_metadata()
                        .map_err(|e| FsError::from_io(&root, e))?;
                    let target_is_dir = match std::fs::metadata(root_path) {
                        Ok(m) => m.is_dir(),
                        Err(e) => return Err(FsError::from_io(&root, e)),
                    };
                    if target_is_dir && !recursive {
                        return Err(FsError::new(
                            "S210",
                            "recursive=false on a directory is unsupported; \
                             pass a file path or set recursive=true",
                        ));
                    }
                    collect_files_to_sed(root_path, recursive, &include_glob, &exclude_glob)
                        .into_iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect()
                }
            };

            // Per-file jail confinement, exactly as on the executor pre-fix:
            // validate EVERY file (S210/S215/S220 on the first violation) BEFORE
            // touching any file, so a single bad path aborts the whole call
            // with nothing written. scope_root (when set) scopes each file.
            for f in &files {
                let canon = confine_path_with_scope_root(
                    f,
                    &host_roots_canon,
                    scope_root_canon.as_deref(),
                    &scope_grants_canon,
                    &denylist_canon,
                )?;
                // D4: a protected file is locked for modification, exactly like
                // shell::fs::write/rm (which route through validate_path_scoped).
                if path_is_non_accessible(&canon, &access_roots, &non_accessible) {
                    return Err(FsError::new(
                        "S215",
                        format!("path is protected (non_accessible): {f}"),
                    ));
                }
            }

            // Anchor every file to its jail-relative operand (the operated-on
            // path, not its canonical target — sed acts on the link itself).
            // The anchor is the session scope_root when set, else the primary root.
            let file_anchor = scope_root_canon
                .as_deref()
                .or(host_roots_canon.first().map(|p| p.as_path()));
            let anchored: Vec<(String, PathBuf)> = files
                .into_iter()
                .map(|f| {
                    let p = lexical_operand_with(&f, file_anchor);
                    (f, p)
                })
                .collect();

            let mut results: Vec<crate::fs::wire::FsSedFileResult> =
                Vec::with_capacity(anchored.len());
            let mut total: u64 = 0;

            for (file, anchored_path) in anchored {
                let p = anchored_path.as_path();
                // Symlink operands are skipped: sed reads through the link
                // (metadata/read_to_string follow it) but rewrites via
                // rename(tmp, p), which would REPLACE the link with a regular
                // file — silently destroying the link and detaching it from its
                // target. Refuse rather than corrupt. (symlink_metadata does
                // not follow, so this detects the link itself. Mirrors the
                // chmod recursive walk's skip-symlinks policy.)
                if let Ok(lmd) = std::fs::symlink_metadata(p) {
                    if lmd.file_type().is_symlink() {
                        results.push(crate::fs::wire::FsSedFileResult {
                            path: file.clone(),
                            replacements: 0,
                            success: false,
                            error: Some(
                                "operand is a symlink; skipped (sed would replace the link \
                                 with a regular file). Pass the resolved target path instead."
                                    .to_string(),
                            ),
                        });
                        continue;
                    }
                }
                // Size cap BEFORE reading: stat the file and skip (per-file
                // error, nothing written) if it exceeds the read cap. Mirrors
                // grep's OOM defense; sed previously read any size unconditionally.
                match std::fs::metadata(p) {
                    Ok(md) if md.len() > read_cap => {
                        results.push(crate::fs::wire::FsSedFileResult {
                            path: file.clone(),
                            replacements: 0,
                            success: false,
                            error: Some(format!(
                                "file size {} exceeds read cap {} bytes; skipped",
                                md.len(),
                                read_cap
                            )),
                        });
                        continue;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        results.push(crate::fs::wire::FsSedFileResult {
                            path: file.clone(),
                            replacements: 0,
                            success: false,
                            error: Some(format!("{e}")),
                        });
                        continue;
                    }
                }
                // Binary skip: reuse grep's heuristic. sed only does text
                // replacement; rewriting a binary would corrupt it.
                if looks_binary(p) {
                    results.push(crate::fs::wire::FsSedFileResult {
                        path: file.clone(),
                        replacements: 0,
                        success: false,
                        error: Some("binary file; skipped".to_string()),
                    });
                    continue;
                }
                let original = match std::fs::read_to_string(p) {
                    Ok(s) => s,
                    Err(e) => {
                        results.push(crate::fs::wire::FsSedFileResult {
                            path: file.clone(),
                            replacements: 0,
                            success: false,
                            error: Some(format!("{e}")),
                        });
                        continue;
                    }
                };
                let mut replacements: u64 = 0;
                let mut output = String::with_capacity(original.len());
                for line in original.split_inclusive('\n') {
                    let (new_line, n) = match &matcher {
                        Some(re) => {
                            let mut count_here = 0u64;
                            let produced = if first_only {
                                re.replacen(line, 1, |caps: &regex::Captures| {
                                    count_here += 1;
                                    expand_regex_replacement(caps, &replacement)
                                })
                                .into_owned()
                            } else {
                                re.replace_all(line, |caps: &regex::Captures| {
                                    count_here += 1;
                                    expand_regex_replacement(caps, &replacement)
                                })
                                .into_owned()
                            };
                            (produced, count_here)
                        }
                        None => literal_replace_line(
                            line,
                            &pattern,
                            ci_matcher.as_ref(),
                            &replacement,
                            first_only,
                        ),
                    };
                    replacements += n;
                    output.push_str(&new_line);
                }
                let tmp = temp_sibling(p);
                let write_result: Result<(), std::io::Error> = (|| {
                    let original_md = std::fs::metadata(p)?;
                    std::fs::write(&tmp, output.as_bytes())?;
                    std::fs::set_permissions(
                        &tmp,
                        std::fs::Permissions::from_mode(original_md.permissions().mode()),
                    )?;
                    std::fs::rename(&tmp, p)?;
                    Ok(())
                })();
                match write_result {
                    Ok(()) => {
                        total += replacements;
                        results.push(crate::fs::wire::FsSedFileResult {
                            path: file,
                            replacements,
                            success: true,
                            error: None,
                        });
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&tmp);
                        results.push(crate::fs::wire::FsSedFileResult {
                            path: file,
                            replacements: 0,
                            success: false,
                            error: Some(format!("{e}")),
                        });
                    }
                }
            }
            Ok((results, total))
        });

        // JoinError -> S216; the inner Result carries the per-file
        // validation / dir-walk S-codes (S210/S215/io).
        let (results, total) = join
            .await
            .map_err(|e| FsError::new("S216", format!("sed task join failed: {e}")))??;
        Ok(crate::fs::SedResponse {
            results,
            total_replacements: total,
        })
    }
    async fn write(&self, req: crate::fs::WriteArgs) -> FsCallResult<crate::fs::WriteResponse> {
        let base = self.confine_scope_root(crate::fs::scope_anchor(req.fs_scope.as_ref()))?;
        let extra = self.confine_scope_grants(crate::fs::scope_grants(req.fs_scope.as_ref()));
        let p = self.validate_path_scoped(
            &req.path,
            base.as_deref(),
            &extra,
            crate::fs::scope_boundary(req.fs_scope.as_ref()),
        )?;
        let bits = crate::fs::error::parse_mode(&req.mode)?;
        check_special_bits(bits, self.cfg.allow_special_bits)?;

        // Defense-in-depth: re-check parent against the effective canonical
        // ceiling(s) before creating intermediate directories. validate_path
        // already enforces this on `p`, but parents:true is S-C1's second site
        // so we keep the belt. The ceiling is the session scope_root when set,
        // else ALL host roots — so parents can never climb out of the session
        // directory (or, unscoped, out of every allowed root).
        let parent_ceilings = effective_roots(&self.host_roots_canon, base.as_deref(), &extra);
        if req.parents {
            if let Some(parent) = p.parent() {
                if !parent_ceilings.is_empty()
                    && !parent_ceilings
                        .iter()
                        .any(|c| parent.starts_with(c.as_path()))
                {
                    let ceil_display = parent_ceilings
                        .iter()
                        .map(|c| c.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(FsError::new(
                        "S215",
                        format!(
                            "parent escapes the allowed roots [{ceil_display}]: {}",
                            req.path
                        ),
                    ));
                }
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| FsError::from_io(&req.path, e))?;
            }
        }

        let suffix = format!(".tmp.{}", uuid::Uuid::new_v4().simple());
        let mut temp_str = p.as_os_str().to_os_string();
        temp_str.push(&suffix);
        let temp = std::path::PathBuf::from(temp_str);

        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .await
            .map_err(|e| FsError::from_io(&req.path, e))?;
        let guard = TempGuard::new(temp.clone());

        let cap = self.cfg.max_write_bytes;
        let total: u64 = match &req.content {
            // Inline path: the bytes are right here, no channel to drain. Enforce
            // max_write_bytes up front so an oversize inline payload is rejected
            // before it touches the temp file.
            crate::fs::WriteContent::Inline(data) => {
                let bytes = data.as_bytes();
                if cap > 0 && bytes.len() as u64 > cap as u64 {
                    return Err(FsError::new(
                        "S218",
                        format!(
                            "inline write payload {} bytes exceeds max_write_bytes {}",
                            bytes.len(),
                            cap
                        ),
                    ));
                }
                file.write_all(bytes)
                    .await
                    .map_err(|e| FsError::from_io(&req.path, e))?;
                bytes.len() as u64
            }
            // Streaming path: drain the caller's write channel chunk by chunk.
            crate::fs::WriteContent::Stream(channel) => {
                let reader =
                    iii_sdk::channels::ChannelReader::new(&self.chan.engine_address(), channel);
                let mut total: u64 = 0;
                // Per-chunk idle timeout: if the caller opens a write but never
                // sends data and never closes the channel, the worker would hold
                // the temp file open indefinitely. N parked writers = exhaustion.
                let idle = std::time::Duration::from_secs(30);
                loop {
                    let next = match tokio::time::timeout(idle, reader.next_binary()).await {
                        Ok(r) => r,
                        Err(_) => {
                            return Err(FsError::new(
                                "S216",
                                format!("channel idle for {}s — aborting write", idle.as_secs()),
                            ));
                        }
                    };
                    match next {
                        Ok(Some(chunk)) => {
                            let new_total = total + chunk.len() as u64;
                            if cap > 0 && new_total > cap as u64 {
                                return Err(FsError::new(
                                    "S218",
                                    format!(
                                        "write payload exceeds max_write_bytes {} (after {} bytes)",
                                        cap, total
                                    ),
                                ));
                            }
                            if let Err(e) = file.write_all(&chunk).await {
                                return Err(FsError::from_io(&req.path, e));
                            }
                            total = new_total;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            return Err(FsError::new("S216", format!("channel read failed: {e}")));
                        }
                    }
                }
                total
            }
        };

        if let Err(e) = file.flush().await {
            return Err(FsError::from_io(&req.path, e));
        }
        if let Err(e) = file.sync_all().await {
            return Err(FsError::from_io(&req.path, e));
        }
        drop(file);

        let perms = std::fs::Permissions::from_mode(bits);
        if let Err(e) = tokio::fs::set_permissions(guard.path(), perms).await {
            return Err(FsError::from_io(&req.path, e));
        }
        if let Err(e) = tokio::fs::rename(guard.path(), &p).await {
            return Err(FsError::from_io(&req.path, e));
        }
        guard.commit();

        Ok(crate::fs::WriteResponse {
            bytes_written: total,
            path: req.path,
            files: Vec::new(),
        })
    }
    async fn read(&self, req: crate::fs::ReadArgs) -> FsCallResult<crate::fs::ReadResponse> {
        use std::os::unix::fs::PermissionsExt;

        let base = self.confine_scope_root(crate::fs::scope_anchor(req.fs_scope.as_ref()))?;
        let extra = self.confine_scope_grants(crate::fs::scope_grants(req.fs_scope.as_ref()));
        let p = self.validate_path_scoped(
            &req.path,
            base.as_deref(),
            &extra,
            crate::fs::scope_boundary(req.fs_scope.as_ref()),
        )?;
        let md = tokio::fs::symlink_metadata(&p)
            .await
            .map_err(|e| FsError::from_io(&req.path, e))?;
        if md.is_dir() {
            return Err(FsError::new(
                "S212",
                format!("path is a directory, not a file: {}", req.path),
            ));
        }
        let cap = self.cfg.max_read_bytes;
        if cap > 0 && (md.len() as usize) > cap {
            return Err(FsError::new(
                "S218",
                format!("file size {} exceeds max_read_bytes {}", md.len(), cap),
            ));
        }
        let size = md.len();
        let mode = format!("{:04o}", md.permissions().mode() & 0o7777);
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let channel = self
            .chan
            .create_channel(64)
            .await
            .map_err(|e| FsError::new("S216", format!("create_channel: {e}")))?;
        let reader_ref = channel.reader_ref.clone();
        let writer = channel.writer;
        let path_for_pump = p;

        tokio::spawn(async move {
            let result = pump_file_to_channel(&path_for_pump, &writer).await;
            if let Err(msg) = result {
                let _ = writer
                    .send_message(&serde_json::json!({"error": msg}).to_string())
                    .await;
            }
            let _ = writer.close().await;
        });

        Ok(crate::fs::ReadResponse {
            content: reader_ref,
            size,
            mode,
            mtime,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("shell-fs-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[derive(Debug)]
    struct StubChan;
    #[async_trait::async_trait]
    impl super::ChannelMaker for StubChan {
        async fn create_channel(
            &self,
            _: usize,
        ) -> Result<iii_sdk::channel::Channel, iii_sdk::errors::Error> {
            Err(iii_sdk::errors::Error::Handler("stub".into()))
        }
        fn engine_address(&self) -> String {
            "ws://stub:0".into()
        }
    }
    fn stub_backend(cfg: HostFsConfig) -> HostFsBackend {
        HostFsBackend::new(Arc::new(cfg), Arc::new(StubChan))
    }

    #[test]
    fn try_new_returns_err_on_unreachable_jail_root() {
        let cfg = Arc::new(HostFsConfig {
            host_roots: vec![PathBuf::from("/nonexistent/shell-jail-xyz")],
            ..HostFsConfig::default()
        });
        let chan: Arc<dyn ChannelMaker> = Arc::new(StubChan);
        let res = HostFsBackend::try_new(cfg, chan);
        assert!(res.is_err());
        assert_eq!(res.err().unwrap().code, "S216");
    }

    #[test]
    fn confine_path_multi_root_absolute_any_relative_primary() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("fa.txt"), b"a").unwrap();
        std::fs::write(b.path().join("fb.txt"), b"b").unwrap();
        let roots = vec![
            std::fs::canonicalize(a.path()).unwrap(),
            std::fs::canonicalize(b.path()).unwrap(),
        ];
        // Absolute inside the SECOND root is accepted.
        let in_b =
            confine_path(&b.path().join("fb.txt").display().to_string(), &roots, &[]).unwrap();
        assert!(in_b.starts_with(std::fs::canonicalize(b.path()).unwrap()));
        // A relative path anchors at the PRIMARY (first) root, never root #2.
        let rel = confine_path("fa.txt", &roots, &[]).unwrap();
        assert!(rel.starts_with(std::fs::canonicalize(a.path()).unwrap()));
        // A path outside BOTH roots is rejected S215, naming the roots.
        let err = confine_path("/etc/passwd", &roots, &[]).unwrap_err();
        assert_eq!(err.code, "S215");
        assert!(
            err.message.contains("fs jail roots"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn confine_path_nested_roots_contained() {
        // Root A contains root B (nested/overlapping). A path inside B is
        // accepted; the inner root is honored as its own boundary.
        let a = tempfile::tempdir().unwrap();
        let b = a.path().join("inner");
        std::fs::create_dir(&b).unwrap();
        std::fs::write(b.join("x.txt"), b"x").unwrap();
        let roots = vec![
            std::fs::canonicalize(a.path()).unwrap(),
            std::fs::canonicalize(&b).unwrap(),
        ];
        let got = confine_path(&b.join("x.txt").display().to_string(), &roots, &[]).unwrap();
        assert!(got.starts_with(std::fs::canonicalize(&b).unwrap()));
    }

    #[test]
    fn try_new_dedups_canonically_equal_roots() {
        // The same dir listed twice collapses to one canonical root.
        let a = tempfile::tempdir().unwrap();
        let canon = std::fs::canonicalize(a.path()).unwrap();
        let cfg = Arc::new(HostFsConfig {
            host_roots: vec![a.path().to_path_buf(), canon.clone()],
            ..HostFsConfig::default()
        });
        let chan: Arc<dyn ChannelMaker> = Arc::new(StubChan);
        let backend = HostFsBackend::try_new(cfg, chan).unwrap();
        assert_eq!(backend.host_roots_canon, vec![canon]);
    }

    #[test]
    fn validate_path_rejects_non_accessible_glob_but_allows_others() {
        // D4: shell::fs hard-rejects a path matching the unified protected
        // globs (the same list the code surface uses), while non-matching paths
        // pass. The directory listing is NOT gated here, so the file stays
        // visible to `ls` — visible but locked.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".env"), b"secret").unwrap();
        std::fs::write(tmp.path().join("ok.txt"), b"x").unwrap();
        let cfg = Arc::new(HostFsConfig {
            host_roots: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["**/.env".to_string()],
            ..HostFsConfig::default()
        });
        let backend = HostFsBackend::try_new(cfg, Arc::new(StubChan)).unwrap();
        let err = backend.validate_path(".env").unwrap_err();
        assert_eq!(err.code, "S215");
        assert!(err.message.contains("protected"), "got: {}", err.message);
        assert!(backend.validate_path("ok.txt").is_ok());
    }

    #[test]
    fn try_new_rejects_bad_non_accessible_glob() {
        // Fail-closed (D2): a malformed protected glob aborts backend init.
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Arc::new(HostFsConfig {
            host_roots: vec![tmp.path().to_path_buf()],
            non_accessible_globs: vec!["[".to_string()],
            ..HostFsConfig::default()
        });
        let err = HostFsBackend::try_new(cfg, Arc::new(StubChan)).unwrap_err();
        assert_eq!(err.code, "S210");
    }

    fn stub_ref() -> iii_sdk::channels::StreamChannelRef {
        iii_sdk::channels::StreamChannelRef {
            channel_id: "c".into(),
            access_key: "k".into(),
            direction: iii_sdk::helpers::ChannelDirection::Read,
        }
    }

    #[test]
    fn rejects_relative_path() {
        let h = stub_backend(HostFsConfig::default());
        let err = h.validate_path("relative/path").unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[test]
    fn relative_path_resolves_under_jail_root() {
        // Regression: agents commonly probe with `.` or bare names; under a
        // jail there is exactly one sensible base, so resolve instead of
        // erroring with S210.
        let root = tmp();
        fs::create_dir(root.join("sub")).unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let dot = h.validate_path(".").unwrap();
        assert_eq!(dot, root.canonicalize().unwrap());
        let sub = h.validate_path("sub").unwrap();
        assert_eq!(sub, root.join("sub").canonicalize().unwrap());
    }

    #[test]
    fn relative_dotdot_cannot_escape_jail_root() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let err = h.validate_path("../../etc/passwd").unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[test]
    fn empty_path_rejected_even_under_jail_root() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let err = h.validate_path("").unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[test]
    fn escape_error_names_the_jail_root() {
        // Regression: the pre-fix S215 wording named no root at all ("path
        // escapes the jail: <path>"), giving the caller no way to recover —
        // the agent burned turns guessing. The message must name the jail root.
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let err = h.validate_path("/etc").unwrap_err();
        assert_eq!(err.code, "S215");
        let root_canon = root.canonicalize().unwrap();
        assert!(
            err.message.contains(&root_canon.display().to_string()),
            "S215 message must name the jail root, got: {}",
            err.message
        );
    }

    #[test]
    fn jail_escape_appends_machine_parseable_filesystem_access_request_tail() {
        let root = tmp();
        let outside = tmp();
        let outside_file = outside.join("nested/file.txt");
        fs::create_dir_all(outside_file.parent().unwrap()).unwrap();
        fs::write(&outside_file, "x").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root],
            ..Default::default()
        };
        let h = stub_backend(cfg);

        let err = h
            .validate_path(&outside_file.display().to_string())
            .unwrap_err();
        assert_eq!(err.code, "S215");
        let marker = "filesystem_access_request=";
        let idx = err
            .message
            .rfind(marker)
            .expect("jail rejection must append filesystem_access_request");
        let hint: serde_json::Value = serde_json::from_str(&err.message[idx + marker.len()..])
            .expect("filesystem_access_request tail must be valid JSON");
        assert_eq!(hint["v"], 1);
        assert_eq!(hint["error_code"], "S215");
        assert_eq!(hint["attempted_path"], outside_file.display().to_string());
        assert_eq!(
            hint["requested_root"],
            std::fs::canonicalize(outside_file.parent().unwrap())
                .unwrap()
                .display()
                .to_string()
        );
    }

    #[tokio::test]
    async fn scope_grants_allow_absolute_path_outside_session_scope() {
        let root = tmp();
        let session = root.join("session");
        fs::create_dir(&session).unwrap();
        let granted = tmp();
        fs::write(granted.join("allowed.txt"), "ok").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let path = granted.join("allowed.txt").display().to_string();

        let denied = h
            .stat(crate::fs::StatArgs {
                path: path.clone(),
                fs_scope: Some(crate::fs::FsScope {
                    root: session.display().to_string(),
                    grants: Vec::new(),
                    boundary: crate::fs::FsBoundary::Workspace,
                }),
            })
            .await
            .unwrap_err();
        assert_eq!(denied.code, "S215");

        let allowed = h
            .stat(crate::fs::StatArgs {
                path,
                fs_scope: Some(crate::fs::FsScope {
                    root: session.display().to_string(),
                    grants: vec![granted.display().to_string()],
                    boundary: crate::fs::FsBoundary::Workspace,
                }),
            })
            .await
            .unwrap();
        assert_eq!(allowed.0.name, "allowed.txt");
    }

    #[test]
    fn empty_path_rejected_when_unjailed() {
        let h = stub_backend(HostFsConfig::default());
        let err = h.validate_path("").unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn rm_relative_path_operates_on_jail_file_not_cwd() {
        // Regression: rm validated <jail root>/<rel> but removed <cwd>/<rel>
        // (the operand was rebuilt from the raw request string). The operand
        // must be the SAME jail-anchored path validate_path saw.
        let root = tmp();
        fs::write(root.join("victim.txt"), "x").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let res = b
            .rm(crate::fs::RmArgs {
                fs_scope: None,
                path: "victim.txt".into(),
                recursive: false,
            })
            .await
            .unwrap();
        assert!(res.removed);
        assert!(!root.join("victim.txt").exists());
    }

    #[tokio::test]
    async fn mv_relative_paths_operate_on_jail_files_not_cwd() {
        let root = tmp();
        fs::write(root.join("a.txt"), "content").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let res = b
            .mv(crate::fs::MvArgs {
                fs_scope: None,
                src: "a.txt".into(),
                dst: "b.txt".into(),
                overwrite: false,
            })
            .await
            .unwrap();
        assert!(res.moved);
        assert!(!root.join("a.txt").exists());
        assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "content");
    }

    #[tokio::test]
    async fn chmod_relative_path_operates_on_jail_file_not_cwd() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp();
        fs::write(root.join("f.txt"), "x").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let res = b
            .chmod(crate::fs::ChmodArgs {
                fs_scope: None,
                path: "f.txt".into(),
                mode: "0600".into(),
                uid: None,
                gid: None,
                recursive: false,
            })
            .await
            .unwrap();
        assert_eq!(res.entries_changed, 1);
        let mode = fs::metadata(root.join("f.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn sed_relative_file_edits_jail_file_not_cwd() {
        let root = tmp();
        fs::write(root.join("s.txt"), "hello world\n").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let res = b
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec!["s.txt".into()],
                path: None,
                recursive: true,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "world".into(),
                replacement: "jail".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap();
        assert_eq!(res.total_replacements, 1);
        assert_eq!(
            fs::read_to_string(root.join("s.txt")).unwrap(),
            "hello jail\n"
        );
    }

    #[test]
    fn path_is_non_accessible_checks_all_nested_roots() {
        // D4 fix: with nested roots, a ROOT-ANCHORED glob ("secrets.json") must
        // match against the INNER root's relative form, not only the outer's.
        let outer = tmp();
        let inner = outer.join("proj");
        fs::create_dir(&inner).unwrap();
        let roots = vec![
            std::fs::canonicalize(&outer).unwrap(),
            std::fs::canonicalize(&inner).unwrap(),
        ];
        let mut bld = GlobSetBuilder::new();
        bld.add(Glob::new("secrets.json").unwrap());
        let gs = bld.build().unwrap();
        let secret = std::fs::canonicalize(&inner).unwrap().join("secrets.json");
        assert!(
            path_is_non_accessible(&secret, &roots, &gs),
            "must block via the inner root's relative form"
        );
        let ok = std::fs::canonicalize(&outer).unwrap().join("ok.txt");
        assert!(!path_is_non_accessible(&ok, &roots, &gs));
    }

    #[test]
    fn path_is_non_accessible_protects_unjailed_paths_without_roots() {
        // The unjailed gap (MOT-4099 audit): with empty host_roots the glob
        // loop never ran and secrets globs were silently skipped. The
        // absolute-form fallback must protect them; a non-matching neighbor
        // stays accessible.
        let dir = tmp();
        let secret = std::fs::canonicalize(&dir).unwrap().join(".env");
        let plain = std::fs::canonicalize(&dir).unwrap().join("notes.txt");
        let mut bld = GlobSetBuilder::new();
        bld.add(Glob::new("**/.env").unwrap());
        let gs = bld.build().unwrap();

        assert!(
            path_is_non_accessible(&secret, &[], &gs),
            "empty roots must fall back to absolute-form matching"
        );
        assert!(!path_is_non_accessible(&plain, &[], &gs));

        // A contained path keeps root-relative-only matching: a glob that
        // would only match the absolute form must NOT fire inside a root.
        let roots = vec![std::fs::canonicalize(&dir).unwrap()];
        assert!(path_is_non_accessible(&secret, &roots, &gs));
        assert!(!path_is_non_accessible(&plain, &roots, &gs));
    }

    #[tokio::test]
    async fn sed_rejects_non_accessible_file_s215() {
        // D4: sed confines in a spawn_blocking closure (not validate_path_scoped)
        // — it must still hard-reject a protected file, exactly like write/rm.
        let root = tmp();
        fs::write(root.join(".env"), "SECRET=hello\n").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            non_accessible_globs: vec!["**/.env".into()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec![".env".into()],
                path: None,
                recursive: true,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "hello".into(),
                replacement: "pwned".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S215");
        // The protected file is untouched.
        assert_eq!(
            fs::read_to_string(root.join(".env")).unwrap(),
            "SECRET=hello\n"
        );
    }

    #[tokio::test]
    async fn grep_skips_non_accessible_files_in_dir_walk() {
        // D4: a directory grep must not LEAK the content of a protected file.
        let root = tmp();
        fs::write(root.join(".env"), "SECRET=needle\n").unwrap();
        fs::write(root.join("ok.txt"), "needle here\n").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            non_accessible_globs: vec!["**/.env".into()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let res = b
            .grep(crate::fs::GrepArgs {
                path: ".".into(),
                pattern: "needle".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 0,
                max_line_bytes: 0,
                fs_scope: None,
            })
            .await
            .unwrap();
        assert_eq!(res.matches.len(), 1, "the protected .env must be skipped");
        assert!(
            !res.matches.iter().any(|m| m.path.contains(".env")),
            "no .env content leaked through grep"
        );
    }

    #[tokio::test]
    async fn write_parents_true_escaping_path_returns_s215_naming_root() {
        // validate_path pre-empts the parents:true defense-in-depth branch,
        // but the contract under test is the same either way: an escaping
        // write with parents:true is S215 and the message names the root.
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .write(crate::fs::WriteArgs {
                fs_scope: None,
                path: "/etc/shell-escape/nested".into(),
                mode: "0644".into(),
                parents: true,
                content: crate::fs::WriteContent::Stream(stub_ref()),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S215");
        let root_canon = root.canonicalize().unwrap();
        assert!(
            err.message.contains(&root_canon.display().to_string()),
            "S215 message must name the jail root, got: {}",
            err.message
        );
    }

    #[test]
    fn allows_absolute_when_no_root() {
        let h = stub_backend(HostFsConfig::default());
        let _ = h.validate_path("/tmp").unwrap();
    }

    #[test]
    fn rejects_path_outside_jail_root() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let err = h.validate_path("/etc").unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[test]
    fn allows_descendant_of_jail_root() {
        let root = tmp();
        fs::create_dir(root.join("sub")).unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let _ = h.validate_path(root.join("sub").to_str().unwrap()).unwrap();
    }

    #[test]
    fn denylist_blocks_even_inside_root() {
        let root = tmp();
        fs::create_dir(root.join("etc")).unwrap();
        fs::write(root.join("etc/shadow"), b"x").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            denylist_paths: vec![root.join("etc/shadow")],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let err = h
            .validate_path(root.join("etc/shadow").to_str().unwrap())
            .unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[test]
    fn nonexistent_path_with_dotdot_cannot_escape_root() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let escape = root.join("nonexistent/../../../etc/passwd");
        let err = h.validate_path(escape.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[tokio::test]
    async fn ls_lists_dir_entries() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), b"hi").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .ls(LsArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
            })
            .await
            .unwrap();
        assert_eq!(resp.entries.len(), 1);
        assert_eq!(resp.entries[0].name, "a.txt");
        assert_eq!(resp.entries[0].size, 2);
        assert!(!resp.entries[0].is_dir);
    }

    #[tokio::test]
    async fn ls_missing_returns_s211() {
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .ls(LsArgs {
                fs_scope: None,
                path: "/nope/never/exists/iii".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn stat_returns_file_entry() {
        let root = tmp();
        let f = root.join("x.txt");
        std::fs::write(&f, b"data").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .stat(StatArgs {
                fs_scope: None,
                path: f.to_str().unwrap().into(),
            })
            .await
            .unwrap();
        assert_eq!(resp.0.name, "x.txt");
        assert_eq!(resp.0.size, 4);
    }

    #[tokio::test]
    async fn ls_on_file_returns_s212() {
        let root = tmp();
        let f = root.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .ls(LsArgs {
                fs_scope: None,
                path: f.to_str().unwrap().into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S212");
    }

    #[tokio::test]
    async fn ls_includes_symlink_with_is_symlink_true() {
        use std::os::unix::fs::symlink;
        let root = tmp();
        let target = root.join("real.txt");
        std::fs::write(&target, b"x").unwrap();
        symlink(&target, root.join("alias")).unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .ls(LsArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
            })
            .await
            .unwrap();
        let alias = resp.entries.iter().find(|e| e.name == "alias").unwrap();
        assert!(
            alias.is_symlink,
            "ls should report is_symlink=true for symlink entries"
        );
    }

    #[tokio::test]
    async fn ls_with_broken_symlink_does_not_abort() {
        use std::os::unix::fs::symlink;
        let root = tmp();
        std::fs::write(root.join("real.txt"), b"x").unwrap();
        symlink(root.join("nonexistent"), root.join("dangling")).unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .ls(LsArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
            })
            .await
            .unwrap();
        assert_eq!(resp.entries.len(), 2);
    }

    #[tokio::test]
    async fn ls_returns_mode_padded_to_four_digits() {
        let root = tmp();
        let f = root.join("p.txt");
        std::fs::write(&f, b"x").unwrap();
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&f, perms).unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .ls(LsArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
            })
            .await
            .unwrap();
        let entry = resp.entries.iter().find(|e| e.name == "p.txt").unwrap();
        assert_eq!(entry.mode, "0600");
    }

    #[tokio::test]
    async fn mkdir_creates_directory() {
        let root = tmp();
        let p = root.join("new");
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .mkdir(crate::fs::MkdirArgs {
                fs_scope: None,
                path: p.to_str().unwrap().into(),
                mode: "0755".into(),
                parents: false,
            })
            .await
            .unwrap();
        assert!(resp.created);
        assert!(p.is_dir());
    }

    #[tokio::test]
    async fn mkdir_returns_s213_on_exists_without_parents() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .mkdir(crate::fs::MkdirArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
                mode: "0755".into(),
                parents: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S213");
    }

    #[tokio::test]
    async fn mkdir_with_parents_tolerates_existing_path() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .mkdir(crate::fs::MkdirArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
                mode: "0755".into(),
                parents: true,
            })
            .await
            .unwrap();
        assert!(!resp.created);
    }

    #[tokio::test]
    async fn mkdir_bad_mode_returns_s210() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .mkdir(crate::fs::MkdirArgs {
                fs_scope: None,
                path: root.join("x").to_str().unwrap().into(),
                mode: "garbage".into(),
                parents: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn rm_file_succeeds() {
        let root = tmp();
        let f = root.join("d.txt");
        std::fs::write(&f, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .rm(crate::fs::RmArgs {
                fs_scope: None,
                path: f.to_str().unwrap().into(),
                recursive: false,
            })
            .await
            .unwrap();
        assert!(resp.removed);
        assert!(!f.exists());
    }

    #[tokio::test]
    async fn rm_missing_path_returns_s211() {
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .rm(crate::fs::RmArgs {
                fs_scope: None,
                path: "/nope/never/iii-rm-test".into(),
                recursive: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn rm_nonempty_dir_returns_s214() {
        let root = tmp();
        let sub = root.join("d");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("f"), b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .rm(crate::fs::RmArgs {
                fs_scope: None,
                path: sub.to_str().unwrap().into(),
                recursive: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S214");
    }

    #[tokio::test]
    async fn rm_symlink_to_dir_uses_remove_file() {
        use std::os::unix::fs::symlink;
        let root = tmp();
        let target = root.join("real-dir");
        std::fs::create_dir(&target).unwrap();
        let link = root.join("link-to-dir");
        symlink(&target, &link).unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .rm(crate::fs::RmArgs {
                fs_scope: None,
                path: link.to_str().unwrap().into(),
                recursive: false,
            })
            .await
            .unwrap();
        assert!(resp.removed);
        assert!(!link.exists());
        assert!(target.is_dir());
    }

    #[tokio::test]
    async fn mkdir_with_parents_creates_nested() {
        let root = tmp();
        let deep = root.join("a/b/c");
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .mkdir(crate::fs::MkdirArgs {
                fs_scope: None,
                path: deep.to_str().unwrap().into(),
                mode: "0755".into(),
                parents: true,
            })
            .await
            .unwrap();
        assert!(resp.created);
        assert!(deep.is_dir());
        assert!(root.join("a").is_dir());
        assert!(root.join("a/b").is_dir());
    }

    #[tokio::test]
    async fn rm_recursive_removes_tree() {
        let root = tmp();
        let tree = root.join("tree");
        std::fs::create_dir(&tree).unwrap();
        std::fs::create_dir(tree.join("sub")).unwrap();
        std::fs::write(tree.join("sub/leaf.txt"), b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .rm(crate::fs::RmArgs {
                fs_scope: None,
                path: tree.to_str().unwrap().into(),
                recursive: true,
            })
            .await
            .unwrap();
        assert!(resp.removed);
        assert!(!tree.exists());
    }

    #[tokio::test]
    async fn chmod_changes_mode() {
        let root = tmp();
        let f = root.join("c.txt");
        std::fs::write(&f, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .chmod(crate::fs::ChmodArgs {
                fs_scope: None,
                path: f.to_str().unwrap().into(),
                mode: "0600".into(),
                uid: None,
                gid: None,
                recursive: false,
            })
            .await
            .unwrap();
        assert_eq!(resp.entries_changed, 1);
        let perms = std::fs::metadata(&f).unwrap().permissions().mode() & 0o7777;
        assert_eq!(perms, 0o600);
    }

    #[tokio::test]
    async fn chmod_missing_returns_s211() {
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .chmod(crate::fs::ChmodArgs {
                fs_scope: None,
                path: "/nope/never/iii-chmod".into(),
                mode: "0600".into(),
                uid: None,
                gid: None,
                recursive: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn chmod_bad_mode_returns_s210() {
        let root = tmp();
        let f = root.join("c.txt");
        std::fs::write(&f, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .chmod(crate::fs::ChmodArgs {
                fs_scope: None,
                path: f.to_str().unwrap().into(),
                mode: "garbage".into(),
                uid: None,
                gid: None,
                recursive: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn chmod_recursive_walks_tree_and_counts() {
        let root = tmp();
        let tree = root.join("tree");
        std::fs::create_dir(&tree).unwrap();
        std::fs::create_dir(tree.join("sub")).unwrap();
        std::fs::write(tree.join("sub/leaf.txt"), b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .chmod(crate::fs::ChmodArgs {
                fs_scope: None,
                path: tree.to_str().unwrap().into(),
                mode: "0700".into(),
                uid: None,
                gid: None,
                recursive: true,
            })
            .await
            .unwrap();
        assert_eq!(resp.entries_changed, 3);
        let leaf_perms = std::fs::metadata(tree.join("sub/leaf.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(leaf_perms, 0o700);
    }

    #[tokio::test]
    async fn mv_renames_file() {
        let root = tmp();
        let a = root.join("a");
        let b = root.join("b");
        std::fs::write(&a, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .mv(crate::fs::MvArgs {
                fs_scope: None,
                src: a.to_str().unwrap().into(),
                dst: b.to_str().unwrap().into(),
                overwrite: false,
            })
            .await
            .unwrap();
        assert!(resp.moved);
        assert!(!a.exists());
        assert!(b.exists());
    }

    #[tokio::test]
    async fn mv_missing_src_returns_s211() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .mv(crate::fs::MvArgs {
                fs_scope: None,
                src: root.join("nope").to_str().unwrap().into(),
                dst: root.join("dst").to_str().unwrap().into(),
                overwrite: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
        assert!(err.message.contains("src not found"));
    }

    #[tokio::test]
    async fn mv_existing_dst_no_overwrite_returns_s213() {
        let root = tmp();
        let a = root.join("a");
        let b = root.join("b");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"y").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .mv(crate::fs::MvArgs {
                fs_scope: None,
                src: a.to_str().unwrap().into(),
                dst: b.to_str().unwrap().into(),
                overwrite: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S213");
        assert!(err.message.contains("dst already exists"));
    }

    #[tokio::test]
    async fn mv_with_overwrite_replaces_dst() {
        let root = tmp();
        let a = root.join("a");
        let b = root.join("b");
        std::fs::write(&a, b"new").unwrap();
        std::fs::write(&b, b"old").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .mv(crate::fs::MvArgs {
                fs_scope: None,
                src: a.to_str().unwrap().into(),
                dst: b.to_str().unwrap().into(),
                overwrite: true,
            })
            .await
            .unwrap();
        assert!(resp.moved);
        assert_eq!(std::fs::read(&b).unwrap(), b"new");
        assert!(!a.exists());
    }

    #[tokio::test]
    async fn chmod_with_uid_unprivileged_returns_s215() {
        let root = tmp();
        let f = root.join("c.txt");
        std::fs::write(&f, b"x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .chmod(crate::fs::ChmodArgs {
                fs_scope: None,
                path: f.to_str().unwrap().into(),
                mode: "0644".into(),
                uid: Some(0),
                gid: None,
                recursive: false,
            })
            .await;
        match resp {
            Ok(_) => {}

            Err(e) => assert_eq!(e.code, "S215"),
        }
    }

    #[tokio::test]
    async fn write_rejects_non_absolute_path_with_s210() {
        let b = stub_backend(HostFsConfig::default());
        let err = b
            .write(crate::fs::WriteArgs {
                fs_scope: None,
                path: "rel/path".into(),
                mode: "0644".into(),
                parents: false,
                content: crate::fs::WriteContent::Stream(stub_ref()),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn write_rejects_invalid_mode_with_s210() {
        let b = stub_backend(HostFsConfig::default());
        let err = b
            .write(crate::fs::WriteArgs {
                fs_scope: None,
                path: "/tmp/shell-write-bad-mode".into(),
                mode: "not-octal".into(),
                parents: false,
                content: crate::fs::WriteContent::Stream(stub_ref()),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn write_rejects_path_outside_jail_root_with_s215() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .write(crate::fs::WriteArgs {
                fs_scope: None,
                path: "/etc/shell-escape".into(),
                mode: "0644".into(),
                parents: false,
                content: crate::fs::WriteContent::Stream(stub_ref()),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[tokio::test]
    async fn write_inline_string_creates_file_with_content_and_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        // Inline content takes NO channel (StubChan's create_channel errors) —
        // proving an agent can write a file with a plain string.
        let resp = b
            .write(crate::fs::WriteArgs {
                fs_scope: None,
                path: "hello.txt".into(),
                mode: "0644".into(),
                parents: false,
                content: crate::fs::WriteContent::Inline("hello world\n".into()),
            })
            .await
            .expect("inline write succeeds without a channel");
        assert_eq!(resp.bytes_written, 12);
        assert_eq!(resp.path, "hello.txt");
        assert!(resp.files.is_empty(), "single write leaves files empty");
        assert_eq!(
            fs::read_to_string(root.join("hello.txt")).unwrap(),
            "hello world\n"
        );
        let mode = fs::metadata(root.join("hello.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o644);
    }

    #[tokio::test]
    async fn write_inline_respects_max_write_bytes_cap() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            max_write_bytes: 4,
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .write(crate::fs::WriteArgs {
                fs_scope: None,
                path: "big.txt".into(),
                mode: "0644".into(),
                parents: false,
                content: crate::fs::WriteContent::Inline("too many bytes".into()),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S218");
        assert!(
            !root.join("big.txt").exists(),
            "an over-cap inline write must not leave a partial file"
        );
    }

    #[tokio::test]
    async fn write_inline_escaping_path_returns_s215() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .write(crate::fs::WriteArgs {
                fs_scope: None,
                path: "../../etc/evil".into(),
                mode: "0644".into(),
                parents: false,
                content: crate::fs::WriteContent::Inline("x".into()),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[tokio::test]
    async fn write_with_missing_parent_and_parents_false_returns_s211() {
        let root = tmp();
        let target = root
            .join("nonexistent")
            .join("file.txt")
            .to_string_lossy()
            .to_string();
        let b = stub_backend(HostFsConfig::default());
        let err = b
            .write(crate::fs::WriteArgs {
                fs_scope: None,
                path: target,
                mode: "0644".into(),
                parents: false,
                content: crate::fs::WriteContent::Stream(stub_ref()),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn read_rejects_missing_file_with_s211() {
        let root = tmp();
        let canon_root = std::fs::canonicalize(&root).unwrap();
        let missing = canon_root.join("nope.txt").to_string_lossy().to_string();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .read(crate::fs::ReadArgs {
                fs_scope: None,
                path: missing,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S211");
    }

    #[tokio::test]
    async fn read_rejects_directory_with_s212() {
        let root = tmp();
        let dir = root.to_string_lossy().to_string();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .read(crate::fs::ReadArgs {
                fs_scope: None,
                path: dir,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S212");
    }

    #[tokio::test]
    async fn read_rejects_oversize_file_with_s218_preflight() {
        let root = tmp();
        let f = root.join("big.bin");
        std::fs::write(&f, vec![0u8; 1024]).unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            max_read_bytes: 10,
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .read(crate::fs::ReadArgs {
                fs_scope: None,
                path: f.to_string_lossy().to_string(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S218");
    }

    #[tokio::test]
    async fn read_rejects_path_outside_jail_root_with_s215() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .read(crate::fs::ReadArgs {
                fs_scope: None,
                path: "/etc/shell-escape".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S215");
    }

    #[tokio::test]
    async fn read_create_channel_failure_maps_to_s216() {
        let root = tmp();
        let f = root.join("ok.txt");
        std::fs::write(&f, b"hello").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .read(crate::fs::ReadArgs {
                fs_scope: None,
                path: f.to_string_lossy().to_string(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S216");
    }

    #[tokio::test]
    async fn grep_finds_matches_in_dir() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "alpha\nbeta\nalpha-beta\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .grep(crate::fs::GrepArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
                pattern: "alpha".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 100,
                max_line_bytes: 8192,
            })
            .await
            .unwrap();
        assert_eq!(resp.matches.len(), 2);
        assert!(!resp.truncated);
        assert_eq!(resp.matches[0].line, 1);
    }

    #[tokio::test]
    async fn grep_recursive_false_on_dir_returns_s210() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .grep(crate::fs::GrepArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
                pattern: "x".into(),
                recursive: false,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 100,
                max_line_bytes: 8192,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn grep_bad_regex_returns_s217() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .grep(crate::fs::GrepArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
                pattern: "[unclosed".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 1,
                max_line_bytes: 8192,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S217");
    }

    #[tokio::test]
    async fn grep_truncates_at_max_matches() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "x\nx\nx\nx\nx\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .grep(crate::fs::GrepArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
                pattern: "x".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 2,
                max_line_bytes: 8192,
            })
            .await
            .unwrap();
        assert_eq!(resp.matches.len(), 2);
        assert!(resp.truncated);
    }

    #[tokio::test]
    async fn grep_skips_binary_files() {
        let root = tmp();
        std::fs::write(root.join("bin.dat"), b"x\0x\0x\nx\nx\n").unwrap();
        std::fs::write(root.join("a.txt"), "x\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .grep(crate::fs::GrepArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
                pattern: "x".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 100,
                max_line_bytes: 8192,
            })
            .await
            .unwrap();
        assert!(resp.matches.iter().all(|m| m.path.ends_with("a.txt")));
    }

    #[tokio::test]
    async fn grep_include_glob_filters_by_extension() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "match\n").unwrap();
        std::fs::write(root.join("b.md"), "match\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .grep(crate::fs::GrepArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
                pattern: "match".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec!["*.txt".into()],
                exclude_glob: vec![],
                max_matches: 100,
                max_line_bytes: 8192,
            })
            .await
            .unwrap();
        assert_eq!(resp.matches.len(), 1);
        assert!(resp.matches[0].path.ends_with("a.txt"));
    }

    #[tokio::test]
    async fn sed_literal_replaces_files_form() {
        let root = tmp();
        let f = root.join("s.txt");
        std::fs::write(&f, "hello world hello").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec![f.to_str().unwrap().into()],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "hello".into(),
                replacement: "HI".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap();
        assert_eq!(resp.total_replacements, 2);
        assert_eq!(resp.results.len(), 1);
        assert!(resp.results[0].success);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "HI world HI");
    }

    #[tokio::test]
    async fn sed_first_only_stops_after_one() {
        let root = tmp();
        let f = root.join("s.txt");
        std::fs::write(&f, "x x x").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec![f.to_str().unwrap().into()],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "x".into(),
                replacement: "Y".into(),
                regex: false,
                first_only: true,
                ignore_case: false,
            })
            .await
            .unwrap();
        assert_eq!(resp.total_replacements, 1);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "Y x x");
    }

    #[tokio::test]
    async fn sed_neither_files_nor_path_returns_s210() {
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec![],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "x".into(),
                replacement: "Y".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn sed_both_files_and_path_returns_s210() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec!["/x".into()],
                path: Some(root.to_str().unwrap().into()),
                recursive: true,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "x".into(),
                replacement: "Y".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn sed_path_form_recursive_false_on_dir_returns_s210() {
        let root = tmp();
        let h = stub_backend(HostFsConfig::default());
        let err = h
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec![],
                path: Some(root.to_str().unwrap().into()),
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "x".into(),
                replacement: "Y".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
    }

    #[tokio::test]
    async fn sed_regex_with_capture_refs_works() {
        let root = tmp();
        let f = root.join("r.txt");
        std::fs::write(&f, "Alice loves Bob\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec![f.to_str().unwrap().into()],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "(\\w+) loves (\\w+)".into(),
                replacement: "$2 then $1".into(),
                regex: true,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap();
        assert_eq!(resp.total_replacements, 1);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "Bob then Alice\n");
    }

    /// Regression: with `regex=false, ignore_case=true`, replacement is
    /// supposed to be literal text. The case-insensitive path used to
    /// `replace('$', "$$")` before returning from a closure passed to
    /// `Regex::replace_all`, but closure returns are inserted verbatim
    /// (no $N substitution), so the escape doubled `$` and corrupted
    /// user-supplied literals like "$1" into "$$1".
    #[tokio::test]
    async fn sed_literal_ignore_case_preserves_dollar_in_replacement() {
        let root = tmp();
        let f = root.join("d.txt");
        std::fs::write(&f, "HELLO world\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec![f.to_str().unwrap().into()],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: "hello".into(),
                replacement: "$1".into(),
                regex: false,
                first_only: false,
                ignore_case: true,
            })
            .await
            .unwrap();
        assert_eq!(resp.total_replacements, 1);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "$1 world\n");
    }

    // --- glob matcher unit coverage (glob_match / glob_match_simple) ---

    #[test]
    fn glob_double_star_slash_extension_matches_nested() {
        // `**/*.rs` must match both a top-level and a deeply nested .rs file.
        assert!(glob_matches_path("**/*.rs", "main.rs"));
        assert!(glob_matches_path("**/*.rs", "a/b/c/lib.rs"));
        assert!(!glob_matches_path("**/*.rs", "a/b/c/lib.txt"));
    }

    #[test]
    fn glob_question_mark_is_single_non_slash_char() {
        assert!(glob_match_simple("a?c", "abc"));
        assert!(!glob_match_simple("a?c", "ac"));
        // `?` must not consume a path separator.
        assert!(!glob_match_simple("a?c", "a/c"));
    }

    #[test]
    fn glob_pattern_with_slash_matches_full_relpath() {
        // A pattern containing `/` is matched against the full relpath, not
        // just the basename.
        assert!(glob_matches_path("src/*.rs", "src/main.rs"));
        assert!(!glob_matches_path("src/*.rs", "other/main.rs"));
        // `*` does not cross `/`, so a nested file under src fails this glob.
        assert!(!glob_matches_path("src/*.rs", "src/inner/main.rs"));
    }

    #[test]
    fn glob_double_star_catch_all_matches_anything() {
        assert!(glob_matches_path("**", "anything"));
        assert!(glob_matches_path("**", "a/b/c.txt"));
        assert!(glob_match("**", "deep/nested/path"));
    }

    // --- setuid/setgid/sticky bit rejection ---

    #[tokio::test]
    async fn chmod_setuid_mode_rejected_by_default_s210() {
        let root = tmp();
        let f = root.join("c.txt");
        fs::write(&f, b"x").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let err = h
            .chmod(crate::fs::ChmodArgs {
                fs_scope: None,
                path: "c.txt".into(),
                mode: "4755".into(),
                uid: None,
                gid: None,
                recursive: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
        assert!(
            err.message.contains("allow_special_bits"),
            "message should name the flag, got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn chmod_setuid_mode_allowed_when_opted_in() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp();
        let f = root.join("c.txt");
        fs::write(&f, b"x").unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            allow_special_bits: true,
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let resp = h
            .chmod(crate::fs::ChmodArgs {
                fs_scope: None,
                path: "c.txt".into(),
                mode: "4755".into(),
                uid: None,
                gid: None,
                recursive: false,
            })
            .await
            .unwrap();
        assert_eq!(resp.entries_changed, 1);
        let mode = fs::metadata(&f).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o4755);
    }

    #[tokio::test]
    async fn mkdir_setgid_mode_rejected_by_default_s210() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let err = h
            .mkdir(crate::fs::MkdirArgs {
                fs_scope: None,
                path: "newdir".into(),
                mode: "2755".into(),
                parents: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
        assert!(!root.join("newdir").exists(), "dir must not be created");
    }

    #[tokio::test]
    async fn mkdir_setgid_mode_allowed_when_opted_in() {
        use std::os::unix::fs::PermissionsExt;
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            allow_special_bits: true,
            ..Default::default()
        };
        let h = stub_backend(cfg);
        let resp = h
            .mkdir(crate::fs::MkdirArgs {
                fs_scope: None,
                path: "newdir".into(),
                mode: "2755".into(),
                parents: false,
            })
            .await
            .unwrap();
        assert!(resp.created);
        let mode = fs::metadata(root.join("newdir"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(mode, 0o2755);
    }

    #[tokio::test]
    async fn write_sticky_mode_rejected_by_default_s210() {
        let root = tmp();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);
        let err = b
            .write(crate::fs::WriteArgs {
                fs_scope: None,
                path: "f.txt".into(),
                mode: "1644".into(),
                parents: false,
                content: crate::fs::WriteContent::Stream(stub_ref()),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
        assert!(err.message.contains("allow_special_bits"));
        assert!(!root.join("f.txt").exists());
    }

    // --- regex / pattern length caps ---

    #[tokio::test]
    async fn grep_over_long_pattern_rejected_fast() {
        let root = tmp();
        fs::write(root.join("a.txt"), "x\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let huge = "a".repeat(MAX_PATTERN_BYTES + 1);
        let started = std::time::Instant::now();
        let err = h
            .grep(crate::fs::GrepArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
                pattern: huge,
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 100,
                max_line_bytes: 8192,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "over-long pattern must be rejected without a compile stall"
        );
    }

    #[tokio::test]
    async fn grep_normal_pattern_still_works_after_caps() {
        let root = tmp();
        fs::write(root.join("a.txt"), "alpha\nbeta\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let resp = h
            .grep(crate::fs::GrepArgs {
                fs_scope: None,
                path: root.to_str().unwrap().into(),
                pattern: "alpha".into(),
                recursive: true,
                ignore_case: false,
                include_glob: vec![],
                exclude_glob: vec![],
                max_matches: 100,
                max_line_bytes: 8192,
            })
            .await
            .unwrap();
        assert_eq!(resp.matches.len(), 1);
        assert_eq!(resp.matches[0].content, "alpha");
    }

    #[tokio::test]
    async fn sed_over_long_pattern_rejected_fast() {
        let root = tmp();
        let f = root.join("s.txt");
        fs::write(&f, "x\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let huge = "a".repeat(MAX_PATTERN_BYTES + 1);
        let started = std::time::Instant::now();
        let err = h
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec![f.to_str().unwrap().into()],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: huge,
                replacement: "Y".into(),
                regex: true,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        // File must be untouched.
        assert_eq!(fs::read_to_string(&f).unwrap(), "x\n");
    }

    /// Companion to `sed_over_long_pattern_rejected_fast`, which exercises the
    /// `regex=true` path. The literal path (`regex=false`) uses the same
    /// `check_pattern_len` cap; this asserts it independently so the
    /// literal-mode huge-pattern branch can't regress unnoticed.
    #[tokio::test]
    async fn sed_over_long_literal_pattern_rejected_fast() {
        let root = tmp();
        let f = root.join("s.txt");
        fs::write(&f, "x\n").unwrap();
        let h = stub_backend(HostFsConfig::default());
        let huge = "a".repeat(MAX_PATTERN_BYTES + 1);
        let started = std::time::Instant::now();
        let err = h
            .sed(crate::fs::SedArgs {
                fs_scope: None,
                files: vec![f.to_str().unwrap().into()],
                path: None,
                recursive: false,
                include_glob: vec![],
                exclude_glob: vec![],
                pattern: huge,
                replacement: "Y".into(),
                regex: false,
                first_only: false,
                ignore_case: false,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code, "S210");
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        // File must be untouched.
        assert_eq!(fs::read_to_string(&f).unwrap(), "x\n");
    }

    // --- jail escape via a LIVE (non-dangling) symlink whose target is
    // outside the jail. The canonicalize + jail-root containment gate in
    // validate_path is the core security control; this exact vector was
    // untested. We point <jail root>/escape at a real existing dir outside the
    // jail and assert read/stat/ls all reject with S215.

    #[tokio::test]
    async fn live_symlink_resolving_outside_jail_is_rejected_s215() {
        use std::os::unix::fs::symlink;
        let root = tmp();
        // /etc exists and is outside the jail. A non-dangling symlink whose
        // target resolves there must be caught by the canonical jail check.
        symlink("/etc", root.join("escape")).unwrap();
        let cfg = HostFsConfig {
            host_roots: vec![root.clone()],
            ..Default::default()
        };
        let b = stub_backend(cfg);

        let read_err = b
            .read(crate::fs::ReadArgs {
                fs_scope: None,
                path: "escape/hostname".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(read_err.code, "S215", "read through escape symlink");

        let stat_err = b
            .stat(StatArgs {
                fs_scope: None,
                path: "escape/hostname".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(stat_err.code, "S215", "stat through escape symlink");

        let ls_err = b
            .ls(LsArgs {
                fs_scope: None,
                path: "escape".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(ls_err.code, "S215", "ls through escape symlink");
    }

    // --- per-call scope_root (session scope) on the fs backend ---

    /// Helper: a jailed backend rooted at `root`.
    fn jailed_backend(root: &std::path::Path) -> HostFsBackend {
        stub_backend(HostFsConfig {
            host_roots: vec![root.to_path_buf()],
            ..Default::default()
        })
    }

    /// A write with a relative path anchors at scope_root, not at the jail root: the
    /// file lands under <jail root>/session/, proving scope_root re-anchors the
    /// relative path.
    #[tokio::test]
    async fn write_relative_path_anchors_at_scope_root() {
        let root = tmp();
        fs::create_dir_all(root.join("session")).unwrap();
        let b = jailed_backend(&root);
        let base = root.join("session").to_string_lossy().into_owned();
        let resp = b
            .write(crate::fs::WriteArgs {
                path: "out.txt".into(),
                mode: "0644".into(),
                parents: false,
                content: crate::fs::WriteContent::Inline("scoped\n".into()),
                fs_scope: Some(crate::fs::FsScope {
                    root: base,
                    grants: Vec::new(),
                    boundary: crate::fs::FsBoundary::Workspace,
                }),
            })
            .await
            .expect("relative write under scope_root succeeds");
        assert_eq!(resp.bytes_written, 7);
        assert_eq!(
            fs::read_to_string(root.join("session/out.txt")).unwrap(),
            "scoped\n"
        );
        // It must NOT have landed at the jail-root level.
        assert!(!root.join("out.txt").exists());
    }

    /// rm with a relative path is confined to scope_root: the victim under
    /// <jail root>/session is removed, while an identically-named file at the
    /// jail-root level is untouched.
    #[tokio::test]
    async fn rm_relative_path_is_confined_to_scope_root() {
        let root = tmp();
        fs::create_dir_all(root.join("session")).unwrap();
        fs::write(root.join("session/victim.txt"), "x").unwrap();
        fs::write(root.join("victim.txt"), "sibling").unwrap();
        let b = jailed_backend(&root);
        let base = root.join("session").to_string_lossy().into_owned();
        let resp = b
            .rm(crate::fs::RmArgs {
                path: "victim.txt".into(),
                recursive: false,
                fs_scope: Some(crate::fs::FsScope {
                    root: base,
                    grants: Vec::new(),
                    boundary: crate::fs::FsBoundary::Workspace,
                }),
            })
            .await
            .expect("rm under scope_root succeeds");
        assert!(resp.removed);
        assert!(
            !root.join("session/victim.txt").exists(),
            "session file gone"
        );
        assert!(
            root.join("victim.txt").exists(),
            "jail-root sibling untouched — rm was confined to scope_root"
        );
    }

    /// mv with relative src/dst confines BOTH operands to scope_root.
    #[tokio::test]
    async fn mv_relative_src_and_dst_confined_to_scope_root() {
        let root = tmp();
        fs::create_dir_all(root.join("session")).unwrap();
        fs::write(root.join("session/a.txt"), "content").unwrap();
        let b = jailed_backend(&root);
        let base = root.join("session").to_string_lossy().into_owned();
        let resp = b
            .mv(crate::fs::MvArgs {
                src: "a.txt".into(),
                dst: "b.txt".into(),
                overwrite: false,
                fs_scope: Some(crate::fs::FsScope {
                    root: base,
                    grants: Vec::new(),
                    boundary: crate::fs::FsBoundary::Workspace,
                }),
            })
            .await
            .expect("mv under scope_root succeeds");
        assert!(resp.moved);
        assert!(!root.join("session/a.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("session/b.txt")).unwrap(),
            "content"
        );
    }

    /// DX-1: an ABSOLUTE path that is inside the jail root but OUTSIDE scope_root is
    /// rejected with the new S220 code, and the message NAMES the session dir
    /// (not the generic "escapes the fs jail" S215, which would contradict the
    /// tool's own configured roots).
    #[tokio::test]
    async fn abs_path_inside_jail_root_outside_scope_root_is_s220_naming_session() {
        let root = tmp();
        fs::create_dir_all(root.join("session")).unwrap();
        fs::create_dir_all(root.join("other")).unwrap();
        fs::write(root.join("other/secret.txt"), "x").unwrap();
        let b = jailed_backend(&root);
        // Absolute path that resolves inside <jail root>/other — a sibling of the
        // session dir, still inside an allowed root, but not this session.
        let abs = root.join("other/secret.txt").canonicalize().unwrap();
        let base = root.join("session").to_string_lossy().into_owned();
        let err = b
            .read(crate::fs::ReadArgs {
                path: abs.to_string_lossy().into_owned(),
                fs_scope: Some(crate::fs::FsScope {
                    root: base,
                    grants: Vec::new(),
                    boundary: crate::fs::FsBoundary::Workspace,
                }),
            })
            .await
            .expect_err("abs path outside scope_root must reject");
        assert_eq!(err.code, "S220", "distinct session-scope code, not S215");
        let session_canon = root.join("session").canonicalize().unwrap();
        assert!(
            err.message.contains(&session_canon.display().to_string()),
            "S220 must name the session dir, got: {}",
            err.message
        );
        assert!(
            !err.message.contains("escapes the fs jail"),
            "must not reuse the generic jail-escape wording, got: {}",
            err.message
        );
    }

    /// A harness-provided scope_root is trusted as the per-call root even when it
    /// sits outside the configured host roots. The operation is then confined
    /// under that selected directory.
    #[tokio::test]
    async fn scope_root_outside_jail_root_is_honored_as_selected_root() {
        let root = tmp();
        let selected = tmp();
        fs::create_dir_all(selected.join("project")).unwrap();
        fs::write(selected.join("project/a.txt"), "x").unwrap();
        let b = jailed_backend(&root);
        let resp = b
            .ls(LsArgs {
                path: ".".into(),
                fs_scope: Some(crate::fs::FsScope {
                    root: selected.join("project").to_string_lossy().into_owned(),
                    grants: Vec::new(),
                    boundary: crate::fs::FsBoundary::Workspace,
                }),
            })
            .await
            .expect("selected scope_root outside host roots should be honored");
        assert_eq!(resp.entries.len(), 1);
        assert_eq!(resp.entries[0].name, "a.txt");
    }

    #[tokio::test]
    async fn scope_root_outside_jail_root_still_applies_non_accessible_globs() {
        let root = tmp();
        let selected = tmp();
        fs::write(selected.join(".env"), "secret").unwrap();
        let b = stub_backend(HostFsConfig {
            host_roots: vec![root],
            non_accessible_globs: vec!["**/.env".into()],
            ..Default::default()
        });
        let err = b
            .read(crate::fs::ReadArgs {
                path: ".env".into(),
                fs_scope: Some(crate::fs::FsScope {
                    root: selected.to_string_lossy().into_owned(),
                    grants: Vec::new(),
                    boundary: crate::fs::FsBoundary::Workspace,
                }),
            })
            .await
            .expect_err("protected file under selected scope_root must stay locked");
        assert_eq!(err.code, "S215");
    }

    #[tokio::test]
    async fn relative_scope_root_is_rejected() {
        let root = tmp();
        let b = jailed_backend(&root);
        let err = b
            .ls(LsArgs {
                path: ".".into(),
                fs_scope: Some(crate::fs::FsScope {
                    root: "../../etc".into(),
                    grants: Vec::new(),
                    boundary: crate::fs::FsBoundary::Workspace,
                }),
            })
            .await
            .expect_err("relative scope_root is not part of the trusted contract");
        assert_eq!(err.code, "S210");
    }

    /// A genuinely jail-escaping absolute path (outside the jail root entirely)
    /// under a scope_root still rejects S215 — the DX-1 refinement only applies
    /// to paths that ARE inside an allowed root.
    #[tokio::test]
    async fn abs_path_outside_jail_root_under_scope_root_still_s215() {
        let root = tmp();
        fs::create_dir_all(root.join("session")).unwrap();
        let b = jailed_backend(&root);
        let base = root.join("session").to_string_lossy().into_owned();
        let err = b
            .read(crate::fs::ReadArgs {
                path: "/etc/hostname".into(),
                fs_scope: Some(crate::fs::FsScope {
                    root: base,
                    grants: Vec::new(),
                    boundary: crate::fs::FsBoundary::Workspace,
                }),
            })
            .await
            .expect_err("abs path outside the jail must reject");
        assert_eq!(err.code, "S215", "outside every allowed root stays S215");
    }

    /// With no session scope, a relative write still anchors at the jail root (not
    /// at any session dir).
    #[tokio::test]
    async fn scope_root_none_reproduces_jail_root_anchoring() {
        let root = tmp();
        let b = jailed_backend(&root);
        let resp = b
            .write(crate::fs::WriteArgs {
                path: "top.txt".into(),
                mode: "0644".into(),
                parents: false,
                content: crate::fs::WriteContent::Inline("legacy\n".into()),
                fs_scope: None,
            })
            .await
            .expect("relative write with scope_root=None anchors at the jail root");
        assert_eq!(resp.bytes_written, 7);
        assert_eq!(
            fs::read_to_string(root.join("top.txt")).unwrap(),
            "legacy\n",
            "scope_root=None ⇒ anchors at the jail root exactly as before"
        );
    }

    #[tokio::test]
    async fn configured_roots_boundary_anchors_relative_paths_without_blocking_siblings() {
        let root = tmp();
        fs::create_dir_all(root.join("session")).unwrap();
        fs::create_dir_all(root.join("sibling")).unwrap();
        fs::write(root.join("session/local.txt"), "local").unwrap();
        fs::write(root.join("sibling/shared.txt"), "shared").unwrap();
        let backend = jailed_backend(&root);
        let scope = crate::fs::FsScope {
            root: root.join("session").to_string_lossy().into_owned(),
            grants: Vec::new(),
            boundary: crate::fs::FsBoundary::ConfiguredRoots,
        };

        let relative = backend
            .stat(crate::fs::StatArgs {
                path: "local.txt".into(),
                fs_scope: Some(scope.clone()),
            })
            .await
            .expect("relative paths stay anchored at the working directory");
        assert_eq!(relative.0.name, "local.txt");

        let sibling = backend
            .stat(crate::fs::StatArgs {
                path: root.join("sibling/shared.txt").display().to_string(),
                fs_scope: Some(scope),
            })
            .await
            .expect("configured-roots mode permits siblings inside shell policy");
        assert_eq!(sibling.0.name, "shared.txt");
    }
}
