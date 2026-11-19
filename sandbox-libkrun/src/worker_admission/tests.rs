// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED ONLY. No fixture in this module was compiled or run in source stage.
use super::*;
use std::{
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{PermissionsExt, symlink},
    },
};
struct Fixture {
    _temp: tempfile::TempDir,
    catalog: PathBuf,
    data: PathBuf,
    context: AdmissionContext,
}
fn private(path: &Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let catalog = temp.path().join("catalog");
    private(&catalog);
    let data = temp.path().join("data");
    private(&data);
    let context = testing::provision_context(&catalog).unwrap();
    Fixture {
        _temp: temp,
        catalog,
        data,
        context,
    }
}
fn id(key: &str) -> String {
    hex::encode(Sha256::digest(key))[..32].to_string()
}
// Exercises the real interval exclusion primitive, NOT a migration proof issuer.
fn exclusive(context: &AdmissionContext, path: &Path) -> io::Result<Lease> {
    start(
        context.namespace.clone(),
        vec![entry(path, PathRole::MutableTree)?],
        None,
        true,
    )
}
fn mutable(lease: Lease, path: &Path, role: PathRole) -> Lease {
    with_paths(lease, &[PathRequest { path, role }]).unwrap()
}
fn claim(path: &Path) -> File {
    let file = File::open(path).unwrap();
    assert_eq!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    file
}

#[test]
fn c1_nested_whole_base_remains_fenced_before_and_after_hierarchy_creation() {
    let f = fixture();
    let a = f.data.join("workers");
    let b = a.join("nestedbase");
    let lease = f.context.worker(&a, Some("a")).unwrap();
    let lease = mutable(lease, &b, PathRole::MutableTree);
    assert!(exclusive(&f.context, &b.join(id("b"))).is_err());
    // Creating a's unrelated ancestor is outside b's mutation footprint. The
    // caller declares the encompassing new tree, rather than unprotected mkdir.
    drop(lease);
    let lease = f.context.local(&a).unwrap();
    let mut guard = lease
        .begin(MutationPlan::Create { root: a.clone() })
        .unwrap();
    guard.create_dir_all(&b, 0o700).unwrap();
    guard.commit().unwrap();
    assert!(exclusive(&f.context, &b).is_err());
    lease.revalidate().unwrap();
    drop(lease);
    assert!(exclusive(&f.context, &b).is_ok());
}
#[test]
fn c1_registration_cannot_evade_an_existing_ancestor_interval() {
    let f = fixture();
    let base = f.data.join("new/nestedbase");
    let lease = f.context.local(&f.data.join("new")).unwrap();
    assert!(f.context.worker(&base, Some("b")).is_err());
    drop(lease);
    assert!(f.context.worker(&base, Some("b")).is_ok());
}
#[test]
fn c2_chained_inputs_in_both_orders_never_lock_claim_roots() {
    for reverse in [false, true] {
        let f = fixture();
        let a = f.data.join("a");
        let b = f.data.join("b");
        private(&a);
        private(&b);
        let cache = b.join("cache.erofs");
        std::fs::write(&cache, b"lower").unwrap();
        let vm = b.join(id("b")).join("root");
        let (first, second) = if reverse {
            (&vm, &cache)
        } else {
            (&cache, &vm)
        };
        let lease = with_inputs(f.context.worker(&a, Some("a")).unwrap(), &[first]).unwrap();
        let original = lease.clone();
        let lease = with_inputs(lease, &[second]).unwrap();
        assert!(Arc::ptr_eq(&lease.0, &original.0));
        let _a_claim = claim(&a);
        let _b_claim = claim(&b);
        assert!(exclusive(&f.context, &b).is_err());
    }
}
#[test]
fn c3_cold_output_image_pair_commits_and_clones_observe_core_receipts() {
    let f = fixture();
    let out = f.data.join("output");
    let image = out.join("image.erofs");
    let image_id = out.join("image.id");
    let lease = f.context.local(&out).unwrap();
    let lease = with_paths(
        lease,
        &[
            PathRequest {
                path: &image,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &image_id,
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap();
    let clone = lease.clone();
    let mut guard = lease
        .begin(MutationPlan::ImagePair {
            directory: out.clone(),
        })
        .unwrap();
    guard.create_dir_all(&out, 0o700).unwrap();
    let mut stage = guard.stage_file(&image, 0o600).unwrap();
    stage.file_mut().write_all(b"NEW").unwrap();
    guard.publish_file(stage).unwrap();
    let mut stage = guard.stage_file(&image_id, 0o600).unwrap();
    stage.file_mut().write_all(b"new-id").unwrap();
    guard.publish_file(stage).unwrap();
    assert!(clone.use_path(&image).is_err());
    guard.commit().unwrap();
    clone.revalidate().unwrap();
    assert_eq!(std::fs::read(&image).unwrap(), b"NEW");
    assert!(exclusive(&f.context, &out).is_err());
    drop((lease, clone));
    assert!(exclusive(&f.context, &out).is_ok());
}
#[test]
fn c3_local_new_state_runtime_and_cold_cache_slugs_use_stable_intervals() {
    for name in ["state", "cache"] {
        let f = fixture();
        let root = f.data.join(name);
        let child = root.join("runtime/slug");
        let lower = root.join("slug.erofs");
        let lease = f.context.local(&root).unwrap();
        let lease = with_paths(
            lease,
            &[
                PathRequest {
                    path: &child,
                    role: PathRole::MutableTree,
                },
                PathRequest {
                    path: &lower,
                    role: PathRole::MutableFile,
                },
            ],
        )
        .unwrap();
        let mut guard = lease
            .begin(MutationPlan::Cache { root: root.clone() })
            .unwrap();
        assert!(exclusive(&f.context, &root).is_err());
        guard.create_dir_all(&child, 0o700).unwrap();
        let mut stage = guard.stage_file(&lower, 0o600).unwrap();
        stage.file_mut().write_all(b"lower").unwrap();
        guard.publish_file(stage).unwrap();
        guard.commit().unwrap();
        lease.revalidate().unwrap();
        assert!(exclusive(&f.context, &child).is_err());
    }
}
#[test]
fn c4_warm_file_publication_retains_old_copy_and_updates_only_owned_epoch() {
    let f = fixture();
    let output = f.data.join("image.erofs");
    std::fs::write(&output, b"OLD").unwrap();
    let lease = f.context.local(&f.data.join("state")).unwrap();
    let lease = mutable(lease, &output, PathRole::MutableFile);
    let clone = lease.clone();
    let spool = f.data.join("spool-slot");
    let lease = mutable(lease, &spool, PathRole::MutableFile);
    let mut guard = lease
        .begin(MutationPlan::Erofs {
            output: output.clone(),
            spool: spool.clone(),
        })
        .unwrap();
    let temporary = guard.stage_file(&spool, 0o600).unwrap();
    guard.discard_file(temporary).unwrap();
    let mut stage = guard.stage_file(&output, 0o600).unwrap();
    stage.file_mut().write_all(b"NEW").unwrap();
    guard.publish_file(stage).unwrap();
    guard.commit().unwrap();
    clone.revalidate().unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), b"NEW");
    let prior = std::fs::read_dir(&f.data)
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.file_name().to_string_lossy().starts_with(".iii-prior-"))
        .unwrap();
    assert_eq!(std::fs::read(prior.path()).unwrap(), b"OLD");
}
#[test]
fn c4_external_replacement_poison_is_terminal_and_never_refreshes_strict_input() {
    let f = fixture();
    let output = f.data.join("image");
    std::fs::write(&output, b"OLD").unwrap();
    let lease = mutable(
        f.context.local(&f.data.join("state")).unwrap(),
        &output,
        PathRole::MutableFile,
    );
    let spool = f.data.join("spool-slot");
    let lease = mutable(lease, &spool, PathRole::MutableFile);
    let mut guard = lease
        .begin(MutationPlan::Erofs {
            output: output.clone(),
            spool: spool.clone(),
        })
        .unwrap();
    let temporary = guard.stage_file(&spool, 0o600).unwrap();
    guard.discard_file(temporary).unwrap();
    let mut stage = guard.stage_file(&output, 0o600).unwrap();
    stage.file_mut().write_all(b"NEW").unwrap();
    std::fs::rename(&output, f.data.join("retained-old")).unwrap();
    std::fs::write(&output, b"FOREIGN").unwrap();
    assert!(guard.publish_file(stage).is_err());
    assert!(guard.checkpoint().is_err());
    drop(guard);
    assert!(lease.revalidate().is_err());
    drop(lease);
    assert!(exclusive(&f.context, &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"FOREIGN");
}
#[test]
fn c4_adopt_preserves_root_and_descendant_pins_then_allows_new_root_creation() {
    let f = fixture();
    let vm = f.data.join("vm");
    private(&vm);
    let root = vm.join("root");
    private(&root);
    let upper = root.join("upper.ext4");
    std::fs::write(&upper, b"OLD").unwrap();
    let lease = f.context.local(&vm).unwrap();
    let lease = mutable(lease, &upper, PathRole::MutableFile);
    let lease = mutable(lease, &root, PathRole::MutableTree);
    let mut guard = lease
        .begin(MutationPlan::Adopt { root: vm.clone() })
        .unwrap();
    guard
        .rename_preserving(&root, &vm.join("root.pre-build"))
        .unwrap();
    guard.create_dir_all(&root, 0o700).unwrap();
    guard.commit().unwrap();
    lease.revalidate().unwrap();
    assert_eq!(
        std::fs::read(vm.join("root.pre-build/upper.ext4")).unwrap(),
        b"OLD"
    );
}
#[test]
fn c4_reclone_tree_uses_owned_stage_and_refuses_other_mutations_stage() {
    let f = fixture();
    let root = f.data.join("root");
    private(&root);
    std::fs::write(root.join("old"), b"OLD").unwrap();
    let lease = f.context.local(&root).unwrap();
    let mut guard = lease
        .begin(MutationPlan::Reclone { root: root.clone() })
        .unwrap();
    let stage = guard.stage_tree(&root).unwrap();
    std::fs::write(stage.path().join("new"), b"NEW").unwrap();
    guard.publish_tree(stage).unwrap();
    guard.commit().unwrap();
    lease.revalidate().unwrap();
    assert_eq!(std::fs::read(root.join("new")).unwrap(), b"NEW");
}
#[test]
fn c5_whole_vm_legacy_refuses_with_persistent_registered_base_context() {
    let f = fixture();
    let base = f.data.join("workers");
    private(&base);
    let admission = f.context.worker(&base, Some("a")).unwrap();
    let vm = base.join(id("b"));
    private(&vm);
    private(&vm.join("compat"));
    assert!(with_inputs(admission.clone(), &[&vm]).is_err());
    assert!(
        testing::open_context(&f.catalog)
            .unwrap()
            .reject_retained_path(&vm)
            .is_err()
    );
    let application = f.data.join(id("app"));
    private(&application);
    private(&application.join("compat"));
    std::fs::write(application.join("active.json"), b"ordinary").unwrap();
    assert!(with_inputs(admission, &[&application]).is_ok());
}
#[test]
fn sibling_enrollment_refuses_original_storage_with_missing_tail() {
    let f = fixture();
    let base = f.data.join("workers");
    drop(f.context.worker(&base, Some("register")).unwrap());
    let address = custody(&base, &id("b")).unwrap();
    private(address.parent().unwrap());
    private(&address);
    let input = base.join(id("b")).join("absent/root/upper.ext4");
    assert!(with_inputs(f.context.worker(&base, Some("a")).unwrap(), &[&input]).is_err());
}
#[test]
fn generic_marker_names_in_project_and_unrelated_ancestor_remain_normal() {
    let f = fixture();
    for directory in [&f.data] {
        for name in [
            "active.json",
            "receipt.json",
            "backup.json",
            "compat-mode",
            "compat-generation",
        ] {
            std::fs::write(directory.join(name), b"app").unwrap();
        }
        private(&directory.join("compat"));
    }
    let app = f.data.join("app");
    private(&app);
    let lease = f.context.local(&f.data.join("state")).unwrap();
    let lease = with_inputs(lease, &[&app]).unwrap();
    lease.revalidate().unwrap();
}
#[test]
fn empty_and_failed_composition_preserve_original_arc_and_prior_coverage() {
    let f = fixture();
    let root = f.data.join("state");
    let lease = f.context.local(&root).unwrap();
    assert!(Arc::ptr_eq(
        &lease.0,
        &with_inputs(lease.clone(), &[]).unwrap().0
    ));
    let conflict = f.data.join("busy");
    let _other = exclusive(&f.context, &conflict).unwrap();
    assert!(with_inputs(lease.clone(), &[&f.data.join("free"), &conflict]).is_err());
    assert!(exclusive(&f.context, &root).is_err());
    assert!(exclusive(&f.context, &f.data.join("free")).is_ok());
}
#[test]
fn independent_owners_and_shared_strict_inputs_remain_concurrent() {
    let f = fixture();
    let source = f.data.join("source");
    private(&source);
    let a = with_inputs(f.context.local(&f.data.join("a")).unwrap(), &[&source]).unwrap();
    let b = with_inputs(f.context.local(&f.data.join("b")).unwrap(), &[&source]).unwrap();
    assert!(exclusive(&f.context, &source).is_err());
    drop(a);
    assert!(exclusive(&f.context, &source).is_err());
    drop(b);
    assert!(exclusive(&f.context, &source).is_ok());
}
#[test]
fn mutation_blocks_read_of_same_path_but_not_disjoint_strict_source() {
    let f = fixture();
    let output = f.data.join("out");
    let source = f.data.join("source");
    std::fs::write(&source, b"input").unwrap();
    let lease = with_inputs(f.context.local(&output).unwrap(), &[&source]).unwrap();
    let guard = lease
        .begin(MutationPlan::Create {
            root: output.clone(),
        })
        .unwrap();
    assert!(lease.use_path(&output).is_err());
    let read = lease.use_path(&source).unwrap();
    read.revalidate().unwrap();
    drop(read);
    drop(guard);
    lease.revalidate().unwrap();
}
#[test]
fn strict_role_cannot_be_upgraded_and_external_appearance_is_not_core_creation() {
    let f = fixture();
    let input = f.data.join("absent");
    let lease = with_inputs(f.context.local(&f.data.join("state")).unwrap(), &[&input]).unwrap();
    assert!(
        with_paths(
            lease.clone(),
            &[PathRequest {
                path: &input,
                role: PathRole::MutableFile
            }]
        )
        .is_err()
    );
    std::fs::write(&input, b"foreign").unwrap();
    assert!(lease.revalidate().is_err());
}
#[test]
fn cancelled_actual_mutation_leaves_durable_uncertain_fence_after_drop() {
    let f = fixture();
    let out = f.data.join("output");
    let lease = f.context.local(&out).unwrap();
    let mut guard = lease
        .begin(MutationPlan::Create { root: out.clone() })
        .unwrap();
    guard.create_dir_all(&out, 0o700).unwrap();
    drop(guard);
    drop(lease);
    assert!(exclusive(&testing::open_context(&f.catalog).unwrap(), &out).is_err());
}
#[test]
fn writer_lease_explicit_reap_and_clones_keep_same_operation_until_last_release() {
    let f = fixture();
    let root = f.data.join("root");
    let lease = f.context.local(&root).unwrap();
    let clone = lease.clone();
    let writer = WriterLease::new(vec![lease]);
    writer.reaped();
    assert!(exclusive(&f.context, &root).is_err());
    drop(clone);
    assert!(exclusive(&f.context, &root).is_ok());
}
#[test]
fn inherited_description_imports_same_union_and_never_unlocks_parent() {
    let f = fixture();
    let root = f.data.join("root");
    let lease = f.context.local(&root).unwrap();
    let child = lease.export_child().unwrap();
    let duplicate = child.duplicate_for_exec().unwrap();
    let imported = testing::open_context(&f.catalog)
        .unwrap()
        .import_child(duplicate)
        .unwrap();
    let extra = f.data.join("extra");
    let imported = with_inputs(imported, &[&extra]).unwrap();
    assert_eq!(lease.0.id, imported.0.id);
    assert!(exclusive(&f.context, &extra).is_err());
    drop(imported);
    assert!(exclusive(&f.context, &root).is_err());
    drop(child);
    drop(lease);
    assert!(exclusive(&f.context, &root).is_ok());
}
#[test]
fn independently_opened_sentinel_is_not_an_inherited_child_handle() {
    let f = fixture();
    let lease = f.context.local(&f.data.join("root")).unwrap();
    let fresh = f
        .context
        .namespace
        .directory
        .file(&format!("op-{}", lease.0.id), false)
        .unwrap();
    assert!(f.context.import_child(fresh.into()).is_err());
    lease.revalidate().unwrap();
}
#[test]
fn exported_fd_keeps_fence_even_if_parent_lease_is_dropped_before_import() {
    let f = fixture();
    let root = f.data.join("root");
    let lease = f.context.local(&root).unwrap();
    let child = lease.export_child().unwrap();
    let fd = child.duplicate_for_exec().unwrap();
    drop(child);
    drop(lease);
    assert!(exclusive(&f.context, &root).is_err());
    assert!(f.context.import_child(fd).is_err()); // closed record is not revived
    assert!(exclusive(&f.context, &root).is_ok());
}
#[test]
fn namespaces_are_explicit_isolated_and_cannot_be_payloads() {
    let a = fixture();
    let b = fixture();
    let path = a.data.join("shared-name");
    let lease = a.context.local(&path).unwrap();
    assert!(b.context.local(&path).is_ok());
    assert!(
        testing::open_context(&a.catalog)
            .unwrap()
            .local(&path)
            .is_err()
    );
    assert!(with_inputs(lease, &[&a.catalog]).is_err());
}
#[test]
fn open_context_missing_namespace_does_not_provision() {
    let f = tempfile::tempdir().unwrap();
    assert!(testing::open_context(f.path()).is_err());
    assert_eq!(std::fs::read_dir(f.path()).unwrap().count(), 0);
}
#[test]
fn unknown_corrupt_hardlinked_and_replaced_catalog_fail_closed() {
    for kind in ["unknown", "corrupt", "hardlink", "replace"] {
        let f = fixture();
        let catalog = f.catalog.join("catalog.json");
        match kind {
            "unknown" => std::fs::write(f.catalog.join("foreign"), b"?").unwrap(),
            "corrupt" => std::fs::write(&catalog, b"{broken").unwrap(),
            "hardlink" => std::fs::hard_link(&catalog, f.data.join("alias")).unwrap(),
            _ => {
                std::fs::rename(&catalog, f.data.join("prior")).unwrap();
                std::fs::write(&catalog, b"{}").unwrap();
            }
        }
        assert!(f.context.local(&f.data.join("root")).is_err());
    }
}
#[test]
fn root_replacement_symlinks_and_permissions_are_never_normal_namespace() {
    let f = fixture();
    let lease = f.context.local(&f.data.join("root")).unwrap();
    std::fs::rename(&f.catalog, f.catalog.with_extension("old")).unwrap();
    private(&f.catalog);
    assert!(lease.revalidate().is_err());
    let f = fixture();
    std::fs::set_permissions(&f.catalog, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(testing::open_context(&f.catalog).is_err());
    let f = fixture();
    let alias = f.data.join("namespace-alias");
    symlink(&f.catalog, &alias).unwrap();
    assert!(testing::open_context(&alias).is_err());
}
#[test]
fn symlink_traversal_hardlinked_strict_file_and_special_file_refuse() {
    let f = fixture();
    let source = f.data.join("source");
    std::fs::write(&source, b"data").unwrap();
    let alias = f.data.join("alias");
    symlink(&source, &alias).unwrap();
    assert!(f.context.reject_retained_path(&alias).is_err());
    assert!(f.context.local(&f.data.join("missing/../root")).is_err());
    std::fs::hard_link(&source, f.data.join("second")).unwrap();
    assert!(f.context.reject_retained_path(&source).is_err());
}
#[test]
fn bounded_catalog_and_conflict_checks_do_not_omit_excess_inputs() {
    let f = fixture();
    let lease = f.context.local(&f.data.join("state")).unwrap();
    let paths: Vec<_> = (0..catalog::MAX_PATHS)
        .map(|n| f.data.join(format!("p{n}")))
        .collect();
    let refs: Vec<_> = paths.iter().map(PathBuf::as_path).collect();
    assert!(with_inputs(lease.clone(), &refs).is_err());
    assert_eq!(
        f.context
            .namespace
            .transaction()
            .unwrap()
            .operation(&lease.0.id)
            .unwrap()
            .entries
            .len(),
        1
    );
}
#[test]
fn orphaned_nonclosed_record_is_not_reconciled_as_retired_writer() {
    let f = fixture();
    let path = f.data.join("root");
    let lease = f.context.local(&path).unwrap();
    {
        let mut tx = f.context.namespace.transaction().unwrap();
        let op = tx.operation_mut(&lease.0.id).unwrap();
        op.holders += 1;
        op.holder_ids.push(new_id());
        tx.save().unwrap();
    }
    drop(lease);
    assert!(exclusive(&f.context, &path).is_err());
}

#[test]
fn diagnostic_pid_clear_preserves_inode_and_requires_owned_flock() {
    let f = fixture();
    let root = f.data.join("cache");
    private(&root);
    let lock = root.join("image.lock");
    std::fs::write(&lock, b"long prior diagnostic contents").unwrap();
    let before = std::fs::metadata(&lock).unwrap().ino();
    let lease = mutable(
        f.context.local(&root).unwrap(),
        &lock,
        PathRole::MutableFile,
    );
    let lease = mutable(lease, &root.join("image"), PathRole::MutableFile);
    let mut guard = lease.begin(MutationPlan::Cache { root }).unwrap();
    let mut diagnostic = guard.diagnostic_lock(&lock).unwrap();
    assert!(diagnostic.try_lock_exclusive().unwrap());
    diagnostic.write_pid(123).unwrap();
    assert_eq!(std::fs::read(&lock).unwrap(), b"123\n");
    diagnostic.clear().unwrap();
    drop(diagnostic);
    guard.commit().unwrap();
    assert_eq!(std::fs::metadata(&lock).unwrap().ino(), before);
    assert!(std::fs::read(&lock).unwrap().is_empty());
    lease.revalidate().unwrap();
}
#[test]
fn diagnostic_contention_returns_false_without_write_or_mutex_retention() {
    let f = fixture();
    let root = f.data.join("cache");
    private(&root);
    let lock = root.join("image.lock");
    std::fs::write(&lock, b"owner").unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let lease = mutable(
        f.context.local(&root).unwrap(),
        &lock,
        PathRole::MutableFile,
    );
    let lease = mutable(lease, &root.join("image"), PathRole::MutableFile);
    let mut guard = lease.begin(MutationPlan::Cache { root }).unwrap();
    let mut diagnostic = guard.diagnostic_lock(&lock).unwrap();
    assert!(!diagnostic.try_lock_exclusive().unwrap());
    assert_eq!(std::fs::read(&lock).unwrap(), b"owner");
    assert!(f.context.namespace.transaction().is_ok());
    drop(held);
    diagnostic.lock_exclusive().unwrap();
    diagnostic.clear().unwrap();
    drop(diagnostic);
    guard.commit().unwrap();
}
#[test]
fn diagnostic_commit_refuses_outstanding_handle_and_external_replacement_is_terminal() {
    let f = fixture();
    let root = f.data.join("cache");
    private(&root);
    let lock = root.join("image.lock");
    let lease = mutable(
        f.context.local(&root).unwrap(),
        &lock,
        PathRole::MutableFile,
    );
    let lease = mutable(lease, &root.join("image"), PathRole::MutableFile);
    let mut guard = lease.begin(MutationPlan::Cache { root }).unwrap();
    let diagnostic = guard.diagnostic_lock(&lock).unwrap();
    assert!(guard.commit().is_err());
    assert!(diagnostic.revalidate().is_err());
    drop(diagnostic);
    assert!(lease.revalidate().is_err());
}
#[test]
fn diagnostic_refuses_undeclared_sidecar_symlink_mode_and_write_without_acquisition() {
    let f = fixture();
    let root = f.data.join("cache");
    private(&root);
    let lease = f.context.local(&root).unwrap();
    let mut guard = lease
        .begin(MutationPlan::Cache { root: root.clone() })
        .unwrap();
    assert!(guard.diagnostic_lock(&root.join("anything.lock")).is_err());
    let f = fixture();
    let root = f.data.join("cache");
    private(&root);
    let lock = root.join("image.lock");
    let lease = mutable(
        f.context.local(&root).unwrap(),
        &lock,
        PathRole::MutableFile,
    );
    let lease = mutable(lease, &root.join("image"), PathRole::MutableFile);
    let mut guard = lease.begin(MutationPlan::Cache { root }).unwrap();
    let mut diagnostic = guard.diagnostic_lock(&lock).unwrap();
    assert!(diagnostic.write_pid(7).is_err());
    assert!(diagnostic.try_lock_exclusive().is_err());
    assert!(std::fs::read(&lock).unwrap().is_empty());
}

// Socket fixtures below are AUTHORED only; source delivery did not bind sockets.
fn endpoint_lease(f: &Fixture, path: &Path) -> Lease {
    start(
        f.context.namespace.clone(),
        vec![entry(path, PathRole::Endpoint).unwrap()],
        None,
        false,
    )
    .unwrap()
}
#[test]
fn c6_live_endpoint_refuses_unknown_replacement_and_owned_close_cleans_exact_inode() {
    let f = fixture();
    let private_parent = f.data.join("endpoints");
    private(&private_parent);
    let path = private_parent.join("shell.sock");
    let lease = endpoint_lease(&f, &path);
    let bound = lease.endpoint(&path).unwrap().bind().unwrap();
    assert!(
        start(
            f.context.namespace.clone(),
            vec![entry(&path, PathRole::Endpoint).unwrap()],
            None,
            false
        )
        .is_err()
    );
    bound.close().unwrap();
    assert!(!path.exists());
    lease.revalidate().unwrap();
}
#[test]
fn c6_unrecorded_stale_socket_and_foreign_plain_file_are_not_deleted() {
    let f = fixture();
    let parent = f.data.join("endpoints");
    private(&parent);
    let path = parent.join("shell.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    drop(listener);
    let lease = endpoint_lease(&f, &path);
    assert!(lease.endpoint(&path).unwrap().bind().is_err());
    assert!(path.exists());
    let f = fixture();
    let path = f.data.join("not-socket");
    std::fs::write(&path, b"foreign").unwrap();
    let lease = endpoint_lease(&f, &path);
    assert!(lease.endpoint(&path).unwrap().bind().is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"foreign");
}
#[test]
fn c6_verified_receipt_and_released_previous_sentinel_allow_restart() {
    let f = fixture();
    let parent = f.data.join("endpoints");
    private(&parent);
    let path = parent.join("shell.sock");
    let lease = endpoint_lease(&f, &path);
    let bound = lease.endpoint(&path).unwrap().bind().unwrap();
    // cfg(test) closes the only listener then releases endpoint OFD ownership;
    // this models crash evidence, not a production retirement API.
    super::endpoint::abandon_closed_test_listener(bound);
    drop(lease);
    let restarted = endpoint_lease(&f, &path);
    restarted
        .endpoint(&path)
        .unwrap()
        .bind()
        .unwrap()
        .close()
        .unwrap();
}
#[test]
fn c6_endpoint_parent_must_be_private_and_outside_payload_scopes() {
    let f = fixture();
    let root = f.data.join("guest");
    private(&root);
    let path = root.join("shell.sock");
    assert!(
        with_paths(
            f.context.local(&root).unwrap(),
            &[PathRequest {
                path: &path,
                role: PathRole::Endpoint
            }]
        )
        .is_err()
    );
}

#[test]
fn discard_own_spool_preserves_target_and_allows_image_publication() {
    for existing in [false, true] {
        let f = fixture();
        let root = f.data.join("output");
        private(&root);
        let spool = root.join("spool-slot");
        let image = root.join("image.erofs");
        if existing {
            std::fs::write(&spool, b"OLD TARGET").unwrap();
        }
        let before = std::fs::symlink_metadata(&spool).ok().map(|m| m.ino());
        let lease = with_paths(
            f.context.local(&root).unwrap(),
            &[
                PathRequest {
                    path: &spool,
                    role: PathRole::MutableFile,
                },
                PathRequest {
                    path: &image,
                    role: PathRole::MutableFile,
                },
            ],
        )
        .unwrap();
        let mut guard = lease
            .begin(MutationPlan::ImagePair { directory: root })
            .unwrap();
        let mut temporary = guard.stage_file(&spool, 0o600).unwrap();
        temporary.file_mut().write_all(b"spooled").unwrap();
        let stage_path = temporary.path().to_path_buf();
        let mut output = guard.stage_file(&image, 0o600).unwrap();
        output.file_mut().write_all(b"IMAGE").unwrap();
        // No cloned writer/spool handles survive this point in the fixture.
        guard.discard_file(temporary).unwrap();
        assert!(!stage_path.exists());
        assert_eq!(
            std::fs::symlink_metadata(&spool).ok().map(|m| m.ino()),
            before
        );
        if existing {
            assert_eq!(std::fs::read(&spool).unwrap(), b"OLD TARGET");
        }
        guard.publish_file(output).unwrap();
        guard.commit().unwrap();
        lease.revalidate().unwrap();
        assert_eq!(std::fs::read(&image).unwrap(), b"IMAGE");
        let tx = f.context.namespace.transaction().unwrap();
        let op = tx.operation(&lease.0.id).unwrap();
        assert!(
            op.journals
                .iter()
                .any(|j| j.committed && j.disposals.len() == 1 && j.disposals[0].discarded)
        );
    }
}
#[test]
fn discard_boundary_failures_remain_terminal_and_never_promote_output() {
    for boundary in [
        "before-intent",
        "after-intent",
        "after-unlink",
        "after-sync",
        "after-completion",
    ] {
        let f = fixture();
        let root = f.data.join("output");
        private(&root);
        let spool = root.join("spool");
        let image = root.join("image");
        let lease = with_paths(
            f.context.local(&root).unwrap(),
            &[
                PathRequest {
                    path: &spool,
                    role: PathRole::MutableFile,
                },
                PathRequest {
                    path: &image,
                    role: PathRole::MutableFile,
                },
            ],
        )
        .unwrap();
        let mut guard = lease
            .begin(MutationPlan::ImagePair { directory: root })
            .unwrap();
        let stage = guard.stage_file(&spool, 0o600).unwrap();
        let name = stage.path().to_path_buf();
        let output = guard.stage_file(&image, 0o600).unwrap();
        guard.fault_discard_at(boundary);
        assert!(guard.discard_file(stage).is_err());
        assert!(guard.publish_file(output).is_err());
        assert!(guard.commit().is_err());
        assert!(!image.exists());
        let tx = f.context.namespace.transaction().unwrap();
        let op = tx.operation(&lease.0.id).unwrap();
        assert!(op.uncertain);
        if boundary == "before-intent" {
            assert!(name.exists());
            assert!(op.journals[0].disposals.is_empty());
        } else {
            assert_eq!(op.journals[0].disposals.len(), 1);
        }
    }
}
#[test]
fn abandoned_stage_is_not_silently_discarded_by_drop_or_commit() {
    let f = fixture();
    let root = f.data.join("out");
    private(&root);
    let slot = root.join("spool");
    let lease = mutable(
        f.context.local(&root).unwrap(),
        &slot,
        PathRole::MutableFile,
    );
    let mut guard = lease
        .begin(MutationPlan::ImagePair { directory: root })
        .unwrap();
    let stage = guard.stage_file(&slot, 0o600).unwrap();
    let path = stage.path().to_path_buf();
    drop(stage);
    assert!(guard.commit().is_err());
    assert!(path.exists());
    assert!(!slot.exists());
}
#[test]
fn discard_foreign_context_or_replaced_stage_never_deletes_unknown_bytes() {
    let a = fixture();
    let b = fixture();
    let ar = a.data.join("out");
    let br = b.data.join("out");
    private(&ar);
    private(&br);
    let ap = ar.join("spool");
    let bp = br.join("spool");
    let al = mutable(a.context.local(&ar).unwrap(), &ap, PathRole::MutableFile);
    let bl = mutable(b.context.local(&br).unwrap(), &bp, PathRole::MutableFile);
    let mut ag = al.begin(MutationPlan::ImagePair { directory: ar }).unwrap();
    let mut bg = bl.begin(MutationPlan::ImagePair { directory: br }).unwrap();
    let foreign = ag.stage_file(&ap, 0o600).unwrap();
    let foreign_path = foreign.path().to_path_buf();
    assert!(bg.discard_file(foreign).is_err());
    assert!(foreign_path.exists());
    let f = fixture();
    let root = f.data.join("out");
    private(&root);
    let slot = root.join("spool");
    let lease = mutable(
        f.context.local(&root).unwrap(),
        &slot,
        PathRole::MutableFile,
    );
    let mut guard = lease
        .begin(MutationPlan::ImagePair { directory: root })
        .unwrap();
    let stage = guard.stage_file(&slot, 0o600).unwrap();
    let path = stage.path().to_path_buf();
    std::fs::rename(&path, path.with_extension("retained")).unwrap();
    std::fs::write(&path, b"REPLACEMENT").unwrap();
    assert!(guard.discard_file(stage).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"REPLACEMENT");
}
#[test]
fn discard_refuses_hardlinked_stage_and_missing_explicit_slot() {
    let f = fixture();
    let root = f.data.join("out");
    private(&root);
    let slot = root.join("spool");
    let lease = mutable(
        f.context.local(&root).unwrap(),
        &slot,
        PathRole::MutableFile,
    );
    let mut guard = lease
        .begin(MutationPlan::ImagePair { directory: root })
        .unwrap();
    let stage = guard.stage_file(&slot, 0o600).unwrap();
    let path = stage.path().to_path_buf();
    std::fs::hard_link(&path, f.data.join("second-name")).unwrap();
    assert!(guard.discard_file(stage).is_err());
    assert!(path.exists());
    let f = fixture();
    let root = f.data.join("out");
    private(&root);
    let lease = f.context.local(&root).unwrap();
    let mut guard = lease
        .begin(MutationPlan::ImagePair {
            directory: root.clone(),
        })
        .unwrap();
    let stage = guard
        .stage_file(&root.join("undeclared-slot"), 0o600)
        .unwrap();
    let path = stage.path().to_path_buf();
    assert!(guard.discard_file(stage).is_err());
    assert!(path.exists());
}
#[test]
fn reopened_context_cannot_import_another_namespace_sentinel() {
    let a = fixture();
    let b = fixture();
    let lease = a.context.local(&a.data.join("root")).unwrap();
    let child = lease.export_child().unwrap();
    assert!(
        b.context
            .import_child(child.duplicate_for_exec().unwrap())
            .is_err()
    );
    lease.revalidate().unwrap();
}
#[test]
fn mutex_contention_is_nonblocking_and_does_not_touch_payload() {
    let f = fixture();
    let held = f
        .context
        .namespace
        .directory
        .file("catalog.lock", false)
        .unwrap();
    flock(&held).unwrap();
    let root = f.data.join("absent");
    assert!(f.context.local(&root).is_err());
    assert!(!root.exists());
    drop(held);
    assert!(f.context.local(&root).is_ok());
}

fn recovery_request(f: &Fixture, operation: &str, action: RecoveryAction) -> RecoveryRequest {
    let tx = f.context.namespace.transaction().unwrap();
    RecoveryRequest {
        operation_id: operation.into(),
        expected_record_sha256: recovery::digest(tx.operation(operation).unwrap()).unwrap(),
        action,
    }
}
fn erofs_lease(f: &Fixture, output: &Path, spool: &Path) -> Lease {
    with_paths(
        f.context.local(&f.data.join("state")).unwrap(),
        &[
            PathRequest {
                path: output,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: spool,
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap()
}
#[test]
fn amendment_c_erofs_exact_union_blocks_readers_of_either_then_resolves_both() {
    let f = fixture();
    let output = f.data.join("image.erofs");
    let spool = f.data.join("spool");
    std::fs::write(&output, b"OLD").unwrap();
    std::fs::write(&spool, b"OLD SPOOL SLOT").unwrap();
    let lease = erofs_lease(&f, &output, &spool);
    for path in [&output, &spool] {
        let read = lease.use_path(path).unwrap();
        assert!(
            lease
                .begin(MutationPlan::Erofs {
                    output: output.clone(),
                    spool: spool.clone()
                })
                .is_err()
        );
        let tx = f.context.namespace.transaction().unwrap();
        assert_eq!(tx.operation(&lease.0.id).unwrap().holds.len(), 1);
        drop(tx);
        drop(read);
    }
    let mut guard = lease
        .begin(MutationPlan::Erofs {
            output: output.clone(),
            spool: spool.clone(),
        })
        .unwrap();
    assert!(exclusive(&f.context, &output).is_err());
    assert!(exclusive(&f.context, &spool).is_err());
    let temporary = guard.stage_file(&spool, 0o600).unwrap();
    let mut image = guard.stage_file(&output, 0o600).unwrap();
    image.file_mut().write_all(b"NEW").unwrap();
    guard.discard_file(temporary).unwrap();
    guard.publish_file(image).unwrap();
    guard.commit().unwrap();
    lease.revalidate().unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), b"NEW");
    assert_eq!(std::fs::read(&spool).unwrap(), b"OLD SPOOL SLOT");
}
#[test]
fn amendment_c_erofs_missing_wrong_role_equal_alias_and_third_sibling_refuse() {
    let f = fixture();
    let output = f.data.join("image");
    let spool = f.data.join("spool");
    let lease = mutable(
        f.context.local(&f.data.join("state")).unwrap(),
        &output,
        PathRole::MutableFile,
    );
    assert!(
        lease
            .begin(MutationPlan::Erofs {
                output: output.clone(),
                spool: spool.clone()
            })
            .is_err()
    );
    let lease = mutable(lease, &spool, PathRole::MutableTree);
    assert!(
        lease
            .begin(MutationPlan::Erofs {
                output: output.clone(),
                spool: spool.clone()
            })
            .is_err()
    );
    let f = fixture();
    let output = f.data.join("image");
    let spool = f.data.join("spool");
    let lease = erofs_lease(&f, &output, &spool);
    assert!(
        lease
            .begin(MutationPlan::Erofs {
                output: output.clone(),
                spool: output.clone()
            })
            .is_err()
    );
    let mut guard = lease
        .begin(MutationPlan::Erofs {
            output: output.clone(),
            spool,
        })
        .unwrap();
    assert!(guard.stage_file(&f.data.join("third"), 0o600).is_err());
    assert!(guard.commit().is_err());
    let f = fixture();
    let output = f.data.join("image");
    let spool = f.data.join("spool");
    std::fs::write(&output, b"OLD").unwrap();
    std::fs::hard_link(&output, &spool).unwrap();
    assert!(
        with_paths(
            f.context.local(&f.data.join("state")).unwrap(),
            &[
                PathRequest {
                    path: &output,
                    role: PathRole::MutableFile
                },
                PathRequest {
                    path: &spool,
                    role: PathRole::MutableFile
                }
            ]
        )
        .is_err()
    );
}
#[test]
fn amendment_c_erofs_strict_alias_and_discard_failure_prevent_publication() {
    let f = fixture();
    let output = f.data.join("image");
    let spool = f.data.join("spool");
    let lease = erofs_lease(&f, &output, &spool);
    let lease = with_inputs(lease, &[&f.data]).unwrap();
    assert!(lease.begin(MutationPlan::Erofs { output, spool }).is_err());
    let f = fixture();
    let output = f.data.join("image");
    let spool = f.data.join("spool");
    let lease = erofs_lease(&f, &output, &spool);
    let mut guard = lease
        .begin(MutationPlan::Erofs {
            output: output.clone(),
            spool: spool.clone(),
        })
        .unwrap();
    let temporary = guard.stage_file(&spool, 0o600).unwrap();
    let image = guard.stage_file(&output, 0o600).unwrap();
    guard.fault_discard_at("after-intent");
    assert!(guard.discard_file(temporary).is_err());
    assert!(guard.publish_file(image).is_err());
    assert!(!output.exists());
}
#[test]
fn amendment_c_erofs_diagnostic_finish_before_commit_and_never_spool_lock() {
    let f = fixture();
    let output = f.data.join("image.erofs");
    let spool = f.data.join("spool");
    let lock = f.data.join("image.erofs.lock");
    let lease = mutable(
        erofs_lease(&f, &output, &spool),
        &lock,
        PathRole::MutableFile,
    );
    let mut guard = lease
        .begin(MutationPlan::Erofs {
            output: output.clone(),
            spool: spool.clone(),
        })
        .unwrap();
    let mut diagnostic = guard.diagnostic_lock(&lock).unwrap();
    diagnostic.lock_exclusive().unwrap();
    diagnostic.write_pid(9).unwrap();
    let temporary = guard.stage_file(&spool, 0o600).unwrap();
    let image = guard.stage_file(&output, 0o600).unwrap();
    guard.discard_file(temporary).unwrap();
    guard.publish_file(image).unwrap();
    diagnostic.clear().unwrap();
    drop(diagnostic);
    guard.commit().unwrap();
    lease.revalidate().unwrap();
}
#[test]
fn amendment_c_mixed_dead_endpoint_restart_never_retires_data_uncertainty() {
    let f = fixture();
    let data = f.data.join("vm-data");
    let sockets = f.data.join("sockets");
    private(&sockets);
    let path = sockets.join("shell.sock");
    let lease = mutable(f.context.local(&data).unwrap(), &path, PathRole::Endpoint);
    let original = lease.0.id.clone();
    let bound = lease.endpoint(&path).unwrap().bind().unwrap();
    {
        let mut tx = f.context.namespace.transaction().unwrap();
        tx.operation_mut(&original).unwrap().uncertain = true;
        tx.save().unwrap();
    }
    super::endpoint::abandon_closed_test_listener(bound);
    drop(lease);
    let restarted = endpoint_lease(&f, &path);
    let bound = restarted.endpoint(&path).unwrap().bind().unwrap();
    assert!(exclusive(&f.context, &data).is_err());
    {
        let tx = f.context.namespace.transaction().unwrap();
        let old = tx.operation(&original).unwrap();
        assert!(old.uncertain);
        assert!(!old.closed);
        assert!(old.entries.iter().any(|e| e.path.get().unwrap() == data));
    }
    bound.close().unwrap();
    assert!(exclusive(&f.context, &data).is_err());
}
#[test]
fn amendment_c_live_endpoint_sentinel_survives_mixed_parent_handle_drop() {
    let f = fixture();
    let data = f.data.join("vm");
    let sockets = f.data.join("sockets");
    private(&sockets);
    let path = sockets.join("shell.sock");
    let lease = mutable(f.context.local(&data).unwrap(), &path, PathRole::Endpoint);
    let bound = lease.endpoint(&path).unwrap().bind().unwrap();
    drop(lease);
    assert!(
        start(
            f.context.namespace.clone(),
            vec![entry(&path, PathRole::Endpoint).unwrap()],
            None,
            false
        )
        .is_err()
    );
    bound.close().unwrap();
}
#[test]
fn amendment_c_failed_bind_receipt_gap_is_not_unrecorded_cleanup_permission() {
    let f = fixture();
    let parent = f.data.join("sockets");
    private(&parent);
    let path = parent.join("x".repeat(150));
    let lease = endpoint_lease(&f, &path);
    assert!(lease.endpoint(&path).unwrap().bind().is_err());
    let tx = f.context.namespace.transaction().unwrap();
    assert!(
        tx.catalog
            .endpoints
            .iter()
            .any(|e| e.operation == lease.0.id && e.state == catalog::EndpointState::BindIntent)
    );
    assert!(!tx.operation(&lease.0.id).unwrap().uncertain);
}
#[tokio::test]
async fn amendment_c_owned_async_endpoint_closes_without_raw_listener_escape() {
    let f = fixture();
    let parent = f.data.join("sockets");
    private(&parent);
    let path = parent.join("shell.sock");
    let lease = endpoint_lease(&f, &path);
    let listener = lease
        .endpoint(&path)
        .unwrap()
        .bind()
        .unwrap()
        .into_async()
        .unwrap();
    let client = tokio::net::UnixStream::connect(&path).await.unwrap();
    let (stream, _) = listener.accept().await.unwrap();
    drop((client, stream));
    listener.close().unwrap();
    assert!(!path.exists());
}
#[test]
fn amendment_c_recovery_inspection_readonly_cas_and_active_child_refuse() {
    let f = fixture();
    let path = f.data.join("output");
    let lease = f.context.local(&path).unwrap();
    let operation = lease.0.id.clone();
    let before = std::fs::read(f.catalog.join("catalog.json")).unwrap();
    testing::inspect_recovery(&f.context).unwrap();
    assert_eq!(
        before,
        std::fs::read(f.catalog.join("catalog.json")).unwrap()
    );
    let child = lease.export_child().unwrap();
    let imported = f
        .context
        .import_child(child.duplicate_for_exec().unwrap())
        .unwrap();
    let request = recovery_request(&f, &operation, RecoveryAction::ReconcileVerified);
    assert!(testing::recover(&f.context, request).is_err());
    drop((imported, child));
    drop(lease);
    let mut request = recovery_request(&f, &operation, RecoveryAction::ArchiveCompleted);
    request.expected_record_sha256 = "0".repeat(64);
    assert!(testing::recover(&f.context, request).is_err());
}
#[test]
fn amendment_c_recovery_reconciles_discard_after_unlink_without_deleting_replacement() {
    for replacement in [false, true] {
        let f = fixture();
        let root = f.data.join("out");
        private(&root);
        let spool = root.join("spool");
        let lease = mutable(
            f.context.local(&root).unwrap(),
            &spool,
            PathRole::MutableFile,
        );
        let operation = lease.0.id.clone();
        let mut guard = lease
            .begin(MutationPlan::ImagePair { directory: root })
            .unwrap();
        let stage = guard.stage_file(&spool, 0o600).unwrap();
        let stage_path = stage.path().to_path_buf();
        guard.fault_discard_at("after-unlink");
        assert!(guard.discard_file(stage).is_err());
        drop(guard);
        drop(lease);
        if replacement {
            std::fs::write(&stage_path, b"replacement").unwrap();
        }
        let request = recovery_request(&f, &operation, RecoveryAction::ReconcileVerified);
        let report = testing::recover(&f.context, request).unwrap();
        let value = serde_json::to_value(report).unwrap();
        if replacement {
            assert!(value["status"].as_str().unwrap().starts_with("unresolved"));
            assert_eq!(std::fs::read(stage_path).unwrap(), b"replacement");
        } else {
            assert_eq!(value["status"], "completed");
            assert!(
                f.context
                    .namespace
                    .transaction()
                    .unwrap()
                    .operation(&operation)
                    .unwrap()
                    .closed
            );
        }
    }
}
#[test]
fn amendment_c_recovery_accepts_proven_new_publication_and_preserves_old_prior() {
    let f = fixture();
    let output = f.data.join("image");
    let spool = f.data.join("spool");
    std::fs::write(&output, b"OLD").unwrap();
    let lease = erofs_lease(&f, &output, &spool);
    let operation = lease.0.id.clone();
    let mut guard = lease
        .begin(MutationPlan::Erofs {
            output: output.clone(),
            spool: spool.clone(),
        })
        .unwrap();
    let temporary = guard.stage_file(&spool, 0o600).unwrap();
    guard.discard_file(temporary).unwrap();
    let mut image = guard.stage_file(&output, 0o600).unwrap();
    image.file_mut().write_all(b"NEW").unwrap();
    guard.publish_file(image).unwrap();
    drop(guard);
    drop(lease);
    let report = testing::recover(
        &f.context,
        recovery_request(&f, &operation, RecoveryAction::ReconcileVerified),
    )
    .unwrap();
    assert_eq!(serde_json::to_value(report).unwrap()["status"], "completed");
    assert_eq!(std::fs::read(&output).unwrap(), b"NEW");
    assert!(
        std::fs::read_dir(&f.data)
            .unwrap()
            .map(Result::unwrap)
            .any(
                |e| e.file_name().to_string_lossy().starts_with(".iii-prior-")
                    && std::fs::read(e.path()).unwrap() == b"OLD"
            )
    );
}
#[test]
fn amendment_c_archive_257_completed_operations_preserves_all_evidence() {
    let f = fixture();
    for n in 0..257 {
        let lease = f.context.local(&f.data.join(format!("root-{n}"))).unwrap();
        let operation = lease.0.id.clone();
        drop(lease);
        testing::recover(
            &f.context,
            recovery_request(&f, &operation, RecoveryAction::ArchiveCompleted),
        )
        .unwrap();
        assert!(
            f.catalog
                .join("archive")
                .join(format!("{operation}.json"))
                .exists()
        );
        assert!(
            f.catalog
                .join("archive")
                .join(format!("op-{operation}"))
                .exists()
        );
    }
    let tx = f.context.namespace.transaction().unwrap();
    assert!(tx.catalog.operations.is_empty());
    assert_eq!(tx.catalog.archives.len(), 257);
}
#[test]
fn amendment_c_archive_boundaries_resume_without_unaccounted_live_removal() {
    for boundary in [
        "archive-intent",
        "archive-stage",
        "archive-promoted",
        "archive-receipt",
        "archive-sentinels",
    ] {
        let f = fixture();
        let lease = f.context.local(&f.data.join("root")).unwrap();
        let operation = lease.0.id.clone();
        drop(lease);
        let request = recovery_request(&f, &operation, RecoveryAction::ArchiveCompleted);
        assert!(recovery::recover_fault_test(&f.context, request, boundary).is_err());
        testing::open_context(&f.catalog).unwrap();
        let tx = f.context.namespace.transaction().unwrap();
        assert!(tx.catalog.operations.iter().any(|op| op.id == operation));
        drop(tx);
        testing::recover(
            &f.context,
            recovery_request(&f, &operation, RecoveryAction::ArchiveCompleted),
        )
        .unwrap();
        assert!(
            f.context
                .namespace
                .transaction()
                .unwrap()
                .catalog
                .operations
                .is_empty()
        );
    }
}
#[test]
fn amendment_c_corrupt_current_catalog_recovery_never_uses_previous_snapshot() {
    let f = fixture();
    let lease = f.context.local(&f.data.join("root")).unwrap();
    let operation = lease.0.id.clone();
    drop(lease);
    let request = recovery_request(&f, &operation, RecoveryAction::ArchiveCompleted);
    let previous = std::fs::read(f.catalog.join("catalog.previous")).unwrap();
    std::fs::write(f.catalog.join("catalog.json"), b"{torn").unwrap();
    assert!(testing::inspect_recovery(&f.context).is_err());
    assert!(testing::recover(&f.context, request).is_err());
    assert_eq!(
        std::fs::read(f.catalog.join("catalog.json")).unwrap(),
        b"{torn"
    );
    assert_eq!(
        std::fs::read(f.catalog.join("catalog.previous")).unwrap(),
        previous
    );
}

#[test]
fn amendment_c_recovery_proves_publication_gap_but_refuses_external_new_replacement() {
    for replace in [false, true] {
        let f = fixture();
        let output = f.data.join("image");
        let spool = f.data.join("spool");
        std::fs::write(&output, b"OLD").unwrap();
        let lease = erofs_lease(&f, &output, &spool);
        let operation = lease.0.id.clone();
        let mut guard = lease
            .begin(MutationPlan::Erofs {
                output: output.clone(),
                spool: spool.clone(),
            })
            .unwrap();
        let temporary = guard.stage_file(&spool, 0o600).unwrap();
        guard.discard_file(temporary).unwrap();
        let mut image = guard.stage_file(&output, 0o600).unwrap();
        image.file_mut().write_all(b"NEW").unwrap();
        guard.publish_file(image).unwrap();
        drop(guard);
        drop(lease);
        // Model only the completion-record crash boundary: before-effect intent
        // and actual owned NEW placement remain. This is not a power-loss proof.
        {
            let mut tx = f.context.namespace.transaction().unwrap();
            let op = tx.operation_mut(&operation).unwrap();
            let journal = &mut op.journals[0];
            let intent = journal
                .intents
                .iter_mut()
                .find(|i| i.kind == catalog::EffectKind::Publish)
                .unwrap();
            intent.completed = false;
            tx.save().unwrap();
        }
        if replace {
            std::fs::rename(&output, f.data.join("retained-new")).unwrap();
            std::fs::write(&output, b"FOREIGN").unwrap();
        }
        let result = testing::recover(
            &f.context,
            recovery_request(&f, &operation, RecoveryAction::ReconcileVerified),
        )
        .unwrap();
        let status = serde_json::to_value(result).unwrap();
        if replace {
            assert!(status["status"].as_str().unwrap().starts_with("unresolved"));
            assert_eq!(std::fs::read(&output).unwrap(), b"FOREIGN");
        } else {
            assert_eq!(status["status"], "completed");
            assert_eq!(std::fs::read(&output).unwrap(), b"NEW");
        }
    }
}
#[test]
fn amendment_c_drop_owned_listener_never_unlinks_or_claims_known_retirement() {
    let f = fixture();
    let parent = f.data.join("sockets");
    private(&parent);
    let path = parent.join("shell.sock");
    let lease = endpoint_lease(&f, &path);
    let bound = lease.endpoint(&path).unwrap().bind().unwrap();
    drop(bound);
    drop(lease);
    assert!(path.exists());
    assert!(
        start(
            f.context.namespace.clone(),
            vec![entry(&path, PathRole::Endpoint).unwrap()],
            None,
            false
        )
        .is_err()
    );
}
#[test]
fn amendment_c_new_payload_cannot_overlap_live_endpoint_private_parent() {
    let f = fixture();
    let parent = f.data.join("sockets");
    private(&parent);
    let path = parent.join("shell.sock");
    let lease = endpoint_lease(&f, &path);
    let bound = lease.endpoint(&path).unwrap().bind().unwrap();
    assert!(f.context.local(&parent.join("guest-data")).is_err());
    assert!(with_inputs(lease.clone(), &[&parent.join("export")]).is_err());
    bound.close().unwrap();
}
#[test]
fn amendment_c_recovery_does_not_close_unknown_plain_orphan_or_unpublished_stage() {
    let f = fixture();
    let path = f.data.join("root");
    let lease = f.context.local(&path).unwrap();
    let operation = lease.0.id.clone();
    {
        let mut tx = f.context.namespace.transaction().unwrap();
        let op = tx.operation_mut(&operation).unwrap();
        op.holders += 1;
        op.holder_ids.push(new_id());
        tx.save().unwrap();
    }
    drop(lease);
    let report = testing::recover(
        &f.context,
        recovery_request(&f, &operation, RecoveryAction::ReconcileVerified),
    )
    .unwrap();
    assert!(
        serde_json::to_value(report).unwrap()["status"]
            .as_str()
            .unwrap()
            .starts_with("unresolved")
    );
    assert!(exclusive(&f.context, &path).is_err());
    let f = fixture();
    let root = f.data.join("out");
    private(&root);
    let lease = f.context.local(&root).unwrap();
    let operation = lease.0.id.clone();
    let mut guard = lease
        .begin(MutationPlan::ImagePair {
            directory: root.clone(),
        })
        .unwrap();
    let stage = guard.stage_file(&root.join("image"), 0o600).unwrap();
    let path = stage.path().to_path_buf();
    drop(stage);
    drop(guard);
    drop(lease);
    let report = testing::recover(
        &f.context,
        recovery_request(&f, &operation, RecoveryAction::ReconcileVerified),
    )
    .unwrap();
    assert!(
        serde_json::to_value(report).unwrap()["status"]
            .as_str()
            .unwrap()
            .starts_with("unresolved")
    );
    assert!(path.exists());
}

#[test]
fn amendment_d_runtime_prepare_cancel_and_exact_activity_file() {
    let f = fixture();
    let root = f.data.join("guest");
    private(&root);
    let upper = f.data.join("upper.ext4");
    std::fs::write(&upper, b"disk").unwrap();
    let activity = f.data.join("activity");
    let lease = with_paths(
        f.context.local(&root).unwrap(),
        &[
            PathRequest {
                path: &upper,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &activity,
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap();
    let launch = lease
        .prepare_runtime(&[
            RuntimePath {
                path: &root,
                kind: RuntimePathKind::GuestTree,
            },
            RuntimePath {
                path: &upper,
                kind: RuntimePathKind::UpperImage,
            },
            RuntimePath {
                path: &activity,
                kind: RuntimePathKind::ActivityStamp,
            },
        ])
        .unwrap();
    assert!(activity.is_file());
    assert!(exclusive(&f.context, &root).is_err());
    assert!(
        lease
            .begin(MutationPlan::Create { root: root.clone() })
            .is_err()
    );
    drop(launch);
    lease.revalidate().unwrap();
    let tx = f.context.namespace.transaction().unwrap();
    assert!(
        tx.catalog
            .runtimes
            .iter()
            .all(|r| r.state == catalog::RuntimeState::Cancelled
                && !r.writer_retired
                && !r.guest_consistent)
    );
}
#[test]
fn amendment_d_runtime_rejects_strict_subtree_and_unprepared_upper() {
    let f = fixture();
    let root = f.data.join("guest");
    private(&root);
    let readonly = root.join("source");
    private(&readonly);
    let lease = with_inputs(f.context.local(&root).unwrap(), &[&readonly]).unwrap();
    assert!(
        lease
            .prepare_runtime(&[RuntimePath {
                path: &root,
                kind: RuntimePathKind::GuestTree
            }])
            .is_err()
    );
    let f = fixture();
    let upper = f.data.join("upper.ext4");
    let lease = mutable(
        f.context.local(&f.data.join("state")).unwrap(),
        &upper,
        PathRole::MutableFile,
    );
    assert!(
        lease
            .prepare_runtime(&[RuntimePath {
                path: &upper,
                kind: RuntimePathKind::UpperImage
            }])
            .is_err()
    );
}
#[test]
fn amendment_d_runtime_import_requires_dedicated_fd_not_operation_sentinel() {
    let f = fixture();
    let root = f.data.join("guest");
    private(&root);
    let lease = f.context.local(&root).unwrap();
    let _launch = lease
        .prepare_runtime(&[RuntimePath {
            path: &root,
            kind: RuntimePathKind::GuestTree,
        }])
        .unwrap();
    let child = lease.export_child().unwrap();
    assert!(import_runtime_writer(&lease, child.duplicate_for_exec().unwrap()).is_err());
}
#[test]
fn amendment_d_owner_read_0000_internal_links_restore_original_guest_modes() {
    use std::io::Read;
    assert_ne!(
        uid(),
        0,
        "run this fixture as an unprivileged owner; root would mask the permission contract"
    );
    let f = fixture();
    let cache = f.data.join("cache");
    private(&cache);
    let dir = cache.join("sealed");
    private(&dir);
    let file = dir.join("file");
    std::fs::write(&file, b"DATA").unwrap();
    std::fs::hard_link(&file, dir.join("alias")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0)).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0)).unwrap();
    let lease = mutable(
        f.context.local(&f.data.join("state")).unwrap(),
        &cache,
        PathRole::ManagedCacheRead,
    );
    let mut read = lease.owner_read_cache(&cache).unwrap();
    let entries = read.read_dir(&dir).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|e| e.metadata.mode() & 0o777 == 0));
    let mut opened = read.open_file(&file).unwrap();
    let mut bytes = String::new();
    opened.read_to_string(&mut bytes).unwrap();
    assert_eq!(bytes, "DATA");
    drop(opened);
    assert_eq!(read.original_metadata(&file).unwrap().mode() & 0o777, 0);
    read.finish().unwrap();
    assert_eq!(std::fs::symlink_metadata(&dir).unwrap().mode() & 0o777, 0);
    // Test-only retained access for inspecting restored children; no production
    // recovery relies on this explicit fixture permission change.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(std::fs::metadata(&file).unwrap().mode() & 0o777, 0);
    lease.revalidate().unwrap();
}
#[test]
fn amendment_d_owner_read_external_links_and_strict_roles_refuse() {
    let f = fixture();
    let cache = f.data.join("cache");
    private(&cache);
    let file = cache.join("file");
    std::fs::write(&file, b"data").unwrap();
    std::fs::hard_link(&file, f.data.join("external")).unwrap();
    let lease = mutable(
        f.context.local(&f.data.join("state")).unwrap(),
        &cache,
        PathRole::ManagedCacheRead,
    );
    assert!(lease.owner_read_cache(&cache).is_err());
    let f = fixture();
    let cache = f.data.join("cache");
    private(&cache);
    let lease = with_inputs(f.context.local(&f.data.join("state")).unwrap(), &[&cache]).unwrap();
    assert!(lease.owner_read_cache(&cache).is_err());
}
#[test]
fn amendment_d_owner_read_cancel_and_replacement_retain_fence() {
    let f = fixture();
    let cache = f.data.join("cache");
    private(&cache);
    let file = cache.join("file");
    std::fs::write(&file, b"old").unwrap();
    let lease = mutable(
        f.context.local(&f.data.join("state")).unwrap(),
        &cache,
        PathRole::ManagedCacheRead,
    );
    let mut read = lease.owner_read_cache(&cache).unwrap();
    std::fs::rename(&file, cache.join("retained")).unwrap();
    std::fs::write(&file, b"foreign").unwrap();
    assert!(read.open_file(&file).is_err());
    assert!(read.finish().is_err());
    assert!(lease.revalidate().is_err());
}
#[test]
fn amendment_d_remove_exact_runtime_targets_preserves_retained_upper() {
    let f = fixture();
    let vm = f.data.join("vm");
    private(&vm);
    let runtime = vm.join("runtime");
    let upper = vm.join("upper.ext4");
    std::fs::write(&upper, b"KEEP").unwrap();
    let lease = f.context.local(&vm).unwrap();
    let lease = mutable(lease, &runtime, PathRole::MutableTree);
    let mut create = lease
        .begin(MutationPlan::Create {
            root: runtime.clone(),
        })
        .unwrap();
    create.create_dir_all(&runtime, 0o700).unwrap();
    create.commit().unwrap();
    let receipt = lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    let mut remove = receipt.begin_remove(&lease, &vm, &[&runtime]).unwrap();
    remove.remove_path(&runtime).unwrap();
    remove.commit().unwrap();
    assert_eq!(std::fs::read(&upper).unwrap(), b"KEEP");
    assert!(!runtime.exists());
    let repeat = lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    let mut remove = repeat.begin_remove(&lease, &vm, &[&runtime]).unwrap();
    remove.remove_path(&runtime).unwrap();
    remove.commit().unwrap();
}
#[test]
fn amendment_d_remove_refuses_unlisted_protected_backup_and_external_hardlink() {
    let f = fixture();
    let root = f.data.join("vm");
    let lease = f.context.local(&root).unwrap();
    let mut create = lease
        .begin(MutationPlan::Create { root: root.clone() })
        .unwrap();
    create.create_dir_all(&root, 0o700).unwrap();
    create.commit().unwrap();
    let receipt = lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    assert!(
        receipt
            .begin_remove(&lease, &root, &[&root.join("not-listed")])
            .is_err()
    );
    private(&root.join("backup-old"));
    let mut remove = receipt.begin_remove(&lease, &root, &[&root]).unwrap();
    assert!(remove.remove_path(&root).is_err());
    assert!(root.join("backup-old").exists());
    let f = fixture();
    let root = f.data.join("vm");
    let lease = f.context.local(&root).unwrap();
    let mut create = lease
        .begin(MutationPlan::Create { root: root.clone() })
        .unwrap();
    create.create_dir_all(&root, 0o700).unwrap();
    create.commit().unwrap();
    let file = root.join("file");
    std::fs::write(&file, b"KEEP").unwrap();
    std::fs::hard_link(&file, f.data.join("external")).unwrap();
    let receipt = lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    let mut remove = receipt.begin_remove(&lease, &root, &[&root]).unwrap();
    assert!(remove.remove_path(&root).is_err());
    assert!(file.exists());
}
#[test]
fn amendment_d_remove_nested_symlink_never_follows_external_tree() {
    let f = fixture();
    let root = f.data.join("vm");
    let foreign = f.data.join("foreign");
    private(&foreign);
    std::fs::write(foreign.join("data"), b"KEEP").unwrap();
    let lease = f.context.local(&root).unwrap();
    let mut create = lease
        .begin(MutationPlan::Create { root: root.clone() })
        .unwrap();
    create.create_dir_all(&root, 0o700).unwrap();
    create.commit().unwrap();
    symlink(&foreign, root.join("link")).unwrap();
    let receipt = lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    let mut remove = receipt.begin_remove(&lease, &root, &[&root]).unwrap();
    remove.remove_path(&root).unwrap();
    remove.commit().unwrap();
    assert_eq!(std::fs::read(foreign.join("data")).unwrap(), b"KEEP");
}

#[test]
fn amendment_e_control_files_exact_roles_duplicates_aliases_and_siblings_refuse() {
    let f = fixture();
    let a = f.data.join("console.log");
    let b = f.data.join("console.log.1");
    let lease = with_paths(
        f.context.local(&f.data.join("state")).unwrap(),
        &[
            PathRequest {
                path: &a,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &b,
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap();
    assert!(
        lease
            .begin(MutationPlan::ControlFiles { files: Vec::new() })
            .is_err()
    );
    assert!(
        lease
            .begin(MutationPlan::ControlFiles {
                files: vec![a.clone(), a.clone()]
            })
            .is_err()
    );
    assert!(
        lease
            .begin(MutationPlan::ControlFiles {
                files: vec![PathBuf::from("relative")]
            })
            .is_err()
    );
    let mut control = lease
        .begin(MutationPlan::ControlFiles {
            files: vec![a.clone(), b],
        })
        .unwrap();
    assert!(
        control
            .stage_file(&f.data.join("not-declared"), 0o600)
            .is_err()
    );
    assert!(control.commit().is_err());
    let f = fixture();
    let a = f.data.join("a");
    let b = f.data.join("b");
    std::fs::write(&a, b"old").unwrap();
    std::fs::hard_link(&a, &b).unwrap();
    assert!(
        with_paths(
            f.context.local(&f.data.join("state")).unwrap(),
            &[
                PathRequest {
                    path: &a,
                    role: PathRole::MutableFile
                },
                PathRequest {
                    path: &b,
                    role: PathRole::MutableFile
                }
            ]
        )
        .is_err()
    );
}
#[test]
fn amendment_e_control_files_partial_publication_keeps_uncertainty_and_prior_log() {
    let f = fixture();
    let a = f.data.join("console.log");
    let b = f.data.join("console.log.1");
    std::fs::write(&a, b"old-current").unwrap();
    std::fs::write(&b, b"old-rotated").unwrap();
    let lease = with_paths(
        f.context.local(&f.data.join("state")).unwrap(),
        &[
            PathRequest {
                path: &a,
                role: PathRole::MutableFile,
            },
            PathRequest {
                path: &b,
                role: PathRole::MutableFile,
            },
        ],
    )
    .unwrap();
    let mut control = lease
        .begin(MutationPlan::ControlFiles {
            files: vec![a.clone(), b.clone()],
        })
        .unwrap();
    let mut first = control.stage_file(&b, 0o600).unwrap();
    first.file_mut().write_all(b"old-current").unwrap();
    control.publish_file(first).unwrap();
    let second = control.stage_file(&a, 0o600).unwrap();
    std::fs::rename(&a, f.data.join("retained-current")).unwrap();
    std::fs::write(&a, b"external").unwrap();
    assert!(control.publish_file(second).is_err());
    assert!(control.commit().is_err());
    assert!(lease.revalidate().is_err());
    assert_eq!(std::fs::read(&b).unwrap(), b"old-current");
    assert_eq!(std::fs::read(&a).unwrap(), b"external");
    assert_eq!(
        std::fs::read(f.data.join("retained-current")).unwrap(),
        b"old-current"
    );
}

#[test]
fn amendment_e_control_publication_each_boundary_remains_terminal() {
    for second in [false, true] {
        for boundary in ["before-rename", "after-rename"] {
            let f = fixture();
            let current = f.data.join("console.log");
            let rotated = f.data.join("console.log.1");
            std::fs::write(&current, b"current").unwrap();
            std::fs::write(&rotated, b"prior").unwrap();
            let lease = with_paths(
                f.context.local(&f.data.join("state")).unwrap(),
                &[
                    PathRequest {
                        path: &current,
                        role: PathRole::MutableFile,
                    },
                    PathRequest {
                        path: &rotated,
                        role: PathRole::MutableFile,
                    },
                ],
            )
            .unwrap();
            let mut guard = lease
                .begin(MutationPlan::ControlFiles {
                    files: vec![current.clone(), rotated.clone()],
                })
                .unwrap();
            let mut first = guard.stage_file(&rotated, 0o600).unwrap();
            first.file_mut().write_all(b"current").unwrap();
            if second {
                guard.publish_file(first).unwrap();
                let next = guard.stage_file(&current, 0o600).unwrap();
                guard.fault_publish_at(boundary);
                assert!(guard.publish_file(next).is_err());
            } else {
                guard.fault_publish_at(boundary);
                assert!(guard.publish_file(first).is_err());
            }
            assert!(guard.commit().is_err());
            assert!(lease.revalidate().is_err());
            assert!(
                std::fs::read_dir(&f.data)
                    .unwrap()
                    .map(Result::unwrap)
                    .any(|e| e.file_name().to_string_lossy().starts_with(".iii-prior-"))
            );
        }
    }
}

#[test]
fn f_receipt_refuses_foreign_stale_legacy_and_raw_remove_paths() {
    let f = fixture();
    let old = f.data.join("legacy");
    private(&old);
    std::fs::write(old.join("data"), b"KEEP").unwrap();
    let lease = f.context.local(&old).unwrap();
    assert!(
        lease
            .begin(MutationPlan::Remove {
                root: old.clone(),
                targets: vec![old.clone()]
            })
            .is_err()
    );
    let proof = lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    assert!(proof.begin_remove(&lease, &old, &[&old]).is_err());
    let f = fixture();
    let root = f.data.join("new");
    let lease = f.context.local(&root).unwrap();
    let mut create = lease
        .begin(MutationPlan::Create { root: root.clone() })
        .unwrap();
    create.create_dir_all(&root, 0o700).unwrap();
    create.commit().unwrap();
    let proof = lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    let other = f.context.local(&f.data.join("other")).unwrap();
    assert!(proof.begin_remove(&other, &root, &[&root]).is_err());
    let newer = lease.prepare_runtime(&[]).unwrap();
    assert!(proof.begin_remove(&lease, &root, &[&root]).is_err());
    drop(newer);
    assert!(root.exists());
}
#[test]
fn f_never_spawned_receipt_rejects_replaced_layout_and_is_one_shot() {
    let f = fixture();
    let root = f.data.join("new");
    let lease = f.context.local(&root).unwrap();
    let mut create = lease
        .begin(MutationPlan::Create { root: root.clone() })
        .unwrap();
    create.create_dir_all(&root, 0o700).unwrap();
    create.commit().unwrap();
    let proof = lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    std::fs::rename(&root, f.data.join("retained")).unwrap();
    private(&root);
    assert!(proof.begin_remove(&lease, &root, &[&root]).is_err());
}

#[test]
fn f_receipt_is_stale_after_owned_target_mutation_without_new_runtime() {
    let f = fixture();
    let root = f.data.join("new");
    let lease = f.context.local(&root).unwrap();
    let mut create = lease
        .begin(MutationPlan::Create { root: root.clone() })
        .unwrap();
    create.create_dir_all(&root, 0o700).unwrap();
    create.commit().unwrap();
    let proof = lease
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    let mut mutation = lease
        .begin(MutationPlan::Reclone { root: root.clone() })
        .unwrap();
    let stage = mutation.stage_tree(&root).unwrap();
    std::fs::write(stage.path().join("data"), b"NEW").unwrap();
    mutation.publish_tree(stage).unwrap();
    mutation.commit().unwrap();
    assert!(proof.begin_remove(&lease, &root, &[&root]).is_err());
    assert_eq!(std::fs::read(root.join("data")).unwrap(), b"NEW");
}
