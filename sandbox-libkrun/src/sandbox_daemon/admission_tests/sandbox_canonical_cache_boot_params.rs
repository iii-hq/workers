//! Daemon-side checks on what a sandbox boots from, complementing
//! `sandbox_shared_rootfs_boot_params`.
//!
//! - Both seed one real prepared cache under a per-instance temporary root.
//! - This one records the full `BootParams`: the cache must arrive only as
//!   `base_rootfs` (the erofs lower source), never as the guest root, and the
//!   guest root must be a per-sandbox trampoline holding nothing but the
//!   staging dirs iii-init needs.
//! - A failed launcher `preflight` must stop `handle_create` before the image
//!   is resolved or pulled.
//!
//! No VM: launchers record params and attach an owned test child with real
//! inherited admission/runtime descriptors before any artificial delay.

use super::{TestRegistry as SandboxRegistry, handle_create, owned_boot};
use crate::sandbox_daemon::config::SandboxConfig;
use crate::sandbox_daemon::overlay::TRAMPOLINE_STAGING_DIRS;
use crate::sandbox_daemon::{
    SandboxError, catalog,
    create::{BootHandle, BootParams, CreateRequest, VmLauncher},
    exec::EnvShape,
};
use std::path::PathBuf;
use std::sync::Mutex;

/// What one `boot` call was handed, observed at boot time.
struct Seen {
    rootfs: PathBuf,
    base_rootfs: PathBuf,
    /// Entries of `rootfs` at boot time, sorted.
    trampoline_entries: Vec<String>,
}

struct RecordingLauncher {
    seen: Mutex<Vec<Seen>>,
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
        let mut trampoline_entries: Vec<String> = std::fs::read_dir(&p.rootfs)
            .map(|rd| {
                rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        trampoline_entries.sort();
        self.seen.lock().unwrap().push(Seen {
            rootfs: p.rootfs.clone(),
            base_rootfs: p.base_rootfs.clone(),
            trampoline_entries,
        });
        owned_boot(p, lease, reg, id).await
    }
}

/// Launcher whose host cannot boot an isolated root (no embedded iii-init).
struct RefusingLauncher;
#[async_trait::async_trait]
impl VmLauncher for RefusingLauncher {
    fn preflight(&self) -> Result<(), SandboxError> {
        Err(SandboxError::BootFailed("no embedded iii-init".into()))
    }
    async fn boot(&self, _p: &BootParams) -> Result<BootHandle, SandboxError> {
        panic!("boot must not run after a failed preflight");
    }
}

fn cfg() -> SandboxConfig {
    SandboxConfig {
        auto_install: false,
        image_allowlist: vec!["python".into()],
        ..Default::default()
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
async fn canonical_cache_is_only_the_lower_never_the_guest_root() {
    let reg = SandboxRegistry::new();
    let cfg = cfg();
    let oci_ref = catalog::resolve_image("python", &cfg.custom_images).unwrap();
    let canonical = reg.seed("python");
    assert!(canonical.starts_with(reg.cache()));
    let l = RecordingLauncher {
        seen: Mutex::new(vec![]),
    };
    let a = handle_create(req(), &cfg, &reg, &l, |_| {}).await.unwrap();
    let b = handle_create(req(), &cfg, &reg, &l, |_| {}).await.unwrap();
    for created in [&a, &b] {
        let session = reg
            .lifecycle(uuid::Uuid::parse_str(&created.sandbox_id).unwrap())
            .await
            .unwrap();
        assert!(
            session.observation().os_status().is_none(),
            "both consumers must remain alive"
        );
    }
    let third = reg
        .admission_context()
        .unwrap()
        .local(&reg.data_path().join("third-writer"))
        .unwrap();
    assert!(
        third
            .prepare_cache(&[crate::worker_admission::CacheEntry { root: &canonical }])
            .is_err(),
        "the two real consumers must exclude a cache writer"
    );
    let seen = std::mem::take(&mut *l.seen.lock().unwrap());
    eprintln!("oci_ref         = {oci_ref}");
    eprintln!("canonical cache = {}", canonical.display());
    for (id, s) in [(&a.sandbox_id, &seen[0]), (&b.sandbox_id, &seen[1])] {
        eprintln!(
            "sandbox {id}: rootfs = {} base_rootfs = {} trampoline = {:?}",
            s.rootfs.display(),
            s.base_rootfs.display(),
            s.trampoline_entries
        );
    }

    let mut staging: Vec<String> = TRAMPOLINE_STAGING_DIRS
        .iter()
        .map(|d| d.to_string())
        .collect();
    staging.sort();
    for s in &seen {
        assert_eq!(
            s.base_rootfs, canonical,
            "the cache must arrive as the lower source"
        );
        assert_ne!(
            s.rootfs, canonical,
            "the VM root handed to __vm-boot is the shared canonical cache itself"
        );
        assert!(
            !s.rootfs.starts_with(&canonical),
            "the VM root must live outside the cache"
        );
        assert_eq!(
            s.trampoline_entries, staging,
            "the trampoline must hold only iii-init's staging dirs"
        );
    }
    assert_ne!(
        seen[0].rootfs, seen[1].rootfs,
        "two sandboxes with the same writable root"
    );
    assert_eq!(
        std::fs::read(canonical.join("bin/marker")).unwrap(),
        b"original",
        "create must not write into the shared cache"
    );
    assert_eq!(reg.count().await, 2);
    reg.finish().await;
}

#[tokio::test]
async fn failed_preflight_stops_create_before_the_image_is_resolved() {
    // No cached image and no auto-install: resolving the image first would
    // fail with RootfsMissing (S101); the preflight must win with S300.
    let reg = SandboxRegistry::new();
    let err = handle_create(req(), &cfg(), &reg, &RefusingLauncher, |_| {})
        .await
        .expect_err("create must fail when preflight fails");
    assert_eq!(err.code().as_str(), "S300");
    assert_eq!(reg.count().await, 0);
}

/// Boots slowly, so parallel creates overlap inside `boot`.
struct SlowLauncher;
#[async_trait::async_trait]
impl VmLauncher for SlowLauncher {
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
        let boot = owned_boot(p, lease, reg, id).await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        Ok(boot)
    }
}

#[tokio::test]
async fn parallel_creates_stop_at_max_concurrent_sandboxes() {
    let reg = SandboxRegistry::new();
    let mut cfg = cfg();
    cfg.max_concurrent_sandboxes = 2;
    reg.seed("python");
    let results = futures::future::join_all(
        (0..4).map(|_| handle_create(req(), &cfg, &reg, &SlowLauncher, |_| {})),
    )
    .await;
    let codes: Vec<&str> = results
        .iter()
        .map(|r| r.as_ref().map_or_else(|e| e.code().as_str(), |_| "ok"))
        .collect();
    assert_eq!(codes.iter().filter(|c| **c == "ok").count(), 2, "{codes:?}");
    assert_eq!(
        codes.iter().filter(|c| **c == "S400").count(),
        2,
        "{codes:?}"
    );
    assert_eq!(reg.count().await, 2);
    reg.finish().await;
}
