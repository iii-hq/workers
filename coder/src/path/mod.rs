//! Path resolution and access control.
//!
//! All wire-facing paths are *relative to `base_path`*. `PathResolver`
//! canonicalises them (symlink-aware) and verifies they remain inside
//! `base_root` so `..` and crafted symlinks cannot escape. A `GlobSet`
//! built from `non_accessible_globs` further blocks read/write/delete on
//! sensitive entries (`.env`, `*.pem`, …) while still allowing them to
//! appear in `list-folder`/`tree` listings.
//!
//! The symlink-safe canonicalisation walks to the longest existing
//! ancestor, canonicalises that, then lexically collapses the tail —
//! mirroring [`shell/src/fs/host.rs`](../../../shell/src/fs/host.rs)
//! `canonicalize_with_fallback`.

use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::config::CoderConfig;
use crate::error::CoderError;

#[derive(Debug)]
pub struct PathResolver {
    base_root_canon: PathBuf,
    non_accessible: GlobSet,
}

impl PathResolver {
    pub fn new(cfg: &CoderConfig) -> Result<Self, CoderError> {
        let base_root_canon = std::fs::canonicalize(&cfg.base_path).map_err(|e| {
            CoderError::Io(format!(
                "base_path {} cannot be canonicalized: {e}",
                cfg.base_path.display()
            ))
        })?;
        let mut builder = GlobSetBuilder::new();
        for pat in &cfg.non_accessible_globs {
            let g = Glob::new(pat).map_err(|e| {
                CoderError::BadInput(format!("invalid non_accessible_glob {pat:?}: {e}"))
            })?;
            builder.add(g);
        }
        let non_accessible = builder
            .build()
            .map_err(|e| CoderError::BadInput(format!("globset build failed: {e}")))?;
        Ok(Self {
            base_root_canon,
            non_accessible,
        })
    }

    pub fn base_root(&self) -> &Path {
        &self.base_root_canon
    }

    /// Resolve `rel` to a canonical absolute path under `base_root`. The
    /// path need not exist; we walk to the longest existing ancestor,
    /// canonicalise that, then lexically collapse the tail. Rejects:
    ///
    /// - absolute inputs → `C210` (the wire API is base-relative)
    /// - results outside `base_root` → `C215`
    /// - dangling symlinks in the tail → `C215`
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, CoderError> {
        let rel_path = Path::new(rel);
        if rel_path.is_absolute() {
            return Err(CoderError::BadInput(format!(
                "path must be relative to base_path: {rel}"
            )));
        }
        let joined = self.base_root_canon.join(rel_path);
        let canon = canonicalize_with_fallback(&joined).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("dangling symlink in path") {
                CoderError::OutsideBase(format!("{rel}: {msg}"))
            } else if e.kind() == std::io::ErrorKind::InvalidInput
                || e.kind() == std::io::ErrorKind::NotFound
            {
                CoderError::NotFoundOrDenied(format!("{rel}: {msg}"))
            } else {
                CoderError::Io(format!("canonicalize {rel}: {e}"))
            }
        })?;
        if !canon.starts_with(&self.base_root_canon) {
            return Err(CoderError::OutsideBase(format!(
                "path escapes base_path: {rel}"
            )));
        }
        Ok(canon)
    }

    /// Path's location relative to `base_root` as a forward-slash string,
    /// suitable for glob matching and wire responses. Returns `None` if
    /// `abs` isn't under `base_root` (should never happen for paths that
    /// came out of `resolve`).
    pub fn relative(&self, abs: &Path) -> Option<String> {
        abs.strip_prefix(&self.base_root_canon)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    /// True if `abs`'s base-relative form matches any non-accessible glob.
    /// `abs` is expected to be a path previously returned by `resolve`.
    pub fn is_non_accessible(&self, abs: &Path) -> bool {
        let Some(rel) = self.relative(abs) else {
            return false;
        };
        if rel.is_empty() {
            return false;
        }
        self.non_accessible.is_match(&rel)
    }

    /// Resolve and reject if the result is on the non-accessible list.
    /// Used by every mutating operation and by `read-file` so the same
    /// glob hides both reads and writes.
    pub fn require_writable(&self, rel: &str) -> Result<PathBuf, CoderError> {
        let abs = self.resolve(rel)?;
        if self.is_non_accessible(&abs) {
            return Err(CoderError::NotFoundOrDenied(format!(
                "path is non-accessible per config: {rel}"
            )));
        }
        Ok(abs)
    }
}

fn canonicalize_with_fallback(p: &Path) -> std::io::Result<PathBuf> {
    if let Ok(c) = std::fs::canonicalize(p) {
        return Ok(c);
    }
    for anc in p.ancestors().skip(1) {
        if let Ok(canon_anc) = std::fs::canonicalize(anc) {
            let suffix = match p.strip_prefix(anc) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut walk = canon_anc;
            for component in suffix.components() {
                walk.push(component);
                if let Ok(md) = std::fs::symlink_metadata(&walk) {
                    if md.file_type().is_symlink() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!("dangling symlink in path: {}", walk.display()),
                        ));
                    }
                }
            }
            return Ok(normalize_lexical(&walk));
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "no existing ancestor to canonicalize",
    ))
}

fn normalize_lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn cfg_with(base: PathBuf, globs: Vec<&str>) -> CoderConfig {
        CoderConfig {
            base_path: base,
            non_accessible_globs: globs.into_iter().map(String::from).collect(),
            ..CoderConfig::default()
        }
    }

    #[test]
    fn resolve_dot_returns_base_root() {
        let tmp = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let got = r.resolve(".").unwrap();
        assert_eq!(got, std::fs::canonicalize(tmp.path()).unwrap());
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
    fn resolve_absolute_input_rejected_as_bad_input() {
        let tmp = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let err = r.resolve("/etc/passwd").unwrap_err();
        assert_eq!(err.code(), "C210");
    }

    #[test]
    fn resolve_dotdot_escape_rejected_as_outside_base() {
        let tmp = tempdir().unwrap();
        let r = PathResolver::new(&cfg_with(tmp.path().to_path_buf(), vec![])).unwrap();
        let err = r.resolve("../etc/passwd").unwrap_err();
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
    fn new_with_missing_base_path_returns_io_error() {
        let cfg = CoderConfig {
            base_path: PathBuf::from("/this/does/not/exist/probably/xyz123"),
            ..CoderConfig::default()
        };
        let err = PathResolver::new(&cfg).unwrap_err();
        assert_eq!(err.code(), "C216");
    }
}
