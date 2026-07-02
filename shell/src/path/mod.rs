//! Shared jail-safety leaf: symlink-safe path canonicalization.
//!
//! `canonicalize_with_fallback` + `normalize_lexical` are the LEAF of the
//! path-jail algorithm. They used to live byte-identically in both
//! `shell/src/fs/host.rs` and `coder/src/path/mod.rs` under a MIRROR-INVARIANT
//! note ("port any fix in one to the other"). The coder/shell merge removes
//! that hazard by hoisting the leaf here so every jail surface in this crate
//! shares ONE implementation:
//!   * `fs::host` (the `shell::fs::*` jail) uses it directly.
//!   * `code` (the folded `coder::*` `PathResolver`) uses it too.
//!
//! Only the LEAF is shared. The policy layers above it stay per-surface
//! (multi-root containment, the `non_accessible`/redaction model, and the
//! C2xx vs S2xx error spaces) — they have different contracts and different
//! golden tests, so unifying them is deliberately out of scope.

use std::path::{Component, Path, PathBuf};

/// Resolve `p` to a canonical path that is symlink-free for every existing
/// ancestor, even when `p` itself doesn't yet exist. The naive fallback —
/// "canonicalize, on ENOENT use the lexical path" — is a jail-escape vector
/// when the path traverses a symlink whose target is outside the jail: the
/// lexical form still `starts_with(<jail root>)`, but the kernel will follow
/// the link on the subsequent syscall. Walking up to the longest existing
/// ancestor and canonicalizing *that* forces every symlink in the existing
/// portion to be resolved; the non-existent tail can't itself contain
/// symlinks (it doesn't exist) but can still contain `..`/`.`, which we
/// then collapse lexically against the canonical prefix so the
/// jail-root containment check is sound.
pub(crate) fn canonicalize_with_fallback(p: &Path) -> std::io::Result<PathBuf> {
    if let Ok(c) = std::fs::canonicalize(p) {
        return Ok(c);
    }
    // Walk ancestors top-down (longest prefix first) until canonicalize
    // succeeds. ancestors() yields p first; skip it because we already
    // tried it. After we find the longest canonicalizable ancestor, walk
    // forward through each tail component and reject if any of them is a
    // *dangling* symlink: canonicalize fails on dangling symlinks (target
    // doesn't exist), so they'd otherwise survive into the lexical tail
    // and let the jail-root containment check succeed against a path the kernel
    // would resolve outside the jail. Existing-but-resolvable symlinks
    // are caught by the top-of-function canonicalize.
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

/// Lexically collapse `.`/`..` against a path WITHOUT touching the
/// filesystem. Only sound on a prefix already known to be symlink-free
/// (e.g. the canonical ancestor `canonicalize_with_fallback` walks from).
pub(crate) fn normalize_lexical(p: &Path) -> PathBuf {
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

    #[test]
    fn normalize_lexical_collapses_dot_and_dotdot() {
        assert_eq!(
            normalize_lexical(Path::new("/a/./b/../c")),
            PathBuf::from("/a/c")
        );
        assert_eq!(
            normalize_lexical(Path::new("a/b/../../c")),
            PathBuf::from("c")
        );
        assert_eq!(normalize_lexical(Path::new("/a/b")), PathBuf::from("/a/b"));
    }

    #[test]
    fn canonicalize_existing_path_resolves_symlinks() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("real")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("link")).unwrap();
        let got = canonicalize_with_fallback(&tmp.path().join("link")).unwrap();
        assert_eq!(got, std::fs::canonicalize(tmp.path().join("real")).unwrap());
    }

    #[test]
    fn canonicalize_nonexistent_tail_uses_longest_existing_ancestor() {
        let tmp = tempdir().unwrap();
        let canon = std::fs::canonicalize(tmp.path()).unwrap();
        let got = canonicalize_with_fallback(&tmp.path().join("does/not/exist.txt")).unwrap();
        assert_eq!(got, canon.join("does/not/exist.txt"));
    }

    #[test]
    fn canonicalize_through_escaping_symlink_resolves_outside() {
        // An existing symlink whose target is outside is RESOLVED (not left
        // lexical) so a containment check above can reject it.
        let tmp = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("escape")).unwrap();
        let got = canonicalize_with_fallback(&tmp.path().join("escape/child.txt")).unwrap();
        assert!(got.starts_with(std::fs::canonicalize(outside.path()).unwrap()));
        assert!(!got.starts_with(std::fs::canonicalize(tmp.path()).unwrap()));
    }

    #[test]
    fn canonicalize_dangling_symlink_in_tail_errors() {
        let tmp = tempdir().unwrap();
        std::os::unix::fs::symlink(tmp.path().join("missing"), tmp.path().join("dangle")).unwrap();
        let err = canonicalize_with_fallback(&tmp.path().join("dangle/child.txt")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("dangling symlink in path"));
    }
}
