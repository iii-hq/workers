//! Supervisor behavior over stub child processes (spec § Verification:
//! early exit, timeout precedence via early-exit detection, signal
//! escalation, complete teardown, environment isolation).

use std::path::PathBuf;

use harness_integration::stack::{RunPaths, Stack};

fn stub_stack(dir: &tempfile::TempDir) -> (Stack, RunPaths) {
    let paths = RunPaths::allocate(dir.path(), "test-run").expect("allocate");
    (Stack::for_tests(paths.clone()), paths)
}

fn sh(args: &str) -> (PathBuf, Vec<String>) {
    (
        PathBuf::from("/bin/sh"),
        vec!["-c".to_string(), args.to_string()],
    )
}

#[tokio::test]
async fn early_exit_is_detected_and_named() {
    let dir = tempfile::tempdir().unwrap();
    let (mut stack, paths) = stub_stack(&dir);
    let (bin, args) = sh("exit 7");
    stack.spawn_child("stub", &bin, &args, &paths.root).unwrap();
    let mut seen = None;
    for _ in 0..50 {
        if let Some(exit) = stack.early_exit() {
            seen = Some(exit);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let exit = seen.expect("exit detected");
    assert_eq!(exit.name, "stub");
    assert!(exit.status.contains('7'), "status: {}", exit.status);
    stack.teardown().await;
}

#[tokio::test]
async fn teardown_terminates_a_cooperative_child_promptly() {
    let dir = tempfile::tempdir().unwrap();
    let (mut stack, paths) = stub_stack(&dir);
    let (bin, args) = sh("sleep 60");
    stack
        .spawn_child("sleeper", &bin, &args, &paths.root)
        .unwrap();
    let started = std::time::Instant::now();
    stack.teardown().await;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "SIGTERM should end a cooperative child well before the SIGKILL grace"
    );
    assert!(stack.early_exit().is_some(), "child must be reaped");
}

#[tokio::test]
async fn teardown_escalates_to_sigkill_for_a_term_ignoring_child() {
    let dir = tempfile::tempdir().unwrap();
    let (mut stack, paths) = stub_stack(&dir);
    let (bin, args) = sh("trap '' TERM; sleep 60");
    stack
        .spawn_child("stubborn", &bin, &args, &paths.root)
        .unwrap();
    // Let the shell install its trap before signalling.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let started = std::time::Instant::now();
    stack.teardown().await;
    let elapsed = started.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_secs(4),
        "must wait the SIGTERM grace before killing ({elapsed:?})"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "SIGKILL must end the child right after the grace ({elapsed:?})"
    );
}

#[tokio::test]
async fn children_see_only_the_environment_allowlist() {
    // A canary secret in the runner's environment must not reach children.
    std::env::set_var("INTEGRATION_TEST_SECRET", "leak-me");
    let dir = tempfile::tempdir().unwrap();
    let (mut stack, paths) = stub_stack(&dir);
    let (bin, args) = sh("env");
    stack
        .spawn_child("env-dump", &bin, &args, &paths.root)
        .unwrap();
    for _ in 0..50 {
        if stack.early_exit().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    stack.teardown().await;
    let dump = std::fs::read_to_string(paths.logs_dir.join("env-dump.out")).unwrap();
    assert!(
        !dump.contains("INTEGRATION_TEST_SECRET"),
        "runner env leaked into a child:\n{dump}"
    );
    for line in dump.lines().filter(|l| l.contains('=')) {
        let key = line.split('=').next().unwrap();
        assert!(
            ["PATH", "HOME", "LANG", "RUST_LOG", "PWD", "SHLVL", "_"].contains(&key),
            "unexpected env var {key} in child environment"
        );
    }
}
