//! Integration coverage for `coder::apply-patch` — the V4A whole-patch
//! applier. Exercises the real handler: multi-hunk success, all-or-nothing
//! failure atomicity, move semantics, and jail rejection.

use std::path::PathBuf;
use std::sync::Arc;

use shell::code::config::CoderConfig;
use shell::code::functions::apply_patch::{handle as apply_handle, ApplyPatchInput};
use shell::code::path::PathResolver;
use tempfile::tempdir;

fn make(base: PathBuf) -> (Arc<PathResolver>, Arc<CoderConfig>) {
    let cfg = Arc::new(CoderConfig {
        base_paths: vec![base],
        max_read_bytes: 1024 * 1024,
        max_write_bytes: 1024 * 1024,
        ..CoderConfig::default()
    });
    let r = Arc::new(PathResolver::new(&cfg).unwrap());
    (r, cfg)
}

fn input(patch: &str) -> ApplyPatchInput {
    ApplyPatchInput {
        patch: patch.to_string(),
        fs_scope: None,
    }
}

#[tokio::test]
async fn applies_add_update_delete_in_one_patch() {
    let tmp = tempdir().unwrap();
    std::fs::write(
        tmp.path().join("calc.py"),
        "def add(a, b):\n    return a - b\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("legacy.py"), "old\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf());

    let out = apply_handle(
        r,
        c,
        input(
            "*** Begin Patch\n\
             *** Update File: calc.py\n\
             @@ def add(a, b):\n\
             -    return a - b\n\
             +    return a + b\n\
             *** Add File: util/helpers.py\n\
             +def helper():\n\
             +    return 1\n\
             *** Delete File: legacy.py\n\
             *** End Patch",
        ),
    )
    .await
    .unwrap();

    assert_eq!(out.results.len(), 3);
    assert_eq!(out.results[0].kind, "modified");
    assert_eq!(out.results[1].kind, "added");
    assert_eq!(out.results[2].kind, "deleted");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("calc.py")).unwrap(),
        "def add(a, b):\n    return a + b\n"
    );
    // Add created parent dirs automatically.
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("util/helpers.py")).unwrap(),
        "def helper():\n    return 1\n"
    );
    assert!(!tmp.path().join("legacy.py").exists());
    // Modified files echo the first changed region.
    let echo = out.results[0].echo.as_ref().expect("modified file echoes");
    assert!(echo.lines.iter().any(|l| l.contains("return a + b")));
}

#[tokio::test]
async fn context_mismatch_fails_whole_patch_with_nothing_written() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "alpha\n").unwrap();
    std::fs::write(tmp.path().join("b.txt"), "beta\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf());

    // First hunk is valid, second has stale context — NOTHING may land.
    let err = apply_handle(
        r,
        c,
        input(
            "*** Begin Patch\n\
             *** Update File: a.txt\n\
             @@\n\
             -alpha\n\
             +ALPHA\n\
             *** Update File: b.txt\n\
             @@\n\
             -does-not-exist\n\
             +x\n\
             *** End Patch",
        ),
    )
    .await
    .unwrap_err();

    assert!(err.contains("C210"), "{err}");
    assert!(err.contains("re-read the file"), "{err}");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("a.txt")).unwrap(),
        "alpha\n",
        "valid first hunk must not land when a later hunk fails"
    );
}

#[tokio::test]
async fn update_with_move_relocates_the_file() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("old_name.py"), "value = 1\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf());

    let out = apply_handle(
        r,
        c,
        input(
            "*** Begin Patch\n\
             *** Update File: old_name.py\n\
             *** Move to: new_name.py\n\
             @@\n\
             -value = 1\n\
             +value = 2\n\
             *** End Patch",
        ),
    )
    .await
    .unwrap();

    assert_eq!(out.results[0].kind, "moved");
    assert!(!tmp.path().join("old_name.py").exists());
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("new_name.py")).unwrap(),
        "value = 2\n"
    );
}

#[tokio::test]
async fn jail_escape_is_rejected() {
    let tmp = tempdir().unwrap();
    let (r, c) = make(tmp.path().to_path_buf());
    let err = apply_handle(
        r,
        c,
        input(
            "*** Begin Patch\n\
             *** Add File: ../outside.txt\n\
             +nope\n\
             *** End Patch",
        ),
    )
    .await
    .unwrap_err();
    assert!(err.contains("C215") || err.contains("C211"), "{err}");
}

#[tokio::test]
async fn add_existing_file_is_rejected_with_guidance() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("x.txt"), "here\n").unwrap();
    let (r, c) = make(tmp.path().to_path_buf());
    let err = apply_handle(
        r,
        c,
        input("*** Begin Patch\n*** Add File: x.txt\n+dup\n*** End Patch"),
    )
    .await
    .unwrap_err();
    assert!(err.contains("Update File"), "{err}");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("x.txt")).unwrap(),
        "here\n"
    );
}
