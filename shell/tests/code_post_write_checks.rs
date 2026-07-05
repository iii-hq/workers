//! Integration coverage for `post_write_checks` — report-only diagnostics
//! attached to coder write responses.

use std::path::PathBuf;
use std::sync::Arc;

use shell::code::config::{CoderConfig, PostWriteCheck};
use shell::code::functions::update_file::{
    handle as update_handle, UpdateFileInput, UpdateFileSpec, UpdateOp,
};
use shell::code::path::PathResolver;
use tempfile::tempdir;

fn make_with_checks(
    base: PathBuf,
    checks: Vec<PostWriteCheck>,
) -> (Arc<PathResolver>, Arc<CoderConfig>) {
    let cfg = Arc::new(CoderConfig {
        base_paths: vec![base],
        post_write_checks: checks,
        ..CoderConfig::default()
    });
    let r = Arc::new(PathResolver::new(&cfg).unwrap());
    (r, cfg)
}

fn check(glob: &str, command: &str) -> PostWriteCheck {
    PostWriteCheck {
        match_glob: glob.into(),
        command: command.into(),
        timeout_ms: 10_000,
    }
}

fn str_replace_input(path: &str, old: &str, new: &str) -> UpdateFileInput {
    UpdateFileInput {
        files: vec![UpdateFileSpec {
            path: path.into(),
            ops: vec![UpdateOp::StrReplace {
                old_str: old.into(),
                new_str: new.into(),
                replace_all: false,
            }],
        }],
        fs_scope: None,
    }
}

#[tokio::test]
async fn matching_check_runs_and_reports_failure_without_failing_the_edit() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("app.py"), "x = (1\n").unwrap();
    let (r, c) = make_with_checks(
        tmp.path().to_path_buf(),
        vec![check("**/*.py", "python3 -m py_compile app.py")],
    );

    // The edit keeps the file syntactically broken — the check must flag
    // it while the edit itself still succeeds.
    let out = update_handle(r, c, str_replace_input("app.py", "x = (1", "y = (2"))
        .await
        .unwrap();
    assert!(out.results[0].success, "edit succeeds regardless of checks");
    assert_eq!(out.checks.len(), 1);
    let outcome = &out.checks[0];
    assert_ne!(outcome.exit_code, Some(0), "py_compile must fail");
    assert!(
        outcome.output.contains("SyntaxError") || !outcome.output.is_empty(),
        "check output is surfaced: {outcome:?}"
    );
}

#[tokio::test]
async fn non_matching_glob_runs_nothing() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("notes.txt"), "hello\n").unwrap();
    let (r, c) = make_with_checks(
        tmp.path().to_path_buf(),
        vec![check("**/*.py", "echo should-not-run")],
    );
    let out = update_handle(r, c, str_replace_input("notes.txt", "hello", "hi"))
        .await
        .unwrap();
    assert!(out.results[0].success);
    assert!(out.checks.is_empty());
}

#[tokio::test]
async fn duplicate_commands_run_once_and_output_is_bounded() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("a.py"), "a = 1\n").unwrap();
    let (r, c) = make_with_checks(
        tmp.path().to_path_buf(),
        vec![
            // Same command via two globs — must run once.
            check("**/*.py", "yes x | head -c 10000"),
            check("a.*", "yes x | head -c 10000"),
        ],
    );
    let out = update_handle(r, c, str_replace_input("a.py", "a = 1", "a = 2"))
        .await
        .unwrap();
    assert_eq!(out.checks.len(), 1, "deduplicated by command");
    assert!(out.checks[0].truncated);
    assert!(out.checks[0].output.len() <= 4 * 1024);
}

#[tokio::test]
async fn failed_file_does_not_trigger_checks() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("a.py"), "a = 1\n").unwrap();
    let (r, c) = make_with_checks(tmp.path().to_path_buf(), vec![check("**/*.py", "echo ran")]);
    // Ambiguity failure: nothing written, so no check runs.
    let out = update_handle(r, c, str_replace_input("a.py", "does-not-exist", "x"))
        .await
        .unwrap();
    assert!(!out.results[0].success);
    assert!(out.checks.is_empty());
}

#[tokio::test]
async fn timeout_is_reported_not_fatal() {
    let tmp = tempdir().unwrap();
    std::fs::write(tmp.path().join("a.py"), "a = 1\n").unwrap();
    let (r, c) = make_with_checks(
        tmp.path().to_path_buf(),
        vec![PostWriteCheck {
            match_glob: "**/*.py".into(),
            command: "sleep 5".into(),
            timeout_ms: 200,
        }],
    );
    let out = update_handle(r, c, str_replace_input("a.py", "a = 1", "a = 3"))
        .await
        .unwrap();
    assert!(out.results[0].success);
    assert_eq!(out.checks.len(), 1);
    assert!(out.checks[0]
        .error
        .as_deref()
        .unwrap()
        .contains("timed out"));
}
