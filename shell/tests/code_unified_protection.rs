//! D4 — the unified protected-path WIRING test.
//!
//! After the coder→shell fold, secrets are declared ONCE under
//! `code.non_accessible_globs`, and the merge's promise is that the SAME
//! globs guard BOTH surfaces: the `coder::*` code surface (C211 show-but-lock)
//! AND the `shell::fs::*` host surface (S211 hard-reject, redacted like C211).
//!
//! `configuration::build_runtime` is the single place that copies
//! `code.non_accessible_globs` onto the host fs backend
//! (`non_accessible_globs: config.code.non_accessible_globs.clone()`). The
//! per-surface enforcement is unit-tested in each module, but NOTHING pinned
//! that one wiring hop — a regression turning that line into `Vec::new()`
//! would silently disable fs secret protection while every other test stayed
//! green. These tests close that hole end-to-end: a glob declared ONLY under
//! `code` must cause the host fs backend that `build_runtime` produces to
//! protect a matching in-jail path with the redacted S211.

use shell::code::config::CoderConfig;
use shell::config::{FsConfig, ShellConfig};
use shell::configuration::build_runtime;
use shell::fs::{ReadArgs, StatArgs};

/// Build a jailed runtime over `root` with the given code-surface globs.
/// A stub engine address is fine: the protection check fires in
/// `validate_path` BEFORE any channel is created.
fn runtime_for(
    root: &std::path::Path,
    code_globs: Vec<&str>,
) -> shell::configuration::ShellRuntime {
    let cfg = ShellConfig {
        fs: FsConfig {
            host_roots: vec![root.to_path_buf()],
            ..FsConfig::default()
        },
        code: CoderConfig {
            non_accessible_globs: code_globs.into_iter().map(String::from).collect(),
            ..CoderConfig::default()
        },
        ..ShellConfig::default()
    };
    let iii = iii_sdk::IIIClient::new("ws://stub-not-connected:0");
    build_runtime(&cfg, &iii).expect("build_runtime must succeed for a jailed config")
}

/// A glob declared ONLY under `code.non_accessible_globs` must reject a
/// matching `shell::fs::read` with the redacted S211 (REDACTION INVARIANT:
/// indistinguishable from missing). The protected read rejects in
/// `validate_path` (before the streaming channel), so this stays fast and
/// deterministic even with a stub engine.
#[tokio::test]
async fn code_non_accessible_globs_block_the_fs_read() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(".env"), "API_KEY=secret\n").unwrap();
    let runtime = runtime_for(tmp.path(), vec!["**/.env"]);

    let abs_env = tmp.path().join(".env").display().to_string();
    let err = runtime
        .host_backend
        .read(ReadArgs {
            path: abs_env,
            fs_scope: None,
        })
        .await
        .expect_err("a glob declared under code.non_accessible_globs must block the fs read too");
    assert_eq!(
        err.code, "S211",
        "the fs surface must reject the code-declared protected path with the redacted S211; got {err:?}"
    );
    assert!(
        err.message.contains("not found or not accessible"),
        "redacted wording required; got {err:?}"
    );
}

/// Causality control: with NO globs under `code`, the SAME in-jail path is no
/// longer protected on the fs surface. `stat` exercises the identical
/// `validate_path` gate as `read` but creates no channel, so the positive
/// case is fast and deterministic — proving the rejection above is caused
/// specifically by the propagated glob, not by some unrelated rejection.
#[tokio::test]
async fn fs_surface_not_protected_without_code_glob() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(".env"), "API_KEY=secret\n").unwrap();

    // With the glob, stat is rejected at the same gate read uses…
    let guarded = runtime_for(tmp.path(), vec!["**/.env"]);
    let abs_env = tmp.path().join(".env").display().to_string();
    let err = guarded
        .host_backend
        .stat(StatArgs {
            path: abs_env.clone(),
            fs_scope: None,
        })
        .await
        .expect_err("stat must hit the same protection gate as read");
    assert_eq!(err.code, "S211", "got {err:?}");

    // …and without it, the very same stat succeeds.
    let open = runtime_for(tmp.path(), vec![]);
    let ok = open
        .host_backend
        .stat(StatArgs {
            path: abs_env,
            fs_scope: None,
        })
        .await;
    assert!(
        ok.is_ok(),
        "without code.non_accessible_globs the path must be reachable; got {:?}",
        ok.err()
    );
}
