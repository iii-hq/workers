use crate::cli::rootfs_cache;
use crate::sandbox_daemon::config::SandboxConfig;
use crate::sandbox_daemon::{
    auto_install, catalog,
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
}

pub struct BootParams {
    /// Per-sandbox trampoline served read-only as the guest's `/dev/root`.
    /// Never the shared image cache.
    pub rootfs: PathBuf,
    /// The cached image rootfs (`rootfs_cache::resolve_cached`), shared by
    /// every sandbox of the image. Only ever exposed to the guest as a
    /// read-only erofs lower built from it.
    pub base_rootfs: PathBuf,
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
    mut on_event: F,
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

    let rootfs = match rootfs_cache::resolve_cached(&oci_ref) {
        Some(path) => path,
        None => {
            if !cfg.auto_install {
                return Err(SandboxError::RootfsMissing {
                    image: req.image.clone(),
                });
            }
            let t_ai = Instant::now();
            // auto_install_image requires `impl FnMut(SandboxCreateEvent) + Send + 'static`.
            // We can't borrow on_event into it (lifetime), so we buffer events
            // through a channel: the Sender is Send+'static, the future is
            // awaited inline, then we drain the channel and forward to on_event
            // before continuing.
            let (tx, rx) = std::sync::mpsc::channel::<SandboxCreateEvent>();
            let dest = auto_install::auto_install_image(&req.image, &oci_ref, move |e| {
                let _ = tx.send(e);
            })
            .await?;
            for e in rx.try_iter() {
                on_event(e);
            }
            tracing::info!(
                ms = t_ai.elapsed().as_millis() as u64,
                image = %req.image,
                rootfs = %dest.display(),
                "create_phase: auto_install (pull+extract)"
            );
            dest
        }
    };

    let id = Uuid::new_v4();
    // Removes the dirs unless the create succeeds: a failed boot, a failed
    // `ensure_dirs`, or a caller that drops this future mid-boot.
    let mut dirs = DirsGuard {
        layout: OverlayLayout::for_sandbox(id),
        keep: false,
    };
    let layout = &dirs.layout;
    layout
        .ensure_dirs()
        .map_err(|e| SandboxError::BootFailed(format!("overlay dirs: {e}")))?;
    let shell_sock = layout.base().join("shell.sock");
    let workdir = layout.merged.clone();

    on_event(SandboxCreateEvent::BootingVm);
    let boot = launcher
        .boot(&BootParams {
            rootfs: layout.root.clone(),
            base_rootfs: rootfs.clone(),
            workdir: workdir.clone(),
            shell_sock: shell_sock.clone(),
            cpus,
            memory_mb,
            env: env_vec,
            network: req.network.unwrap_or(false),
        })
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
    dirs.keep = true;

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

/// Removes a sandbox's directories on drop unless told to keep them.
struct DirsGuard {
    layout: OverlayLayout,
    keep: bool,
}

impl Drop for DirsGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = self.layout.cleanup();
        }
    }
}

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
    }
    #[async_trait::async_trait]
    impl VmLauncher for Scripted {
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

    /// Only a finished create keeps its sandbox dir: a failed boot and a
    /// create dropped mid-boot remove it.
    #[test]
    fn only_a_finished_create_keeps_its_sandbox_dir() {
        let _g = crate::cli::test_support::lock_home();
        let home = tempfile::tempdir().unwrap();
        let prior = std::env::var_os("HOME");
        // SAFETY: TEST_HOME_LOCK serializes all env mutation.
        unsafe { std::env::set_var("HOME", home.path()) };
        let oci_ref = "ghcr.io/acme/my-app:1";
        std::fs::create_dir_all(rootfs_cache::canonical_path(oci_ref).join("bin")).unwrap();
        let mut cfg = cfg_with_python_allowed();
        cfg.custom_images.insert("my-app".into(), oci_ref.into());
        cfg.image_allowlist.push("my-app".into());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut kept = vec![];
        for boot in [Boot::Fails, Boot::Hangs, Boot::Succeeds] {
            let launcher = Scripted {
                boot,
                base: Default::default(),
            };
            let req = CreateRequest {
                image: "my-app".into(),
                cpus: None,
                memory_mb: None,
                name: None,
                network: None,
                idle_timeout_secs: None,
                env: crate::sandbox_daemon::exec::EnvShape::default(),
            };
            let reg = SandboxRegistry::new();
            let create = handle_create(req, &cfg, &reg, &launcher, |_| {});
            // Only the hanging boot needs cutting short; a tight bound on the others flakes on slow runners.
            let wait = std::time::Duration::from_millis(if matches!(boot, Boot::Hangs) {
                200
            } else {
                10_000
            });
            let _ = rt.block_on(async { tokio::time::timeout(wait, create).await });
            let base = launcher.base.lock().unwrap().take();
            kept.push(base.as_ref().map(|b| b.exists()));
            if let Some(base) = base {
                let _ = std::fs::remove_dir_all(base);
            }
        }
        match prior {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        assert_eq!(
            kept,
            [Some(false), Some(false), Some(true)],
            "fails, hangs, succeeds"
        );
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
