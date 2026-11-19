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
    assert!(child.wait().await.is_err());
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
