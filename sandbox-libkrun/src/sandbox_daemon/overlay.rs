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
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "existing sandbox parent is not private",
        ));
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

#[derive(Clone)]
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

    /// Endpoint namespace is disjoint from all exported/data roots. Consumers
    /// use this function together with registry::net_activity_path; no guest path changes.
    pub fn shell_sock(&self) -> PathBuf {
        let id = self.base().file_name().expect("sandbox id").to_owned();
        self.base()
            .parent()
            .unwrap()
            .join(".endpoints")
            .join(id)
            .join("shell.sock")
    }

    pub(crate) fn prepare_parents(
        &self,
        context: &crate::worker_admission::AdmissionContext,
    ) -> io::Result<()> {
        for parent in [
            self.base().parent().unwrap().to_path_buf(),
            self.shell_sock().parent().unwrap().to_path_buf(),
        ] {
            let mut create_root = parent.as_path();
            while let Some(ancestor) = create_root.parent() {
                match std::fs::symlink_metadata(ancestor) {
                    Ok(_) => break,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => create_root = ancestor,
                    Err(e) => return Err(e),
                }
            }
            if let Ok(meta) = std::fs::symlink_metadata(&parent) {
                if !meta.is_dir() || meta.uid() != euid() || meta.mode() & 0o777 != 0o700 {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "existing sandbox parent is not private; refusing chmod",
                    ));
                }
                // Read-only parent check. Do not acquire its whole subtree:
                // another admitted sandbox may already live beneath it.
                context.reject_retained_path(&parent)?;
            } else {
                let lease = context.local(create_root)?;
                let mut guard = lease.begin(crate::worker_admission::MutationPlan::Create {
                    root: create_root.to_path_buf(),
                })?;
                guard.create_dir_all(&parent, 0o700)?;
                guard.commit()?;
                drop(lease);
            }
        }
        Ok(())
    }

    pub(crate) fn admit(
        &self,
        context: &crate::worker_admission::AdmissionContext,
    ) -> io::Result<crate::worker_admission::Lease> {
        use crate::worker_admission::{PathRequest, PathRole};
        let base = self.base();
        let lease = context.local(&base)?;
        crate::worker_admission::with_paths(
            lease,
            &[
                PathRequest {
                    path: &base,
                    role: PathRole::MutableTree,
                },
                PathRequest {
                    path: &self.root,
                    role: PathRole::MutableTree,
                },
                PathRequest {
                    path: &self.shell_sock(),
                    role: PathRole::Endpoint,
                },
                PathRequest {
                    path: &base.join("vm-boot.stderr.log"),
                    role: PathRole::MutableFile,
                },
                PathRequest {
                    path: &base.join("net-activity"),
                    role: PathRole::MutableFile,
                },
            ],
        )
    }
    pub(crate) fn ensure_admitted(&self, lease: &crate::worker_admission::Lease) -> io::Result<()> {
        let base = self.base();
        let mut guard =
            lease.begin(crate::worker_admission::MutationPlan::Create { root: base.clone() })?;
        for path in [&self.upper, &self.work, &self.merged] {
            guard.create_dir_all(path, 0o700)?;
        }
        for child in TRAMPOLINE_STAGING_DIRS {
            guard.create_dir_all(&self.root.join(child), 0o700)?;
        }
        let mut owner = guard.stage_file(&base.join(OWNER_FILE), 0o600)?;
        owner
            .file_mut()
            .write_all(std::process::id().to_string().as_bytes())?;
        guard.publish_file(owner)?;
        guard.commit()?;
        Ok(())
    }
    pub(crate) fn cleanup_retired(
        &self,
        lease: &crate::worker_admission::Lease,
        receipt: &crate::worker_admission::WriterRetirement,
    ) -> io::Result<()> {
        let base = self.base();
        let mut guard = receipt.begin_remove(lease, &base, &[&base])?;
        guard.remove_path(&base)?;
        guard.commit()?;
        Ok(())
    }
    pub(crate) fn rollback_never_spawned(
        &self,
        lease: &crate::worker_admission::Lease,
    ) -> io::Result<()> {
        let proof = lease.prepare_runtime(&[])?.cancel_before_spawn()?;
        let base = self.base();
        let mut removal = proof.begin_remove(lease, &base, &[&base])?;
        removal.remove_path(&base)?;
        removal.commit()?;
        Ok(())
    }

    /// Create the directory structure, including the trampoline's
    /// [`TRAMPOLINE_STAGING_DIRS`]; does NOT mount anything. This helper
    /// exists so the layout is deterministic and easily reaped.
    #[deprecated(note = "production creation requires ensure_admitted with the original lifecycle")]
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        #[cfg(test)]
        {
            return self.ensure_dirs_fixture();
        }
        #[cfg(not(test))]
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "layout creation requires an explicit admitted operation",
        ))
    }

    #[cfg(test)]
    fn ensure_dirs_fixture(&self) -> std::io::Result<()> {
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

    #[deprecated(
        note = "use cleanup_retired or rollback_never_spawned with the original operation"
    )]
    pub fn cleanup(&self) -> std::io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "sandbox cleanup requires its original lifecycle and core retirement receipt",
        ))
    }
}

/// Report legacy/unresolved sandbox layouts without inferring deletion authority
/// from owner PIDs. Only explicit receipt-based cleanup or recovery may remove.
pub fn sweep_stale(root: &Path) {
    // Names and owner PIDs are diagnostic only. Never mutate this root at startup.
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            tracing::warn!(%error, root=%root.display(), "cannot inspect retained sandbox evidence");
            return;
        }
    };
    for entry in entries.take(4096) {
        match entry {
            Ok(entry) => {
                tracing::warn!(path=%entry.path().display(), "sandbox layout retained for explicit retirement recovery")
            }
            Err(error) => tracing::warn!(%error, "cannot inspect sandbox entry"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_operation_never_spawned_rollback_removes_only_owned_new_layout() {
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
        let ns = t.path().join("catalog");
        std::fs::create_dir(&ns).unwrap();
        std::fs::set_permissions(&ns, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&ns).unwrap();
        let base = t.path().join("new-layout");
        let layout = OverlayLayout {
            upper: base.join("upper"),
            work: base.join("work"),
            merged: base.join("merged"),
            root: base.join("root"),
        };
        let lease = context.local(&base).unwrap();
        layout.ensure_admitted(&lease).unwrap();
        assert!(layout.root.join("dev").is_dir());
        layout.rollback_never_spawned(&lease).unwrap();
        assert!(!base.exists());
        let legacy = t.path().join("legacy");
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("keep"), b"old").unwrap();
        let lease = context.local(&legacy).unwrap();
        let proof = lease
            .prepare_runtime(&[])
            .unwrap()
            .cancel_before_spawn()
            .unwrap();
        assert!(proof.begin_remove(&lease, &legacy, &[&legacy]).is_err());
        assert_eq!(std::fs::read(legacy.join("keep")).unwrap(), b"old");
    }

    #[test]
    fn layout_paths_are_deterministic() {
        let id = Uuid::nil();
        let l = OverlayLayout::for_sandbox(id);
        assert!(l.upper.ends_with("upper"));
        assert!(l.work.ends_with("work"));
        assert!(l.merged.ends_with("merged"));
    }

    #[tokio::test]
    async fn ensure_dirs_and_cleanup_roundtrip() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let base = data.path().join("sandbox");
        let layout = OverlayLayout {
            upper: base.join("upper"),
            work: base.join("work"),
            merged: base.join("merged"),
            root: base.join("root"),
        };
        layout.prepare_parents(&context).unwrap();
        let lease = layout.admit(&context).unwrap();
        layout.ensure_admitted(&lease).unwrap();
        assert!(layout.upper.exists());
        let session = super::super::adapters::runtime_test_session_resources(
            lease,
            namespace.path(),
            false,
            super::super::adapters::RuntimeTestResources {
                reads: vec![layout.root.clone()],
                endpoints: vec![layout.shell_sock()],
                exit_without_drop: true,
                ..Default::default()
            },
        )
        .await;
        let receipt = session.wait().await.1.unwrap();
        session.finish_resources(&receipt).await.unwrap();
        layout.cleanup_retired(session.lease(), &receipt).unwrap();
        assert!(!layout.base().exists());
        assert!(!layout.shell_sock().exists());
    }

    #[test]
    fn raw_cleanup_without_receipt_preserves_existing_layout() {
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let base = data.path().join("sandbox");
        let layout = OverlayLayout {
            upper: base.join("upper"),
            work: base.join("work"),
            merged: base.join("merged"),
            root: base.join("root"),
        };
        std::fs::create_dir_all(&layout.root).unwrap();
        std::fs::write(layout.root.join("keep"), b"retained").unwrap();
        assert!(layout.cleanup().is_err());
        assert_eq!(
            std::fs::read(layout.root.join("keep")).unwrap(),
            b"retained"
        );
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
        assert!(
            l.cleanup().is_err(),
            "raw cleanup must never acquire authority"
        );
    }

    /// Legacy owners and unresolved layouts all remain for explicit recovery;
    /// an exited diagnostic PID does not make deletion safe.
    #[test]
    fn sweep_preserves_dead_live_unowned_and_foreign_layouts() {
        let mut gone = std::process::Command::new("true").spawn().unwrap();
        gone.wait().unwrap();
        let root = crate::worker_admission::testing::private_tempdir().unwrap();
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

        assert!(dead.exists(), "a dead PID is not a retirement receipt");
        for kept in [live, unowned, foreign] {
            assert!(kept.exists(), "{}", kept.display());
        }
    }

    #[test]
    fn ensure_dirs_records_this_process_as_owner() {
        let l = OverlayLayout::for_sandbox(Uuid::new_v4());
        l.ensure_dirs().unwrap();
        let owner = std::fs::read_to_string(l.base().join(OWNER_FILE)).unwrap();
        assert!(
            l.cleanup().is_err(),
            "raw cleanup must never acquire authority"
        );
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
        assert!(
            l.cleanup().is_err(),
            "raw cleanup must never acquire authority"
        );
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
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
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

    /// Setup never tightens existing insecure parents to manufacture admission.
    #[test]
    fn a_lax_root_of_this_user_is_refused_without_chmod() {
        use std::os::unix::fs::PermissionsExt;
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
        let root = t.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(ensure_private_root(&root).is_err());
        let mode = std::fs::metadata(&root).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o777);
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
        assert!(
            l.cleanup().is_err(),
            "raw cleanup must never acquire authority"
        );
        assert!(planted.is_err());
        assert_eq!(std::fs::read(target.path()).unwrap(), b"");
    }
}
