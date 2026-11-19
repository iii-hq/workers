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

/// Reclone into an owned stage. The copy process inherits only the selected
/// operation FD; the parent retains its lease until a successful wait.
pub(crate) fn clone_rootfs_admitted(
    lease: &crate::worker_admission::Lease,
    base: &std::path::Path,
    dest: &std::path::Path,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    // H keeps cache consumption Strict. This legacy clone remains supported for
    // readable trees; source0000 requires an exclusive pre-handoff copy plan.
    // Do not chmod the shared source or obtain a new writer after read handoff.
    let used = lease.use_path(base).map_err(|e| e.to_string())?;
    let mut mutation = lease
        .begin(crate::worker_admission::MutationPlan::Reclone {
            root: dest.to_path_buf(),
        })
        .map_err(|e| e.to_string())?;
    let stage = mutation.stage_tree(dest).map_err(|e| e.to_string())?;
    let child_admission = lease.export_child().map_err(|e| e.to_string())?;
    let fd = child_admission
        .duplicate_for_exec()
        .map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new("cp");
    cmd.args(if cfg!(target_os = "macos") {
        vec!["-c", "-a"]
    } else {
        vec!["--reflink=auto", "-a"]
    })
    .arg(used.path().join("."))
    .arg(stage.path());
    unsafe {
        cmd.pre_exec(move || {
            let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFD);
            if flags < 0
                || libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    mutation.checkpoint().map_err(|e| e.to_string())?;
    let mut child = cmd.spawn().map_err(|e| format!("cp: {e}"))?;
    let held = crate::worker_admission::WriterLease::new(vec![lease.clone()]);
    let status = child.wait().map_err(|e| format!("wait cp: {e}"))?;
    held.reaped();
    drop(child_admission);
    if !status.success() {
        return Err(format!("cp exited with {status}"));
    }
    used.revalidate().map_err(|e| e.to_string())?;
    mutation.checkpoint().map_err(|e| e.to_string())?;
    mutation.publish_tree(stage).map_err(|e| e.to_string())?;
    mutation.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            std::fs::write(dest.join("OLD"), b"prior guest data").unwrap();
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
        // Runs the actual joined cp -a source/. STAGE path, not a mock producer
        // or direct chmod of the core's stage identity. SOURCEONLY: authored only.
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
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("data"), b"original").unwrap();
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
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("data"), b"foreign").unwrap();
        assert!(clone_rootfs_admitted(&lease, &source, &dest).is_err());
        assert!(!dest.exists());
        assert_eq!(
            std::fs::read(t.path().join("held-source/data")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read(source.join("data")).unwrap(), b"foreign");
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
