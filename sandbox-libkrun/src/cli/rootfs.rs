// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Shared rootfs cloning utility.

/// Clone a rootfs directory using APFS clonefile (macOS) or reflink (Linux).
pub fn clone_rootfs(base: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {}", e))?;
    }
    let status = std::process::Command::new("cp")
        .args(if cfg!(target_os = "macos") {
            vec!["-c", "-a"]
        } else {
            vec!["--reflink=auto", "-a"]
        })
        .arg(base.as_os_str())
        .arg(dest.as_os_str())
        .status()
        .map_err(|e| format!("cp: {}", e))?;
    if !status.success() {
        return Err(format!("cp exited with {}", status));
    }
    Ok(())
}

/// Reclone a readable source through the core-owned producer: the source is
/// captured under a Strict use (O_NOATIME descriptors, metadata, xattrs and
/// internal link groups inventoried before any destination effect), then its
/// bytes are copied by the SAME core writer that seals and publishes the exact
/// Reclone root. No `cp -a`, external stage pathname, chmod or completion bool.
/// An unreadable source (e.g. an already-shared Strict 0000 root without a
/// retained readable descriptor) fails capture and is refused before effects.
pub(crate) fn clone_rootfs_admitted(
    lease: &crate::worker_admission::Lease,
    base: &std::path::Path,
    dest: &std::path::Path,
) -> Result<(), String> {
    let source = lease
        .capture_payload(base)
        .map_err(|e| format!("capture readable clone source {}: {e}", base.display()))?;
    let guard = lease
        .begin(crate::worker_admission::MutationPlan::Reclone {
            root: dest.to_path_buf(),
        })
        .map_err(|e| e.to_string())?;
    source
        .produce(guard, dest)
        .and_then(|seal| seal.publish())
        .map_err(|e| format!("clone {} into {}: {e}", base.display(), dest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exact fixture modes, independent of the host umask: readable source
    /// data is 0644/0755 (as the core payload tests), private data 0600.
    fn r4_write(path: &std::path::Path, bytes: &[u8], mode: u32) {
        std::fs::write(path, bytes).unwrap();
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode)).unwrap();
    }
    fn r4_mkdir(path: &std::path::Path, mode: u32) {
        std::fs::create_dir(path).unwrap();
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode)).unwrap();
    }

    fn r4_admitted_clone_fixture(existing: bool) {
        use crate::worker_admission::{PathRequest, PathRole};
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
        let namespace = t.path().join("catalog");
        std::fs::create_dir(&namespace).unwrap();
        std::fs::set_permissions(&namespace, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&namespace).unwrap();
        let source = t.path().join("source");
        std::fs::create_dir_all(source.join("bin")).unwrap();
        std::fs::write(source.join("bin/hello"), b"readable source").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(source.join("bin"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::fs::set_permissions(
            source.join("bin/hello"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::os::unix::fs::symlink("bin/hello", source.join("hello")).unwrap();
        let source_identity = std::fs::metadata(&source).unwrap();
        let dest = t.path().join("managed");
        let old_identity = if existing {
            std::fs::create_dir(&dest).unwrap();
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o700)).unwrap();
            r4_write(&dest.join("OLD"), b"prior guest data", 0o600);
            Some(std::fs::metadata(&dest).unwrap())
        } else {
            None
        };
        let lease = crate::worker_admission::with_paths(
            context.local(&dest).unwrap(),
            &[
                PathRequest {
                    path: &source,
                    role: PathRole::Strict,
                },
                PathRequest {
                    path: &dest,
                    role: PathRole::MutableTree,
                },
            ],
        )
        .unwrap();
        // Runs the actual core capture -> producer -> seal route of the clone
        // entry (no cp -a, mock producer or chmod of the core's stage identity).
        clone_rootfs_admitted(&lease, &source, &dest).unwrap();
        assert_eq!(std::fs::metadata(&dest).unwrap().mode() & 0o7777, 0o755);
        assert_eq!(
            std::fs::metadata(dest.join("bin")).unwrap().mode() & 0o7777,
            0o755
        );
        assert_eq!(
            std::fs::metadata(dest.join("bin/hello")).unwrap().mode() & 0o7777,
            0o755
        );
        assert_eq!(
            std::fs::read(dest.join("bin/hello")).unwrap(),
            b"readable source"
        );
        assert_eq!(
            std::fs::read_link(dest.join("hello")).unwrap(),
            std::path::PathBuf::from("bin/hello")
        );
        let source_after = std::fs::metadata(&source).unwrap();
        assert_eq!(
            (
                source_identity.dev(),
                source_identity.ino(),
                source_identity.mode()
            ),
            (source_after.dev(), source_after.ino(), source_after.mode())
        );
        assert_eq!(
            std::fs::metadata(&namespace).unwrap().mode() & 0o7777,
            0o700
        );
        if let Some(old) = old_identity {
            let retained: Vec<_> = std::fs::read_dir(t.path())
                .unwrap()
                .filter_map(|entry| {
                    let entry = entry.unwrap();
                    let meta = entry.metadata().unwrap();
                    (meta.dev() == old.dev() && meta.ino() == old.ino()).then_some(entry.path())
                })
                .collect();
            assert_eq!(
                retained.len(),
                1,
                "replacement must retain the original tree inode"
            );
            assert_eq!(
                std::fs::read(retained[0].join("OLD")).unwrap(),
                b"prior guest data"
            );
            assert!(!dest.join("OLD").exists());
        }
        lease.clone().revalidate().unwrap();
    }

    #[test]
    fn r4_readable_0755_root_clones_through_the_actual_owned_producer() {
        r4_admitted_clone_fixture(false);
    }

    #[test]
    fn r4_readable_0755_reclone_preserves_the_prior_tree_and_final_metadata() {
        r4_admitted_clone_fixture(true);
    }

    #[test]
    fn r4_clone_refuses_a_substituted_strict_source_without_creating_destination() {
        use crate::worker_admission::{PathRequest, PathRole};
        use std::os::unix::fs::PermissionsExt;
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
        let namespace = t.path().join("catalog");
        std::fs::create_dir(&namespace).unwrap();
        std::fs::set_permissions(&namespace, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&namespace).unwrap();
        let source = t.path().join("source");
        r4_mkdir(&source, 0o755);
        r4_write(&source.join("data"), b"original", 0o644);
        let dest = t.path().join("destination");
        let lease = crate::worker_admission::with_paths(
            context.local(&dest).unwrap(),
            &[
                PathRequest {
                    path: &source,
                    role: PathRole::Strict,
                },
                PathRequest {
                    path: &dest,
                    role: PathRole::MutableTree,
                },
            ],
        )
        .unwrap();
        std::fs::rename(&source, t.path().join("held-source")).unwrap();
        r4_mkdir(&source, 0o755);
        r4_write(&source.join("data"), b"foreign", 0o644);
        assert!(clone_rootfs_admitted(&lease, &source, &dest).is_err());
        assert!(!dest.exists());
        assert_eq!(
            std::fs::read(t.path().join("held-source/data")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read(source.join("data")).unwrap(), b"foreign");
    }

    fn r4_clone_lease(
        t: &std::path::Path,
        source: &std::path::Path,
        dest: &std::path::Path,
    ) -> crate::worker_admission::Lease {
        use crate::worker_admission::{PathRequest, PathRole};
        use std::os::unix::fs::PermissionsExt;
        let namespace = t.join("catalog");
        std::fs::create_dir(&namespace).unwrap();
        std::fs::set_permissions(&namespace, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&namespace).unwrap();
        crate::worker_admission::with_paths(
            context.local(dest).unwrap(),
            &[
                PathRequest {
                    path: source,
                    role: PathRole::Strict,
                },
                PathRequest {
                    path: dest,
                    role: PathRole::MutableTree,
                },
            ],
        )
        .unwrap()
    }

    fn r4_data_extents(path: &std::path::Path) -> Vec<(i64, i64)> {
        use std::os::fd::AsRawFd;
        let file = std::fs::File::open(path).unwrap();
        let size = file.metadata().unwrap().len() as i64;
        let mut extents = Vec::new();
        let mut at = 0;
        while at < size {
            let start = unsafe { libc::lseek(file.as_raw_fd(), at, libc::SEEK_DATA) };
            if start < 0 {
                break;
            }
            let end = unsafe { libc::lseek(file.as_raw_fd(), start, libc::SEEK_HOLE) };
            assert!(end > start);
            extents.push((start, end.min(size)));
            at = end;
        }
        extents
    }

    /// R4 through the real clone entry: 1777/0777 children, an internal
    /// hardlink group and a sparse file keep exact metadata, link count and
    /// data/hole map; the source is untouched and no stage path escapes.
    #[test]
    fn r4_readable_clone_keeps_1777_internal_hardlinks_and_sparse_extents() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
        let source = t.path().join("source");
        std::fs::create_dir_all(source.join("tmp")).unwrap();
        std::fs::create_dir_all(source.join("shared")).unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(source.join("tmp"), std::fs::Permissions::from_mode(0o1777))
            .unwrap();
        std::fs::set_permissions(
            source.join("shared"),
            std::fs::Permissions::from_mode(0o777),
        )
        .unwrap();
        r4_write(&source.join("data"), b"linked bytes", 0o644);
        std::fs::hard_link(source.join("data"), source.join("shared/alias")).unwrap();
        {
            use std::io::{Seek, SeekFrom, Write};
            let mut sparse = std::fs::File::create(source.join("sparse")).unwrap();
            sparse.seek(SeekFrom::Start(8 * 1024 * 1024)).unwrap();
            sparse.write_all(b"tail").unwrap();
            sparse.set_len(16 * 1024 * 1024).unwrap();
            sparse
                .set_permissions(std::fs::Permissions::from_mode(0o644))
                .unwrap();
        }
        let source_extents = r4_data_extents(&source.join("sparse"));
        let source_modes: Vec<_> = ["", "tmp", "shared", "data", "sparse"]
            .iter()
            .map(|name| std::fs::symlink_metadata(source.join(name)).unwrap().mode())
            .collect();
        let dest = t.path().join("managed");
        let lease = r4_clone_lease(t.path(), &source, &dest);
        clone_rootfs_admitted(&lease, &source, &dest).unwrap();
        assert_eq!(std::fs::metadata(&dest).unwrap().mode() & 0o7777, 0o755);
        assert_eq!(
            std::fs::metadata(dest.join("tmp")).unwrap().mode() & 0o7777,
            0o1777
        );
        assert_eq!(
            std::fs::metadata(dest.join("shared")).unwrap().mode() & 0o7777,
            0o777
        );
        let data = std::fs::metadata(dest.join("data")).unwrap();
        let alias = std::fs::metadata(dest.join("shared/alias")).unwrap();
        assert_eq!((data.dev(), data.ino()), (alias.dev(), alias.ino()));
        assert_eq!(data.nlink(), 2);
        assert_eq!(
            std::fs::read(dest.join("shared/alias")).unwrap(),
            b"linked bytes"
        );
        assert_eq!(r4_data_extents(&dest.join("sparse")), source_extents);
        let sparse = std::fs::read(dest.join("sparse")).unwrap();
        assert_eq!(sparse.len(), 16 * 1024 * 1024);
        assert_eq!(&sparse[8 * 1024 * 1024..8 * 1024 * 1024 + 4], b"tail");
        let after: Vec<_> = ["", "tmp", "shared", "data", "sparse"]
            .iter()
            .map(|name| std::fs::symlink_metadata(source.join(name)).unwrap().mode())
            .collect();
        assert_eq!(after, source_modes, "the source is never chmodded");
        assert!(!std::fs::read_dir(t.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".iii-stage")
        }));
        lease.revalidate().unwrap();
    }

    /// R4 user-approved refusal, and only this one: a new clone of an already
    /// shared Strict 0000 source without a retained readable descriptor is
    /// refused before any destination effect, and the source is not chmodded.
    #[test]
    fn r4_shared_0000_source_without_readable_fd_is_refused_before_effects() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "the 0000 refusal premise requires an owner without DAC override"
        );
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
        let source = t.path().join("source");
        r4_mkdir(&source, 0o755);
        r4_write(&source.join("data"), b"shared", 0o644);
        // Already shared as 0000 when this operation admitted it as Strict.
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o000)).unwrap();
        let dest = t.path().join("managed");
        let lease = r4_clone_lease(t.path(), &source, &dest);
        let result = clone_rootfs_admitted(&lease, &source, &dest);
        let mode = std::fs::symlink_metadata(&source).unwrap().mode() & 0o7777;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(mode, 0o000, "refusal must not chmod the shared source");
        assert!(!dest.exists());
        assert_eq!(std::fs::read(source.join("data")).unwrap(), b"shared");
    }

    /// R4 negative: a source hardlink that escapes the captured tree is a
    /// metadata-loss risk and is refused, never copied as an unrelated file.
    #[test]
    fn r4_clone_refuses_a_source_hardlink_outside_the_captured_tree() {
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
        let source = t.path().join("source");
        r4_mkdir(&source, 0o755);
        r4_write(&source.join("data"), b"linked", 0o644);
        std::fs::hard_link(source.join("data"), t.path().join("external")).unwrap();
        let dest = t.path().join("managed");
        let lease = r4_clone_lease(t.path(), &source, &dest);
        assert!(clone_rootfs_admitted(&lease, &source, &dest).is_err());
        assert!(!dest.exists());
        assert_eq!(std::fs::read(t.path().join("external")).unwrap(), b"linked");
    }

    #[test]
    fn clone_rootfs_copies_directory() {
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("bin")).unwrap();
        std::fs::write(src.path().join("bin/hello"), "world").unwrap();
        std::fs::write(src.path().join("root.txt"), "data").unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("cloned");

        clone_rootfs(src.path(), &dest).unwrap();

        assert!(dest.join("bin/hello").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("bin/hello")).unwrap(),
            "world"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("root.txt")).unwrap(),
            "data"
        );
    }

    #[test]
    fn clone_rootfs_creates_parent_dirs() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("file.txt"), "content").unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("deep").join("nested").join("clone");

        clone_rootfs(src.path(), &dest).unwrap();

        assert!(dest.join("file.txt").exists());
    }

    #[test]
    fn clone_rootfs_fails_on_missing_source() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does_not_exist");
        let dest = tmp.path().join("dest");

        let result = clone_rootfs(&missing, &dest);
        assert!(result.is_err());
    }
}
