//! Per-sandbox host directory layout.
//!
//! Isolation model: a sandbox never boots on the shared image
//! cache. `__vm-boot` gets this sandbox's `root/` trampoline, served
//! read-only (`--rootfs-readonly`) so no guest write can reach the host, plus
//! the cached image as a read-only erofs lower. iii-init assembles `/` as
//! overlayfs with an in-guest tmpfs upper, so every write is private to the
//! VM and vanishes with it.
//!
//! Directory layout per sandbox (ID = UUID), under the per-user root
//! `/tmp/iii-sandbox-<euid>` (0700, see [`sandbox_root`]):
//!   <root>/<uuid>/root/      (read-only virtiofs trampoline: `/dev/root`)
//!   <root>/<uuid>/upper/     (reserved; the live upper is an in-guest tmpfs)
//!   <root>/<uuid>/work/      (reserved)
//!   <root>/<uuid>/merged/    (host-side workdir placeholder)
//!   <root>/<uuid>/owner      (pid of the adapter that created it)

use std::fs::File;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Where every sandbox's directory lives: one root per user, so another local
/// user can neither pre-create it nor plant files or sockets in it.
pub fn sandbox_root() -> PathBuf {
    PathBuf::from(format!("/tmp/iii-sandbox-{}", euid()))
}

fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Create `root` (0700) if missing, and refuse it unless it is a directory of
/// this user, not a symlink. A lax mode is tightened: only this user can have
/// made a directory this user owns.
fn ensure_private_root(root: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(root) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    let meta = std::fs::symlink_metadata(root)?;
    if !meta.is_dir() || meta.uid() != euid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} is not a directory owned by this user; remove it",
                root.display()
            ),
        ));
    }
    if meta.mode() & 0o077 != 0 {
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Create `path` as a new 0600 file. `create_new` (O_EXCL) also refuses a
/// symlink planted there, dangling or not.
pub(crate) fn create_private(path: &Path) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

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
        let base = sandbox_root().join(id.to_string());
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
        let base = self.base();
        ensure_private_root(base.parent().expect("base has a parent"))?;
        std::fs::create_dir_all(&self.upper)?;
        std::fs::create_dir_all(&self.work)?;
        std::fs::create_dir_all(&self.merged)?;
        for dir in TRAMPOLINE_STAGING_DIRS {
            std::fs::create_dir_all(self.root.join(dir))?;
        }
        create_private(&base.join(OWNER_FILE))?.write_all(std::process::id().to_string().as_bytes())
    }

    pub fn cleanup(&self) -> std::io::Result<()> {
        let _ = std::fs::remove_dir_all(self.base());
        Ok(())
    }
}

/// Remove the sandbox directories under `root` whose owning adapter is gone,
/// as a SIGKILLed adapter leaves them. A directory with no owner file, or whose
/// owner still runs (another adapter on this host), is left alone, and so is a
/// `root` that is not this user's private directory.
/// shortcut: a dead adapter's pid reused by another process keeps its dirs until
/// that process exits; record the owner's start time too if that ever matters.
pub fn sweep_stale(root: &Path) {
    if let Err(e) = ensure_private_root(root) {
        tracing::warn!(root = %root.display(), error = %e, "not sweeping the sandbox root");
        return;
    }
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

    /// Each user gets a root of their own, closed to everyone else, so a second
    /// local user neither collides with nor tampers with this one's sandboxes.
    #[test]
    fn the_sandbox_root_is_this_users_private_dir() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let root = sandbox_root();
        assert!(root.ends_with(format!("iii-sandbox-{}", euid())));
        let l = OverlayLayout::for_sandbox(Uuid::new_v4());
        assert_eq!(l.base().parent(), Some(root.as_path()));
        l.ensure_dirs().unwrap();
        l.cleanup().unwrap();
        let meta = std::fs::symlink_metadata(&root).unwrap();
        assert!(meta.is_dir());
        assert_eq!(meta.uid(), euid());
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
    }

    /// A root pre-created as a symlink (to a dir the planter controls) is
    /// refused, by `ensure_dirs` and by the sweep alike.
    #[test]
    fn a_symlinked_root_is_refused() {
        let mut gone = std::process::Command::new("true").spawn().unwrap();
        gone.wait().unwrap();
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("real");
        let dead = real.join(Uuid::new_v4().to_string());
        std::fs::create_dir_all(&dead).unwrap();
        std::fs::write(dead.join(OWNER_FILE), gone.id().to_string()).unwrap();
        let root = t.path().join("root");
        std::os::unix::fs::symlink(&real, &root).unwrap();

        assert!(ensure_private_root(&root).is_err());
        sweep_stale(&root);
        assert!(dead.exists());
    }

    /// A root another user made is refused, not adopted.
    #[test]
    fn a_root_of_another_user_is_refused() {
        if euid() == 0 {
            return; // root owns everything; and the check must never chmod `/usr`
        }
        let err = ensure_private_root(Path::new("/usr")).unwrap_err();
        assert!(
            err.to_string()
                .contains("not a directory owned by this user"),
            "{err}"
        );
    }

    /// A lax mode on a root this user owns is tightened to 0700.
    #[test]
    fn a_lax_root_of_this_user_is_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
        ensure_private_root(&root).unwrap();
        let mode = std::fs::metadata(&root).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    /// A symlink planted at `owner` must fail the
    /// write, not redirect it.
    #[test]
    fn ensure_dirs_refuses_a_planted_owner_symlink() {
        let l = OverlayLayout::for_sandbox(Uuid::new_v4());
        std::fs::create_dir_all(l.base()).unwrap();
        let target = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(target.path(), l.base().join(OWNER_FILE)).unwrap();

        let planted = l.ensure_dirs();
        l.cleanup().unwrap();
        assert!(planted.is_err());
        assert_eq!(std::fs::read(target.path()).unwrap(), b"");
    }
}
