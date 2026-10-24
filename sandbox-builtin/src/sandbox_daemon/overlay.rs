//! Per-sandbox host directory layout.
//!
//! Isolation model: a sandbox never boots on the shared image
//! cache. `__vm-boot` gets this sandbox's `root/` trampoline, served
//! read-only (`--rootfs-readonly`) so no guest write can reach the host, plus
//! the cached image as a read-only erofs lower. iii-init assembles `/` as
//! overlayfs with an in-guest tmpfs upper, so every write is private to the
//! VM and vanishes with it.
//!
//! Directory layout per sandbox (ID = UUID):
//!   /tmp/iii-sandbox/<uuid>/root/      (read-only virtiofs trampoline: `/dev/root`)
//!   /tmp/iii-sandbox/<uuid>/upper/     (reserved; the live upper is an in-guest tmpfs)
//!   /tmp/iii-sandbox/<uuid>/work/      (reserved)
//!   /tmp/iii-sandbox/<uuid>/merged/    (host-side workdir placeholder)
//!   /tmp/iii-sandbox/<uuid>/owner      (pid of the adapter that created it)

use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Where every sandbox's directory lives.
pub const SANDBOX_ROOT: &str = "/tmp/iii-sandbox";

/// File in a sandbox's directory naming the adapter process that owns it.
const OWNER_FILE: &str = "owner";

/// Directories iii-init's `overlay_root` creates on the virtiofs root before
/// it pivots (`/dev`, `/overlay-lower`, `/overlay-upperfs`, `/new-root`; see
/// `crates/iii-init/src/root_pivot.rs`). The sandbox trampoline is served
/// read-only, so they are pre-created host-side: the guest `mkdir` then gets
/// `EEXIST`, which iii-init tolerates, instead of `EROFS`.
pub const TRAMPOLINE_STAGING_DIRS: &[&str] =
    &["dev", "overlay-lower", "overlay-upperfs", "new-root"];

pub struct OverlayLayout {
    pub upper: PathBuf,
    pub work: PathBuf,
    pub merged: PathBuf,
    pub root: PathBuf,
}

impl OverlayLayout {
    pub fn for_sandbox(id: Uuid) -> Self {
        let base = PathBuf::from(SANDBOX_ROOT).join(id.to_string());
        Self {
            upper: base.join("upper"),
            work: base.join("work"),
            merged: base.join("merged"),
            root: base.join("root"),
        }
    }

    pub fn base(&self) -> PathBuf {
        self.upper.parent().expect("upper has parent").to_path_buf()
    }

    /// Create the directory structure, including the trampoline's
    /// [`TRAMPOLINE_STAGING_DIRS`]; does NOT mount anything. This helper
    /// exists so the layout is deterministic and easily reaped.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.upper)?;
        std::fs::create_dir_all(&self.work)?;
        std::fs::create_dir_all(&self.merged)?;
        for dir in TRAMPOLINE_STAGING_DIRS {
            std::fs::create_dir_all(self.root.join(dir))?;
        }
        std::fs::write(self.base().join(OWNER_FILE), std::process::id().to_string())
    }

    pub fn cleanup(&self) -> std::io::Result<()> {
        let _ = std::fs::remove_dir_all(self.base());
        Ok(())
    }
}

/// Remove the sandbox directories under `root` whose owning adapter is gone,
/// as a SIGKILLed adapter leaves them. A directory with no owner file, or whose
/// owner still runs (another adapter on this host), is left alone.
/// shortcut: a dead adapter's pid reused by another process keeps its dirs until
/// that process exits; record the owner's start time too if that ever matters.
pub fn sweep_stale(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| Uuid::parse_str(name).is_ok());
        let owner = std::fs::read_to_string(dir.join(OWNER_FILE))
            .ok()
            .and_then(|pid| pid.trim().parse::<i32>().ok());
        if let (true, Some(pid)) = (ours, owner)
            && pid > 0
            && nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
                == Err(nix::errno::Errno::ESRCH)
        {
            tracing::info!(dir = %dir.display(), owner = pid, "removing a dead adapter's sandbox dir");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_paths_are_deterministic() {
        let id = Uuid::nil();
        let l = OverlayLayout::for_sandbox(id);
        assert!(l.upper.ends_with("upper"));
        assert!(l.work.ends_with("work"));
        assert!(l.merged.ends_with("merged"));
    }

    #[test]
    fn ensure_dirs_and_cleanup_roundtrip() {
        let id = Uuid::new_v4();
        let l = OverlayLayout::for_sandbox(id);
        l.ensure_dirs().unwrap();
        assert!(l.upper.exists());
        l.cleanup().unwrap();
        assert!(!l.base().exists());
    }

    #[test]
    fn ensure_dirs_prepares_read_only_trampoline() {
        let l = OverlayLayout::for_sandbox(Uuid::new_v4());
        l.ensure_dirs().unwrap();
        assert!(l.root.starts_with(l.base()));
        for dir in TRAMPOLINE_STAGING_DIRS {
            assert!(l.root.join(dir).is_dir(), "missing trampoline dir {dir}");
        }
        // Nothing else: the trampoline must not carry image content.
        assert_eq!(
            std::fs::read_dir(&l.root).unwrap().count(),
            TRAMPOLINE_STAGING_DIRS.len()
        );
        l.cleanup().unwrap();
    }

    /// A SIGKILLed adapter's dirs go at the next start; a live adapter's, and
    /// anything without an owner file or a sandbox-id name, stay.
    #[test]
    fn sweep_removes_only_dirs_of_dead_owners() {
        let mut gone = std::process::Command::new("true").spawn().unwrap();
        gone.wait().unwrap();
        let root = tempfile::tempdir().unwrap();
        let dir = |name: &str, owner: Option<u32>| {
            let d = root.path().join(name);
            std::fs::create_dir_all(d.join("root")).unwrap();
            if let Some(pid) = owner {
                std::fs::write(d.join(OWNER_FILE), pid.to_string()).unwrap();
            }
            d
        };
        let dead = dir(&Uuid::new_v4().to_string(), Some(gone.id()));
        let live = dir(&Uuid::new_v4().to_string(), Some(std::process::id()));
        let unowned = dir(&Uuid::new_v4().to_string(), None);
        let foreign = dir("not-a-sandbox", Some(gone.id()));

        sweep_stale(root.path());

        assert!(!dead.exists());
        for kept in [live, unowned, foreign] {
            assert!(kept.exists(), "{}", kept.display());
        }
    }

    #[test]
    fn ensure_dirs_records_this_process_as_owner() {
        let l = OverlayLayout::for_sandbox(Uuid::new_v4());
        l.ensure_dirs().unwrap();
        let owner = std::fs::read_to_string(l.base().join(OWNER_FILE)).unwrap();
        l.cleanup().unwrap();
        assert_eq!(owner, std::process::id().to_string());
    }
}
