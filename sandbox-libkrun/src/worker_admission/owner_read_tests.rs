// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN: Linux6.13 getxattrat/ACL filesystem matrix remains admitted work.
use super::*;
use crate::worker_admission::{PathRequest, testing, with_paths};
use std::os::unix::fs::PermissionsExt;
fn fixture() -> (
    tempfile::TempDir,
    crate::worker_admission::AdmissionContext,
    PathBuf,
) {
    let root = tempfile::tempdir().unwrap();
    let namespace = root.path().join("catalog");
    let cache = root.path().join("cache");
    for p in [&namespace, &cache] {
        std::fs::create_dir(p).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let context = testing::provision_context(&namespace).unwrap();
    (root, context, cache)
}
fn lease(context: &crate::worker_admission::AdmissionContext, cache: &Path) -> Lease {
    with_paths(
        context.local(&cache.with_file_name("state")).unwrap(),
        &[PathRequest {
            path: cache,
            role: PathRole::ManagedCacheRead,
        }],
    )
    .unwrap()
}
fn acl_bytes(owner: u16) -> Vec<u8> {
    let mut bytes = 2u32.to_le_bytes().to_vec();
    for (tag, permissions, id) in [
        (1u16, owner, u32::MAX),
        (2, 4, 12345),
        (4, 0, u32::MAX),
        (16, 0, u32::MAX),
        (32, 0, u32::MAX),
    ] {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(permissions.to_le_bytes());
        bytes.extend(id.to_le_bytes());
    }
    bytes
}
#[cfg(target_os = "linux")]
fn set_acl(file: &File, attribute: &str, bytes: &[u8]) {
    let attribute = std::ffi::CString::new(attribute).unwrap();
    assert_eq!(
        unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                attribute.as_ptr(),
                bytes.as_ptr().cast(),
                bytes.len(),
                0,
            )
        },
        0,
        "fixture requires actual POSIX ACL support, not a skipped PASS"
    );
}
#[test]
fn e_acl_uapi_layout_and_owner_grant_comparison_preserve_named_mask_entries() {
    assert_eq!(std::mem::size_of::<XattrArgs>(), 16);
    assert_eq!(std::mem::align_of::<XattrArgs>(), 8);
    let original = acl_bytes(0);
    let granted = acl_bytes(5);
    assert!(granted_access_matches(
        &Some(original.clone()),
        &Some(granted.clone()),
        0o500
    ));
    let mut changed = granted;
    changed[16] = 7;
    assert!(!granted_access_matches(
        &Some(original.clone()),
        &Some(changed),
        0o500
    ));
    assert_eq!(original, acl_bytes(0));
}
#[test]
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn e_0000_directory_records_access_and_default_acl_before_grant_and_restores_exactly() {
    assert_ne!(
        uid(),
        0,
        "must run as nonroot; root masks the required lookup capability"
    );
    let (_tmp, context, cache) = fixture();
    // Armed before the chmod, bound after the TempDir: it only acts at scope
    // end, after the exact-restore oracles below, so the tree stays removable.
    let _access = testing::OwnerAccessOnDrop::arm(&cache);
    let file = File::open(&cache).unwrap();
    let access = acl_bytes(0);
    let defaults = acl_bytes(7);
    set_acl(&file, "system.posix_acl_access", &access);
    set_acl(&file, "system.posix_acl_default", &defaults);
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0)).unwrap();
    let lease = lease(&context, &cache);
    let original = std::fs::symlink_metadata(&cache).unwrap();
    let guard = lease.owner_read_cache(&cache).unwrap();
    {
        let tx = context.namespace.transaction().unwrap();
        let permission = &tx.operation(&lease.0.id).unwrap().journals[0].permissions[0];
        assert!(permission.acl_captured && permission.acl_before_grant);
        assert_eq!(permission.acl, Some(hex::encode(&access)));
        assert_eq!(permission.default_acl, Some(hex::encode(&defaults)));
        assert_eq!(permission.original.mode, original.mode());
    }
    assert_eq!(
        guard.original_metadata(&cache).unwrap().mode(),
        original.mode()
    );
    guard.finish().unwrap();
    // The product itself restored the exact original mode (also implied by
    // capture_acls' same_inode check below); the cleanup guard has not run.
    assert_eq!(
        std::fs::symlink_metadata(&cache).unwrap().mode(),
        original.mode()
    );
    let parent = Directory::open(cache.parent().unwrap()).unwrap();
    let child = open_path(&parent.file, cache.file_name().unwrap()).unwrap();
    assert_eq!(
        capture_acls(
            &parent,
            cache.file_name().unwrap(),
            &child,
            &Identity::metadata(&original)
        )
        .unwrap(),
        Acls {
            access: Some(access),
            default: Some(defaults)
        }
    );
}
#[test]
fn e_pregrant_faults_never_chmod_and_old_incomplete_records_are_not_inferred() {
    for fault in [
        GrantFault::AccessSyscall(libc::ENOSYS),
        GrantFault::AccessSyscall(libc::EINVAL),
        GrantFault::AccessSyscall(libc::EACCES),
        GrantFault::AccessSyscall(libc::ENOTSUP),
        GrantFault::DefaultRead,
        GrantFault::Journal,
        GrantFault::AfterIntent,
    ] {
        let (_tmp, context, cache) = fixture();
        let _access = testing::OwnerAccessOnDrop::arm(&cache);
        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0)).unwrap();
        let lease = lease(&context, &cache);
        let id = lease.0.id.clone();
        assert!(OwnerReadGuard::begin_inner(lease.clone(), &cache, Some(fault)).is_err());
        assert_eq!(std::fs::symlink_metadata(&cache).unwrap().mode() & 0o777, 0);
        let tx = context.namespace.transaction().unwrap();
        let journal = &tx.operation(&id).unwrap().journals[0];
        if matches!(
            fault,
            GrantFault::AccessSyscall(_) | GrantFault::DefaultRead | GrantFault::Journal
        ) {
            assert!(journal.permissions.is_empty());
        } else {
            assert_eq!(journal.permissions.len(), 1);
            assert!(journal.permissions[0].acl_before_grant);
        }
    }
}
#[test]
fn e_old_aftergrant_acl_evidence_stays_unresolved() {
    let (_tmp, context, cache) = fixture();
    let lease = lease(&context, &cache);
    let mut guard = OwnerReadGuard::begin(lease, &cache).unwrap();
    let mut tx = context.namespace.transaction().unwrap();
    let journal = &mut tx.operation_mut(&guard.lease.0.id).unwrap().journals[0];
    journal.permissions.push(PermissionIntent {
        path: HostPath::new(&cache).unwrap(),
        parent: Identity::of(&Directory::open(cache.parent().unwrap()).unwrap().file).unwrap(),
        original: Identity::metadata(&std::fs::metadata(&cache).unwrap()),
        granted_mode: 0o700,
        acl: None,
        default_acl: None,
        acl_captured: true,
        acl_before_grant: false,
        restored: false,
        ctime_changed: true,
    });
    let mut record = journal.clone();
    drop(tx);
    assert!(!restore_verified(&mut record, uid()).unwrap());
    guard.finish().unwrap();
}
#[test]
fn e_acl_single_component_and_replacement_checks_refuse_before_permission_write() {
    let (_tmp, _context, cache) = fixture();
    let parent = Directory::open(cache.parent().unwrap()).unwrap();
    let child = open_path(&parent.file, cache.file_name().unwrap()).unwrap();
    let original = Identity::of(&child).unwrap();
    assert!(capture_acls(&parent, "../cache".as_ref(), &child, &original).is_err());
    std::fs::rename(&cache, cache.with_file_name("retained")).unwrap();
    std::fs::create_dir(&cache).unwrap();
    assert!(capture_acls(&parent, cache.file_name().unwrap(), &child, &original).is_err());
}
