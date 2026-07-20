//! Supervisor behavior over stub child processes (spec § Verification:
//! early exit, timeout precedence via early-exit detection, signal
//! escalation, complete teardown, environment isolation).

use std::path::PathBuf;
use std::time::Duration;

use harness_integration::stack::{RunPaths, Stack};

fn stub_stack(dir: &tempfile::TempDir) -> (Stack, RunPaths) {
    let paths = RunPaths::allocate(dir.path(), "test-run").expect("allocate");
    (
        Stack::for_tests_with_teardown_budget(paths.clone(), Duration::from_millis(400)),
        paths,
    )
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
        started.elapsed() < Duration::from_secs(2),
        "SIGTERM should end a cooperative child well before the SIGKILL grace"
    );
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
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = std::time::Instant::now();
    stack.teardown().await;
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(300),
        "must wait the SIGTERM grace before killing ({elapsed:?})"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "SIGKILL must end the child right after the grace ({elapsed:?})"
    );
}

#[tokio::test]
async fn teardown_kills_descendants_in_the_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let (mut stack, paths) = stub_stack(&dir);
    let pid_file = paths.root.join("descendant.pid");
    let args = vec![
        "-c".to_string(),
        "trap '' TERM; sleep 60 & echo $! > \"$1\"; wait".to_string(),
        "group-stub".to_string(),
        pid_file.to_string_lossy().into_owned(),
    ];
    stack
        .spawn_child(
            "process-tree",
            PathBuf::from("/bin/sh").as_path(),
            &args,
            &paths.root,
        )
        .unwrap();

    let descendant = wait_for_pid(&pid_file).await;
    stack.teardown().await;
    assert!(
        wait_until_not_running(descendant).await,
        "descendant {descendant} survived process-group teardown"
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

async fn wait_for_pid(path: &std::path::Path) -> u32 {
    for _ in 0..50 {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(pid) = raw.trim().parse() {
                return pid;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("numeric descendant pid was not written");
}

async fn wait_until_not_running(pid: u32) -> bool {
    for _ in 0..50 {
        if !process_is_running(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

fn process_is_running(pid: u32) -> bool {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(_) => return false,
    };
    stat.rsplit_once(") ")
        .and_then(|(_, rest)| rest.chars().next())
        .is_some_and(|state| state != 'Z')
}
