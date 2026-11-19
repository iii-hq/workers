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
    let mut guard = lease
        .begin(MutationPlan::Erofs {
            output: output.clone(),
        })
        .unwrap();
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
    let mut guard = lease
        .begin(MutationPlan::Erofs {
            output: output.clone(),
        })
        .unwrap();
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
        tx.operation_mut(&lease.0.id).unwrap().holders += 1;
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
    let (listener, lifetime) = bound.into_parts();
    drop(listener);
    lifetime.close().unwrap();
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
    let (listener, lifetime) = lease.endpoint(&path).unwrap().bind().unwrap().into_parts();
    drop(listener);
    // Explicit cfg(test) crash model: do not pretend Drop or a PID is proof.
    // Consume the test lifetime without its conservative leak, then release
    // every OFD. Production has no path for this operation.
    super::endpoint::abandon_closed_test_listener(lifetime);
    drop(lease);
    let restarted = endpoint_lease(&f, &path);
    let (listener, lifetime) = restarted
        .endpoint(&path)
        .unwrap()
        .bind()
        .unwrap()
        .into_parts();
    drop(listener);
    lifetime.close().unwrap();
}
#[test]
fn c6_endpoint_parent_must_be_private_and_outside_payload_scopes() {
    let f = fixture();
    let root = f.data.join("guest");
    private(&root);
    let path = root.join("shell.sock");
    let lease = mutable(f.context.local(&root).unwrap(), &path, PathRole::Endpoint);
    assert!(lease.endpoint(&path).is_err());
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
