//! Integration tests for the sandbox subsystem. Uses fake trait impls for
//! VmLauncher/ShellRunner/VmStopper so no real libkrun boot is required.

use super::{TestRegistry as SandboxRegistry, handle_create, owned_boot};
use crate::sandbox_daemon::config::SandboxConfig;
use crate::sandbox_daemon::{
    SandboxError,
    create::{BootHandle, BootParams, CreateRequest, VmLauncher},
    exec::{EnvShape, ExecRequest, ExecResponse, ShellRunner, handle_exec},
    stop::{StopRequest, VmStopper, handle_stop},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

struct TestHarness {
    launcher: Arc<FakeLauncher>,
    runner: Arc<FakeRunner>,
    stopper: Arc<FakeStopper>,
    registry: SandboxRegistry,
}

struct FakeLauncher {
    boot_count: AtomicU32,
}
#[async_trait::async_trait]
impl VmLauncher for FakeLauncher {
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
        self.boot_count.fetch_add(1, Ordering::SeqCst);
        owned_boot(p, lease, reg, id).await
    }
}

struct FakeRunner {
    stdout: String,
    exit: i32,
}
#[async_trait::async_trait]
impl ShellRunner for FakeRunner {
    async fn run(
        &self,
        _sock: std::path::PathBuf,
        _r: &ExecRequest,
    ) -> Result<ExecResponse, SandboxError> {
        Ok(ExecResponse {
            stdout: self.stdout.clone(),
            stderr: String::new(),
            exit_code: Some(self.exit),
            timed_out: false,
            duration_ms: 1,
            success: self.exit == 0,
            stdout_truncated: false,
            stderr_truncated: false,
        })
    }
}

struct FakeStopper {
    stops: AtomicU32,
}
#[async_trait::async_trait]
impl VmStopper for FakeStopper {
    async fn stop(&self, _pid: u32) -> Result<(), SandboxError> {
        self.stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn harness(runner_stdout: &str, runner_exit: i32) -> TestHarness {
    TestHarness {
        launcher: Arc::new(FakeLauncher {
            boot_count: AtomicU32::new(0),
        }),
        runner: Arc::new(FakeRunner {
            stdout: runner_stdout.into(),
            exit: runner_exit,
        }),
        stopper: Arc::new(FakeStopper {
            stops: AtomicU32::new(0),
        }),
        registry: SandboxRegistry::new(),
    }
}

fn cfg_all_presets_allowed() -> SandboxConfig {
    SandboxConfig {
        auto_install: false,
        image_allowlist: vec!["python".into(), "node".into()],
        ..Default::default()
    }
}

#[tokio::test]
async fn end_to_end_create_exec_stop() {
    let h = harness("hi\n", 0);
    h.registry.seed("python");
    let cfg = cfg_all_presets_allowed();
    let req = CreateRequest {
        image: "python".into(),
        cpus: None,
        memory_mb: None,
        name: None,
        network: None,
        idle_timeout_secs: None,
        env: EnvShape::default(),
    };
    let create = handle_create(req, &cfg, &h.registry, &*h.launcher, |_| {})
        .await
        .unwrap();
    let sid = create.sandbox_id;

    let exec = handle_exec(
        ExecRequest {
            sandbox_id: sid.clone(),
            cmd: "/bin/echo".into(),
            args: vec!["hi".into()],

            argv: vec![],
            stdin: None,
            env: EnvShape::default(),
            timeout_ms: None,
            workdir: None,
        },
        &h.registry,
        &*h.runner,
    )
    .await
    .unwrap();
    assert_eq!(exec.stdout, "hi\n");
    assert_eq!(exec.exit_code, Some(0));

    let stop = handle_stop(
        StopRequest {
            sandbox_id: sid,
            wait: true,
        },
        &h.registry,
        &*h.stopper,
    )
    .await
    .unwrap();
    assert!(stop.stopped);
    assert_eq!(
        h.stopper.stops.load(Ordering::SeqCst),
        0,
        "core control owns signalling"
    );
    assert_eq!(h.registry.count().await, 0);
}

#[tokio::test]
async fn exec_beyond_the_concurrency_cap_returns_s003() {
    let h = harness("", 0);
    h.registry.seed("python");
    let cfg = cfg_all_presets_allowed();
    let create = handle_create(
        CreateRequest {
            image: "python".into(),
            cpus: None,
            memory_mb: None,
            name: None,
            network: None,
            idle_timeout_secs: None,
            env: EnvShape::default(),
        },
        &cfg,
        &h.registry,
        &*h.launcher,
        |_| {},
    )
    .await
    .unwrap();

    // A second exec no longer bounces — both ends of the exec channel
    // multiplex by `corr_id`, and the registry now admits up to a cap.
    // Saturate that cap so the NEXT one is the rejected case this test is
    // about. `harness()` builds the registry with the default cap.
    let id = uuid::Uuid::parse_str(&create.sandbox_id).unwrap();
    for _ in 0..crate::sandbox_daemon::registry::DEFAULT_MAX_EXEC_IN_FLIGHT {
        h.registry.begin_exec(id).await.unwrap();
    }

    let err = handle_exec(
        ExecRequest {
            sandbox_id: create.sandbox_id,
            cmd: "/bin/true".into(),
            args: vec![],

            argv: vec![],
            stdin: None,
            env: EnvShape::default(),
            timeout_ms: None,
            workdir: None,
        },
        &h.registry,
        &*h.runner,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code().as_str(), "S003");
    h.registry.finish().await;
}

#[tokio::test]
async fn image_not_allowlisted_returns_s100() {
    let h = harness("", 0);
    let mut cfg = cfg_all_presets_allowed();
    cfg.image_allowlist = vec!["python".into()];
    let req = CreateRequest {
        image: "node".into(),
        cpus: None,
        memory_mb: None,
        name: None,
        network: None,
        idle_timeout_secs: None,
        env: EnvShape::default(),
    };
    let err = handle_create(req, &cfg, &h.registry, &*h.launcher, |_| {})
        .await
        .unwrap_err();
    assert_eq!(err.code().as_str(), "S100");
}

#[tokio::test]
async fn rootfs_missing_no_autoinstall_returns_s101() {
    let h = harness("", 0);
    let mut cfg = cfg_all_presets_allowed();
    cfg.auto_install = false;
    let err = handle_create(
        CreateRequest {
            image: "python".into(),
            cpus: None,
            memory_mb: None,
            name: None,
            network: None,
            idle_timeout_secs: None,
            env: EnvShape::default(),
        },
        &cfg,
        &h.registry,
        &*h.launcher,
        |_| {},
    )
    .await
    .unwrap_err();
    assert_eq!(err.code().as_str(), "S101");
}

#[tokio::test]
async fn memory_cap_returns_s400() {
    let h = harness("", 0);
    let mut cfg = cfg_all_presets_allowed();
    cfg.per_image_caps.insert(
        "python".into(),
        crate::sandbox_daemon::config::PerImageCap {
            max_cpus: 4,
            max_memory_mb: 1024,
        },
    );
    let err = handle_create(
        CreateRequest {
            image: "python".into(),
            cpus: None,
            memory_mb: Some(9999),
            name: None,
            network: None,
            idle_timeout_secs: None,
            env: EnvShape::default(),
        },
        &cfg,
        &h.registry,
        &*h.launcher,
        |_| {},
    )
    .await
    .unwrap_err();
    assert_eq!(err.code().as_str(), "S400");
}

#[tokio::test]
async fn list_returns_every_sandbox() {
    let h = harness("", 0);
    h.registry.seed("python");
    let cfg = cfg_all_presets_allowed();
    let create = handle_create(
        CreateRequest {
            image: "python".into(),
            cpus: None,
            memory_mb: None,
            name: None,
            network: None,
            idle_timeout_secs: None,
            env: EnvShape::default(),
        },
        &cfg,
        &h.registry,
        &*h.launcher,
        |_| {},
    )
    .await
    .unwrap();
    assert!(
        h.registry
            .get(uuid::Uuid::parse_str(&create.sandbox_id).unwrap())
            .await
            .is_ok()
    );
    let create = handle_create(
        CreateRequest {
            image: "python".into(),
            cpus: None,
            memory_mb: None,
            name: None,
            network: None,
            idle_timeout_secs: None,
            env: EnvShape::default(),
        },
        &cfg,
        &h.registry,
        &*h.launcher,
        |_| {},
    )
    .await
    .unwrap();
    assert!(
        h.registry
            .get(uuid::Uuid::parse_str(&create.sandbox_id).unwrap())
            .await
            .is_ok()
    );

    use crate::sandbox_daemon::list::{ListRequest, handle_list};
    let resp = handle_list(ListRequest::default(), &h.registry).await;
    assert_eq!(resp.sandboxes.len(), 2);
    h.registry.finish().await;
}
