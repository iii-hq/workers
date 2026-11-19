// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! AUTHORED_NOTRUN (H1-OVERLAP). Core-level H1 warm append over a proved
//! cache read whose parent directory gains a sibling subdirectory, through the
//! real spawn -> import -> retirement path. Every fixture object has an
//! explicit mode.
use super::*;
use crate::worker_admission::{CacheEntry, PathRequest, testing, with_paths};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
fn mkdir_mode(path: &Path, mode: u32) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}
fn write_mode(path: &Path, bytes: &[u8], mode: u32) {
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}
fn side(root: &Path, suffix: &str) -> PathBuf {
    let mut name = root.file_name().unwrap().to_os_string();
    name.push(suffix);
    root.with_file_name(name)
}
/// A warm image: root directory plus `.erofs` and `.digest` siblings.
fn populate(root: &Path) {
    mkdir_mode(root, 0o700);
    write_mode(&root.join("content"), b"SOURCE", 0o600);
    write_mode(&side(root, ".erofs"), b"LOWER", 0o600);
    write_mode(&side(root, ".digest"), b"sha256:fixture", 0o600);
}
fn local_runtime(child: &RuntimeChild) -> RuntimeWriter {
    let exported = child.lease.export_child().unwrap();
    let context = crate::worker_admission::AdmissionContext {
        namespace: child.lease.0.namespace.clone(),
    };
    let imported = context
        .import_child(exported.duplicate_for_exec().unwrap())
        .unwrap();
    import_runtime_writer(
        &imported,
        duplicate(child.sentinel.as_ref().unwrap()).unwrap(),
    )
    .unwrap()
}
struct Ready {
    _temp: tempfile::TempDir,
    _context: crate::worker_admission::AdmissionContext,
    lease: Lease,
    receipt: WriterRetirement,
    data: PathBuf,
}
/// One actual owned runtime retirement of a `vm` lifecycle holding a Strict
/// input, so the receipt can continue it with cache reads.
async fn ready() -> Ready {
    let temp = tempfile::tempdir().unwrap();
    let catalog = temp.path().join("catalog");
    let data = temp.path().join("data");
    mkdir_mode(&catalog, 0o700);
    mkdir_mode(&data, 0o700);
    let context = testing::provision_context(&catalog).unwrap();
    let input = data.join("input");
    write_mode(&input, b"READ", 0o600);
    let vm = data.join("vm");
    mkdir_mode(&vm, 0o700);
    let lease = with_paths(
        context.local(&vm).unwrap(),
        &[PathRequest {
            path: &input,
            role: PathRole::Strict,
        }],
    )
    .unwrap();
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg("-c").arg("exit 0");
    let child = lease.prepare_runtime(&[]).unwrap().spawn(command).unwrap();
    let runtime = local_runtime(&child);
    let read = runtime.use_path(&input).unwrap();
    read.revalidate().unwrap();
    drop(read);
    drop(runtime);
    let completion = child.wait().await;
    assert!(completion.os_status().unwrap().success());
    let receipt = completion.retirement().unwrap().clone();
    Ready {
        _temp: temp,
        _context: context,
        lease,
        receipt,
        data,
    }
}
/// The receipt's first warm append of `shared`.
fn append_shared(r: &Ready, shared: &Path) {
    populate(shared);
    r.receipt
        .probe_cache(&r.lease, &[CacheEntry { root: shared }])
        .unwrap()
        .handoff(&r.lease, shared)
        .unwrap();
}
fn reprobe(r: &Ready, shared: &Path, pinned: &Path) -> io::Result<()> {
    r.receipt
        .probe_cache(
            &r.lease,
            &[CacheEntry { root: shared }, CacheEntry { root: pinned }],
        )?
        .handoff(&r.lease, pinned)
        .map(drop)
}
#[tokio::test]
async fn h1_new_sibling_subdirectory_beside_a_proved_read_keeps_warm_append_only_continuation() {
    let r = ready().await;
    let shared = r.data.join("shared");
    append_shared(&r, &shared);
    let before = std::fs::metadata(&r.data).unwrap().nlink();
    // A new candidate directory beside the proved read: the parent's link count
    // moves, no recorded inode, owner, mode or file link count does.
    let pinned = r.data.join("pinned");
    populate(&pinned);
    let after = std::fs::metadata(&r.data).unwrap().nlink();
    // ext4/tmpfs count subdirectories in nlink (btrfs keeps 1): non-vacuous there.
    if before > 1 {
        assert_eq!(after, before + 1);
    }
    reprobe(&r, &shared, &pinned).expect("a sibling subdirectory does not change the proved read");
    let tx = r.lease.0.namespace.transaction().unwrap();
    let op = tx.operation(&r.lease.0.id).unwrap();
    for path in [&shared, &pinned] {
        assert_eq!(
            op.entries
                .iter()
                .filter(|e| e.path.get().unwrap() == *path && e.role == PathRole::Strict)
                .count(),
            1
        );
    }
    assert_eq!(op.cache_reads.len(), 2);
    drop(tx);
    // Append-only: the continuation still cannot prepare over held reads.
    assert_eq!(
        r.receipt
            .prepare_cache(
                &r.lease,
                &[CacheEntry { root: &shared }, CacheEntry { root: &pinned }],
            )
            .err()
            .map(|e| e.kind()),
        Some(io::ErrorKind::WouldBlock)
    );
    drop(r.lease.use_path(&shared).expect("proved read stays usable"));
    drop(r.lease.use_path(&pinned).expect("appended read is usable"));
}
#[tokio::test]
async fn h1_replaced_rechmodded_or_hardlinked_proved_read_still_refuses_continuation() {
    for change in ["replace", "chmod", "hardlink"] {
        let r = ready().await;
        let shared = r.data.join("shared");
        append_shared(&r, &shared);
        let pinned = r.data.join("pinned");
        populate(&pinned);
        let expected = match change {
            "replace" => {
                std::fs::rename(&shared, r.data.join("retained")).unwrap();
                mkdir_mode(&shared, 0o700);
                "admitted identity changed outside an owned mutation"
            }
            "chmod" => {
                std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o500)).unwrap();
                "admitted identity changed outside an owned mutation"
            }
            _ => {
                std::fs::hard_link(side(&shared, ".erofs"), r.data.join("alias")).unwrap();
                "symlink, hardlinked or special input"
            }
        };
        let refused = reprobe(&r, &shared, &pinned)
            .err()
            .unwrap_or_else(|| panic!("{change}: a changed proved read must refuse"));
        assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{change}");
        assert_eq!(refused.to_string(), expected, "{change}");
        let tx = r.lease.0.namespace.transaction().unwrap();
        let op = tx.operation(&r.lease.0.id).unwrap();
        assert_eq!(op.cache_reads.len(), 1, "{change}: nothing appended");
        assert!(
            !op.entries.iter().any(|e| e.path.get().unwrap() == pinned),
            "{change}"
        );
        drop(tx);
        if change == "chmod" {
            std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
}
