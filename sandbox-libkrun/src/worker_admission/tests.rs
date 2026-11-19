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
fn claim(base: &Path) -> File {
    let file = File::open(base).unwrap();
    assert_eq!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    file
}
fn enroll(base: &Path, key: &str) -> PathBuf {
    let path = custody(base, &id_for(key)).unwrap();
    private(path.parent().unwrap());
    private(&path);
    path
}
fn owner(root: &Path, name: &str) -> PathBuf {
    let parent = root.join(name);
    private(&parent);
    parent.join("workers")
}

#[test]
fn missing_owner_parent_refuses_before_normal_effects_instead_of_moving_fence() {
    let r = root();
    let base = r.path().join("new-parent/workers");
    assert!(worker(&base, Some("a")).is_err());
    assert!(ancestor_lease(&base, true).is_err());
    assert_eq!(std::fs::read_dir(r.path()).unwrap().count(), 0);
    // Provisioning happens outside admission, before any normal use. There is
    // no successful old lease on r that could coexist with this new domain.
    private(base.parent().unwrap());
    let lease = worker(&base, Some("a")).unwrap();
    private(&base);
    std::fs::create_dir_all(base.join(id_for("a")).join("root/new")).unwrap();
    lease.revalidate().unwrap();
    assert!(ancestor_lease(&base, true).is_err());
    drop(lease);
    assert!(ancestor_lease(&base, true).is_ok());
}
#[test]
fn absent_base_with_existing_owner_creates_nothing_and_allows_shared_mutations() {
    let r = root();
    let base = r.path().join("workers");
    let a = worker(&base, Some("a")).unwrap();
    let b = worker(&base, Some("b")).unwrap();
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
    drop(worker(&base, Some("a")).unwrap());
    let exclusive = ancestor_lease(&base, true).unwrap();
    enroll(&base, "a");
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
    let base = owner(r.path(), "owner");
    let parent = base.parent().unwrap();
    let guard = worker(&base, Some("a")).unwrap();
    std::fs::rename(parent, r.path().join("retained")).unwrap();
    private(parent);
    assert!(guard.revalidate().is_err());
}
#[test]
fn incomplete_corrupt_and_symlinked_custody_are_not_normal() {
    for kind in ["empty", "corrupt", "symlink"] {
        let r = root();
        let base = r.path().join("workers");
        let path = enroll(&base, "a");
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
fn unsafe_registry_permissions_refuse_even_an_unenrolled_key() {
    let r = root();
    let base = r.path().join("workers");
    let path = enroll(&base, "b");
    std::fs::set_permissions(
        path.parent().unwrap(),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(worker(&base, Some("a")).is_err());
}
#[test]
fn local_builders_refuse_legacy_markers_only_at_vm_directory() {
    for marker in ["compat", "compat-mode", "compat-generation"] {
        let r = root();
        let base = r.path().join("workers");
        private(&base);
        let vm = base.join(id_for("a"));
        private(&vm);
        private(&vm.join(marker));
        assert!(local(&vm.join("root")).is_err());
        assert!(local(&vm.join("build-1/root")).is_err());
        assert!(reject_retained_path(&vm.join("root/missing/upper.ext4")).is_err());
    }
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
    let _claim = claim(&base);
}
#[test]
fn input_batch_fences_do_not_interfere_with_claims_in_either_order() {
    let r = root();
    let a = owner(r.path(), "a");
    let b = owner(r.path(), "b");
    private(&a);
    private(&b);
    let vm = b.join(id_for("b"));
    private(&vm);
    let image = vm.join("root/absent.ext4");
    for paths in [
        [b.as_path(), image.as_path()],
        [image.as_path(), b.as_path()],
    ] {
        let _lease = with_inputs(worker(&a, Some("a")).unwrap(), &paths).unwrap();
        let _a_claim = claim(&a);
        let _b_claim = claim(&b);
    }
    let _b_claim = claim(&b);
    assert!(with_inputs(worker(&a, Some("a")).unwrap(), &[&image]).is_ok());
}
#[test]
fn input_under_known_base_without_vm_component_avoids_claim_lock() {
    let r = root();
    let base = r.path().join("workers");
    private(&base);
    let file = base.join("cache.erofs");
    std::fs::write(&file, b"image").unwrap();
    let _lease = with_inputs(worker(&base, Some("a")).unwrap(), &[&file]).unwrap();
    let _claim = claim(&base);
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
    let a = owner(r.path(), "a");
    let b = owner(r.path(), "b");
    let _exclusive = ancestor_lease(&a, true).unwrap();
    assert!(worker(&b, Some("key")).is_ok());
    assert!(local(&b.join(id_for("key")).join("root")).is_ok());
}
#[test]
fn root_wide_fence_is_refused_without_attempting_global_exclusion() {
    assert!(ancestor_lease(Path::new("/workers"), false).is_err());
    assert!(ancestor_lease(Path::new("/workers"), true).is_err());
}
#[test]
fn reserved_custody_ancestors_refuse_even_absent_generation_and_backup_inputs() {
    let r = root();
    let base = r.path().join("workers");
    let retained = custody(&base, &id_for("b")).unwrap();
    for suffix in [
        "generation-new/data/root",
        "backup/data/upper.ext4",
        "lower.erofs",
    ] {
        let input = retained.join(suffix);
        assert!(reject_retained_path(&input).is_err());
        assert!(local(&input).is_err());
        assert!(with_inputs(worker(&base, Some("a")).unwrap(), &[&input]).is_err());
    }
    assert_eq!(std::fs::read_dir(r.path()).unwrap().count(), 0);
}
#[test]
fn custody_survives_absent_vm_and_cannot_be_erased_by_normal_path_recreation() {
    let r = root();
    let base = r.path().join("workers");
    enroll(&base, "key");
    assert!(!base.exists());
    assert!(worker(&base, Some("key")).is_err());
    assert!(!base.exists());
}
#[test]
fn normal_a_refuses_original_storage_of_enrolled_b_in_sibling_namespace() {
    let r = root();
    let base = r.path().join("workers");
    private(&base);
    let b = base.join(id_for("b"));
    private(&b);
    private(&b.join("root"));
    let upper = b.join("root/upper.ext4");
    std::fs::write(&upper, b"OLD").unwrap();
    enroll(&base, "b");
    assert!(worker(&base, Some("a")).is_ok());
    assert!(reject_retained_path(&upper).is_err());
    assert!(local(&b.join("root")).is_err());
    assert!(with_inputs(worker(&base, Some("a")).unwrap(), &[&upper]).is_err());
    assert_eq!(std::fs::read(&upper).unwrap(), b"OLD");
}
#[test]
fn enrolled_b_with_absent_base_vm_and_input_still_refuses() {
    let r = root();
    let a = owner(r.path(), "a");
    let b = owner(r.path(), "b");
    enroll(&b, "b");
    let input = b.join(id_for("b")).join("root/absent.ext4");
    assert!(with_inputs(worker(&a, Some("a")).unwrap(), &[&input]).is_err());
    assert!(reject_retained_path(&input).is_err());
    assert!(!b.exists());
}
#[test]
fn retained_input_blocks_future_enrollment_across_new_vm_hierarchy_until_last_clone() {
    let r = root();
    let a = owner(r.path(), "a");
    let b = owner(r.path(), "b");
    let input = b.join(id_for("b")).join("root/new.ext4");
    let admission = with_inputs(worker(&a, Some("a")).unwrap(), &[&input]).unwrap();
    let writer = admission.clone();
    drop(admission);
    std::fs::create_dir_all(input.parent().unwrap()).unwrap();
    std::fs::write(&input, b"NEW").unwrap();
    writer.revalidate().unwrap();
    assert!(ancestor_lease(&a, true).is_err());
    assert!(ancestor_lease(&b, true).is_err());
    drop(writer);
    assert!(ancestor_lease(&a, true).is_ok());
    assert!(ancestor_lease(&b, true).is_ok());
}
#[test]
fn composite_keeps_all_inputs_not_just_the_last() {
    let r = root();
    let a = owner(r.path(), "a");
    let b = owner(r.path(), "b");
    let c = owner(r.path(), "c");
    let b_input = b.join(id_for("b")).join("source");
    let c_input = c.join(id_for("c")).join("output.ext4");
    let lease = with_inputs(worker(&a, Some("a")).unwrap(), &[&b_input, &c_input]).unwrap();
    for base in [&a, &b, &c] {
        assert!(ancestor_lease(base, true).is_err());
    }
    drop(lease);
    for base in [&a, &b, &c] {
        assert!(ancestor_lease(base, true).is_ok());
    }
}
#[test]
fn failed_input_batch_releases_partial_fences_but_not_an_external_original_clone() {
    let r = root();
    let a = owner(r.path(), "a");
    let b = owner(r.path(), "b");
    let c = owner(r.path(), "c");
    enroll(&c, "c");
    let admission = worker(&a, Some("a")).unwrap();
    let b_input = b.join(id_for("b")).join("root");
    let c_input = c.join(id_for("c")).join("root");
    assert!(with_inputs(admission.clone(), &[&b_input, &c_input]).is_err());
    assert!(ancestor_lease(&a, true).is_err());
    assert!(ancestor_lease(&b, true).is_ok());
    drop(admission);
    assert!(ancestor_lease(&a, true).is_ok());
}
#[test]
fn empty_inputs_preserve_the_original_arc() {
    let r = root();
    let admission = worker(&r.path().join("workers"), Some("a")).unwrap();
    let returned = with_inputs(admission.clone(), &[]).unwrap();
    assert!(Arc::ptr_eq(&admission.0, &returned.0));
}
#[test]
fn composite_revalidates_each_original_and_input_owner_identity() {
    for replace in ["a", "b", "c"] {
        let r = root();
        let a = owner(r.path(), "a");
        let b = owner(r.path(), "b");
        let c = owner(r.path(), "c");
        let b_input = b.join(id_for("b")).join("root");
        let c_input = c.join(id_for("c")).join("root");
        let lease = with_inputs(worker(&a, Some("a")).unwrap(), &[&b_input, &c_input]).unwrap();
        let changed = r.path().join(replace);
        std::fs::rename(&changed, r.path().join("retained")).unwrap();
        private(&changed);
        assert!(lease.revalidate().is_err(), "component {replace}");
    }
}
#[test]
fn revalidation_rechecks_new_input_tails_and_uncooperative_enrollment() {
    let r = root();
    let a = owner(r.path(), "a");
    let b = owner(r.path(), "b");
    let input = b.join(id_for("b")).join("root/image.ext4");
    let lease = with_inputs(worker(&a, Some("a")).unwrap(), &[&input]).unwrap();
    std::fs::create_dir_all(input.parent().unwrap()).unwrap();
    symlink("elsewhere", &input).unwrap();
    assert!(lease.revalidate().is_err());
    let r = root();
    let a = owner(r.path(), "a");
    let b = owner(r.path(), "b");
    let input = b.join(id_for("b")).join("absent/image.ext4");
    let lease = with_inputs(worker(&a, Some("a")).unwrap(), &[&input]).unwrap();
    enroll(&b, "b"); // Deliberately bypass the protocol; this is NOT authority.
    assert!(lease.revalidate().is_err());
}
#[test]
fn ordinary_project_and_ancestor_marker_names_remain_normal() {
    let r = root();
    let project = r.path().join("project");
    private(&project);
    for directory in [r.path(), project.as_path()] {
        for marker in [
            "active.json",
            "receipt.json",
            "backup.json",
            "compat-mode",
            "compat-generation",
        ] {
            std::fs::write(directory.join(marker), b"ordinary application data").unwrap();
        }
        private(&directory.join("compat"));
    }
    let root = project.join("root");
    let lease = local(&root).unwrap();
    let lease = with_inputs(lease, &[&project, &project.join("active.json")]).unwrap();
    lease.revalidate().unwrap();
    assert!(reject_retained_path(&project).is_ok());
    assert!(local(&r.path().join("compat/root")).is_ok());
    assert!(worker(&r.path().join("workers"), Some("a")).is_ok());
}
#[test]
fn generic_markers_inside_vm_root_are_application_data_not_vm_markers() {
    let r = root();
    let base = r.path().join("workers");
    let root = base.join(id_for("a")).join("root");
    std::fs::create_dir_all(&root).unwrap();
    private(&root.join("compat"));
    for marker in [
        "active.json",
        "receipt.json",
        "backup.json",
        "compat-generation",
    ] {
        std::fs::write(root.join(marker), b"application").unwrap();
    }
    assert!(local(&root).is_ok());
    assert!(with_inputs(worker(&base, Some("a")).unwrap(), &[&root]).is_ok());
}
#[test]
fn input_traversal_symlinks_and_missing_generic_parent_refuse_without_creation() {
    let r = root();
    let base = r.path().join("workers");
    let target = r.path().join("target");
    private(&target);
    let alias = r.path().join("alias");
    symlink(&target, &alias).unwrap();
    for input in [
        alias.join("absent/image"),
        r.path().join("missing/../target"),
        r.path().join("missing/image"),
    ] {
        assert!(with_inputs(worker(&base, Some("a")).unwrap(), &[&input]).is_err());
    }
    assert!(!r.path().join("missing").exists());
}
#[test]
fn absent_plain_image_is_allowed_only_with_existing_stable_parent() {
    let r = root();
    let base = r.path().join("workers");
    let image = r.path().join("new.ext4");
    let lease = with_inputs(worker(&base, Some("a")).unwrap(), &[&image]).unwrap();
    std::fs::write(&image, b"NEW").unwrap();
    lease.revalidate().unwrap();
    assert!(ancestor_lease(&base, true).is_err());
}
#[test]
fn nested_input_identity_cannot_hide_an_outer_enrollment() {
    let r = root();
    let base = r.path().join("workers");
    enroll(&base, "b");
    let input = base
        .join(id_for("b"))
        .join("nested")
        .join(id_for("c"))
        .join("image");
    assert!(reject_retained_path(&input).is_err());
    assert!(with_inputs(worker(&base, Some("a")).unwrap(), &[&input]).is_err());
}
#[test]
fn writer_lease_keeps_original_and_input_fences_until_explicit_reap_release() {
    let r = root();
    let a = owner(r.path(), "a");
    let b = owner(r.path(), "b");
    let input = b.join(id_for("b")).join("root");
    let admission = with_inputs(worker(&a, Some("a")).unwrap(), &[&input]).unwrap();
    let writer = WriterLease::new(vec![admission]);
    assert!(ancestor_lease(&a, true).is_err());
    assert!(ancestor_lease(&b, true).is_err());
    writer.reaped();
    assert!(ancestor_lease(&a, true).is_ok());
    assert!(ancestor_lease(&b, true).is_ok());
}

#[test]
fn unknown_registry_entry_and_symlinked_input_registry_refuse() {
    for symlinked in [false, true] {
        let r = root();
        let a = owner(r.path(), "a");
        let b = owner(r.path(), "b");
        let address = custody(&b, &id_for("b")).unwrap();
        let registry = address.parent().unwrap();
        if symlinked {
            symlink("missing", registry).unwrap();
        } else {
            private(registry);
            std::fs::write(registry.join("unexpected.json"), b"unknown").unwrap();
        }
        let input = b.join(id_for("b")).join("absent/image.ext4");
        assert!(with_inputs(worker(&a, Some("a")).unwrap(), &[&input]).is_err());
    }
}
#[test]
fn whole_base_input_cannot_hide_a_sibling_registry() {
    let r = root();
    let a = owner(r.path(), "a");
    let b = owner(r.path(), "b");
    enroll(&b, "b");
    assert!(reject_retained_path(&b).is_err());
    assert!(with_inputs(worker(&a, Some("a")).unwrap(), &[&b]).is_err());
}
#[test]
fn hex_named_project_without_vm_layout_is_not_a_legacy_vm() {
    let r = root();
    let project = r.path().join(id_for("application-version"));
    private(&project);
    private(&project.join("compat"));
    for marker in [
        "active.json",
        "receipt.json",
        "backup.json",
        "compat-mode",
        "compat-generation",
    ] {
        std::fs::write(project.join(marker), b"application data").unwrap();
    }
    assert!(reject_retained_path(&project.join("src/main.rs")).is_ok());
    assert!(local(&project.join("scratch")).is_ok());
}
