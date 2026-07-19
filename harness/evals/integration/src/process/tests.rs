use std::ffi::{OsStr, OsString};
use std::time::Duration;

use super::*;

#[test]
fn builder_preserves_args_and_environment() {
    let spec = ProcessSpec::new("worker", "/bin/echo", "/", "/tmp/out", "/tmp/err")
        .args(["one", "two"])
        .env("LANG", "C");
    assert_eq!(spec.args, [OsStr::new("one"), OsStr::new("two")]);
    assert_eq!(spec.env.get(OsStr::new("LANG")), Some(&OsString::from("C")));
}

#[test]
fn drop_cleans_up_a_partially_started_supervisor() {
    let dir = tempfile::tempdir().unwrap();
    let mut supervisor = ProcessSupervisor::new(Duration::from_millis(100));
    let running = ProcessSpec::new(
        "running",
        "/bin/sh",
        dir.path(),
        dir.path().join("running.out"),
        dir.path().join("running.err"),
    )
    .args(["-c", "exec sleep 60"]);
    let pid = supervisor.spawn(running).unwrap();

    let invalid = ProcessSpec::new(
        "missing",
        dir.path().join("does-not-exist"),
        dir.path(),
        dir.path().join("missing.out"),
        dir.path().join("missing.err"),
    );
    assert!(supervisor.spawn(invalid).is_err());

    drop(supervisor);
    assert!(
        !process_is_running(pid),
        "first child survived a later spawn failure"
    );
}

#[tokio::test]
async fn kill_now_is_idempotent_after_a_synchronous_reap() {
    let dir = tempfile::tempdir().unwrap();
    let mut supervisor = ProcessSupervisor::new(Duration::from_millis(100));
    let running = ProcessSpec::new(
        "running",
        "/bin/sh",
        dir.path(),
        dir.path().join("running.out"),
        dir.path().join("running.err"),
    )
    .args(["-c", "exec sleep 60"]);
    let pid = supervisor.spawn(running).unwrap();
    let mut child = supervisor.remove("running").unwrap();

    child.kill_now().await.unwrap();
    assert!(!process_is_running(pid));
    child.kill_now().await.unwrap();
}

#[tokio::test]
async fn observing_a_leader_exit_does_not_abandon_its_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let descendant_pid = dir.path().join("descendant.pid");
    let mut supervisor = ProcessSupervisor::new(Duration::from_millis(100));
    let process_tree = ProcessSpec::new(
        "process-tree",
        "/bin/sh",
        dir.path(),
        dir.path().join("process-tree.out"),
        dir.path().join("process-tree.err"),
    )
    .args([
        OsString::from("-c"),
        OsString::from("sleep 60 & echo $! > \"$1\""),
        OsString::from("process-tree"),
        descendant_pid.as_os_str().to_owned(),
    ]);
    supervisor.spawn(process_tree).unwrap();
    let mut child = supervisor.remove("process-tree").unwrap();

    let descendant = wait_for_pid(&descendant_pid).await;
    let mut observed_exit = false;
    for _ in 0..50 {
        if child.try_wait().unwrap().is_some() {
            observed_exit = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(observed_exit, "process-group leader did not exit");
    drop(child);

    for _ in 0..50 {
        if !process_is_running(descendant) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("descendant {descendant} survived after the observed leader was dropped");
}

async fn wait_for_pid(path: &std::path::Path) -> u32 {
    for _ in 0..50 {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(pid) = raw.trim().parse() {
                return pid;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("descendant pid was not written");
}

fn process_is_running(pid: u32) -> bool {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(_) => return false,
    };
    // A zombie has already stopped executing and only awaits its parent
    // reading the exit status.
    stat.rsplit_once(") ")
        .and_then(|(_, rest)| rest.chars().next())
        .is_some_and(|state| state != 'Z')
}
