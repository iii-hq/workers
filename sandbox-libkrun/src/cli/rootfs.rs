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
