// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN. Imports the actual inherited OFD through an isolated context;
//! positive cross-exec/_exit coverage needs the coordinator-owned test dispatcher.
use super::*;
use crate::worker_admission::{MutationPlan, PathRequest, testing, with_paths};
use std::os::unix::fs::PermissionsExt;
/// Umask-independent fixture objects: the core refuses group/other-writable
/// footprints, runtime roots, control parents and staged content by design,
/// so every created fixture gets the exact mode its scenario intends.
fn mkdir_mode(path: impl AsRef<Path>, mode: u32) {
    std::fs::create_dir(path.as_ref()).unwrap();
    std::fs::set_permissions(path.as_ref(), std::fs::Permissions::from_mode(mode)).unwrap();
}
fn write_mode(path: impl AsRef<Path>, bytes: &[u8], mode: u32) {
    std::fs::write(path.as_ref(), bytes).unwrap();
    std::fs::set_permissions(path.as_ref(), std::fs::Permissions::from_mode(mode)).unwrap();
}
fn fixture() -> (
    tempfile::TempDir,
    crate::worker_admission::AdmissionContext,
    std::path::PathBuf,
) {
    let temp = tempfile::tempdir().unwrap();
    let catalog = temp.path().join("catalog");
    let data = temp.path().join("data");
    for p in [&catalog, &data] {
        std::fs::create_dir(p).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let context = testing::provision_context(&catalog).unwrap();
    (temp, context, data)
}
#[test]
fn runtime_imported_writer_allows_bytes_but_not_root_replacement() {
    let (_temp, context, data) = fixture();
    let upper = data.join("upper");
    write_mode(&upper, b"OLD", 0o600);
    let lease = with_paths(
        context.local(&data.join("state")).unwrap(),
        &[PathRequest {
            path: &upper,
            role: PathRole::MutableFile,
        }],
    )
    .unwrap();
    let launch = lease
        .prepare_runtime(&[RuntimePath {
            path: &upper,
            kind: RuntimePathKind::UpperImage,
        }])
        .unwrap();
    let exported = lease.export_child().unwrap();
    let imported = context
        .import_child(exported.duplicate_for_exec().unwrap())
        .unwrap();
    let writer = import_runtime_writer(
        &imported,
        duplicate(launch.sentinel.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    std::fs::write(&upper, b"NEW").unwrap(); // in place: the admitted inode keeps 0600
    writer.checkpoint().unwrap();
    std::fs::rename(&upper, data.join("retained")).unwrap();
    write_mode(&upper, b"FOREIGN", 0o600);
    assert!(writer.checkpoint().is_err());
    std::fs::remove_file(&upper).unwrap();
    std::fs::rename(data.join("retained"), &upper).unwrap();
    assert!(writer.checkpoint().is_err());
}
#[test]
fn runtime_guest_tree_descendants_change_and_activity_opens_exact_registered_file() {
    let (_temp, context, data) = fixture();
    let tree = data.join("tree");
    mkdir_mode(&tree, 0o700);
    let activity = data.join("activity");
    let lease = with_paths(
        context.local(&tree).unwrap(),
        &[PathRequest {
            path: &activity,
            role: PathRole::MutableFile,
        }],
    )
    .unwrap();
    let launch = lease
        .prepare_runtime(&[
            RuntimePath {
                path: &tree,
                kind: RuntimePathKind::GuestTree,
            },
            RuntimePath {
                path: &activity,
                kind: RuntimePathKind::ActivityStamp,
            },
        ])
        .unwrap();
    let exported = lease.export_child().unwrap();
    let imported = context
        .import_child(exported.duplicate_for_exec().unwrap())
        .unwrap();
    let writer = import_runtime_writer(
        &imported,
        duplicate(launch.sentinel.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    mkdir_mode(tree.join("child"), 0o700);
    write_mode(tree.join("child/data"), b"NEW", 0o600);
    writer.checkpoint().unwrap();
    let approved = writer.activity(&activity).unwrap();
    let before = std::fs::metadata(&activity).unwrap().ino();
    approved.touch_at(SystemTime::now()).unwrap();
    assert_eq!(std::fs::metadata(&activity).unwrap().ino(), before);
    assert!(writer.activity(&tree.join("child/data")).is_err());
    assert!(writer.checkpoint().is_err());
}
#[tokio::test]
async fn runtime_real_owned_wait_without_child_activation_refuses_retirement() {
    let (_temp, context, data) = fixture();
    let tree = data.join("tree");
    mkdir_mode(&tree, 0o700);
    let lease = context.local(&tree).unwrap();
    let launch = lease
        .prepare_runtime(&[RuntimePath {
            path: &tree,
            kind: RuntimePathKind::GuestTree,
        }])
        .unwrap();
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg("-c").arg("exit 0");
    let child = launch.spawn(command).unwrap();
    let completion = child.wait().await;
    assert!(completion.os_status().is_some());
    assert!(completion.retirement().is_err());
    assert!(lease.begin(MutationPlan::Create { root: tree }).is_err());
}

fn activity_fixture() -> (
    tempfile::TempDir,
    crate::worker_admission::AdmissionContext,
    RuntimeLaunch,
    RuntimeWriter,
    PathBuf,
) {
    let (temp, context, data) = fixture();
    let path = data.join("activity");
    let lease = with_paths(
        context.local(&data.join("state")).unwrap(),
        &[PathRequest {
            path: &path,
            role: PathRole::MutableFile,
        }],
    )
    .unwrap();
    let launch = lease
        .prepare_runtime(&[RuntimePath {
            path: &path,
            kind: RuntimePathKind::ActivityStamp,
        }])
        .unwrap();
    let child = lease.export_child().unwrap();
    let imported = context
        .import_child(child.duplicate_for_exec().unwrap())
        .unwrap();
    let writer = import_runtime_writer(
        &imported,
        duplicate(launch.sentinel.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    (temp, context, launch, writer, path)
}
#[test]
fn e_activity_success_is_bounded_and_never_changes_payload_bytes() {
    let (_temp, context, _launch, writer, path) = activity_fixture();
    write_mode(&path, b"payload", 0o600);
    let activity = writer.activity(&path).unwrap();
    for n in 1..=12 {
        activity
            .touch_at(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(n))
            .unwrap();
    }
    let tx = context.namespace.transaction().unwrap();
    let record = tx
        .catalog
        .runtimes
        .iter()
        .find(|r| r.id == writer.0.id)
        .unwrap();
    assert!(activity_ready(record));
    assert_eq!(record.activity.sequence, 12);
    assert_eq!(
        tx.operation(&writer.0.lease.0.id).unwrap().journals.len(),
        0
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"payload");
}
#[test]
fn e_activity_syscall_and_ledger_errors_latch_across_clones_and_block_clean_wait_gate() {
    for fault in [
        ActivityFault::Pending,
        ActivityFault::Syscall,
        ActivityFault::Outcome,
        ActivityFault::FailurePersistence,
    ] {
        let (_temp, context, _launch, writer, path) = activity_fixture();
        let activity = writer.activity(&path).unwrap();
        let clone = activity.clone();
        writer.0.activity.lock().unwrap().fault = Some(fault);
        assert!(activity.touch_at(SystemTime::UNIX_EPOCH).is_err());
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        writer.0.activity.lock().unwrap().fault = None;
        assert!(clone.touch_at(SystemTime::now()).is_err());
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), first);
        let tx = context.namespace.transaction().unwrap();
        let record = tx
            .catalog
            .runtimes
            .iter()
            .find(|r| r.id == writer.0.id)
            .unwrap();
        assert!(!activity_ready(record));
        if matches!(fault, ActivityFault::FailurePersistence) {
            assert_eq!(record.activity.phase, ActivityPhase::Pending);
        } else {
            assert_eq!(record.activity.phase, ActivityPhase::Failed);
        }
        drop(tx);
        assert!(writer.checkpoint().is_err());
    }
}
#[test]
fn e_activity_pending_crash_gap_cannot_be_overwritten_by_a_new_touch() {
    let (_temp, context, _launch, writer, path) = activity_fixture();
    let activity = writer.activity(&path).unwrap();
    {
        let mut tx = context.namespace.transaction().unwrap();
        let record = tx
            .catalog
            .runtimes
            .iter_mut()
            .find(|r| r.id == writer.0.id)
            .unwrap();
        record.activity = ActivityOutcome {
            sequence: 1,
            path: Some(HostPath::new(&path).unwrap()),
            phase: ActivityPhase::Pending,
            failure: None,
        };
        tx.save().unwrap();
    }
    assert!(activity.touch_at(SystemTime::now()).is_err());
    assert!(writer.checkpoint().is_err());
}
#[test]
fn e_activity_clones_serialize_without_catalog_mutex_during_effect() {
    let (_temp, context, _launch, writer, path) = activity_fixture();
    let activity = writer.activity(&path).unwrap();
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let a = activity.clone();
        tasks.push(std::thread::spawn(move || a.touch_at(SystemTime::now())));
    }
    for task in tasks {
        task.join().unwrap().unwrap();
    }
    let tx = context.namespace.transaction().unwrap();
    let record = tx
        .catalog
        .runtimes
        .iter()
        .find(|r| r.id == writer.0.id)
        .unwrap();
    assert_eq!(record.activity.sequence, 4);
    assert!(activity_ready(record));
}
#[test]
fn e_activity_path_replacement_never_receives_an_mtime_write() {
    let (_temp, _context, _launch, writer, path) = activity_fixture();
    let activity = writer.activity(&path).unwrap();
    std::fs::rename(&path, path.with_extension("retained")).unwrap();
    write_mode(&path, b"foreign", 0o600);
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert!(activity.touch_at(SystemTime::UNIX_EPOCH).is_err());
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );
    assert_eq!(std::fs::read(path).unwrap(), b"foreign");
}
#[test]
fn e_control_files_publish_beside_active_guest_tree_without_parent_authority() {
    let (_temp, context, data) = fixture();
    let tree = data.join("root");
    mkdir_mode(&tree, 0o700);
    let log = data.join("console.log");
    let rotated = data.join("console.log.1");
    write_mode(&log, b"old-log", 0o600);
    let lease = with_paths(
        context.local(&tree).unwrap(),
        &[
            PathRequest {
                path: &log,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &rotated,
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap();
    let launch = lease
        .prepare_runtime(&[RuntimePath {
            path: &tree,
            kind: RuntimePathKind::GuestTree,
        }])
        .unwrap();
    let child = lease.export_child().unwrap();
    let imported = context
        .import_child(child.duplicate_for_exec().unwrap())
        .unwrap();
    let writer = import_runtime_writer(
        &imported,
        duplicate(launch.sentinel.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    let mut control = lease
        .begin(MutationPlan::ControlFiles {
            files: vec![log.clone(), rotated.clone()],
        })
        .unwrap();
    let mut stage = control.stage_file(&rotated, 0o600).unwrap();
    use std::io::Write;
    stage.file_mut().write_all(b"old-log").unwrap();
    control.publish_file(stage).unwrap();
    let mut stage = control.stage_file(&log, 0o600).unwrap();
    stage.file_mut().write_all(b"new-log").unwrap();
    control.publish_file(stage).unwrap();
    control.commit().unwrap();
    writer.checkpoint().unwrap();
    assert_eq!(std::fs::read(&log).unwrap(), b"new-log");
}

#[test]
#[cfg(target_os = "linux")]
fn e_activity_actual_set_modified_error_not_only_checkpoint_refusal_is_terminal() {
    let (_temp, context, _launch, writer, path) = activity_fixture();
    let parent = Directory::open(path.parent().unwrap()).unwrap();
    // Test-only construction keeps a real inode FD of the wrong write capability.
    // metadata/identity precheck succeeds; File::set_modified itself must fail.
    let file = open_at(&parent.file, path.file_name().unwrap(), libc::O_PATH, 0).unwrap();
    let identity = Identity::of(&file).unwrap();
    let activity = RuntimeActivity(Arc::new(ActivityHeld {
        writer: writer.clone(),
        file,
        parent,
        path,
        identity,
    }));
    let error = activity.touch_at(SystemTime::now()).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::EBADF));
    let tx = context.namespace.transaction().unwrap();
    let record = tx
        .catalog
        .runtimes
        .iter()
        .find(|r| r.id == writer.0.id)
        .unwrap();
    assert_eq!(record.activity.failure, Some(ActivityFailure::Syscall));
    assert_eq!(record.activity.phase, ActivityPhase::Failed);
    assert!(!activity_ready(record));
}

/// The holder always runs outside the task invoking a blocking transaction.
/// Drop releases on assertion unwinding too; no watchdog ever signals a PID.
struct R5CatalogHolder {
    release: Option<std::sync::mpsc::Sender<()>>,
    joined: Option<std::thread::JoinHandle<()>>,
}
impl R5CatalogHolder {
    fn new(file: File) -> Self {
        let (held, ready) = std::sync::mpsc::channel();
        let (release, requested) = std::sync::mpsc::channel();
        let joined = std::thread::spawn(move || {
            flock(&file).unwrap();
            held.send(()).unwrap();
            // Independent release, bounded even if the controlling assertion fails.
            let _release = requested.recv_timeout(std::time::Duration::from_secs(5));
            drop(file);
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        Self {
            release: Some(release),
            joined: Some(joined),
        }
    }
    fn finish(mut self) {
        self.release_and_join();
    }
    fn release_and_join(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(joined) = self.joined.take() {
            joined.join().unwrap();
        }
    }
}
impl Drop for R5CatalogHolder {
    fn drop(&mut self) {
        self.release_and_join();
    }
}

#[test]
fn r5_catalog_contention_serializes_activity_without_poisoning_its_writer() {
    let (_temp, context, launch, writer, path) = activity_fixture();
    let activity = writer.activity(&path).unwrap();
    let lock = context
        .namespace
        .directory
        .file("catalog.lock", false)
        .unwrap();
    let contender = context
        .namespace
        .directory
        .file("catalog.lock", false)
        .unwrap();
    let holder = R5CatalogHolder::new(lock);
    let (attempted, seen) = std::sync::mpsc::channel();
    let (finished, result) = std::sync::mpsc::channel();
    let work = std::thread::spawn(move || {
        assert_eq!(
            flock(&contender).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        attempted.send(()).unwrap();
        let outcome =
            activity.touch_at(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(42));
        finished.send(outcome).unwrap();
    });
    seen.recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(matches!(
        result.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    holder.finish();
    result
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    work.join().unwrap();
    assert!(!failed_ofd(launch.sentinel.as_ref().unwrap()).unwrap());
    writer.checkpoint().unwrap();
    let tx = context.namespace.transaction().unwrap();
    let record = tx
        .catalog
        .runtimes
        .iter()
        .find(|r| r.id == writer.0.id)
        .unwrap();
    assert_eq!(record.activity.phase, ActivityPhase::Clean);
    assert_eq!(record.activity.sequence, 1);
}

#[test]
fn e_activity_failure_marks_parent_shared_description_even_when_catalog_is_busy() {
    let (_temp, context, launch, writer, path) = activity_fixture();
    let activity = writer.activity(&path).unwrap();
    let retry = activity.clone();
    // Busy itself is not the error. This existing hook fails actual activity
    // I/O AND failure persistence after durable Pending was recorded.
    writer.0.activity.lock().unwrap().fault = Some(ActivityFault::FailurePersistence);
    let lock = context
        .namespace
        .directory
        .file("catalog.lock", false)
        .unwrap();
    let contender = context
        .namespace
        .directory
        .file("catalog.lock", false)
        .unwrap();
    let holder = R5CatalogHolder::new(lock);
    let (attempted, seen) = std::sync::mpsc::channel();
    let (finished, result) = std::sync::mpsc::channel();
    let work = std::thread::spawn(move || {
        assert_eq!(
            flock(&contender).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        attempted.send(()).unwrap();
        finished.send(activity.touch_at(SystemTime::now())).unwrap();
    });
    seen.recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(matches!(
        result.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    holder.finish();
    let error = result
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap_err();
    work.join().unwrap();
    assert_eq!(error.raw_os_error(), Some(libc::EIO));
    assert!(failed_ofd(launch.sentinel.as_ref().unwrap()).unwrap());
    let first_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    writer.0.activity.lock().unwrap().fault = None;
    assert!(retry.touch_at(SystemTime::now()).is_err());
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        first_mtime
    );
    let tx = context.namespace.transaction().unwrap();
    let record = tx
        .catalog
        .runtimes
        .iter()
        .find(|r| r.id == writer.0.id)
        .unwrap();
    assert_eq!(record.activity.phase, ActivityPhase::Pending);
    assert!(!activity_ready(record));
    drop(tx);
    assert!(writer.checkpoint().is_err());
    // The parent-shared OFD independently forbids a clean RuntimeChild::wait,
    // even though persisting a Failed phase was deliberately unsuccessful.
}

fn simple_command(script: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg("-c").arg(script);
    command
}
#[tokio::test]
async fn f_reap_is_published_before_catalog_failure_and_signals_never_retarget_pid() {
    let (_temp, context, data) = fixture();
    let lease = context.local(&data.join("state")).unwrap();
    let launch = lease.prepare_runtime(&[]).unwrap();
    let child = launch.spawn(simple_command("exit 7")).unwrap();
    let control = child.control();
    let lock = context
        .namespace
        .directory
        .file("catalog.lock", false)
        .unwrap();
    let catalog = context
        .namespace
        .directory
        .file("catalog.json", false)
        .unwrap();
    let observed_control = control.clone();
    let (held, ready) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        use std::os::unix::fs::FileExt;
        flock(&lock).unwrap();
        held.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // Observation/signal consult the core's in-memory reaped state and pidfd,
        // never the blocked catalog. The current-thread Tokio waiter may block
        // on flock; this independent thread can still prove reap and release it.
        loop {
            let observation = observed_control.observation();
            if observation.os_status().is_some() {
                assert_eq!(observation.phase(), RuntimePhase::Reaped);
                assert_eq!(observation.os_status().unwrap().code(), Some(7));
                assert!(
                    observed_control
                        .signal(super::super::RuntimeSignal::Kill)
                        .is_err()
                );
                // Real distinct failure under our fixture-owned lock: invalid
                // durable record, NOT ordinary lock contention treated as failure.
                catalog.write_all_at(b"{", 0).unwrap();
                catalog.set_len(1).unwrap();
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "owned child did not publish reap"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        drop(lock);
    });
    ready
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let completion = child.wait().await;
    holder.join().unwrap();
    assert_eq!(completion.os_status().unwrap().code(), Some(7));
    let failure = match completion.retirement() {
        Ok(_) => panic!("corrupt catalog must not issue retirement authority"),
        Err(failure) => failure.to_io_error(),
    };
    assert!(failure.to_string().contains("record envelope"), "{failure}");
    let observed = control.terminal().await;
    assert_eq!(observed.phase(), RuntimePhase::RetirementFailed);
    assert_eq!(observed.os_status().unwrap().code(), Some(7));
    assert!(control.signal(super::super::RuntimeSignal::Kill).is_err());
    // Keep corrupt catalog evidence for the test to inspect; no forged clean state.
    assert_eq!(
        std::fs::read(context.namespace.directory.path.join("catalog.json")).unwrap(),
        b"{"
    );
}
#[tokio::test]
async fn f_signal_reap_race_uses_original_pidfd_only() {
    let (_temp, context, data) = fixture();
    let lease = context.local(&data.join("state")).unwrap();
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exec sleep 2"))
        .unwrap();
    let control = child.control();
    let signal = control.clone();
    let thread = std::thread::spawn(move || signal.signal(super::super::RuntimeSignal::Terminate));
    let completion = child.wait().await;
    let sent = thread.join().unwrap();
    assert!(
        sent.is_ok()
            || sent
                .as_ref()
                .err()
                .is_some_and(|e| e.raw_os_error() == Some(libc::ESRCH))
    );
    assert!(completion.os_status().is_some());
    assert!(control.signal(super::super::RuntimeSignal::Kill).is_err());
    assert!(control.observation().retirement().is_none());
}
#[tokio::test]
async fn f_wait_error_is_not_reap_and_dropped_waiter_is_terminal_not_hung() {
    let (_temp, context, data) = fixture();
    let lease = context.local(&data.join("state")).unwrap();
    let mut child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    let control = child.control();
    child.wait_failure = true;
    let completion = child.wait().await;
    assert!(completion.os_status().is_none());
    assert!(completion.retirement().is_err());
    let observed = control.terminal().await;
    assert_eq!(observed.phase(), RuntimePhase::WaitFailed);
    let candidate = lease.prepare_runtime(&[]).unwrap();
    assert!(candidate.cancel_before_spawn().is_err());
}
#[tokio::test]
async fn f_cancelled_waiter_retains_fences_but_not_an_unending_terminal_notifier() {
    let (_temp, context, data) = fixture();
    let lease = context.local(&data.join("state")).unwrap();
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    let control = child.control();
    drop(child);
    let observed = control.terminal().await;
    assert_eq!(observed.phase(), RuntimePhase::Abandoned);
    assert!(observed.retirement().is_none());
}
#[tokio::test]
async fn f_pidfd_receive_failure_returns_the_real_owned_child_for_supervision() {
    let (_temp, context, data) = fixture();
    let lease = context.local(&data.join("state")).unwrap();
    let mut launch = lease.prepare_runtime(&[]).unwrap();
    launch.pidfd_receive_failure = true;
    let result = launch.spawn(simple_command("exit 3"));
    let error = match result {
        Err(e) => e,
        Ok(_) => panic!("fault must retain Child in error"),
    };
    let (cause, child) = error.into_parts();
    assert!(cause.to_string().contains("pidfd"));
    let child = child.expect("post-spawn acquisition failure must retain supervision");
    let control = child.control();
    assert!(control.signal(super::super::RuntimeSignal::Kill).is_err());
    let completion = child.wait().await;
    assert_eq!(completion.os_status().unwrap().code(), Some(3));
    assert!(completion.retirement().is_err());
}
#[tokio::test]
async fn f_spawn_error_never_issues_never_spawned_cleanup_receipt() {
    let (_temp, context, data) = fixture();
    let lease = context.local(&data.join("state")).unwrap();
    let launch = lease.prepare_runtime(&[]).unwrap();
    let command = tokio::process::Command::new(data.join("missing-executable"));
    let error = match launch.spawn(command) {
        Err(e) => e,
        Ok(_) => panic!("missing executable"),
    };
    assert!(error.into_parts().1.is_none());
    let tx = context.namespace.transaction().unwrap();
    assert!(
        tx.catalog
            .runtimes
            .iter()
            .any(|r| r.state == RuntimeState::Uncertain)
    );
}

fn activate_for_wait_fixture(child: &RuntimeChild) {
    // Explicit own-context fixture activates the actual runtime OFD in-process;
    // not evidence of the still-pending production cross-exec import journey.
    let exported = child.lease.export_child().unwrap();
    let context = crate::worker_admission::AdmissionContext {
        namespace: child.lease.0.namespace.clone(),
    };
    let imported = context
        .import_child(exported.duplicate_for_exec().unwrap())
        .unwrap();
    let writer = import_runtime_writer(
        &imported,
        duplicate(child.sentinel.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    drop(writer);
    drop(imported);
    drop(exported);
}
#[tokio::test]
async fn f_real_wait_with_free_sentinel_issues_only_same_layout_one_shot_receipt() {
    let (_temp, context, data) = fixture();
    let root = data.join("vm");
    mkdir_mode(&root, 0o700);
    let lease = context.local(&root).unwrap();
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    let control = child.control();
    activate_for_wait_fixture(&child);
    let completion = child.wait().await;
    assert!(completion.os_status().unwrap().success());
    let receipt = completion.retirement().unwrap().clone();
    assert_eq!(control.terminal().await.phase(), RuntimePhase::Retired);
    assert!(control.signal(super::super::RuntimeSignal::Kill).is_err());
    let other = context.local(&data.join("other")).unwrap();
    assert!(receipt.begin_remove(&other, &root, &[&root]).is_err());
    let mut remove = receipt.begin_remove(&lease, &root, &[&root]).unwrap();
    remove.remove_path(&root).unwrap();
    remove.commit().unwrap();
    assert!(!root.exists());
    assert!(receipt.begin_remove(&lease, &root, &[&root]).is_err());
}
#[tokio::test]
async fn f_real_wait_descendant_description_blocks_receipt_after_os_reap() {
    let (_temp, context, data) = fixture();
    let lease = context.local(&data.join("vm")).unwrap();
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    activate_for_wait_fixture(&child);
    let descendant = duplicate(child.sentinel.as_ref().unwrap()).unwrap();
    let control = child.control();
    let completion = child.wait().await;
    assert!(completion.os_status().is_some());
    assert!(completion.retirement().is_err());
    assert_eq!(
        control.observation().phase(),
        RuntimePhase::RetirementFailed
    );
    drop(descendant);
    assert!(control.observation().retirement().is_none());
}

async fn lineage_retired(lease: &Lease) -> WriterRetirement {
    // Real Child.wait and runtime-OFD checks; activation uses the existing explicit
    // own-context import fixture, not a production authority constructor.
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    activate_for_wait_fixture(&child);
    let completion = child.wait().await;
    assert!(completion.os_status().unwrap().success());
    completion.retirement().unwrap().clone()
}
fn publish_control(lease: &Lease, path: &Path, bytes: &[u8]) {
    use std::io::Write;
    let mut mutation = lease
        .begin(MutationPlan::ControlFiles {
            files: vec![path.to_path_buf()],
        })
        .unwrap();
    let mut stage = mutation.stage_file(path, 0o600).unwrap();
    stage.file_mut().write_all(bytes).unwrap();
    mutation.publish_file(stage).unwrap();
    mutation.commit().unwrap();
}
#[tokio::test]
async fn f_lineage_real_wait_then_final_console_rotation_and_multi_step_remove() {
    let (_temp, context, data) = fixture();
    let vm = data.join("vm");
    mkdir_mode(&vm, 0o700);
    let log = vm.join("console.log");
    let rotated = vm.join("console.log.1");
    write_mode(&log, b"running", 0o600);
    let lease = with_paths(
        context.local(&vm).unwrap(),
        &[
            PathRequest {
                path: &log,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &rotated,
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap();
    let receipt = lineage_retired(&lease).await;
    publish_control(&lease, &log, b"drained-1");
    publish_control(&lease, &rotated, b"drained-1");
    publish_control(&lease, &log, b"drained-2");
    lease
        .begin(MutationPlan::ControlFiles {
            files: vec![log.clone()],
        })
        .unwrap()
        .commit()
        .unwrap(); // no-effect journal
    let mut remove = receipt
        .begin_remove(&lease, &vm, &[&log, &rotated])
        .unwrap();
    remove.remove_path(&log).unwrap();
    remove.remove_path(&rotated).unwrap();
    remove.commit().unwrap();
    assert!(!log.exists() && !rotated.exists());
    assert!(vm.exists());
    // Preserved prior files remain; Remove never infers authority to delete them.
    assert!(
        std::fs::read_dir(&vm)
            .unwrap()
            .map(Result::unwrap)
            .any(|e| e.file_name().to_string_lossy().starts_with(".iii-prior-"))
    );
    assert!(receipt.begin_remove(&lease, &vm, &[&log]).is_err());
}
#[tokio::test]
async fn f_lineage_real_wait_then_retain_adopt_removes_runtime_only() {
    let (_temp, context, data) = fixture();
    let vm = data.join("vm");
    mkdir_mode(&vm, 0o700);
    let root = vm.join("root");
    mkdir_mode(&root, 0o700);
    let upper = root.join("upper.ext4");
    write_mode(&upper, b"GUEST DATA", 0o600);
    write_mode(root.join("guest-file"), b"KEEP", 0o600);
    let kept_upper = vm.join("upper.pre-build.ext4");
    let kept_root = vm.join("root.pre-build");
    let log = vm.join("console.log");
    write_mode(&log, b"log", 0o600);
    let lease = with_paths(
        context.local(&vm).unwrap(),
        &[
            PathRequest {
                path: &root,
                role: PathRole::MutableTree,
            },
            PathRequest {
                path: &upper,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &kept_upper,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &kept_root,
                role: PathRole::MutableTree,
            },
            PathRequest {
                path: &log,
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap();
    let receipt = lineage_retired(&lease).await;
    let mut retain = lease
        .begin(MutationPlan::Adopt { root: vm.clone() })
        .unwrap();
    retain.rename_preserving(&upper, &kept_upper).unwrap();
    retain.rename_preserving(&root, &kept_root).unwrap();
    retain.commit().unwrap();
    publish_control(&lease, &log, b"final-control");
    let mut remove = receipt.begin_remove(&lease, &vm, &[&root, &log]).unwrap();
    remove.remove_path(&root).unwrap();
    remove.remove_path(&log).unwrap();
    remove.commit().unwrap();
    assert_eq!(std::fs::read(&kept_upper).unwrap(), b"GUEST DATA");
    assert_eq!(
        std::fs::read(kept_root.join("guest-file")).unwrap(),
        b"KEEP"
    );
    assert!(!log.exists());
}
#[tokio::test]
async fn f_lineage_external_replacement_after_owned_control_is_not_blessed() {
    let (_temp, context, data) = fixture();
    let log = data.join("console.log");
    write_mode(&log, b"OLD", 0o600);
    let lease = with_paths(
        context.local(&data.join("state")).unwrap(),
        &[PathRequest {
            path: &log,
            role: PathRole::MutableFile,
        }],
    )
    .unwrap();
    let receipt = lineage_retired(&lease).await;
    publish_control(&lease, &log, b"OWNED");
    std::fs::rename(&log, data.join("retained-owned")).unwrap();
    write_mode(&log, b"FOREIGN", 0o600);
    assert!(receipt.begin_remove(&lease, &data, &[&log]).is_err());
    assert_eq!(std::fs::read(&log).unwrap(), b"FOREIGN");
}
#[tokio::test]
async fn f_lineage_incomplete_control_consumes_receipt_without_removal() {
    let (_temp, context, data) = fixture();
    let log = data.join("log");
    write_mode(&log, b"OLD", 0o600);
    let lease = with_paths(
        context.local(&data.join("state")).unwrap(),
        &[PathRequest {
            path: &log,
            role: PathRole::MutableFile,
        }],
    )
    .unwrap();
    let receipt = lineage_retired(&lease).await;
    let mut control = lease
        .begin(MutationPlan::ControlFiles {
            files: vec![log.clone()],
        })
        .unwrap();
    let stage = control.stage_file(&log, 0o600).unwrap();
    control.publish_file(stage).unwrap();
    assert!(receipt.begin_remove(&lease, &data, &[&log]).is_err());
    control.commit().unwrap();
    assert!(receipt.begin_remove(&lease, &data, &[&log]).is_err());
    assert!(log.exists());
}
#[tokio::test]
async fn f_lineage_newly_declared_target_cannot_extend_the_original_receipt() {
    let (_temp, context, data) = fixture();
    let vm = data.join("vm");
    mkdir_mode(&vm, 0o700);
    let lease = context.local(&vm).unwrap();
    let receipt = lineage_retired(&lease).await;
    let added = vm.join("later");
    let _same = with_paths(
        lease.clone(),
        &[PathRequest {
            path: &added,
            role: PathRole::MutableFile,
        }],
    )
    .unwrap();
    publish_control(&lease, &added, b"KEEP");
    assert!(receipt.begin_remove(&lease, &vm, &[&added]).is_err());
    assert_eq!(std::fs::read(&added).unwrap(), b"KEEP");
}
#[tokio::test]
async fn f_lineage_old_new_corruption_and_epoch_reordering_refuse() {
    for reverse in [false, true] {
        let (_temp, context, data) = fixture();
        let log = data.join("log");
        write_mode(&log, b"OLD", 0o600);
        let lease = with_paths(
            context.local(&data.join("state")).unwrap(),
            &[PathRequest {
                path: &log,
                role: PathRole::MutableFile,
            }],
        )
        .unwrap();
        let receipt = lineage_retired(&lease).await;
        publish_control(&lease, &log, b"ONE");
        publish_control(&lease, &log, b"TWO");
        {
            let mut tx = context.namespace.transaction().unwrap();
            let op = tx.operation_mut(&lease.0.id).unwrap();
            if reverse {
                op.journals.reverse();
            } else {
                let intent = op.journals[0]
                    .intents
                    .iter_mut()
                    .find(|i| i.kind == super::super::catalog::EffectKind::Publish)
                    .unwrap();
                intent.old.as_mut().unwrap().ino += 1;
            }
            tx.save().unwrap();
        }
        assert!(receipt.begin_remove(&lease, &data, &[&log]).is_err());
        assert_eq!(std::fs::read(&log).unwrap(), b"TWO");
    }
}

#[tokio::test]
async fn f_lineage_console_guard_open_at_wait_completes_multiple_owned_effects() {
    use std::io::Write;
    let (_temp, context, data) = fixture();
    let log = data.join("console.log");
    write_mode(&log, b"OLD", 0o600);
    let lease = with_paths(
        context.local(&data.join("state")).unwrap(),
        &[PathRequest {
            path: &log,
            role: PathRole::MutableFile,
        }],
    )
    .unwrap();
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    activate_for_wait_fixture(&child);
    let mut control = lease
        .begin(MutationPlan::ControlFiles {
            files: vec![log.clone()],
        })
        .unwrap();
    let mut first = control.stage_file(&log, 0o600).unwrap();
    first.file_mut().write_all(b"ONE").unwrap();
    let completion = child.wait().await;
    let receipt = completion.retirement().unwrap().clone();
    control.publish_file(first).unwrap();
    let mut second = control.stage_file(&log, 0o600).unwrap();
    second.file_mut().write_all(b"TWO").unwrap();
    control.publish_file(second).unwrap();
    control.commit().unwrap();
    let mut remove = receipt.begin_remove(&lease, &data, &[&log]).unwrap();
    remove.remove_path(&log).unwrap();
    remove.commit().unwrap();
    assert!(!log.exists());
}

#[tokio::test]
async fn f_lineage_endpoint_close_does_not_invalidate_disjoint_cleanup_receipt() {
    let (_temp, context, data) = fixture();
    let root = data.join("state");
    mkdir_mode(&root, 0o700);
    let control_dir = data.join("private-control");
    std::fs::create_dir(&control_dir).unwrap();
    std::fs::set_permissions(&control_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = control_dir.join("shell.sock");
    let lease = with_paths(
        context.local(&root).unwrap(),
        &[PathRequest {
            path: &socket,
            role: PathRole::Endpoint,
        }],
    )
    .unwrap();
    let bound = lease.endpoint(&socket).unwrap().bind().unwrap();
    let receipt = lineage_retired(&lease).await;
    bound.close().unwrap();
    let mut remove = receipt.begin_remove(&lease, &root, &[&root]).unwrap();
    remove.remove_path(&root).unwrap();
    remove.commit().unwrap();
    assert!(!root.exists());
    assert!(!socket.exists());
}
#[tokio::test]
async fn f_lineage_new_runtime_after_committed_console_still_invalidates_receipt() {
    let (_temp, context, data) = fixture();
    let log = data.join("log");
    write_mode(&log, b"OLD", 0o600);
    let lease = with_paths(
        context.local(&data.join("state")).unwrap(),
        &[PathRequest {
            path: &log,
            role: PathRole::MutableFile,
        }],
    )
    .unwrap();
    let receipt = lineage_retired(&lease).await;
    publish_control(&lease, &log, b"FINAL");
    drop(lease.prepare_runtime(&[]).unwrap());
    assert!(receipt.begin_remove(&lease, &data, &[&log]).is_err());
    assert_eq!(std::fs::read(log).unwrap(), b"FINAL");
}
#[tokio::test]
async fn f_lineage_retained_destination_replacement_is_not_safe_absence() {
    let (_temp, context, data) = fixture();
    let vm = data.join("vm");
    mkdir_mode(&vm, 0o700);
    let root = vm.join("root");
    mkdir_mode(&root, 0o700);
    let kept = vm.join("retained");
    let lease = with_paths(
        context.local(&vm).unwrap(),
        &[
            PathRequest {
                path: &root,
                role: PathRole::MutableTree,
            },
            PathRequest {
                path: &kept,
                role: PathRole::MutableTree,
            },
        ],
    )
    .unwrap();
    let receipt = lineage_retired(&lease).await;
    let mut retain = lease
        .begin(MutationPlan::Adopt { root: vm.clone() })
        .unwrap();
    retain.rename_preserving(&root, &kept).unwrap();
    retain.commit().unwrap();
    std::fs::rename(&kept, vm.join("actual-data")).unwrap();
    mkdir_mode(&kept, 0o700);
    assert!(receipt.begin_remove(&lease, &vm, &[&root]).is_err());
    assert!(vm.join("actual-data").exists());
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GChildFrame {
    namespace: PathBuf,
    admission_fd: i32,
    source: PathBuf,
    endpoint: PathBuf,
    explicit_close: bool,
    generic: bool,
    scenario: String,
    descendant_fd: Option<i32>,
}
#[test]
fn g_cross_exec_child_resource_fixture() {
    use std::io::Read;
    use std::os::fd::{FromRawFd, OwnedFd};
    let args: Vec<_> = std::env::args().collect();
    if !args.windows(2).any(|a| {
        a[0] == "--exact"
            && a[1] == "worker_admission::runtime::tests::g_cross_exec_child_resource_fixture"
    }) {
        return;
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    if input.is_empty() {
        return;
    } // ordinary suite invocation is not a subprocess run
    let mut lines = input.lines();
    let frame: GChildFrame = serde_json::from_str(lines.next().unwrap()).unwrap();
    let runtime_fd: i32 = lines.next().unwrap().parse().unwrap();
    let context = testing::open_context(&frame.namespace).unwrap();
    let admission = context
        .import_child(unsafe { OwnedFd::from_raw_fd(frame.admission_fd) })
        .unwrap();
    let runtime =
        import_runtime_writer(&admission, unsafe { OwnedFd::from_raw_fd(runtime_fd) }).unwrap();
    if frame.generic {
        assert!(admission.use_path(&frame.source).is_err());
        assert!(admission.endpoint(&frame.endpoint).is_err());
    }
    let input = runtime.use_path(&frame.source).unwrap();
    input.revalidate().unwrap();
    let endpoint = runtime.endpoint(&frame.endpoint).unwrap().bind().unwrap();
    let second = if frame.scenario == "two" {
        Some(
            runtime
                .endpoint(&frame.endpoint.with_file_name("other.sock"))
                .unwrap()
                .bind()
                .unwrap(),
        )
    } else {
        None
    };
    if let Some(second) = second {
        std::mem::forget(second);
    }
    if frame.explicit_close {
        endpoint.close().unwrap();
        drop(input);
    } else {
        std::mem::forget(endpoint);
        std::mem::forget(input);
    }
    runtime.checkpoint().unwrap();
    if frame.scenario == "pending" {
        let mut tx = context.namespace.transaction().unwrap();
        let record = tx
            .catalog
            .runtimes
            .iter_mut()
            .find(|r| r.id == runtime.0.id)
            .unwrap();
        record.activity = ActivityOutcome {
            sequence: 1,
            path: Some(HostPath::new(&frame.source.with_file_name("activity")).unwrap()),
            phase: ActivityPhase::Pending,
            failure: None,
        };
        tx.save().unwrap();
    }
    if let Some(fd) = frame.descendant_fd {
        // This owned fixture descendant actually inherits the runtime/endpoint
        // descriptions. Parent releases it through a private pipe, never a PID signal.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            let mut byte = 0u8;
            unsafe {
                libc::read(fd, (&mut byte as *mut u8).cast(), 1);
                libc::_exit(0)
            };
        }
    }
    // Actual process exit closes inherited OFDs but skips every Rust Drop and
    // async relay close, exactly the production VMM boundary under test.
    unsafe { libc::_exit(0) };
}
struct GFixture {
    _temp: tempfile::TempDir,
    context: crate::worker_admission::AdmissionContext,
    lease: Lease,
    child: RuntimeChild,
    data: PathBuf,
    source: PathBuf,
    endpoint: PathBuf,
    vm: PathBuf,
    log: PathBuf,
    root: PathBuf,
    kept: PathBuf,
    parent_read: Option<crate::worker_admission::UseGuard>,
    sibling_read: Option<crate::worker_admission::UseGuard>,
    sibling: Option<Lease>,
    descendant_release: Option<std::fs::File>,
}
fn g_spawn(explicit_close: bool) -> GFixture {
    g_spawn_scenario(explicit_close, "normal")
}
fn g_spawn_scenario(explicit_close: bool, scenario: &str) -> GFixture {
    use std::os::fd::AsRawFd;
    let (temp, context, data) = fixture();
    let namespace = temp.path().join("catalog");
    let vm = data.join("vm");
    mkdir_mode(&vm, 0o700);
    let root = vm.join("root");
    mkdir_mode(&root, 0o700);
    write_mode(root.join("data"), b"KEEP", 0o600);
    let kept = vm.join("root.pre-build");
    let log = vm.join("console.log");
    write_mode(&log, b"before", 0o600);
    let source = data.join("source");
    write_mode(&source, b"LOWER", 0o600);
    let control = data.join("private-control");
    std::fs::create_dir(&control).unwrap();
    std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o700)).unwrap();
    let endpoint = control.join("shell.sock");
    let lease = with_paths(
        context.local(&vm).unwrap(),
        &[
            PathRequest {
                path: &root,
                role: PathRole::MutableTree,
            },
            PathRequest {
                path: &kept,
                role: PathRole::MutableTree,
            },
            PathRequest {
                path: &log,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &source,
                role: PathRole::Strict,
            },
            PathRequest {
                path: &endpoint,
                role: PathRole::Endpoint,
            },
        ],
    )
    .unwrap();
    let second = endpoint.with_file_name("other.sock");
    let lease = if scenario == "two" {
        with_paths(
            lease,
            &[PathRequest {
                path: &second,
                role: PathRole::Endpoint,
            }],
        )
        .unwrap()
    } else {
        lease
    };
    let (parent_read, sibling, sibling_read) = if scenario == "host-reads" {
        let parent = lease.use_path(&source).unwrap();
        let export = lease.export_child().unwrap();
        let sibling = context
            .import_child(export.duplicate_for_exec().unwrap())
            .unwrap();
        let read = sibling.use_path(&source).unwrap();
        (Some(parent), Some(sibling), Some(read))
    } else {
        (None, None, None)
    };
    let activity = source.with_file_name("activity");
    let lease = if scenario == "pending" {
        with_paths(
            lease,
            &[PathRequest {
                path: &activity,
                role: PathRole::MutableFile,
            }],
        )
        .unwrap()
    } else {
        lease
    };
    let runtime_paths = if scenario == "pending" {
        vec![RuntimePath {
            path: &activity,
            kind: RuntimePathKind::ActivityStamp,
        }]
    } else {
        Vec::new()
    };
    let launch = lease.prepare_runtime(&runtime_paths).unwrap();
    let (descendant_read, descendant_release) = if scenario == "descendant" {
        use std::os::fd::FromRawFd;
        let mut pair = [-1; 2];
        assert_eq!(unsafe { libc::pipe(pair.as_mut_ptr()) }, 0);
        for fd in pair {
            assert_eq!(
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
                0
            );
        }
        (
            Some(unsafe { std::fs::File::from_raw_fd(pair[0]) }),
            Some(unsafe { std::fs::File::from_raw_fd(pair[1]) }),
        )
    } else {
        (None, None)
    };
    let descendant_fd = descendant_read.as_ref().map(AsRawFd::as_raw_fd);
    let exported = lease.export_child().unwrap();
    let inherited = exported.duplicate_for_exec().unwrap();
    let raw = inherited.as_raw_fd();
    let frame = serde_json::to_string(&GChildFrame {
        namespace: namespace.clone(),
        admission_fd: raw,
        source: source.clone(),
        endpoint: endpoint.clone(),
        explicit_close,
        generic: true,
        scenario: scenario.into(),
        descendant_fd,
    })
    .unwrap();
    // RuntimeLaunch appends $3=--runtime-write-fd,$4=FD. The shell only execs
    // this test binary and supplies the explicit test frame on stdin, no env/HOME
    // override, fixed FD slot, production bootstrap path or guessed receipt.
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg("-c").arg("exec \"$1\" --exact worker_admission::runtime::tests::g_cross_exec_child_resource_fixture --nocapture <<EOF\n$2\n$4\nEOF").arg("g-test").arg(std::env::current_exe().unwrap()).arg(frame);
    unsafe {
        command.pre_exec(move || {
            let flags = libc::fcntl(raw, libc::F_GETFD);
            if flags < 0 {
                return Err(io::Error::last_os_error());
            }
            syscall(libc::fcntl(raw, libc::F_SETFD, flags & !libc::FD_CLOEXEC))?;
            if let Some(fd) = descendant_fd {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 {
                    return Err(io::Error::last_os_error());
                }
                syscall(libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC))?;
            }
            Ok(())
        });
    }
    let child = launch.spawn(command).unwrap();
    drop(descendant_read);
    drop(inherited);
    drop(exported);
    GFixture {
        _temp: temp,
        context,
        lease,
        child,
        data,
        source,
        endpoint,
        vm,
        log,
        root,
        kept,
        parent_read,
        sibling_read,
        sibling,
        descendant_release,
    }
}
#[tokio::test]
async fn g_cross_exec_exit_retires_runtime_reads_then_endpoint_console_retention_remove() {
    use std::io::Write;
    for explicit_close in [false, true] {
        let GFixture {
            _temp,
            context,
            lease,
            child,
            data: _,
            source: _,
            endpoint,
            vm,
            log,
            root,
            kept,
            ..
        } = g_spawn(explicit_close);
        let completion = child.wait().await;
        assert!(completion.os_status().unwrap().success());
        let receipt = completion.retirement().unwrap();
        {
            let tx = context.namespace.transaction().unwrap();
            let op = tx.operation(&lease.0.id).unwrap();
            assert!(!op.holds.iter().any(|h| h.kind == HoldKind::Read));
            assert_eq!(op.holder_ids, vec![lease.0.holder.clone()]);
        }
        receipt
            .retire_runtime_endpoints(&lease, &[&endpoint])
            .unwrap();
        receipt
            .retire_runtime_endpoints(&lease, &[&endpoint])
            .unwrap();
        assert!(!endpoint.exists());
        let mut console = lease
            .begin(MutationPlan::ControlFiles {
                files: vec![log.clone()],
            })
            .unwrap();
        let mut stage = console.stage_file(&log, 0o600).unwrap();
        stage.file_mut().write_all(b"final").unwrap();
        console.publish_file(stage).unwrap();
        console.commit().unwrap();
        let mut retain = lease
            .begin(MutationPlan::Adopt { root: vm.clone() })
            .unwrap();
        retain.rename_preserving(&root, &kept).unwrap();
        retain.commit().unwrap();
        let mut removal = receipt.begin_remove(&lease, &vm, &[&root, &log]).unwrap();
        removal.remove_path(&root).unwrap();
        removal.remove_path(&log).unwrap();
        removal.commit().unwrap();
        assert_eq!(std::fs::read(kept.join("data")).unwrap(), b"KEEP");
        assert!(
            receipt
                .retire_runtime_endpoints(&lease, &[&endpoint])
                .is_err()
        );
        drop(_temp);
    }
}
#[tokio::test]
async fn g_parent_and_unrelated_import_reads_survive_child_exit() {
    let GFixture {
        _temp,
        context,
        lease,
        child,
        endpoint,
        parent_read,
        sibling_read,
        sibling,
        ..
    } = g_spawn_scenario(false, "host-reads");
    let completion = child.wait().await;
    let receipt = completion.retirement().unwrap();
    receipt
        .retire_runtime_endpoints(&lease, &[&endpoint])
        .unwrap();
    let tx = context.namespace.transaction().unwrap();
    let op = tx.operation(&lease.0.id).unwrap();
    assert_eq!(
        op.holds.iter().filter(|h| h.kind == HoldKind::Read).count(),
        2
    );
    assert!(op.holder_ids.contains(&sibling.as_ref().unwrap().0.holder));
    drop(tx);
    drop((parent_read, sibling_read, sibling));
    drop(_temp);
}

#[tokio::test]
async fn g_endpoint_close_faults_resume_only_matching_recorded_intent() {
    for fault in ["after-intent", "after-unlink", "after-sync"] {
        let GFixture {
            _temp,
            context,
            lease,
            child,
            endpoint,
            ..
        } = g_spawn(false);
        let completion = child.wait().await;
        let receipt = completion.retirement().unwrap();
        assert!(
            receipt
                .retire_endpoints_fault(&lease, &[&endpoint], fault)
                .is_err()
        );
        {
            let tx = context.namespace.transaction().unwrap();
            assert!(
                tx.catalog
                    .endpoints
                    .iter()
                    .any(|e| e.path.get().unwrap() == endpoint
                        && e.state == super::super::catalog::EndpointState::CloseIntent)
            );
        }
        receipt
            .retire_runtime_endpoints(&lease, &[&endpoint])
            .unwrap();
        assert!(!endpoint.exists());
        drop(_temp);
    }
}
#[tokio::test]
async fn g_foreign_receipt_replacement_and_new_runtime_refuse_endpoint_cleanup() {
    let GFixture {
        _temp,
        context,
        lease,
        child,
        endpoint,
        data,
        ..
    } = g_spawn(false);
    let completion = child.wait().await;
    let receipt = completion.retirement().unwrap();
    let other = context.local(&data.join("other")).unwrap();
    assert!(
        receipt
            .retire_runtime_endpoints(&other, &[&endpoint])
            .is_err()
    );
    std::fs::rename(&endpoint, endpoint.with_extension("retained")).unwrap();
    let replacement = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
    // A foreign socket of the owner's own mode: refused by identity, not mode.
    std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        receipt
            .retire_runtime_endpoints(&lease, &[&endpoint])
            .is_err()
    );
    assert!(endpoint.exists());
    drop(replacement);
    drop(_temp);
    let GFixture {
        _temp,
        lease,
        child,
        endpoint,
        ..
    } = g_spawn(false);
    let completion = child.wait().await;
    let receipt = completion.retirement().unwrap();
    drop(lease.prepare_runtime(&[]).unwrap());
    assert!(
        receipt
            .retire_runtime_endpoints(&lease, &[&endpoint])
            .is_err()
    );
    assert!(endpoint.exists());
    drop(_temp);
}

fn g_local_runtime(child: &RuntimeChild) -> RuntimeWriter {
    let exported = child.lease.export_child().unwrap();
    let context = crate::worker_admission::AdmissionContext {
        namespace: child.lease.0.namespace.clone(),
    };
    let imported = context
        .import_child(exported.duplicate_for_exec().unwrap())
        .unwrap();
    import_runtime_writer(
        &imported,
        duplicate(child.sentinel.as_ref().unwrap()).unwrap(),
    )
    .unwrap()
}
#[tokio::test]
async fn g_runtime_read_guard_retains_writer_sentinel_after_writer_drop() {
    let (_temp, context, data) = fixture();
    let source = data.join("source");
    write_mode(&source, b"data", 0o600);
    let lease = with_paths(
        context.local(&data.join("state")).unwrap(),
        &[PathRequest {
            path: &source,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    let runtime = g_local_runtime(&child);
    let read = runtime.use_path(&source).unwrap();
    drop(runtime);
    let completion = child.wait().await;
    assert!(completion.os_status().is_some());
    assert!(completion.retirement().is_err());
    drop(read);
}
#[tokio::test]
async fn g_unbound_legacy_read_is_preserved_and_prevents_cleanup() {
    let (_temp, context, data) = fixture();
    let root = data.join("state");
    mkdir_mode(&root, 0o700);
    let source = data.join("source");
    write_mode(&source, b"data", 0o600);
    let lease = with_paths(
        context.local(&root).unwrap(),
        &[PathRequest {
            path: &source,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    let read = lease.use_path(&source).unwrap();
    {
        let mut tx = context.namespace.transaction().unwrap();
        tx.operation_mut(&lease.0.id)
            .unwrap()
            .holds
            .iter_mut()
            .find(|h| h.id == read.id)
            .unwrap()
            .owner = None;
        tx.save().unwrap();
    }
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    activate_for_wait_fixture(&child);
    let completion = child.wait().await;
    let receipt = completion.retirement().unwrap();
    assert!(receipt.begin_remove(&lease, &root, &[&root]).is_err());
    let tx = context.namespace.transaction().unwrap();
    assert!(
        tx.operation(&lease.0.id)
            .unwrap()
            .holds
            .iter()
            .any(|h| h.id == read.id && h.owner.is_none())
    );
}
#[tokio::test]
async fn g_mismatched_runtime_epoch_is_corrupt_not_retired_by_wait() {
    let (_temp, context, data) = fixture();
    let source = data.join("source");
    write_mode(&source, b"data", 0o600);
    let lease = with_paths(
        context.local(&data.join("state")).unwrap(),
        &[PathRequest {
            path: &source,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    let runtime = g_local_runtime(&child);
    let read = runtime.use_path(&source).unwrap();
    {
        let mut tx = context.namespace.transaction().unwrap();
        let hold = tx
            .operation_mut(&lease.0.id)
            .unwrap()
            .holds
            .iter_mut()
            .find(|h| h.id == read.id)
            .unwrap();
        hold.owner.as_mut().unwrap().runtime.as_mut().unwrap().epoch += 1;
        tx.save().unwrap();
    }
    assert!(context.namespace.transaction().is_err());
    drop(read);
    drop(runtime);
    let completion = child.wait().await;
    assert!(completion.os_status().is_some());
    assert!(completion.retirement().is_err());
}
#[tokio::test]
async fn g_endpoint_sentinel_still_held_blocks_parent_settlement() {
    let GFixture {
        _temp,
        context,
        lease,
        child,
        endpoint,
        ..
    } = g_spawn(false);
    let completion = child.wait().await;
    let receipt = completion.retirement().unwrap();
    let held = {
        let tx = context.namespace.transaction().unwrap();
        let record = tx
            .catalog
            .endpoints
            .iter()
            .find(|e| e.path.get().unwrap() == endpoint)
            .unwrap();
        let fd = super::super::endpoint::validate_sentinel(&context.namespace, record).unwrap();
        sentinel(&fd).unwrap();
        fd
    };
    assert!(
        receipt
            .retire_runtime_endpoints(&lease, &[&endpoint])
            .is_err()
    );
    assert!(endpoint.exists());
    drop(held);
    receipt
        .retire_runtime_endpoints(&lease, &[&endpoint])
        .unwrap();
    drop(_temp);
}
#[tokio::test]
async fn g_missing_bound_socket_is_not_implicit_retirement() {
    let GFixture {
        _temp,
        lease,
        child,
        endpoint,
        ..
    } = g_spawn(false);
    let completion = child.wait().await;
    let receipt = completion.retirement().unwrap();
    std::fs::remove_file(&endpoint).unwrap();
    assert!(
        receipt
            .retire_runtime_endpoints(&lease, &[&endpoint])
            .is_err()
    );
    drop(_temp);
}
#[tokio::test]
async fn g_legacy_endpoint_without_owner_is_not_recovered_by_private_receipt() {
    let GFixture {
        _temp,
        context,
        lease,
        child,
        endpoint,
        ..
    } = g_spawn(false);
    let completion = child.wait().await;
    let receipt = completion.retirement().unwrap();
    // Preserve the original sentinel binding; altered catalog ownership must fail
    // rather than assigning old data to whichever runtime happens to match path.
    {
        let mut tx = context.namespace.transaction().unwrap();
        let record = tx
            .catalog
            .endpoints
            .iter_mut()
            .find(|e| e.path.get().unwrap() == endpoint)
            .unwrap();
        record.owner = None;
        record.hold_id = None;
        tx.save().unwrap();
    }
    assert!(
        receipt
            .retire_runtime_endpoints(&lease, &[&endpoint])
            .is_err()
    );
    assert!(endpoint.exists());
    drop(_temp);
}

#[tokio::test]
async fn g_endpoint_requested_set_is_prevalidated_and_partial_close_remains_visible() {
    let GFixture {
        _temp,
        context,
        lease,
        child,
        endpoint,
        ..
    } = g_spawn_scenario(false, "two");
    let second = endpoint.with_file_name("other.sock");
    let completion = child.wait().await;
    let receipt = completion.retirement().unwrap();
    assert!(
        receipt
            .retire_runtime_endpoints(
                &lease,
                &[&endpoint, &endpoint.with_file_name("not-declared")]
            )
            .is_err()
    );
    assert!(endpoint.exists() && second.exists());
    assert!(
        receipt
            .retire_endpoints_fault(&lease, &[&endpoint, &second], "after-first")
            .is_err()
    );
    {
        let tx = context.namespace.transaction().unwrap();
        let records = tx
            .catalog
            .endpoints
            .iter()
            .filter(|e| e.operation == lease.0.id)
            .collect::<Vec<_>>();
        assert_eq!(
            records
                .iter()
                .filter(|e| e.state == super::super::catalog::EndpointState::Retired)
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|e| e.state == super::super::catalog::EndpointState::Bound)
                .count(),
            1
        );
    }
    receipt
        .retire_runtime_endpoints(&lease, &[&endpoint, &second])
        .unwrap();
    assert!(!endpoint.exists() && !second.exists());
    drop(_temp);
}

#[tokio::test]
async fn g_cross_exec_descendant_retains_runtime_and_endpoint_after_original_reap() {
    use std::io::Write;
    let GFixture {
        _temp,
        lease,
        child,
        endpoint,
        descendant_release,
        ..
    } = g_spawn_scenario(false, "descendant");
    let mut release = descendant_release.unwrap();
    let completion = child.wait().await;
    assert!(completion.os_status().unwrap().success());
    assert!(completion.retirement().is_err());
    assert!(endpoint.exists());
    release.write_all(b"x").unwrap();
    drop(release);
    assert!(completion.retirement().is_err());
    drop(lease);
    drop(_temp);
}
#[tokio::test]
async fn g_cross_exec_pending_activity_never_gets_endpoint_retirement_authority() {
    let GFixture {
        _temp,
        child,
        endpoint,
        ..
    } = g_spawn_scenario(false, "pending");
    let completion = child.wait().await;
    assert!(completion.os_status().unwrap().success());
    assert!(completion.retirement().is_err());
    assert!(endpoint.exists());
    drop(_temp);
}

#[test]
fn h_two_runtime_consumers_hold_same_cache_reads_independently() {
    use crate::worker_admission::CacheEntry;
    let (_temp, context, data) = fixture();
    let cache = data.join("cache");
    mkdir_mode(&cache, 0o700);
    let root = cache.join("image");
    mkdir_mode(&root, 0o700);
    write_mode(root.join("content"), b"SOURCE", 0o600);
    let lower = cache.join("image.erofs");
    write_mode(&lower, b"LOWER", 0o600);
    write_mode(cache.join("image.digest"), b"sha256:fixture", 0o600);
    let a = context.local(&data.join("a")).unwrap();
    let a = a
        .probe_cache(&[CacheEntry { root: &root }])
        .unwrap()
        .handoff(&a, &root)
        .unwrap();
    let b = context.local(&data.join("b")).unwrap();
    let b = b
        .probe_cache(&[CacheEntry { root: &root }])
        .unwrap()
        .handoff(&b, &root)
        .unwrap();
    let a_launch = a.prepare_runtime(&[]).unwrap();
    let a_export = a.export_child().unwrap();
    let a_import = context
        .import_child(a_export.duplicate_for_exec().unwrap())
        .unwrap();
    let a_writer = import_runtime_writer(
        &a_import,
        duplicate(a_launch.sentinel.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    let b_launch = b.prepare_runtime(&[]).unwrap();
    let b_export = b.export_child().unwrap();
    let b_import = context
        .import_child(b_export.duplicate_for_exec().unwrap())
        .unwrap();
    let b_writer = import_runtime_writer(
        &b_import,
        duplicate(b_launch.sentinel.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    let a_read = a_writer.use_path(&lower).unwrap();
    let b_read = b_writer.use_path(&lower).unwrap();
    a_read.revalidate().unwrap();
    b_read.revalidate().unwrap();
    let competitor = context.local(&data.join("writer")).unwrap();
    assert!(
        competitor
            .prepare_cache(&[CacheEntry { root: &root }])
            .is_err()
    );
    drop(a_read);
    assert!(
        competitor
            .prepare_cache(&[CacheEntry { root: &root }])
            .is_err()
    );
    drop(b_read);
}

fn h1_populate(root: &Path) {
    mkdir_mode(root, 0o700);
    write_mode(root.join("content"), b"SOURCE", 0o600);
    write_mode(
        root.with_file_name(format!(
            "{}.erofs",
            root.file_name().unwrap().to_string_lossy()
        )),
        b"LOWER",
        0o600,
    );
    write_mode(
        root.with_file_name(format!(
            "{}.digest",
            root.file_name().unwrap().to_string_lossy()
        )),
        b"sha256:fixture",
        0o600,
    );
}
fn h1_prepare(prep: &crate::worker_admission::CachePreparation, root: &Path) {
    use std::io::Write;
    let digest = root.with_file_name(format!(
        "{}.digest",
        root.file_name().unwrap().to_string_lossy()
    ));
    let lower = root.with_file_name(format!(
        "{}.erofs",
        root.file_name().unwrap().to_string_lossy()
    ));
    let spool = root.with_file_name(format!(
        "{}.erofs.spool",
        root.file_name().unwrap().to_string_lossy()
    ));
    let mut write = prep.begin_entry(root).unwrap();
    let tree = write.stage_tree(root).unwrap();
    write_mode(tree.path().join("content"), b"SOURCE B", 0o600);
    write.publish_tree(tree).unwrap();
    let mut metadata = write.stage_file(&digest, 0o600).unwrap();
    metadata.file_mut().write_all(b"sha256:b").unwrap();
    write.publish_file(metadata).unwrap();
    write.commit().unwrap();
    let mut image = prep.begin_erofs(root).unwrap();
    let mut output = image.stage_file(&lower, 0o600).unwrap();
    output.file_mut().write_all(b"LOWER B").unwrap();
    let temporary = image.stage_file(&spool, 0o600).unwrap();
    image.discard_file(temporary).unwrap();
    image.publish_file(output).unwrap();
    image.commit().unwrap();
}
async fn h1_retire(lease: &Lease, input: &Path, endpoint: Option<&Path>) -> WriterRetirement {
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    // Actual runtime importer uses the inherited OFD and an imported holder.
    // Resources close normally in this fixture; G's existing cross-exec tests
    // separately exercise _exit without Drops using the production child path.
    let runtime = g_local_runtime(&child);
    let read = runtime.use_path(input).unwrap();
    let bound = endpoint.map(|p| runtime.endpoint(p).unwrap().bind().unwrap());
    read.revalidate().unwrap();
    drop(read);
    if let Some(endpoint) = bound {
        endpoint.close().unwrap();
    }
    drop(runtime);
    let completion = child.wait().await;
    assert!(completion.os_status().unwrap().success());
    completion.retirement().unwrap().clone()
}
#[tokio::test]
async fn h1_owned_retirement_warm_and_cold_new_reference_keeps_original_arc_and_old_cache() {
    use crate::worker_admission::CacheEntry;
    for cold in [false, true] {
        let (_temp, context, data) = fixture();
        let cache = data.join("cache");
        mkdir_mode(&cache, 0o700);
        let a = cache.join("a");
        let b = cache.join("b");
        h1_populate(&a);
        if !cold {
            h1_populate(&b);
        }
        let private = data.join("control");
        std::fs::create_dir(&private).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = private.join("shell.sock");
        let original = with_paths(
            context.local(&data.join("vm")).unwrap(),
            &[PathRequest {
                path: &socket,
                role: PathRole::Endpoint,
            }],
        )
        .unwrap();
        // H initial consumer excludes endpoint *records*, not declared paths.
        let lease = original
            .probe_cache(&[CacheEntry { root: &a }])
            .unwrap()
            .handoff(&original, &a)
            .unwrap();
        let receipt = h1_retire(&lease, &a, Some(&socket)).await;
        receipt
            .retire_runtime_endpoints(&lease, &[&socket])
            .unwrap();
        let probe = receipt
            .probe_cache(&lease, &[CacheEntry { root: &b }])
            .unwrap();
        let lease_b = if cold {
            assert!(!probe.exists(&b).unwrap());
            let prep = probe.prepare().unwrap();
            h1_prepare(&prep, &b);
            prep.handoff(&lease, &b).unwrap()
        } else {
            assert!(probe.exists(&b).unwrap());
            let read = probe.use_path(&b).unwrap();
            read.revalidate().unwrap();
            drop(read);
            probe.handoff(&lease, &b).unwrap()
        };
        assert!(Arc::ptr_eq(&lease_b.0, &original.0));
        let read_a = lease_b.use_path(&a).unwrap();
        read_a.revalidate().unwrap();
        drop(read_a);
        let outsider = context.local(&data.join("other")).unwrap();
        assert!(outsider.prepare_cache(&[CacheEntry { root: &a }]).is_err());
        assert!(outsider.prepare_cache(&[CacheEntry { root: &b }]).is_err());
        let next = h1_retire(&lease_b, &b, None).await;
        assert!(
            receipt
                .probe_cache(
                    &lease_b,
                    &[CacheEntry {
                        root: &cache.join("stale")
                    }]
                )
                .is_err()
        );
        let third = cache.join("c");
        h1_populate(&third);
        next.probe_cache(&lease_b, &[CacheEntry { root: &third }])
            .unwrap()
            .handoff(&lease_b, &third)
            .unwrap();
        assert!(lease_b.use_path(&a).is_ok());
    }
}
#[tokio::test]
async fn h1_same_exact_strict_selection_is_verified_deduplicated_never_upgraded() {
    use crate::worker_admission::CacheEntry;
    let (_temp, context, data) = fixture();
    let cache = data.join("cache");
    mkdir_mode(&cache, 0o700);
    let root = cache.join("image");
    h1_populate(&root);
    let lease = context.local(&data.join("vm")).unwrap();
    let lease = lease
        .probe_cache(&[CacheEntry { root: &root }])
        .unwrap()
        .handoff(&lease, &root)
        .unwrap();
    let receipt = h1_retire(&lease, &root, None).await;
    let before = context
        .namespace
        .transaction()
        .unwrap()
        .operation(&lease.0.id)
        .unwrap()
        .entries
        .len();
    let probe = receipt
        .probe_cache(&lease, &[CacheEntry { root: &root }])
        .unwrap();
    let read = probe.use_path(&root).unwrap();
    read.revalidate().unwrap();
    drop(read);
    probe.handoff(&lease, &root).unwrap();
    assert_eq!(
        context
            .namespace
            .transaction()
            .unwrap()
            .operation(&lease.0.id)
            .unwrap()
            .entries
            .len(),
        before
    );
    assert!(
        receipt
            .prepare_cache(&lease, &[CacheEntry { root: &root }])
            .is_err()
    );
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &root }])
            .unwrap()
            .prepare()
            .is_err()
    );
    assert!(lease.use_path(&root).is_ok());
}
#[tokio::test]
async fn h1_exact_parent_or_child_overlap_without_cache_binding_refuses() {
    use crate::worker_admission::CacheEntry;
    let (_temp, context, data) = fixture();
    let old = data.join("old");
    mkdir_mode(&old, 0o700);
    let lease = with_paths(
        context.local(&data.join("vm")).unwrap(),
        &[PathRequest {
            path: &old,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    let receipt = h1_retire(&lease, &old, None).await;
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &old }])
            .is_err()
    );
    assert!(
        receipt
            .probe_cache(
                &lease,
                &[CacheEntry {
                    root: &old.join("child")
                }]
            )
            .is_err()
    );
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &data }])
            .is_err()
    );
}
#[tokio::test]
async fn h1_current_receipt_continues_after_owned_control_lineage_and_retention() {
    use crate::worker_admission::CacheEntry;
    let (_temp, context, data) = fixture();
    let vm = data.join("vm");
    mkdir_mode(&vm, 0o700);
    let log = vm.join("console.log");
    write_mode(&log, b"OLD", 0o600);
    let root = vm.join("root");
    mkdir_mode(&root, 0o700);
    let retained = vm.join("root.pre-build");
    let source = data.join("source");
    write_mode(&source, b"READ", 0o600);
    let lease = with_paths(
        context.local(&vm).unwrap(),
        &[
            PathRequest {
                path: &log,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &root,
                role: PathRole::MutableTree,
            },
            PathRequest {
                path: &retained,
                role: PathRole::MutableTree,
            },
            PathRequest {
                path: &source,
                role: PathRole::Strict,
            },
        ],
    )
    .unwrap();
    let receipt = h1_retire(&lease, &source, None).await;
    publish_control(&lease, &log, b"FINAL");
    let mut retain = lease
        .begin(MutationPlan::Adopt { root: vm.clone() })
        .unwrap();
    retain.rename_preserving(&root, &retained).unwrap();
    retain.commit().unwrap();
    let new = data.join("new-cache");
    h1_populate(&new);
    receipt
        .probe_cache(&lease, &[CacheEntry { root: &new }])
        .unwrap()
        .handoff(&lease, &new)
        .unwrap();
    assert!(receipt.begin_remove(&lease, &data, &[&new]).is_err());
    let mut cleanup = receipt.begin_remove(&lease, &vm, &[&root, &log]).unwrap();
    cleanup.remove_path(&root).unwrap();
    cleanup.remove_path(&log).unwrap();
    cleanup.commit().unwrap();
    assert!(retained.exists());
    assert!(new.exists());
}

async fn h1_ready() -> (
    tempfile::TempDir,
    crate::worker_admission::AdmissionContext,
    Lease,
    WriterRetirement,
    PathBuf,
) {
    let (temp, context, data) = fixture();
    let input = data.join("input");
    write_mode(&input, b"READ", 0o600);
    let vm = data.join("vm");
    mkdir_mode(&vm, 0o700);
    let lease = with_paths(
        context.local(&vm).unwrap(),
        &[PathRequest {
            path: &input,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    let receipt = h1_retire(&lease, &input, None).await;
    (temp, context, lease, receipt, data)
}
#[tokio::test]
async fn h1_foreign_used_hold_and_imported_holder_do_not_issue_continuation() {
    use crate::worker_admission::CacheEntry;
    let (_temp, context, lease, receipt, data) = h1_ready().await;
    let new = data.join("cache");
    h1_populate(&new);
    let other = context.local(&data.join("other")).unwrap();
    assert!(
        receipt
            .probe_cache(&other, &[CacheEntry { root: &new }])
            .is_err()
    );
    let hold = lease.use_path(&data.join("input")).unwrap();
    assert!(
        receipt
            .prepare_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
    drop(hold);
    let export = lease.export_child().unwrap();
    let imported = context
        .import_child(export.duplicate_for_exec().unwrap())
        .unwrap();
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
    drop(imported);
    drop(export);
    let vm = data.join("vm");
    let mut remove = receipt.begin_remove(&lease, &vm, &[&vm]).unwrap();
    remove.remove_path(&vm).unwrap();
    remove.commit().unwrap();
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
}
#[tokio::test]
async fn h1_probe_and_preparation_remain_bound_after_receipt_use_or_new_runtime() {
    use crate::worker_admission::CacheEntry;
    for used in [false, true] {
        let (_temp, _context, lease, receipt, data) = h1_ready().await;
        let new = data.join("cache");
        h1_populate(&new);
        let prep = receipt
            .prepare_cache(&lease, &[CacheEntry { root: &new }])
            .unwrap();
        let clone = prep.clone();
        if used {
            let vm = data.join("vm");
            let mut remove = receipt.begin_remove(&lease, &vm, &[&vm]).unwrap();
            remove.remove_path(&vm).unwrap();
            remove.commit().unwrap();
        } else {
            drop(lease.prepare_runtime(&[]).unwrap());
        }
        assert!(clone.checkpoint().is_err());
        assert!(prep.handoff(&lease, &new).is_err());
        assert!(clone.begin_entry(&new).is_err());
    }
}
#[tokio::test]
async fn h1_issued_probe_refuses_even_completed_consumer_mutation_before_transfer() {
    use crate::worker_admission::CacheEntry;
    let (_temp, _context, lease, receipt, data) = h1_ready().await;
    let new = data.join("cache");
    h1_populate(&new);
    let probe = receipt
        .probe_cache(&lease, &[CacheEntry { root: &new }])
        .unwrap();
    lease
        .begin(MutationPlan::Create {
            root: data.join("vm"),
        })
        .unwrap()
        .commit()
        .unwrap();
    assert!(probe.handoff(&lease, &new).is_err());
    receipt
        .probe_cache(&lease, &[CacheEntry { root: &new }])
        .unwrap()
        .handoff(&lease, &new)
        .unwrap();
}
#[tokio::test]
async fn h1_unfinished_ownerread_journal_or_recovery_never_authorizes_continuation() {
    use crate::worker_admission::CacheEntry;
    let (_temp, context, data) = fixture();
    let source = data.join("managed");
    mkdir_mode(&source, 0o700);
    let input = data.join("input");
    write_mode(&input, b"READ", 0o600);
    let lease = with_paths(
        context.local(&data.join("vm")).unwrap(),
        &[
            PathRequest {
                path: &source,
                role: PathRole::ManagedCacheRead,
            },
            PathRequest {
                path: &input,
                role: PathRole::Strict,
            },
        ],
    )
    .unwrap();
    let receipt = h1_retire(&lease, &input, None).await;
    let new = data.join("cache");
    h1_populate(&new);
    let read = lease.owner_read_cache(&source).unwrap();
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
    read.finish().unwrap();
    let mutation = lease
        .begin(MutationPlan::Create {
            root: data.join("vm"),
        })
        .unwrap();
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
    drop(mutation);
    {
        let mut tx = context.namespace.transaction().unwrap();
        tx.operation_mut(&lease.0.id).unwrap().recovery =
            Some(super::super::catalog::RecoveryIntent {
                record_sha256: "0".repeat(64),
                completed: false,
            });
        tx.save().unwrap();
    }
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
}
#[tokio::test]
async fn h1_endpoint_must_be_exactly_retired_before_continuation() {
    use crate::worker_admission::CacheEntry;
    let GFixture {
        _temp,
        lease,
        child,
        endpoint,
        data,
        ..
    } = g_spawn(false);
    let completion = child.wait().await;
    let receipt = completion.retirement().unwrap();
    let new = data.join("cache");
    h1_populate(&new);
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
    receipt
        .retire_runtime_endpoints(&lease, &[&endpoint])
        .unwrap();
    receipt
        .probe_cache(&lease, &[CacheEntry { root: &new }])
        .unwrap()
        .handoff(&lease, &new)
        .unwrap();
}
#[tokio::test]
async fn h1_faulted_transfer_preserves_old_cache_claims_and_rejects_repeat_on_stale_prep() {
    use crate::worker_admission::CacheEntry;
    for fault in ["before-save", "after-save", "partial-save"] {
        let (_temp, context, lease, receipt, data) = h1_ready().await;
        let new = data.join("cache");
        h1_populate(&new);
        let prep = receipt
            .prepare_cache(&lease, &[CacheEntry { root: &new }])
            .unwrap();
        assert!(prep.handoff_fault(&lease, &new, fault).is_err());
        if fault == "before-save" {
            prep.handoff(&lease, &new).unwrap();
            assert!(prep.handoff(&lease, &new).is_err());
        } else {
            assert!(prep.handoff(&lease, &new).is_err());
        }
        assert_eq!(std::fs::read(data.join("input")).unwrap(), b"READ");
        let contender = context.local(&data.join("contender"));
        if let Ok(contender) = contender {
            assert!(
                contender
                    .prepare_cache(&[CacheEntry { root: &new }])
                    .is_err()
            );
        }
    }
}
#[tokio::test]
async fn h1_stale_receipt_after_new_actual_retirement_is_not_revalidated_by_status() {
    use crate::worker_admission::CacheEntry;
    let (_temp, _context, lease, old, data) = h1_ready().await;
    let next = h1_retire(&lease, &data.join("input"), None).await;
    let new = data.join("cache");
    h1_populate(&new);
    assert!(
        old.probe_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
    next.probe_cache(&lease, &[CacheEntry { root: &new }])
        .unwrap()
        .handoff(&lease, &new)
        .unwrap();
}

#[tokio::test]
async fn h1_cancelled_probe_preserves_receipt_and_abandoned_prep_preserves_fence() {
    use crate::worker_admission::CacheEntry;
    let (_temp, context, lease, receipt, data) = h1_ready().await;
    let warm = data.join("warm");
    h1_populate(&warm);
    receipt
        .probe_cache(&lease, &[CacheEntry { root: &warm }])
        .unwrap()
        .cancel()
        .unwrap();
    let prep = receipt
        .prepare_cache(&lease, &[CacheEntry { root: &warm }])
        .unwrap();
    let clone = prep.clone();
    drop(prep);
    clone.handoff(&lease, &warm).unwrap();
    assert!(clone.checkpoint().is_err());
    let cold = data.join("cold");
    let abandoned = receipt
        .prepare_cache(&lease, &[CacheEntry { root: &cold }])
        .unwrap();
    drop(abandoned);
    let other = context.local(&data.join("other")).unwrap();
    assert!(other.prepare_cache(&[CacheEntry { root: &cold }]).is_err());
    assert!(lease.use_path(&warm).is_ok());
}
#[tokio::test]
async fn h1_concurrent_remove_and_cache_transfer_have_only_catalog_ordered_outcomes() {
    use crate::worker_admission::CacheEntry;
    let (_temp, context, lease, receipt, data) = h1_ready().await;
    let new = data.join("cache");
    h1_populate(&new);
    let prep = receipt
        .prepare_cache(&lease, &[CacheEntry { root: &new }])
        .unwrap();
    let vm = data.join("vm");
    let receipt_clone = receipt.clone();
    let consumer_clone = lease.clone();
    let prep_clone = prep.clone();
    let read_target = new.clone();
    let (removal, handoff) = std::thread::scope(|scope| {
        let remove = scope.spawn(|| receipt_clone.begin_remove(&lease, &vm, &[&vm]));
        let transfer = scope.spawn(|| prep_clone.handoff(&consumer_clone, &read_target));
        (remove.join().unwrap(), transfer.join().unwrap())
    });
    if let Ok(mut guard) = removal {
        guard.remove_path(&vm).unwrap();
        guard.commit().unwrap();
    }
    let tx = context.namespace.transaction().unwrap();
    let op = tx.operation(&lease.0.id).unwrap();
    if handoff.is_ok() {
        assert!(
            op.entries
                .iter()
                .any(|e| e.path.get().unwrap() == new && e.role == PathRole::Strict)
        );
    }
    drop(tx);
    assert!(
        receipt
            .probe_cache(
                &lease,
                &[CacheEntry {
                    root: &data.join("never")
                }]
            )
            .is_err()
    );
}
#[tokio::test]
async fn h1_overlapping_proved_candidate_and_new_pinned_candidate_allow_warm_append_only() {
    use crate::worker_admission::CacheEntry;
    let (_temp, _context, lease, receipt, data) = h1_ready().await;
    let shared = data.join("shared");
    h1_populate(&shared);
    receipt
        .probe_cache(&lease, &[CacheEntry { root: &shared }])
        .unwrap()
        .handoff(&lease, &shared)
        .unwrap();
    let pinned = data.join("pinned");
    h1_populate(&pinned);
    let probe = receipt
        .probe_cache(
            &lease,
            &[CacheEntry { root: &shared }, CacheEntry { root: &pinned }],
        )
        .unwrap();
    probe.handoff(&lease, &pinned).unwrap();
    assert!(
        receipt
            .prepare_cache(
                &lease,
                &[CacheEntry { root: &shared }, CacheEntry { root: &pinned }]
            )
            .is_err()
    );
    assert!(lease.use_path(&shared).is_ok());
    assert!(lease.use_path(&pinned).is_ok());
}

#[tokio::test]
async fn h1_issued_preparation_rechecks_imports_and_replacement_before_transfer() {
    use crate::worker_admission::CacheEntry;
    let (_temp, context, lease, receipt, data) = h1_ready().await;
    let new = data.join("cache");
    h1_populate(&new);
    let prep = receipt
        .prepare_cache(&lease, &[CacheEntry { root: &new }])
        .unwrap();
    let export = lease.export_child().unwrap();
    let imported = context
        .import_child(export.duplicate_for_exec().unwrap())
        .unwrap();
    assert!(prep.handoff(&lease, &new).is_err());
    drop(imported);
    drop(export);
    std::fs::rename(&new, data.join("retained-cache")).unwrap();
    mkdir_mode(&new, 0o700);
    assert!(prep.handoff(&lease, &new).is_err());
    assert!(data.join("retained-cache").exists());
}
#[tokio::test]
async fn h1_receipt_does_not_authorize_new_mutable_lifecycle_declarations() {
    use crate::worker_admission::CacheEntry;
    let (_temp, _context, lease, receipt, data) = h1_ready().await;
    let extra = data.join("unproved-layout");
    with_paths(
        lease.clone(),
        &[PathRequest {
            path: &extra,
            role: PathRole::MutableTree,
        }],
    )
    .unwrap();
    let new = data.join("cache");
    h1_populate(&new);
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
    assert!(!extra.exists());
}
#[tokio::test]
async fn h1_prior_receipt_refuses_while_new_sibling_runtime_is_active() {
    use crate::worker_admission::CacheEntry;
    let (_temp, _context, lease, receipt, data) = h1_ready().await;
    let child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(simple_command("exit 0"))
        .unwrap();
    let runtime = g_local_runtime(&child);
    let new = data.join("cache");
    h1_populate(&new);
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
    drop(runtime);
    let done = child.wait().await;
    assert!(done.retirement().is_ok());
    assert!(
        receipt
            .probe_cache(&lease, &[CacheEntry { root: &new }])
            .is_err()
    );
}

#[tokio::test]
async fn h1_cache_guard_and_diagnostic_recheck_consumed_receipt_before_effects() {
    use crate::worker_admission::CacheEntry;
    let (_temp, _context, lease, receipt, data) = h1_ready().await;
    let new = data.join("cache");
    h1_populate(&new);
    let prep = receipt
        .prepare_cache(&lease, &[CacheEntry { root: &new }])
        .unwrap();
    let mut guard = prep.begin_entry(&new).unwrap();
    let lock = new.with_file_name("cache.lock");
    let mut diagnostic = guard.diagnostic_lock(&lock).unwrap();
    diagnostic.lock_exclusive().unwrap();
    let vm = data.join("vm");
    let mut remove = receipt.begin_remove(&lease, &vm, &[&vm]).unwrap();
    remove.remove_path(&vm).unwrap();
    remove.commit().unwrap();
    assert!(diagnostic.write_pid(123).is_err());
    assert!(guard.checkpoint().is_err());
    assert!(prep.handoff(&lease, &new).is_err());
}
#[tokio::test]
async fn h1_runtime_creation_and_handoff_are_catalog_serialized_without_read_gap() {
    use crate::worker_admission::CacheEntry;
    let (_temp, context, lease, receipt, data) = h1_ready().await;
    let new = data.join("cache");
    h1_populate(&new);
    let prep = receipt
        .prepare_cache(&lease, &[CacheEntry { root: &new }])
        .unwrap();
    let (runtime, transfer) = std::thread::scope(|scope| {
        let start = scope.spawn(|| lease.prepare_runtime(&[]));
        let handoff = scope.spawn(|| prep.handoff(&lease, &new));
        (start.join().unwrap(), handoff.join().unwrap())
    });
    let tx = context.namespace.transaction().unwrap();
    let op = tx.operation(&lease.0.id).unwrap();
    if transfer.is_ok() {
        assert!(
            op.entries
                .iter()
                .any(|e| e.path.get().unwrap() == new && e.role == PathRole::Strict)
        );
    } else {
        assert!(tx.catalog.operations.iter().any(|candidate| {
            candidate.cache.as_ref().is_some_and(|c| {
                c.consumer == lease.0.id && c.phase == super::super::catalog::CachePhase::Preparing
            })
        }));
    }
    drop(tx);
    let started = runtime.is_ok();
    drop(runtime);
    let other = context.local(&data.join("other")).unwrap();
    assert!(other.prepare_cache(&[CacheEntry { root: &new }]).is_err());
    if started {
        assert!(
            receipt
                .probe_cache(
                    &lease,
                    &[CacheEntry {
                        root: &data.join("later")
                    }]
                )
                .is_err()
        );
    }
}
