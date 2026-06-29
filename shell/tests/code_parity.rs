//! GOLDEN FAMILY C — canonicalization parity vectors for the code jail.
//!
//! CANONICAL CASE MATRIX for the jail-safety leaf shared by the two
//! path-jail surfaces in this crate: `shell::code::path` (the folded
//! `coder::*` resolver) and `shell::fs::host`. Both build containment on
//! the same `crate::path::{canonicalize_with_fallback, normalize_lexical}`
//! leaf, so canonicalize-before-containment must behave identically
//! whichever surface a path enters through.
//!
//! This VECTOR TABLE pins that behavioral contract against
//! `shell::code::path::PathResolver::resolve`; `shell::fs::host` exercises
//! the same leaf through its own confinement tests. When you change the
//! shared leaf, extend this matrix with the case that motivated the change
//! so the contract stays observable from the code surface.
//!
//! Case matrix:
//!   1. relative resolve            -> Ok, inside primary root
//!   2. `.` resolve                 -> Ok, equals canonical primary root
//!   3. nonexistent inside base     -> Ok via longest-existing-ancestor fallback
//!   4. relative `..` escape        -> C215
//!   5. symlink escape              -> C215 (canonicalized BEFORE containment)
//!   6. dangling symlink in tail    -> C215
//!   7. absolute inside a root      -> Ok, canonical absolute
//!   8. absolute outside all roots  -> C215

use std::path::Path;

use shell::code::config::CoderConfig;
use shell::code::path::PathResolver;

/// What a vector expects from `PathResolver::resolve`.
enum Expect {
    /// Ok; result starts with the canonical primary root and ends with
    /// the given suffix.
    OkInsidePrimary(&'static str),
    /// Ok; result is exactly the canonical primary root.
    OkEqualsPrimaryRoot,
    /// Err with this error code.
    ErrCode(&'static str),
}

struct Vector {
    name: &'static str,
    /// Prepare the jail contents and return the wire path to resolve.
    /// Receives the RAW (pre-canonicalization) primary root.
    arrange: fn(root: &Path) -> String,
    expect: Expect,
}

const VECTORS: &[Vector] = &[
    Vector {
        name: "relative_resolve",
        arrange: |root| {
            std::fs::create_dir(root.join("sub")).unwrap();
            std::fs::write(root.join("sub/a.txt"), b"hi").unwrap();
            "sub/a.txt".into()
        },
        expect: Expect::OkInsidePrimary("sub/a.txt"),
    },
    Vector {
        name: "dot_resolve",
        arrange: |_root| ".".into(),
        expect: Expect::OkEqualsPrimaryRoot,
    },
    Vector {
        name: "nonexistent_inside_base_via_fallback",
        arrange: |_root| "does/not/exist.txt".into(),
        expect: Expect::OkInsidePrimary("does/not/exist.txt"),
    },
    Vector {
        name: "dotdot_escape",
        arrange: |_root| "../escape.txt".into(),
        expect: Expect::ErrCode("C215"),
    },
    Vector {
        name: "symlink_escape",
        arrange: |root| {
            // Symlink inside the jail pointing OUTSIDE it. The lexical
            // form stays inside the root; only canonicalization-before-
            // containment catches the escape (the jail-escape vector the
            // MIRROR-INVARIANT exists for).
            std::os::unix::fs::symlink("/", root.join("escape")).unwrap();
            "escape/etc/passwd".into()
        },
        expect: Expect::ErrCode("C215"),
    },
    Vector {
        name: "dangling_symlink",
        arrange: |root| {
            std::os::unix::fs::symlink(root.join("missing-target"), root.join("dangle")).unwrap();
            "dangle/child.txt".into()
        },
        expect: Expect::ErrCode("C215"),
    },
    Vector {
        name: "absolute_inside_root_accept",
        arrange: |root| {
            std::fs::write(root.join("abs.txt"), b"x").unwrap();
            root.join("abs.txt").display().to_string()
        },
        expect: Expect::OkInsidePrimary("abs.txt"),
    },
    Vector {
        name: "absolute_outside_all_roots",
        arrange: |_root| "/etc/passwd".into(),
        expect: Expect::ErrCode("C215"),
    },
];

#[test]
fn canonicalization_parity_vectors() {
    for v in VECTORS {
        // Fresh jail per vector so arrangements can't interfere.
        let tmp = tempfile::tempdir().unwrap();
        let cfg = CoderConfig {
            base_paths: vec![tmp.path().to_path_buf()],
            ..CoderConfig::default()
        };
        let resolver = PathResolver::new(&cfg)
            .unwrap_or_else(|e| panic!("[{}] resolver construction failed: {e}", v.name));
        let canon_root = std::fs::canonicalize(tmp.path()).unwrap();

        let wire = (v.arrange)(tmp.path());
        let got = resolver.resolve(&wire);

        match (&v.expect, got) {
            (Expect::OkInsidePrimary(suffix), Ok(p)) => {
                assert!(
                    p.starts_with(&canon_root),
                    "[{}] {p:?} must start with canonical primary root {canon_root:?}",
                    v.name
                );
                assert!(
                    p.ends_with(suffix),
                    "[{}] {p:?} must end with {suffix:?}",
                    v.name
                );
            }
            (Expect::OkEqualsPrimaryRoot, Ok(p)) => {
                assert_eq!(
                    p, canon_root,
                    "[{}] must resolve to the canonical primary root",
                    v.name
                );
            }
            (Expect::ErrCode(code), Err(e)) => {
                assert_eq!(
                    e.code(),
                    *code,
                    "[{}] wrong error code; message: {e}",
                    v.name
                );
            }
            (Expect::OkInsidePrimary(_), Err(e)) | (Expect::OkEqualsPrimaryRoot, Err(e)) => {
                panic!("[{}] expected Ok, got {} ({e})", v.name, e.code());
            }
            (Expect::ErrCode(code), Ok(p)) => {
                panic!("[{}] expected {code}, got Ok({p:?})", v.name);
            }
        }
    }
}
