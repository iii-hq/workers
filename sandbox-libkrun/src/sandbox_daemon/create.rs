use crate::cli::rootfs_cache;
use crate::sandbox_daemon::config::SandboxConfig;
use crate::sandbox_daemon::{
    catalog,
    errors::SandboxError,
    events::SandboxCreateEvent,
    overlay::OverlayLayout,
    registry::{SandboxRegistry, SandboxState},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;

/// The smallest guest that boots and runs an exec; at 40 MiB the guest
/// kernel deadlocks on memory, and 48 is the least that booted.
const MIN_MEMORY_MB: u32 = 64;

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "create_request_example")]
pub struct CreateRequest {
    /// Catalog name of the image to boot. Bundled presets are
    /// `"python"` and `"node"`; pass either string verbatim. The only
    /// other accepted values are the literal keys of the adapter's
    /// `custom_images` config — set by the operator. Do NOT pass an OCI
    /// ref like `"ghcr.io/iii-hq/node:latest"` or
    /// `"docker.io/library/node:20"` unless that exact string is the
    /// catalog key. Unknown values return S100 with the allowed set in
    /// the error message.
    pub image: String,
    /// vCPU count, at least 1; daemon/image default applies when omitted.
    #[serde(default)]
    pub cpus: Option<u32>,
    /// Memory cap in MiB, at least 64; daemon/image default applies when omitted.
    #[serde(default)]
    pub memory_mb: Option<u32>,
    /// Human label surfaced by `sandbox::list`; not an identifier.
    #[serde(default)]
    pub name: Option<String>,
    /// Whether the VM gets outbound networking; daemon default when omitted.
    #[serde(default)]
    pub network: Option<bool>,
    /// Auto-stop the VM after this many seconds of inactivity; daemon
    /// default applies when omitted.
    #[serde(default)]
    pub idle_timeout_secs: Option<u64>,
    /// Environment entries injected into the VM. Accepts either a
    /// `Vec<"K=V">` (original wire shape) or a `{ K: V }` map.
    /// `handle_create` normalises to the `Vec<String>` shape the boot
    /// path expects before invoking the launcher.
    #[serde(default)]
    pub env: crate::sandbox_daemon::exec::EnvShape,
}

fn create_request_example() -> serde_json::Value {
    serde_json::json!({
        "image": "node",
        "memory_mb": 512,
        "env": { "NODE_ENV": "production" },
        "idle_timeout_secs": 600
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CreateResponse {
    pub sandbox_id: String,
    pub image: String,
}

#[async_trait::async_trait]
pub trait VmLauncher: Send + Sync + 'static {
    /// Cheap capability check `handle_create` runs before resolving or
    /// pulling the image, so an unbootable host fails fast. Default: OK.
    fn preflight(&self) -> Result<(), SandboxError> {
        Ok(())
    }

    async fn boot(&self, params: &BootParams) -> Result<BootHandle, SandboxError>;

    /// Admitted entry used by production create. Legacy implementations do not
    /// acquire writer authority by returning a fabricated PID.
    async fn boot_admitted(
        &self,
        _params: &BootParams,
        _lease: crate::worker_admission::Lease,
        _registry: &SandboxRegistry,
        _id: Uuid,
    ) -> Result<BootHandle, SandboxError> {
        Err(SandboxError::BootFailed(
            "launcher does not support admitted runtime ownership".into(),
        ))
    }
}

pub struct BootParams {
    /// Per-sandbox trampoline served read-only as the guest's `/dev/root`.
    /// Never the shared image cache.
    pub rootfs: PathBuf,
    /// The cached image rootfs (`rootfs_cache::resolve_cached`), shared by
    /// every sandbox of the image. Only ever exposed to the guest as a
    /// read-only erofs lower built from it.
    pub base_rootfs: PathBuf,
    /// H producer result; the launcher consumes, never rebuilds after handoff.
    pub(crate) prepared: Option<crate::cli::rootfs_cache::PreparedCache>,
    pub workdir: PathBuf,
    pub shell_sock: PathBuf,
    pub cpus: u32,
    pub memory_mb: u32,
    pub env: Vec<String>,
    pub network: bool,
}

pub struct BootHandle {
    pub vm_pid: u32,
    /// Spawner-side write end of the VM's lifeline pipe. Held by the
    /// sandbox's registry entry: if this daemon dies (any way, SIGKILL
    /// included) or the entry is dropped, the kernel/Drop closes it and the
    /// `__vm-boot` process self-terminates instead of living on as an
    /// orphaned session leader holding a multi-GB VM.
    pub lifeline: Option<std::sync::Arc<crate::daemon_exit::Lifeline>>,
}

pub async fn handle_create<L: VmLauncher, F: FnMut(SandboxCreateEvent) + Send + 'static>(
    req: CreateRequest,
    cfg: &SandboxConfig,
    registry: &SandboxRegistry,
    launcher: &L,
    on_event: F,
) -> Result<CreateResponse, SandboxError> {
    handle_create_in(req, cfg, registry, launcher, on_event, None).await
}

struct CreatePaths {
    layout: OverlayLayout,
    cache: PathBuf,
}

async fn handle_create_in<L: VmLauncher, F: FnMut(SandboxCreateEvent) + Send + 'static>(
    req: CreateRequest,
    cfg: &SandboxConfig,
    registry: &SandboxRegistry,
    launcher: &L,
    mut on_event: F,
    paths: Option<CreatePaths>,
) -> Result<CreateResponse, SandboxError> {
    let t_create = Instant::now();
    // Held to the insert below, so parallel creates cannot all pass the cap.
    let Some(_slot) = registry
        .reserve(cfg.max_concurrent_sandboxes as usize)
        .await
    else {
        return Err(SandboxError::ResourceLimit(format!(
            "max_concurrent_sandboxes ({}) reached",
            cfg.max_concurrent_sandboxes
        )));
    };

    // `check_allowlist` already proved the image is known, so
    // `resolve_image` cannot miss here.
    catalog::check_allowlist(&req.image, &cfg.image_allowlist, &cfg.custom_images)?;
    let oci_ref = catalog::resolve_image(&req.image, &cfg.custom_images)
        .expect("allowlist guards against unknown images");

    let cpus = req.cpus.unwrap_or(cfg.default_cpus);
    let memory_mb = req.memory_mb.unwrap_or(cfg.default_memory_mb);
    // Below these the VM fails later, at boot or at its first exec, as S300.
    if cpus == 0 {
        return Err(SandboxError::InvalidRequest(
            "cpus must be at least 1".to_string(),
        ));
    }
    if memory_mb < MIN_MEMORY_MB {
        return Err(SandboxError::InvalidRequest(format!(
            "memory_mb must be at least {MIN_MEMORY_MB}, got {memory_mb}"
        )));
    }
    if let Some(cap) = cfg.per_image_caps.get(&req.image) {
        if cpus > cap.max_cpus {
            return Err(SandboxError::ResourceLimit(format!(
                "cpus={} exceeds per-image cap {}",
                cpus, cap.max_cpus
            )));
        }
        if memory_mb > cap.max_memory_mb {
            return Err(SandboxError::ResourceLimit(format!(
                "memory_mb={} exceeds per-image cap {}",
                memory_mb, cap.max_memory_mb
            )));
        }
    }

    // Normalise env from EnvShape to Vec<String> with the other request
    // checks, before any pull, directory or `BootingVm` event: `into_kv_vec`
    // rejects invalid var names with S001. Accepts both the historical
    // Vec<"K=V"> shape and the agent-natural { K: V } map.
    let env_vec = req.env.clone().into_kv_vec()?;

    // Refuse before resolving or pulling the image when the
    // launcher cannot boot an isolated root (no embedded iii-init).
    launcher.preflight()?;

    let context = registry
        .admission_context()
        .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
    let id = Uuid::new_v4();
    let planned = paths.unwrap_or_else(|| CreatePaths {
        layout: OverlayLayout::for_sandbox(id),
        cache: rootfs_cache::cache_root(),
    });
    let layout = planned.layout;
    // Short private control/endpoint setup completes before the payload lease.
    layout
        .prepare_parents(&context)
        .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
    rootfs_cache::prepare_cache_parent(&context, &planned.cache)
        .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
    let admission = layout
        .admit(&context)
        .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
    let request = rootfs_cache::CacheRequest::new(admission.clone(), &oci_ref, &planned.cache)
        .map_err(|e| SandboxError::BootFailed(format!("{e:#}")))?;
    let pull_started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started = pull_started.clone();
    let events = std::sync::Arc::new(std::sync::Mutex::new(on_event));
    let pulling = events.clone();
    let reference = oci_ref.clone();
    let prepared = request
        .resolve(cfg.auto_install, move || {
            started.store(true, std::sync::atomic::Ordering::SeqCst);
            let mut emit = pulling.lock().unwrap();
            emit(SandboxCreateEvent::PullingImage {
                image_ref: reference,
                progress_bytes: 0,
                total_bytes: None,
            });
            emit(SandboxCreateEvent::Unpacking);
        })
        .await
        .map_err(|e| {
            if pull_started.load(std::sync::atomic::Ordering::SeqCst) {
                SandboxError::auto_install_failed(
                    &req.image,
                    format!("pull_and_extract_rootfs failed: {e:#}"),
                )
            } else {
                SandboxError::BootFailed(format!("{e:#}"))
            }
        })?
        .ok_or_else(|| SandboxError::RootfsMissing {
            image: req.image.clone(),
        })?;
    let admission = prepared.lease.clone();
    let rootfs = prepared.root.clone();
    if let Err(error) = layout.ensure_admitted(&admission) {
        // No launcher has been invoked; only the core can authorize rollback
        // of this operation's created-from-absence effects.
        let rollback = layout.rollback_never_spawned(&admission);
        return Err(SandboxError::BootFailed(match rollback {
            Ok(()) => format!("layout preparation failed; rollback committed: {error}"),
            Err(rollback) => {
                format!("layout preparation failed: {error}; retained for recovery: {rollback}")
            }
        }));
    }
    let shell_sock = layout.shell_sock();
    let workdir = layout.merged.clone();
    // Publish an unresolved/closing registry entry BEFORE boot: cancellation or
    // a post-spawn startup error must retain both layout and discoverable owner.
    registry
        .insert(SandboxState {
            id,
            name: req.name.clone(),
            image: req.image.clone(),
            rootfs: rootfs.clone(),
            workdir: workdir.clone(),
            shell_sock: shell_sock.clone(),
            vm_pid: None,
            lifeline: None,
            created_at: Instant::now(),
            last_exec_at: Instant::now(),
            exec_in_flight: 0,
            idle_timeout_secs: req
                .idle_timeout_secs
                .unwrap_or(cfg.default_idle_timeout_secs),
            stopped: false,
        })
        .await;
    registry.begin_start(id).await;
    events.lock().unwrap()(SandboxCreateEvent::BootingVm);
    let boot = launcher
        .boot_admitted(
            &BootParams {
                rootfs: layout.root.clone(),
                base_rootfs: rootfs.clone(),
                prepared: Some(prepared),
                workdir: workdir.clone(),
                shell_sock: shell_sock.clone(),
                cpus,
                memory_mb,
                env: env_vec,
                network: req.network.unwrap_or(false),
            },
            admission,
            registry,
            id,
        )
        .await?;

    let state = SandboxState {
        id,
        name: req.name.clone(),
        image: req.image.clone(),
        rootfs,
        workdir,
        shell_sock,
        vm_pid: Some(boot.vm_pid),
        lifeline: boot.lifeline,
        created_at: Instant::now(),
        last_exec_at: Instant::now(),
        exec_in_flight: 0,
        idle_timeout_secs: req
            .idle_timeout_secs
            .unwrap_or(cfg.default_idle_timeout_secs),
        stopped: false,
    };
    registry.insert(state).await;
    registry.mark_ready(id).await?;

    on_event(SandboxCreateEvent::Ready {
        sandbox_id: id.to_string(),
    });

    tracing::info!(
        ms = t_create.elapsed().as_millis() as u64,
        sandbox_id = %id,
        image = %req.image,
        "create_phase: handle_create TOTAL"
    );

    Ok(CreateResponse {
        sandbox_id: id.to_string(),
        image: req.image,
    })
}

// No destructive Drop: cancellation is not NeverSpawned or WriterRetirement.
// Explicit pre-spawn rollback is available via OverlayLayout::rollback_never_spawned
// only while the caller still owns the original, never-invoked launch operation.

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeLauncher {
        vm_pid: u32,
    }
    #[async_trait::async_trait]
    impl VmLauncher for FakeLauncher {
        async fn boot(&self, _p: &BootParams) -> Result<BootHandle, SandboxError> {
            Ok(BootHandle {
                vm_pid: self.vm_pid,
                lifeline: None,
            })
        }
    }

    fn cfg_with_python_allowed() -> SandboxConfig {
        SandboxConfig {
            auto_install: false,
            image_allowlist: vec!["python".into()],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn image_not_in_allowlist_returns_s100() {
        let cfg = cfg_with_python_allowed();
        let reg = SandboxRegistry::new();
        let launcher = FakeLauncher { vm_pid: 1 };
        let req = CreateRequest {
            image: "malicious".into(),
            cpus: None,
            memory_mb: None,
            name: None,
            network: None,
            idle_timeout_secs: None,
            env: crate::sandbox_daemon::exec::EnvShape::default(),
        };
        let err = handle_create(req, &cfg, &reg, &launcher, |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.code().as_str(), "S100");
    }

    #[tokio::test]
    async fn custom_image_not_in_allowlist_returns_s100() {
        // custom_images entry exists, but it is not in image_allowlist.
        // Known-to-daemon ≠ permitted. Must still return S100.
        let mut cfg = cfg_with_python_allowed();
        cfg.custom_images
            .insert("my-app".into(), "ghcr.io/acme/my-app:1".into());
        let reg = SandboxRegistry::new();
        let launcher = FakeLauncher { vm_pid: 1 };
        let req = CreateRequest {
            image: "my-app".into(),
            cpus: None,
            memory_mb: None,
            name: None,
            network: None,
            idle_timeout_secs: None,
            env: crate::sandbox_daemon::exec::EnvShape::default(),
        };
        let err = handle_create(req, &cfg, &reg, &launcher, |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.code().as_str(), "S100");
    }

    /// Fails the preflight, the step right after the request checks.
    struct NoInit;
    #[async_trait::async_trait]
    impl VmLauncher for NoInit {
        fn preflight(&self) -> Result<(), SandboxError> {
            Err(SandboxError::BootFailed("no embedded init".into()))
        }
        async fn boot(&self, _p: &BootParams) -> Result<BootHandle, SandboxError> {
            unreachable!("the preflight fails first")
        }
    }

    /// cpus 0 and a guest too small to boot are bad requests, not a later S300.
    #[tokio::test]
    async fn unbootable_cpus_or_memory_is_s001_at_create() {
        let cfg = cfg_with_python_allowed();
        let reg = SandboxRegistry::new();
        let launcher = NoInit;
        for (cpus, memory_mb, code) in [
            (Some(0), None, "S001"),
            (None, Some(32), "S001"),
            (None, Some(MIN_MEMORY_MB - 1), "S001"),
            // Valid: gets as far as the preflight.
            (Some(1), Some(MIN_MEMORY_MB), "S300"),
        ] {
            let req = CreateRequest {
                image: "python".into(),
                cpus,
                memory_mb,
                name: None,
                network: None,
                idle_timeout_secs: None,
                env: crate::sandbox_daemon::exec::EnvShape::default(),
            };
            let err = handle_create(req, &cfg, &reg, &launcher, |_| {})
                .await
                .unwrap_err();
            assert_eq!(
                err.code().as_str(),
                code,
                "cpus={cpus:?} memory_mb={memory_mb:?}: {err}"
            );
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Boot {
        Fails,
        Hangs,
        Succeeds,
    }

    /// Records the sandbox dir boot was handed, then boots as scripted.
    struct Scripted {
        boot: Boot,
        base: std::sync::Mutex<Option<PathBuf>>,
        namespace: PathBuf,
    }
    #[async_trait::async_trait]
    impl VmLauncher for Scripted {
        async fn boot_admitted(
            &self,
            p: &BootParams,
            lease: crate::worker_admission::Lease,
            registry: &SandboxRegistry,
            id: Uuid,
        ) -> Result<BootHandle, SandboxError> {
            *self.base.lock().unwrap() = p.rootfs.parent().map(PathBuf::from);
            match self.boot {
                Boot::Fails => Err(SandboxError::BootFailed("before spawn refusal".into())),
                Boot::Hangs => std::future::pending().await,
                Boot::Succeeds => {
                    let session =
                        super::super::adapters::runtime_test_session(lease, &self.namespace, true)
                            .await;
                    registry.attach_runtime(id, session).await?;
                    Ok(BootHandle {
                        vm_pid: 1,
                        lifeline: None,
                    }) // diagnostic only; authority is the real child above
                }
            }
        }
        async fn boot(&self, p: &BootParams) -> Result<BootHandle, SandboxError> {
            *self.base.lock().unwrap() = p.shell_sock.parent().map(PathBuf::from);
            match self.boot {
                Boot::Fails => Err(SandboxError::BootFailed("the guest died".into())),
                Boot::Hangs => std::future::pending().await,
                Boot::Succeeds => Ok(BootHandle {
                    vm_pid: 1,
                    lifeline: None,
                }),
            }
        }
    }

    /// A completed create has real runtime authority for explicit cleanup;
    /// cancellation or startup failure retains the layout and closing entry.
    #[tokio::test]
    async fn owned_create_can_cleanup_and_interrupted_create_preserves_uncertainty() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let namespace = t.path().join("catalog");
        std::fs::create_dir(&namespace).unwrap();
        std::fs::set_permissions(&namespace, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&namespace).unwrap();
        let cache = t.path().join("cache");
        let reference = "ghcr.io/acme/my-app:1";
        std::fs::create_dir_all(rootfs_cache::canonical_path_at(&cache, reference).join("bin"))
            .unwrap();
        let mut cfg = cfg_with_python_allowed();
        cfg.custom_images.insert("my-app".into(), reference.into());
        cfg.image_allowlist.push("my-app".into());
        for (index, boot) in [Boot::Fails, Boot::Hangs, Boot::Succeeds]
            .into_iter()
            .enumerate()
        {
            let base = t.path().join(format!("sandbox-{index}"));
            let layout = OverlayLayout {
                upper: base.join("upper"),
                work: base.join("work"),
                merged: base.join("merged"),
                root: base.join("root"),
            };
            let reg = SandboxRegistry::new().with_context(context.clone());
            let launcher = Scripted {
                boot,
                base: Default::default(),
                namespace: namespace.clone(),
            };
            let req = CreateRequest {
                image: "my-app".into(),
                cpus: None,
                memory_mb: None,
                name: None,
                network: None,
                idle_timeout_secs: None,
                env: Default::default(),
            };
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                handle_create_in(
                    req,
                    &cfg,
                    &reg,
                    &launcher,
                    |_| {},
                    Some(CreatePaths {
                        layout,
                        cache: cache.clone(),
                    }),
                ),
            )
            .await;
            assert!(
                base.exists(),
                "cancel/failure retains layout until receipt removal"
            );
            if matches!(boot, Boot::Succeeds) {
                let response = result.unwrap().unwrap();
                let id = Uuid::parse_str(&response.sandbox_id).unwrap();
                let session = reg.lifecycle(id).await.unwrap();
                let receipt = session.stop(std::time::Duration::ZERO).await.unwrap();
                session.finish_resources(&receipt).await.unwrap();
                let mut remove = receipt
                    .begin_remove(session.lease(), &base, &[&base])
                    .unwrap();
                remove.remove_path(&base).unwrap();
                remove.commit().unwrap();
                reg.complete_cleanup(id).await;
                assert!(!base.exists());
            } else {
                assert_eq!(reg.count().await, 1);
                let id = reg.list().await[0].id;
                assert!(reg.begin_exec(id).await.is_err());
            }
        }
    }

    /// A bad env name is caught with the other request checks, before any
    /// pull or sandbox dir: found after `ensure_dirs`, it leaked those dirs.
    #[tokio::test]
    async fn invalid_env_name_is_s001_before_the_preflight() {
        let req = CreateRequest {
            image: "python".into(),
            cpus: None,
            memory_mb: None,
            name: None,
            network: None,
            idle_timeout_secs: None,
            env: serde_json::from_value(serde_json::json!({"9bad": "x"})).unwrap(),
        };
        let err = handle_create(
            req,
            &cfg_with_python_allowed(),
            &SandboxRegistry::new(),
            &NoInit,
            |_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(err.code().as_str(), "S001", "{err}");
    }

    #[tokio::test]
    async fn cpu_cap_exceeded_returns_s400() {
        let mut cfg = cfg_with_python_allowed();
        cfg.per_image_caps.insert(
            "python".into(),
            crate::sandbox_daemon::config::PerImageCap {
                max_cpus: 2,
                max_memory_mb: 1024,
            },
        );
        let reg = SandboxRegistry::new();
        let launcher = FakeLauncher { vm_pid: 1 };
        let req = CreateRequest {
            image: "python".into(),
            cpus: Some(99),
            memory_mb: None,
            name: None,
            network: None,
            idle_timeout_secs: None,
            env: crate::sandbox_daemon::exec::EnvShape::default(),
        };
        let err = handle_create(req, &cfg, &reg, &launcher, |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.code().as_str(), "S400");
    }
}
