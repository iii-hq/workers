// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN. Imports the actual inherited OFD through an isolated context;
//! positive cross-exec/_exit coverage needs the coordinator-owned test dispatcher.
use super::*;
use crate::worker_admission::{MutationPlan, PathRequest, testing, with_paths};
use std::os::unix::fs::PermissionsExt;
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
    std::fs::write(&upper, b"OLD").unwrap();
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
    std::fs::write(&upper, b"NEW").unwrap();
    writer.checkpoint().unwrap();
    std::fs::rename(&upper, data.join("retained")).unwrap();
    std::fs::write(&upper, b"FOREIGN").unwrap();
    assert!(writer.checkpoint().is_err());
    std::fs::remove_file(&upper).unwrap();
    std::fs::rename(data.join("retained"), &upper).unwrap();
    assert!(writer.checkpoint().is_err());
}
#[test]
fn runtime_guest_tree_descendants_change_and_activity_opens_exact_registered_file() {
    let (_temp, context, data) = fixture();
    let tree = data.join("tree");
    std::fs::create_dir(&tree).unwrap();
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
    std::fs::create_dir(tree.join("child")).unwrap();
    std::fs::write(tree.join("child/data"), b"NEW").unwrap();
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
    std::fs::create_dir(&tree).unwrap();
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
    std::fs::write(&path, b"payload").unwrap();
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
    std::fs::write(&path, b"foreign").unwrap();
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
    std::fs::create_dir(&tree).unwrap();
    let log = data.join("console.log");
    let rotated = data.join("console.log.1");
    std::fs::write(&log, b"old-log").unwrap();
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

#[test]
fn e_activity_failure_marks_parent_shared_description_even_when_catalog_is_busy() {
    let (_temp, context, launch, writer, path) = activity_fixture();
    let activity = writer.activity(&path).unwrap();
    let held = context
        .namespace
        .directory
        .file("catalog.lock", false)
        .unwrap();
    flock(&held).unwrap();
    assert!(activity.touch_at(SystemTime::now()).is_err());
    assert!(failed_ofd(launch.sentinel.as_ref().unwrap()).unwrap());
    drop(held);
    assert!(writer.checkpoint().is_err());
    // No active writer is retired here: this asserts the exact OFD gate that
    // RuntimeChild::wait checks before releasing its parent duplicate.
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
    flock(&lock).unwrap();
    let completion = child.wait().await;
    assert_eq!(completion.os_status().unwrap().code(), Some(7));
    assert!(completion.retirement().is_err());
    let observed = control.terminal().await;
    assert_eq!(observed.phase(), RuntimePhase::RetirementFailed);
    assert_eq!(observed.os_status().unwrap().code(), Some(7));
    assert!(control.signal(super::super::RuntimeSignal::Kill).is_err());
    drop(lock);
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
    std::fs::create_dir(&root).unwrap();
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
    std::fs::create_dir(&vm).unwrap();
    let log = vm.join("console.log");
    let rotated = vm.join("console.log.1");
    std::fs::write(&log, b"running").unwrap();
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
    std::fs::create_dir(&vm).unwrap();
    let root = vm.join("root");
    std::fs::create_dir(&root).unwrap();
    let upper = root.join("upper.ext4");
    std::fs::write(&upper, b"GUEST DATA").unwrap();
    std::fs::write(root.join("guest-file"), b"KEEP").unwrap();
    let kept_upper = vm.join("upper.pre-build.ext4");
    let kept_root = vm.join("root.pre-build");
    let log = vm.join("console.log");
    std::fs::write(&log, b"log").unwrap();
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
    std::fs::write(&log, b"OLD").unwrap();
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
    std::fs::write(&log, b"FOREIGN").unwrap();
    assert!(receipt.begin_remove(&lease, &data, &[&log]).is_err());
    assert_eq!(std::fs::read(&log).unwrap(), b"FOREIGN");
}
#[tokio::test]
async fn f_lineage_incomplete_control_consumes_receipt_without_removal() {
    let (_temp, context, data) = fixture();
    let log = data.join("log");
    std::fs::write(&log, b"OLD").unwrap();
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
    std::fs::create_dir(&vm).unwrap();
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
        std::fs::write(&log, b"OLD").unwrap();
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
    std::fs::write(&log, b"OLD").unwrap();
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
    std::fs::create_dir(&root).unwrap();
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
    std::fs::write(&log, b"OLD").unwrap();
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
    std::fs::create_dir(&vm).unwrap();
    let root = vm.join("root");
    std::fs::create_dir(&root).unwrap();
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
    std::fs::create_dir(&kept).unwrap();
    assert!(receipt.begin_remove(&lease, &vm, &[&root]).is_err());
    assert!(vm.join("actual-data").exists());
}
