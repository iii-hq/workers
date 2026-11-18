// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Authored only; all filesystem/lock effects are future own-temp fixtures.
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};
fn private(path: &Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}
fn id_for(key: &str) -> String {
    hex::encode(Sha256::digest(key))[..32].to_string()
}

#[test]
fn normal_absent_base_creates_no_files_and_allows_shared_mutations() {
    let r = root();
    let base = r.path().join("new-parent/workers");
    let a = worker(&base, Some("a")).unwrap();
    let b = worker(&base, Some("b")).unwrap();
    assert!(!base.parent().unwrap().exists());
    assert_eq!(std::fs::read_dir(r.path()).unwrap().count(), 0);
    drop((a, b));
}
#[test]
fn shared_guard_survives_caller_clone_until_last_writer_releases() {
    let r = root();
    let base = r.path().join("workers");
    let first = worker(&base, Some("a")).unwrap();
    let writer = first.clone();
    drop(first);
    assert!(ancestor_lease(&base, true).is_err());
    drop(writer);
    let transition = ancestor_lease(&base, true).unwrap();
    assert!(worker(&base, Some("a")).is_err());
    drop(transition);
    assert!(worker(&base, Some("a")).is_ok());
}
#[test]
fn enrollment_between_requests_is_seen_before_any_mutation() {
    let r = root();
    let base = r.path().join("workers");
    let first = worker(&base, Some("a")).unwrap();
    drop(first);
    let exclusive = ancestor_lease(&base, true).unwrap();
    let path = custody(&base, &id_for("a")).unwrap();
    private(path.parent().unwrap());
    private(&path);
    drop(exclusive);
    assert!(worker(&base, Some("a")).is_err());
    assert!(worker(&base, Some("b")).is_ok());
    assert!(worker(&base, None).is_err());
}
#[test]
fn symlink_registry_and_substituted_parent_fail_closed() {
    let r = root();
    let base = r.path().join("workers");
    let path = custody(&base, &id_for("a")).unwrap();
    let foreign = r.path().join("foreign");
    private(&foreign);
    symlink(&foreign, path.parent().unwrap()).unwrap();
    assert!(worker(&base, Some("a")).is_err());
    let r = root();
    let parent = r.path().join("owner");
    private(&parent);
    let base = parent.join("workers");
    let guard = worker(&base, Some("a")).unwrap();
    std::fs::rename(&parent, r.path().join("retained")).unwrap();
    private(&parent);
    assert!(guard.revalidate().is_err());
}
#[test]
fn incomplete_corrupt_and_symlinked_markers_are_not_normal() {
    for kind in ["empty", "corrupt", "symlink"] {
        let r = root();
        let base = r.path().join("workers");
        let path = custody(&base, &id_for("a")).unwrap();
        private(path.parent().unwrap());
        private(&path);
        if kind == "corrupt" {
            std::fs::write(path.join("active.json"), b"{broken").unwrap();
        }
        if kind == "symlink" {
            symlink("missing", path.join("active.json")).unwrap();
        }
        assert!(worker(&base, Some("a")).is_err());
        assert_eq!(std::fs::read_dir(r.path()).unwrap().count(), 1);
    }
}
#[test]
fn local_builders_refuse_runtime_markers_and_generation_ancestors() {
    let r = root();
    let base = r.path().join("workers");
    private(&base);
    let vm = base.join(id_for("a"));
    private(&vm);
    private(&vm.join("compat"));
    assert!(local(&vm.join("root")).is_err());
    assert!(local(&vm.join("build-1/root")).is_err());
    let candidate = r.path().join("candidate");
    private(&candidate);
    std::fs::write(candidate.join("receipt.json"), b"{}\n").unwrap();
    assert!(local(&candidate.join("data/root")).is_err());
}
#[test]
fn local_and_worker_guards_do_not_lock_the_existing_claim_directory() {
    let r = root();
    let base = r.path().join("workers");
    private(&base);
    let vm = base.join(id_for("a"));
    private(&vm);
    private(&vm.join("root"));
    let _worker = worker(&base, Some("a")).unwrap();
    let _local = local(&vm.join("root")).unwrap();
    let file = File::open(&base).unwrap();
    assert_eq!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
}
#[test]
fn local_rejects_symlink_ancestor_without_following_marker() {
    let r = root();
    let target = r.path().join("target");
    private(&target);
    let alias = r.path().join("alias");
    symlink(&target, &alias).unwrap();
    assert!(local(&alias.join("root")).is_err());
    assert_eq!(std::fs::read_dir(target).unwrap().count(), 0);
}
#[test]
fn separate_owners_can_operate_while_unrelated_parent_is_exclusive() {
    let r = root();
    let a = r.path().join("a");
    let b = r.path().join("b");
    private(&a);
    private(&b);
    let _exclusive = ancestor_lease(&a.join("workers"), true).unwrap();
    assert!(worker(&b.join("workers"), Some("key")).is_ok());
}

#[test]
fn retained_source_lower_upper_and_backup_paths_are_never_normal_inputs() {
    let r = root();
    let retained = r.path().join("offline");
    private(&retained);
    private(&retained.join("data"));
    std::fs::write(retained.join("backup.json"), b"receipt").unwrap();
    std::fs::write(retained.join("data/upper.ext4"), b"OLD").unwrap();
    assert!(reject_retained_path(&retained.join("data/upper.ext4")).is_err());
    assert!(local(&retained.join("data/root")).is_err());
    let normal = r.path().join("normal-image");
    std::fs::write(&normal, b"image").unwrap();
    assert!(reject_retained_path(&normal).is_ok());
}

#[test]
fn custody_survives_absent_vm_and_cannot_be_erased_by_normal_path_recreation() {
    let r = root();
    let base = r.path().join("workers");
    let path = custody(&base, &id_for("key")).unwrap();
    private(path.parent().unwrap());
    private(&path);
    assert!(!base.exists());
    assert!(worker(&base, Some("key")).is_err());
    assert!(!base.exists());
}

#[test]
fn writer_lease_keeps_transition_out_until_explicit_reap_release() {
    let r = root();
    let base = r.path().join("workers");
    let admission = worker(&base, Some("key")).unwrap();
    let writer = WriterLease::new(vec![admission]);
    assert!(ancestor_lease(&base, true).is_err());
    writer.reaped();
    let transition = ancestor_lease(&base, true).unwrap();
    drop(transition);
    assert!(worker(&base, Some("key")).is_ok());
}
