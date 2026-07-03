//! Integration coverage for the folded `coder::*` `PathResolver` security
//! properties. The per-module unit tests already exercise most branches;
//! these scenarios re-verify the cross-cutting invariants from outside the
//! module:
//!
//! - `..` escapes must never resolve outside the allowed roots.
//! - Absolute paths are accepted only inside an allowed root.
//! - Symlinks to outside the base must be rejected.
//! - Non-accessible globs must block reading too (not just writing).

use std::path::PathBuf;
use std::sync::Arc;

use shell::code::config::CoderConfig;
use shell::code::functions::create_file::{
    handle as create_handle, CreateFileInput, CreateFileSpec,
};
use shell::code::functions::delete_file::{handle as delete_handle, DeleteFileInput};
use shell::code::functions::read_file::{handle as read_handle, ReadFileInput};
use shell::code::path::PathResolver;
use tempfile::tempdir;

fn make_resolver(base: PathBuf, globs: Vec<&str>) -> (Arc<PathResolver>, Arc<CoderConfig>) {
    let cfg = Arc::new(CoderConfig {
        base_paths: vec![base],
        non_accessible_globs: globs.into_iter().map(String::from).collect(),
        ..CoderConfig::default()
    });
    let resolver = Arc::new(PathResolver::new(&cfg).expect("PathResolver init"));
    (resolver, cfg)
}

#[test]
fn dotdot_in_path_cannot_escape_base_root() {
    let tmp = tempdir().unwrap();
    let (r, _) = make_resolver(tmp.path().to_path_buf(), vec![]);
    let err = r.resolve("../../../etc/passwd").unwrap_err();
    assert_eq!(err.code(), "C215");
}

#[test]
fn absolute_path_outside_all_roots_rejected_with_c215() {
    let tmp = tempdir().unwrap();
    let (r, _) = make_resolver(tmp.path().to_path_buf(), vec![]);
    let err = r.resolve("/etc/passwd").unwrap_err();
    assert_eq!(err.code(), "C215");
}

#[test]
fn absolute_path_inside_a_root_accepted() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("ok.txt"), b"x").unwrap();
    let (r, _) = make_resolver(tmp.path().to_path_buf(), vec![]);
    let abs_input = tmp.path().join("ok.txt").display().to_string();
    let resolved = r.resolve(&abs_input).expect("absolute inside root");
    assert!(resolved.starts_with(r.base_root()));
    assert!(
        resolved.ends_with("ok.txt"),
        "must resolve to the named file, not just somewhere in-root: {resolved:?}"
    );
}

#[test]
fn symlink_to_outside_base_rejected() {
    let tmp = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), b"x").unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret.txt"), tmp.path().join("link")).unwrap();
    let (r, _) = make_resolver(tmp.path().to_path_buf(), vec![]);
    let err = r.resolve("link").unwrap_err();
    assert_eq!(err.code(), "C215");
}

#[test]
fn symlink_inside_base_is_allowed() {
    let tmp = tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("real")).unwrap();
    std::fs::write(tmp.path().join("real/a.txt"), b"hi").unwrap();
    std::os::unix::fs::symlink(tmp.path().join("real/a.txt"), tmp.path().join("link")).unwrap();
    let (r, _) = make_resolver(tmp.path().to_path_buf(), vec![]);
    let resolved = r
        .resolve("link")
        .expect("symlink to inside base must succeed");
    assert!(resolved.starts_with(r.base_root()));
    assert!(
        resolved.ends_with("real/a.txt"),
        "symlink must canonicalize to its in-jail target: {resolved:?}"
    );
}

#[test]
fn non_accessible_glob_blocks_read() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join(".env"), b"API_KEY=secret").unwrap();
    let (r, c) = make_resolver(tmp.path().to_path_buf(), vec!["**/.env"]);
    let err = tokio_test_block_on(read_handle(
        r,
        c,
        ReadFileInput {
            path: Some(".env".into()),
            ..ReadFileInput::default()
        },
    ))
    .unwrap_err();
    assert!(err.contains("C211"), "got: {err}");
}

/// The fold's headline multi-root capability: an absolute path inside a
/// NON-primary allowed root (base_paths[1]) is accepted and resolves under
/// that root — proven here at the integration layer, not just in unit tests.
#[test]
fn absolute_path_inside_non_primary_root_resolves_there() {
    let primary = tempdir().unwrap();
    let secondary = tempdir().unwrap();
    std::fs::write(secondary.path().join("in-second.txt"), b"x").unwrap();
    let cfg = Arc::new(CoderConfig {
        // Primary root is index 0; the target file lives ONLY under the
        // secondary root.
        base_paths: vec![primary.path().to_path_buf(), secondary.path().to_path_buf()],
        ..CoderConfig::default()
    });
    let r = Arc::new(PathResolver::new(&cfg).expect("two-root resolver"));

    let abs = secondary.path().join("in-second.txt").display().to_string();
    let resolved = r
        .resolve(&abs)
        .expect("absolute path inside the non-primary root must resolve");

    let canon_secondary = std::fs::canonicalize(secondary.path()).unwrap();
    let canon_primary = std::fs::canonicalize(primary.path()).unwrap();
    assert!(
        resolved.starts_with(&canon_secondary),
        "must resolve under the secondary root, got {resolved:?}"
    );
    assert!(resolved.ends_with("in-second.txt"));
    assert!(
        !resolved.starts_with(&canon_primary),
        "must not be anchored under the primary root"
    );
}

#[test]
fn nested_dotenv_blocked_by_glob() {
    let tmp = tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("a/b/c")).unwrap();
    std::fs::write(tmp.path().join("a/b/c/.env"), b"x").unwrap();
    let (r, _) = make_resolver(tmp.path().to_path_buf(), vec!["**/.env"]);
    let abs = r.resolve("a/b/c/.env").unwrap();
    assert!(r.is_non_accessible(&abs));
}

#[tokio::test]
async fn scoped_scope_root_blocks_sibling_escape_across_handlers() {
    let tmp = tempdir().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("inside.txt"), b"inside").unwrap();
    std::fs::write(tmp.path().join("sibling.txt"), b"sibling").unwrap();
    let (r, c) = make_resolver(tmp.path().to_path_buf(), vec![]);
    let scope_root = project.to_string_lossy().into_owned();

    // `../sibling.txt` is still inside the worker's allowed root, so a plain
    // jail check is not enough. Session-scoped calls must reject it with C218.
    let read_err = read_handle(
        r.clone(),
        c.clone(),
        ReadFileInput {
            path: Some("../sibling.txt".into()),
            fs_scope: Some(shell::fs::FsScope {
                root: scope_root.clone(),
                grants: Vec::new(),
            }),
            ..ReadFileInput::default()
        },
    )
    .await
    .expect_err("scoped read must not escape the session dir");
    assert_eq!(wire_err_code(&read_err), "C218");

    let create_err = create_handle(
        r.clone(),
        c,
        CreateFileInput {
            files: vec![CreateFileSpec {
                path: "../created-outside.txt".into(),
                content: "oops".into(),
                mode: "0644".into(),
                parents: true,
                overwrite: false,
            }],
            fs_scope: Some(shell::fs::FsScope {
                root: scope_root.clone(),
                grants: Vec::new(),
            }),
        },
    )
    .await
    .expect_err("scoped create must not escape the session dir");
    assert_eq!(wire_err_code(&create_err), "C218");
    assert!(
        !tmp.path().join("created-outside.txt").exists(),
        "rejected create must not write into a sibling of the session dir"
    );

    let delete_err = delete_handle(
        r,
        DeleteFileInput {
            paths: vec!["../sibling.txt".into()],
            recursive: false,
            fs_scope: Some(shell::fs::FsScope {
                root: scope_root,
                grants: Vec::new(),
            }),
        },
    )
    .await
    .expect_err("scoped delete must not escape the session dir");
    assert_eq!(wire_err_code(&delete_err), "C218");
    assert!(
        tmp.path().join("sibling.txt").exists(),
        "rejected delete must leave sibling files untouched"
    );
}

fn tokio_test_block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(fut)
}

fn wire_err_code(err: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(err)
        .unwrap_or_else(|e| panic!("handler error is not JSON: {e}: {err}"));
    v["code"].as_str().expect("wire error code").to_string()
}
