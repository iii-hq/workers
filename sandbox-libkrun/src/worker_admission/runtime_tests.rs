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
    let approved = writer.open_activity_file(&activity).unwrap();
    assert_eq!(
        approved.metadata().unwrap().ino(),
        std::fs::metadata(&activity).unwrap().ino()
    );
    assert!(writer.open_activity_file(&tree.join("child/data")).is_err());
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
