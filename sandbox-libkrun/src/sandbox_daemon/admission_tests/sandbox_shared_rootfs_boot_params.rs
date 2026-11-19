//! The shared image cache must never be a sandbox's root (daemon side).
//!
//! `handle_create` used to hand the launcher the shared, cached image
//! directory itself as the VM root
//! (`BootParams.rootfs = rootfs_cache::resolve_cached(..)`), and
//! `VmBootLauncher` passed it verbatim as `__vm-boot --rootfs`, with no
//! `--rootfs-mode overlay` / read-only lower. Two sandboxes (from two
//! different callers) therefore shared one writable root.
//!
//! Required: each sandbox boots from its own view (a read-only lower plus a
//! per-sandbox upper), so two sandboxes never share a writable root and the
//! cached image is not the guest-visible root.

use super::{TestRegistry as SandboxRegistry, handle_create, owned_boot};
use crate::sandbox_daemon::config::SandboxConfig;
use crate::sandbox_daemon::{
    SandboxError,
    create::{BootHandle, BootParams, CreateRequest, VmLauncher},
    exec::EnvShape,
};
use std::path::PathBuf;
use std::sync::Mutex;

struct RecordingLauncher {
    seen: Mutex<Vec<PathBuf>>,
}
#[async_trait::async_trait]
impl VmLauncher for RecordingLauncher {
    async fn boot(&self, _p: &BootParams) -> Result<BootHandle, SandboxError> {
        panic!("admitted fixture required")
    }
    async fn boot_admitted(
        &self,
        p: &BootParams,
        lease: crate::worker_admission::Lease,
        reg: &crate::sandbox_daemon::registry::SandboxRegistry,
        id: uuid::Uuid,
    ) -> Result<BootHandle, SandboxError> {
        self.seen.lock().unwrap().push(p.rootfs.clone());
        owned_boot(p, lease, reg, id).await
    }
}

fn req() -> CreateRequest {
    CreateRequest {
        image: "python".into(),
        cpus: None,
        memory_mb: None,
        name: None,
        network: None,
        idle_timeout_secs: None,
        env: EnvShape::default(),
    }
}

#[tokio::test]
async fn two_sandboxes_must_not_share_the_cached_image_as_writable_root() {
    let reg = SandboxRegistry::new();
    let cached = reg.seed("python");
    let cfg = SandboxConfig {
        auto_install: false,
        image_allowlist: vec!["python".into()],
        ..Default::default()
    };
    let l = RecordingLauncher {
        seen: Mutex::new(vec![]),
    };
    let a = handle_create(req(), &cfg, &reg, &l, |_| {}).await.unwrap();
    let b = handle_create(req(), &cfg, &reg, &l, |_| {}).await.unwrap();
    for created in [&a, &b] {
        assert!(
            reg.lifecycle(uuid::Uuid::parse_str(&created.sandbox_id).unwrap())
                .await
                .unwrap()
                .observation()
                .os_status()
                .is_none()
        );
    }
    let seen = l.seen.lock().unwrap().clone();
    eprintln!("sandbox A {} rootfs = {}", a.sandbox_id, seen[0].display());
    eprintln!("sandbox B {} rootfs = {}", b.sandbox_id, seen[1].display());
    eprintln!("shared cached image  = {}", cached.display());

    assert_ne!(
        seen[0], cached,
        "the VM root handed to __vm-boot is the shared image cache itself"
    );
    assert_ne!(
        seen[0], seen[1],
        "two sandboxes booted with the same writable root directory"
    );
    assert_eq!(reg.count().await, 2);
    reg.finish().await;
}
