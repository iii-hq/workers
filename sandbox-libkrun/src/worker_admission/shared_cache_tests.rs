// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN. Real production issuers/catalog operations in isolated roots.
use super::*;
use crate::worker_admission::{PathRequest, testing, with_paths};
use std::{fs, io::Write, os::unix::fs::PermissionsExt};
struct Fixture {
    _temp: tempfile::TempDir,
    context: crate::worker_admission::AdmissionContext,
    data: PathBuf,
    cache: PathBuf,
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let catalog = temp.path().join("catalog");
    let data = temp.path().join("data");
    let cache = data.join("cache");
    for p in [&catalog, &data, &cache] {
        fs::create_dir(p).unwrap();
        fs::set_permissions(p, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let context = testing::provision_context(&catalog).unwrap();
    Fixture {
        _temp: temp,
        context,
        data,
        cache,
    }
}
fn consumer(f: &Fixture, name: &str) -> Lease {
    f.context.local(&f.data.join(name)).unwrap()
}
fn prepare(prep: &CachePreparation, root: &Path) {
    let mut root_write = prep.begin_entry(root).unwrap();
    let mut lock = root_write
        .diagnostic_lock(&sibling(root, ".lock").unwrap())
        .unwrap();
    lock.lock_exclusive().unwrap();
    lock.write_pid(1).unwrap();
    let tree = root_write.stage_tree(root).unwrap();
    write_mode(&tree.path().join("content"), b"SOURCE", 0o600);
    root_write.publish_tree(tree).unwrap();
    let mut digest = root_write
        .stage_file(&sibling(root, ".digest").unwrap(), 0o600)
        .unwrap();
    digest.file_mut().write_all(b"sha256:fixture").unwrap();
    root_write.publish_file(digest).unwrap();
    lock.clear().unwrap();
    drop(lock);
    root_write.commit().unwrap();
    let mut erofs = prep.begin_erofs(root).unwrap();
    let mut lock = erofs
        .diagnostic_lock(&sibling(root, ".erofs.lock").unwrap())
        .unwrap();
    lock.lock_exclusive().unwrap();
    let mut image = erofs
        .stage_file(&sibling(root, ".erofs").unwrap(), 0o600)
        .unwrap();
    image.file_mut().write_all(b"LOWER").unwrap();
    let spool = erofs
        .stage_file(&sibling(root, ".erofs.spool").unwrap(), 0o600)
        .unwrap();
    erofs.discard_file(spool).unwrap();
    erofs.publish_file(image).unwrap();
    lock.clear().unwrap();
    drop(lock);
    erofs.commit().unwrap();
}
/// Fixture objects get the exact modes the scenario intends, never the process
/// umask: core correctly refuses group/other-writable mutable or owner-read objects.
fn write_mode(path: &Path, bytes: &[u8], mode: u32) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
fn dir_mode(path: &Path, mode: u32) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
fn populated(root: &Path) {
    dir_mode(root, 0o700);
    write_mode(&root.join("content"), b"SOURCE", 0o600);
    write_mode(&sibling(root, ".erofs").unwrap(), b"LOWER", 0o600);
    write_mode(&sibling(root, ".digest").unwrap(), b"sha256:fixture", 0o600);
}
#[test]
fn h_cold_first_reader_then_warm_second_reader_share_and_exclude_writer() {
    let f = fixture();
    let root = f.cache.join("image");
    let a = consumer(&f, "a");
    let probe = a.probe_cache(&[CacheEntry { root: &root }]).unwrap();
    assert!(!probe.exists(&root).unwrap());
    let prep = probe.prepare().unwrap();
    prepare(&prep, &root);
    let stale = prep.clone();
    let a = prep.handoff(&a, &root).unwrap();
    assert!(stale.checkpoint().is_err());
    assert!(stale.begin_entry(&root).is_err());
    let before = fs::metadata(&root).unwrap().modified().unwrap();
    let digest = fs::read(sibling(&root, ".digest").unwrap()).unwrap();
    let b = consumer(&f, "b");
    let probe = b.probe_cache(&[CacheEntry { root: &root }]).unwrap();
    let read = probe.use_path(&root).unwrap();
    read.revalidate().unwrap();
    assert_eq!(fs::read(read.path().join("content")).unwrap(), b"SOURCE");
    drop(read);
    let b = probe.handoff(&b, &root).unwrap();
    let writer = consumer(&f, "writer");
    assert!(writer.prepare_cache(&[CacheEntry { root: &root }]).is_err());
    assert_eq!(fs::metadata(&root).unwrap().modified().unwrap(), before);
    assert_eq!(
        fs::read(sibling(&root, ".digest").unwrap()).unwrap(),
        digest
    );
    let read_a = a.use_path(&sibling(&root, ".erofs").unwrap()).unwrap();
    let read_b = b.use_path(&sibling(&root, ".erofs").unwrap()).unwrap();
    drop(read_a);
    drop(a);
    drop(prep);
    drop(stale);
    assert!(writer.prepare_cache(&[CacheEntry { root: &root }]).is_err());
    drop(read_b);
    drop(b);
    assert!(writer.prepare_cache(&[CacheEntry { root: &root }]).is_ok());
}
#[test]
fn h_populated_warm_probe_has_no_cache_locks_stages_or_permission_journal() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let before = fs::read_dir(&f.cache).unwrap().count();
    let a = consumer(&f, "a");
    let probe = a.probe_cache(&[CacheEntry { root: &root }]).unwrap();
    assert!(probe.exists(&root).unwrap());
    {
        let tx = f.context.namespace.transaction().unwrap();
        let op = tx.operation(&probe.lease.0.id).unwrap();
        assert!(op.journals.is_empty());
        assert!(op.entries.iter().all(|e| e.role == PathRole::Strict));
    }
    let a = probe.handoff(&a, &root).unwrap();
    assert_eq!(fs::read_dir(&f.cache).unwrap().count(), before);
    assert!(!sibling(&root, ".lock").unwrap().exists());
    assert!(!sibling(&root, ".erofs.lock").unwrap().exists());
    assert!(a.use_path(&root).is_ok());
}
#[test]
fn h_unrelated_image_preparation_does_not_conflict_with_shared_consumers() {
    let f = fixture();
    let aroot = f.cache.join("a");
    let broot = f.cache.join("b");
    populated(&aroot);
    let a = consumer(&f, "a");
    let a = a
        .probe_cache(&[CacheEntry { root: &aroot }])
        .unwrap()
        .handoff(&a, &aroot)
        .unwrap();
    let b = consumer(&f, "b");
    let prep = b.prepare_cache(&[CacheEntry { root: &broot }]).unwrap();
    prepare(&prep, &broot);
    let b = prep.handoff(&b, &broot).unwrap();
    a.revalidate().unwrap();
    b.revalidate().unwrap();
}
#[test]
fn h_absent_pinned_candidate_keeps_selection_claim_without_broad_parent() {
    let f = fixture();
    let shared = f.cache.join("shared");
    let pinned = f.cache.join("pinned");
    let a = consumer(&f, "a");
    let prep = a
        .probe_cache(&[CacheEntry { root: &shared }, CacheEntry { root: &pinned }])
        .unwrap()
        .prepare()
        .unwrap();
    prepare(&prep, &shared);
    let a = prep.handoff(&a, &shared).unwrap();
    assert!(!pinned.exists());
    let tx = f.context.namespace.transaction().unwrap();
    let op = tx.operation(&a.0.id).unwrap();
    assert!(!op.entries.iter().any(|e| e.path.get().unwrap() == f.cache));
    assert!(
        op.entries
            .iter()
            .any(|e| e.path.get().unwrap() == pinned && e.role == PathRole::Strict)
    );
}
#[test]
fn h_no_upgrade_of_live_probe_guard_or_active_consumer_for_stale_rebuild() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let probe = a.probe_cache(&[CacheEntry { root: &root }]).unwrap();
    let held = probe.use_path(&root).unwrap();
    assert!(probe.prepare().is_err());
    drop(held);
    let a = a
        .probe_cache(&[CacheEntry { root: &root }])
        .unwrap()
        .handoff(&a, &root)
        .unwrap();
    assert!(a.prepare_cache(&[CacheEntry { root: &root }]).is_err());
    let b = consumer(&f, "b");
    let probe = b.probe_cache(&[CacheEntry { root: &root }]).unwrap();
    assert!(probe.prepare().is_err());
    a.revalidate().unwrap();
}
#[test]
fn h_handoff_refuses_mutation_diagnostic_and_owner_read_until_finished() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
    let mut mutation = prep.begin_entry(&root).unwrap();
    let diagnostic = mutation
        .diagnostic_lock(&sibling(&root, ".lock").unwrap())
        .unwrap();
    assert!(prep.handoff(&a, &root).is_err());
    drop(diagnostic);
    mutation.commit().unwrap();
    let read = prep.owner_read(&root).unwrap();
    assert!(prep.handoff(&a, &root).is_err());
    read.finish().unwrap();
    let _a = prep.handoff(&a, &root).unwrap();
}
#[test]
fn h_stage_fd_clone_prevents_publication_and_poison_blocks_handoff() {
    let f = fixture();
    let root = f.cache.join("image");
    let a = consumer(&f, "a");
    let prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
    let mut mutation = prep.begin_entry(&root).unwrap();
    let stage = mutation.stage_tree(&root).unwrap();
    mutation.publish_tree(stage).unwrap();
    let mut digest = mutation
        .stage_file(&sibling(&root, ".digest").unwrap(), 0o600)
        .unwrap();
    let writer = digest.file_mut().try_clone().unwrap();
    assert!(mutation.publish_file(digest).is_err());
    drop(writer);
    drop(mutation);
    assert!(prep.handoff(&a, &root).is_err());
}
#[test]
fn h_foreign_context_target_epoch_and_active_import_refuse_transfer() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
    let b = consumer(&f, "b");
    assert!(prep.handoff(&b, &root).is_err());
    assert!(prep.handoff(&a, &f.cache.join("other")).is_err());
    let other = fixture();
    let foreign = consumer(&other, "a");
    assert!(prep.handoff(&foreign, &root).is_err());
    assert!(prep.lease.export_child().is_err());
    assert!(prep.lease.prepare_runtime(&[]).is_err());
    let child = a.export_child().unwrap();
    let imported = f
        .context
        .import_child(child.duplicate_for_exec().unwrap())
        .unwrap();
    assert!(prep.handoff(&a, &root).is_err());
    drop(imported);
    drop(child);
    let extra = f.data.join("new-input");
    let _changed = with_paths(
        a.clone(),
        &[PathRequest {
            path: &extra,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    assert!(prep.handoff(&a, &root).is_err());
}
#[test]
fn h_replacement_alias_and_cache_ancestor_never_become_preparation_authority() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
    fs::rename(sibling(&root, ".erofs").unwrap(), f.cache.join("retained")).unwrap();
    fs::write(sibling(&root, ".erofs").unwrap(), b"FOREIGN").unwrap();
    assert!(prep.handoff(&a, &root).is_err());
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    fs::hard_link(sibling(&root, ".erofs").unwrap(), f.cache.join("alias")).unwrap();
    let a = consumer(&f, "a");
    assert!(a.probe_cache(&[CacheEntry { root: &root }]).is_err());
    let f = fixture();
    let broad = f.context.local(&f.cache).unwrap();
    assert!(
        broad
            .prepare_cache(&[CacheEntry {
                root: &f.cache.join("image")
            }])
            .is_err()
    );
}
#[test]
fn h_handoff_faults_preserve_one_side_of_continuous_exclusion() {
    for fault in ["before-save", "after-save"] {
        let f = fixture();
        let root = f.cache.join("image");
        populated(&root);
        let a = consumer(&f, "a");
        let prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
        assert!(prep.handoff_fault(&a, &root, fault).is_err());
        let writer = consumer(&f, "writer");
        assert!(writer.prepare_cache(&[CacheEntry { root: &root }]).is_err());
        let tx = f.context.namespace.transaction().unwrap();
        let source = tx.operation(&prep.lease.0.id).unwrap();
        let target = tx.operation(&a.0.id).unwrap();
        if fault == "before-save" {
            assert!(!source.entries.is_empty());
            assert!(!target.entries.iter().any(|e| e.path.get().unwrap() == root));
        } else {
            assert!(source.entries.is_empty());
            assert!(source.closed);
            assert!(
                target
                    .entries
                    .iter()
                    .any(|e| e.path.get().unwrap() == root && e.role == PathRole::Strict)
            );
        }
    }
}
#[test]
fn h_cancelled_preparation_retains_unknown_writer_evidence_not_a_shared_exception() {
    let f = fixture();
    let root = f.cache.join("image");
    let a = consumer(&f, "a");
    let prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
    let id = prep.lease.0.id.clone();
    drop(prep);
    assert!(
        consumer(&f, "b")
            .prepare_cache(&[CacheEntry { root: &root }])
            .is_err()
    );
    assert!(
        f.context
            .namespace
            .transaction()
            .unwrap()
            .operation(&id)
            .unwrap()
            .uncertain
    );
}
#[test]
fn h_consumer_retirement_cannot_remove_shared_cache() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let a = a
        .probe_cache(&[CacheEntry { root: &root }])
        .unwrap()
        .handoff(&a, &root)
        .unwrap();
    let rollback = a
        .prepare_runtime(&[])
        .unwrap()
        .cancel_before_spawn()
        .unwrap();
    assert!(rollback.begin_remove(&a, &f.cache, &[&root]).is_err());
    assert!(root.exists());
}

#[test]
fn h_incomplete_permission_intent_and_cancelled_stage_never_transfer() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
    let mut guard = prep.begin_entry(&root).unwrap();
    let stage = guard
        .stage_file(&sibling(&root, ".digest").unwrap(), 0o600)
        .unwrap();
    drop(stage);
    assert!(guard.commit().is_err());
    assert!(prep.handoff(&a, &root).is_err());
}
#[test]
fn h_wrong_selected_root_and_consumer_runtime_do_not_drop_preparation() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
    let launch = a.prepare_runtime(&[]).unwrap();
    assert!(prep.handoff(&a, &root).is_err());
    drop(launch);
    assert!(prep.handoff(&a, &root).is_err());
    let tx = f.context.namespace.transaction().unwrap();
    assert!(!tx.operation(&prep.lease.0.id).unwrap().entries.is_empty());
}

#[test]
fn h_diagnostic_duplicate_still_holding_flock_prevents_transfer() {
    use std::os::fd::AsFd;
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
    let mut mutation = prep.begin_entry(&root).unwrap();
    let mut diagnostic = mutation
        .diagnostic_lock(&sibling(&root, ".lock").unwrap())
        .unwrap();
    diagnostic.lock_exclusive().unwrap();
    let duplicate = diagnostic.as_fd().try_clone_to_owned().unwrap();
    drop(diagnostic);
    mutation.commit().unwrap();
    assert!(prep.handoff(&a, &root).is_err());
    drop(duplicate);
    prep.handoff(&a, &root).unwrap();
}
#[test]
fn h_symlinked_entry_and_duplicate_candidates_are_refused_without_writes() {
    use std::os::unix::fs::symlink;
    let f = fixture();
    let real = f.cache.join("real");
    populated(&real);
    let alias = f.cache.join("alias");
    symlink(&real, &alias).unwrap();
    let a = consumer(&f, "a");
    assert!(a.probe_cache(&[CacheEntry { root: &alias }]).is_err());
    assert!(
        a.prepare_cache(&[CacheEntry { root: &real }, CacheEntry { root: &real }])
            .is_err()
    );
    assert!(!sibling(&real, ".lock").unwrap().exists());
}

#[test]
fn h_provisional_cancel_releases_only_its_own_read_without_touching_lifecycle() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let original = a.0.id.clone();
    a.probe_cache(&[CacheEntry { root: &root }])
        .unwrap()
        .cancel()
        .unwrap();
    a.revalidate().unwrap();
    assert_eq!(a.0.id, original);
    let b = consumer(&f, "b");
    assert!(b.prepare_cache(&[CacheEntry { root: &root }]).is_ok());
}
#[test]
fn h_exact_cache_preparation_does_not_lock_existing_claim_directory() {
    use std::os::fd::AsRawFd;
    let f = fixture();
    let root = f.cache.join("image");
    let a = consumer(&f, "a");
    let _prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
    let claim = std::fs::File::open(&f.cache).unwrap();
    assert_eq!(
        unsafe { libc::flock(claim.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
}

#[test]
fn h_partial_transfer_save_refuses_all_followup_without_previous_snapshot_fallback() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let prep = a.prepare_cache(&[CacheEntry { root: &root }]).unwrap();
    assert!(prep.handoff_fault(&a, &root, "partial-save").is_err());
    assert!(f.context.namespace.transaction().is_err());
    assert!(a.revalidate().is_err());
    assert!(f.context.local(&f.data.join("other")).is_err());
    assert_eq!(
        fs::read(sibling(&root, ".erofs").unwrap()).unwrap(),
        b"LOWER"
    );
}
#[test]
fn h_unpinned_warm_hit_may_hold_absent_digest_without_creating_it() {
    let f = fixture();
    let root = f.cache.join("image");
    dir_mode(&root, 0o700);
    write_mode(&root.join("content"), b"SOURCE", 0o600);
    write_mode(&sibling(&root, ".erofs").unwrap(), b"LOWER", 0o600);
    let a = consumer(&f, "a");
    let probe = a.probe_cache(&[CacheEntry { root: &root }]).unwrap();
    assert!(!probe.exists(&sibling(&root, ".digest").unwrap()).unwrap());
    let _a = probe.handoff(&a, &root).unwrap();
    assert!(!sibling(&root, ".digest").unwrap().exists());
}

#[test]
fn h1_serialized_continuation_metadata_does_not_authorize_issuance() {
    let f = fixture();
    let root = f.cache.join("image");
    populated(&root);
    let a = consumer(&f, "a");
    let probe = a.probe_cache(&[CacheEntry { root: &root }]).unwrap();
    let mut cache = f
        .context
        .namespace
        .transaction()
        .unwrap()
        .operation(&probe.lease.0.id)
        .unwrap()
        .cache
        .clone()
        .unwrap();
    drop(probe);
    cache.continuation = Some(super::super::catalog::CacheContinuationBinding {
        runtime: "0".repeat(32),
        runtime_epoch: 0,
        runtime_history_sha256: "0".repeat(64),
        consumer_state_sha256: "0".repeat(64),
    });
    assert!(issue(a, cache, None).is_err());
}
