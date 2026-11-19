//! Owned-lifecycle handler suites migrated from integration-test crates.
//! Every scenario runs in its own private namespace; no production override.

mod fake_sandbox_relay;
mod sandbox_canonical_cache_boot_params;
mod sandbox_fakes;
mod sandbox_lifecycle_integration;
mod sandbox_shared_rootfs_boot_params;
mod sandbox_workflow_integration;

use crate::sandbox_daemon::{
    self,
    config::SandboxConfig,
    create::{BootHandle, BootParams, CreateRequest, CreateResponse, VmLauncher},
    errors::SandboxError,
    events::SandboxCreateEvent,
    registry::SandboxRegistry,
};
use crate::worker_admission::Lease;
use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub(super) struct TestRegistry(Arc<Fixture>);
struct Fixture {
    registry: SandboxRegistry,
    namespace: PathBuf,
    data: PathBuf,
    temporary: std::sync::Mutex<Option<(tempfile::TempDir, tempfile::TempDir)>>,
    cache: PathBuf,
}
impl Deref for TestRegistry {
    type Target = SandboxRegistry;
    fn deref(&self) -> &Self::Target {
        &self.0.registry
    }
}
impl TestRegistry {
    pub fn new() -> Self {
        Self::with_max_exec_in_flight(sandbox_daemon::registry::DEFAULT_MAX_EXEC_IN_FLIGHT)
    }
    pub fn with_max_exec_in_flight(cap: usize) -> Self {
        let namespace = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let cache = data.path().join("cache");
        crate::cli::rootfs_cache::prepare_cache_parent(&context, &cache).unwrap();
        let registry = SandboxRegistry::with_max_exec_in_flight(cap)
            .with_context(context)
            .with_layout_root(data.path().join("sandboxes"))
            .with_test_namespace(namespace.path().to_path_buf());
        Self(Arc::new(Fixture {
            registry,
            namespace: namespace.path().to_path_buf(),
            data: data.path().to_path_buf(),
            temporary: std::sync::Mutex::new(Some((namespace, data))),
            cache,
        }))
    }
    pub fn cache(&self) -> &Path {
        &self.0.cache
    }
    pub fn namespace(&self) -> &Path {
        self.0.namespace.as_path()
    }
    pub fn data_path(&self) -> &Path {
        self.0.data.as_path()
    }
    /// Failed scenarios retain evidence even when the namespace guard would
    /// otherwise TempDir-clean it. No success receipt is synthesized in Drop.
    pub fn preserve_evidence(&self) {
        if let Some((namespace, data)) = self.0.temporary.lock().unwrap().take() {
            let _namespace = namespace.keep();
            let _data = data.keep();
        }
    }
    pub fn seed(&self, image: &str) -> PathBuf {
        let reference = sandbox_daemon::catalog::resolve_image(image, &Default::default()).unwrap();
        let root = crate::cli::rootfs_cache::canonical_path_at(self.cache(), &reference);
        if root.exists() {
            return root;
        }
        let consumer = self
            .admission_context()
            .unwrap()
            .local(
                &self
                    .0
                    .data
                    .as_path()
                    .join(format!("seed-{}", uuid::Uuid::new_v4())),
            )
            .unwrap();
        let preparation = consumer
            .prepare_cache(&[crate::worker_admission::CacheEntry { root: &root }])
            .unwrap();
        let mut entry = preparation.begin_entry(&root).unwrap();
        let tree = entry.stage_tree(&root).unwrap();
        std::fs::create_dir(tree.path().join("bin")).unwrap();
        std::fs::write(tree.path().join("bin/marker"), b"original").unwrap();
        std::fs::write(
            tree.path().join(".oci-config.json"),
            br#"{"config":{"Env":[]}}"#,
        )
        .unwrap();
        entry.publish_tree(tree).unwrap();
        entry.commit().unwrap();
        crate::cli::erofs::build_prepared_erofs(&preparation, &root).unwrap();
        preparation.handoff(&consumer, &root).unwrap();
        root
    }
    pub async fn insert(&self, mut state: sandbox_daemon::registry::SandboxState) {
        let layout = self.layout(state.id);
        state.rootfs = layout.root.clone();
        state.workdir = layout.merged.clone();
        state.shell_sock = layout.shell_sock();
        state.vm_pid = None;
        self.0.registry.insert(state).await;
    }

    pub async fn insert_owned(
        &self,
        mut state: sandbox_daemon::registry::SandboxState,
    ) -> uuid::Uuid {
        let id = state.id;
        let layout = self.layout(id);
        let context = self.admission_context().unwrap();
        layout.prepare_parents(&context).unwrap();
        let lease = layout.admit(&context).unwrap();
        layout.ensure_admitted(&lease).unwrap();
        state.shell_sock = layout.shell_sock();
        state.rootfs = layout.root.clone();
        state.workdir = layout.merged.clone();
        self.insert(state).await;
        let params = BootParams {
            rootfs: layout.root.clone(),
            base_rootfs: self.cache().to_path_buf(),
            prepared: None,
            workdir: layout.merged.clone(),
            shell_sock: layout.shell_sock(),
            cpus: 1,
            memory_mb: 512,
            env: vec![],
            network: false,
        };
        // Test child has real admission/runtime FDs; fake shell execution is orthogonal.
        owned_boot(&params, lease, self, id).await.unwrap();
        id
    }
    pub async fn finish(&self) {
        sandbox_daemon::stop::stop_all(self, &sandbox_fakes::FakeVmStopper::ok())
            .await
            .unwrap();
        assert_eq!(self.count().await, 0);
    }
}

pub(super) async fn handle_create<L: VmLauncher, F: FnMut(SandboxCreateEvent) + Send + 'static>(
    req: CreateRequest,
    cfg: &SandboxConfig,
    registry: &TestRegistry,
    launcher: &L,
    event: F,
) -> Result<CreateResponse, SandboxError> {
    sandbox_daemon::create::test_handle_create(
        req,
        cfg,
        registry,
        launcher,
        event,
        registry.cache(),
    )
    .await
}

pub(super) async fn owned_boot(
    params: &BootParams,
    lease: Lease,
    registry: &SandboxRegistry,
    id: uuid::Uuid,
) -> Result<BootHandle, SandboxError> {
    let base = params
        .rootfs
        .parent()
        .ok_or_else(|| SandboxError::BootFailed("fixture root has no parent".into()))?
        .to_path_buf();
    registry.test_record_layout(
        id,
        sandbox_daemon::overlay::OverlayLayout {
            upper: base.join("upper"),
            work: base.join("work"),
            merged: base.join("merged"),
            root: params.rootfs.clone(),
        },
    );
    let namespace = registry
        .test_namespace()
        .ok_or_else(|| SandboxError::BootFailed("fixture namespace absent".into()))?;
    let session = sandbox_daemon::adapters::runtime_test_session_registered(
        lease,
        &namespace,
        registry,
        id,
        sandbox_daemon::adapters::RuntimeTestResources {
            reads: vec![params.rootfs.clone()],
            endpoints: vec![params.shell_sock.clone()],
            exit_without_drop: true,
            ..Default::default()
        },
    )
    .await;
    // PID is diagnostic only. All stop/cleanup authority is in the attached session.
    let vm_pid = session.test_diagnostic_pid().ok_or_else(|| {
        SandboxError::BootFailed("owned fixture child has no diagnostic PID".into())
    })?;
    Ok(BootHandle {
        vm_pid,
        lifeline: None,
    })
}
