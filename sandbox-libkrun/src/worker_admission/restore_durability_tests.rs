// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN (R9): exact inode durability before `restored`/Completed;
//! no parent-only, syncfs, mode-observation or regrant completion oracle.
use super::*;
use crate::worker_admission::{PathRequest, testing, with_paths};
use std::os::unix::fs::PermissionsExt;
/// All cache objects exist BEFORE the original Lease declares the cache.
fn setup_with(
    mode: u32,
    extra: impl FnOnce(&Path),
) -> (
    tempfile::TempDir,
    crate::worker_admission::AdmissionContext,
    Lease,
    PathBuf,
) {
    let temp = testing::private_tempdir().unwrap();
    let ns = temp.path().join("catalog");
    let cache = temp.path().join("cache");
    for p in [&ns, &cache] {
        std::fs::create_dir(p).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let leaf = cache.join("leaf");
    std::fs::write(&leaf, b"payload").unwrap();
    std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(mode)).unwrap();
    extra(&cache);
    let context = testing::provision_context(&ns).unwrap();
    let lease = with_paths(
        context.local(&temp.path().join("consumer")).unwrap(),
        &[PathRequest {
            path: &cache,
            role: PathRole::ManagedCacheRead,
        }],
    )
    .unwrap();
    (temp, context, lease, cache)
}
fn setup(
    mode: u32,
) -> (
    tempfile::TempDir,
    crate::worker_admission::AdmissionContext,
    Lease,
    PathBuf,
) {
    setup_with(mode, |_| ())
}
/// The open owner-read journal of `guard`, cloned, plus an exact intent for
/// `cache/leaf` captured from its CURRENT original state.
fn journal_with_intent(
    guard: &OwnerReadGuard,
    cache: &Path,
    restored: bool,
) -> super::super::catalog::Journal {
    let parent = Directory::open(cache).unwrap();
    let pin = open_path(&parent.file, "leaf".as_ref()).unwrap();
    let id = Identity::of(&pin).unwrap();
    let acls = capture_acls(&parent, "leaf".as_ref(), &pin, &id).unwrap();
    let tx = guard.lease.0.namespace.transaction().unwrap();
    let mut journal = tx
        .operation(&guard.lease.0.id)
        .unwrap()
        .journals
        .iter()
        .find(|j| j.id == guard.id)
        .unwrap()
        .clone();
    drop(tx);
    journal.permissions.push(PermissionIntent {
        path: HostPath::new(&cache.join("leaf")).unwrap(),
        parent: Identity::of(&parent.file).unwrap(),
        original: id.clone(),
        granted_mode: (id.mode & 0o7777) | 0o400,
        acl: acls.access.map(hex::encode),
        default_acl: acls.default.map(hex::encode),
        acl_captured: true,
        acl_before_grant: true,
        restored,
        ctime_changed: true,
    });
    journal
}
#[test]
fn r9_live_0000_restoration_fsyncs_retained_exact_inode_before_completion() {
    assert_ne!(uid(), 0, "unprivileged test required");
    let (_t, ctx, lease, cache) = setup(0);
    let mut guard = OwnerReadGuard::begin(lease.clone(), &cache).unwrap();
    let file = guard.open_file(&cache.join("leaf")).unwrap();
    drop(file);
    guard.finish().unwrap();
    let tx = ctx.namespace.transaction().unwrap();
    let j = tx.operation(&lease.0.id).unwrap().journals.last().unwrap();
    assert!(j.committed);
    assert!(!j.permissions.is_empty());
    assert!(j.permissions.iter().all(|p| p.restored));
    assert_eq!(
        std::fs::metadata(cache.join("leaf")).unwrap().mode() & 0o777,
        0
    );
}
#[test]
fn r9_post_chmod_pre_inode_sync_error_does_not_publish_restored() {
    assert_ne!(uid(), 0, "unprivileged test required");
    // Mode observed (post-chmod), fsync failed, flushed but unrecorded: three
    // distinct live faults, one honest restart result for an unreadable inode.
    for fault in [
        GrantFault::PostChmod,
        GrantFault::InodeSync,
        GrantFault::AfterInodeSync,
    ] {
        let (_t, ctx, lease, cache) = setup(0);
        let mut guard = OwnerReadGuard::begin(lease.clone(), &cache).unwrap();
        drop(guard.open_file(&cache.join("leaf")).unwrap());
        guard.fault = Some(fault);
        assert!(guard.finish().is_err());
        let tx = ctx.namespace.transaction().unwrap();
        let op = tx.operation(&lease.0.id).unwrap();
        let mut journal = op.journals.last().unwrap().clone();
        assert!(!journal.committed);
        assert!(journal.permissions.iter().any(|p| !p.restored));
        assert!(op.uncertain);
        drop(tx);
        // Source mode equality is visible, but after restart no syncable FD can
        // be opened. Recovery leaves it unresolved even with the original ACLs.
        assert_eq!(
            std::fs::metadata(cache.join("leaf")).unwrap().mode() & 0o777,
            0
        );
        assert!(!restore_verified(&mut journal, uid()).unwrap());
        assert!(journal.permissions.iter().any(|p| !p.restored));
        // No regrant: the inode stays exactly at its original mode.
        assert_eq!(
            std::fs::metadata(cache.join("leaf")).unwrap().mode() & 0o777,
            0
        );
    }
}
#[test]
fn r9_observed_readable_original_mode_requires_and_receives_exact_inode_barrier() {
    let (_t, _ctx, lease, cache) = setup(0o600);
    let guard = OwnerReadGuard::begin(lease, &cache).unwrap();
    let mut journal = journal_with_intent(&guard, &cache, false);
    assert!(restore_verified(&mut journal, uid()).unwrap());
    assert!(journal.permissions.iter().all(|p| p.restored));
    guard.finish().unwrap();
}
#[test]
fn r9_recorded_restoration_is_the_durable_barrier_for_an_unreadable_inode() {
    assert_ne!(uid(), 0, "unprivileged test required");
    let (_t, _ctx, lease, cache) = setup(0);
    let guard = OwnerReadGuard::begin(lease, &cache).unwrap();
    // `restored` is written only after the exact-inode fsync; a crash between
    // two inodes must not wedge an inode whose barrier was already recorded.
    let mut recorded = journal_with_intent(&guard, &cache, true);
    assert!(restore_verified(&mut recorded, uid()).unwrap());
    // The record never blesses a later change of the observed metadata.
    let mut changed = journal_with_intent(&guard, &cache, true);
    std::fs::set_permissions(cache.join("leaf"), std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(!restore_verified(&mut changed, uid()).unwrap());
    std::fs::set_permissions(cache.join("leaf"), std::fs::Permissions::from_mode(0)).unwrap();
    // Without the record, the same observed 0000 state stays unresolved.
    let mut observed = journal_with_intent(&guard, &cache, false);
    assert!(!restore_verified(&mut observed, uid()).unwrap());
    assert!(observed.permissions.iter().any(|p| !p.restored));
    drop(guard);
}
#[test]
fn r9_wrong_or_opath_descriptor_cannot_claim_inode_durability() {
    let (_t, _ctx, _lease, cache) = setup_with(0o600, |cache| {
        std::fs::write(cache.join("other"), b"foreign").unwrap();
        std::fs::set_permissions(cache.join("other"), std::fs::Permissions::from_mode(0o600))
            .unwrap();
    });
    let parent = Directory::open(&cache).unwrap();
    let exact = parent.file("leaf", false).unwrap();
    let pin = open_path(&parent.file, "leaf".as_ref()).unwrap();
    let identity = Identity::of(&pin).unwrap();
    // Injected fsync failure, an O_PATH (unsyncable) descriptor and a foreign
    // inode are all refusals; only the exact readable inode is a barrier.
    assert!(sync_restored_inode(&exact, &pin, &identity, true).is_err());
    assert!(sync_restored_inode(&pin, &pin, &identity, false).is_err());
    let foreign = parent.file("other", false).unwrap();
    assert!(sync_restored_inode(&foreign, &pin, &identity, false).is_err());
    sync_restored_inode(&exact, &pin, &identity, false).unwrap();
}
#[test]
fn r9_postchmod_readable_directory_recovery_proves_inode_sync_not_only_parent() {
    assert_ne!(uid(), 0, "unprivileged recovery oracle required");
    let (_t, ctx, lease, cache) = setup_with(0o600, |cache| {
        let directory = cache.join("read-only-dir");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o400)).unwrap();
    });
    let directory = cache.join("read-only-dir");
    let mut guard = OwnerReadGuard::begin(lease.clone(), &cache).unwrap();
    guard.fault = Some(GrantFault::PostChmod);
    assert!(guard.finish().is_err());
    let mut journal = {
        let tx = ctx.namespace.transaction().unwrap();
        tx.operation(&lease.0.id)
            .unwrap()
            .journals
            .last()
            .unwrap()
            .clone()
    };
    assert!(journal.permissions.iter().any(|p| !p.restored));
    // Original mode 0400 is readable: recovery opens the exact inode and
    // fsyncs it before recording restoration.
    assert!(restore_verified(&mut journal, uid()).unwrap());
    assert!(journal.permissions.iter().all(|p| p.restored));
    assert_eq!(std::fs::metadata(&directory).unwrap().mode() & 0o777, 0o400);
}
