// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! `sandbox-libkrun::worker::*`: worker VMs that Compose or `iii sandbox run` supervises.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use iii_sdk::IIIClient;
use iii_sdk::channels::ChannelWriter;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::watch;

use crate::cli::local_worker::{
    BootImage, VmOverride, WorkerProject, open_build_export, validate_source,
};
#[cfg(test)]
use crate::cli::local_worker::{validate_binary, validate_source_with};
use crate::cli::worker_manager::oci;
use crate::daemon_exit::Lifeline;
use crate::sandbox_daemon::SandboxError;
use crate::wire::{
    BaseImageRef, LogRecord, LogStream, State, WorkerBuildRef, WorkerBuildRequest,
    WorkerBuildResponse, WorkerDestroyRequest, WorkerDestroyResponse, WorkerLogsRequest,
    WorkerLogsResponse, WorkerPrepareRequest, WorkerPrepareResponse, WorkerSource,
    WorkerStartRequest, WorkerStartResponse, WorkerStatus, WorkerStatusRequest, WorkerStopRequest,
    scoped_auth_matches,
};

/// Compose's own worker-log cap: past it `console.log` rotates to `console.log.1`.
const CONSOLE_LOG_CAP: u64 = 10 * 1024 * 1024;
/// Longer output without a newline is split, so a guest cannot grow the adapter's memory.
const MAX_LINE: u64 = 64 * 1024;
/// How long the waiter lets the pumps drain after the VM exits: a grandchild
/// that inherited the pipes could hold them open forever.
const DRAIN_GRACE: Duration = Duration::from_secs(1);
/// How often a following `worker::logs` looks for new records.
const FOLLOW_POLL: Duration = Duration::from_millis(200);
const CONSOLE_LOG: &str = "console.log";
/// Where `console.log` rotates to; one rotated file is kept.
const CONSOLE_LOG_OLD: &str = "console.log.1";
const STATE_FILE: &str = "state.json";
/// How long a leased VM may go without a log follower. `iii sandbox run` gives up on an
/// adapter that has not answered for as long (its `LOST_AFTER`).
const LEASE_GRACE: Duration = Duration::from_secs(45);
/// How long a VM whose lease lapsed gets to exit before it is killed.
const LEASE_STOP_GRACE: Duration = Duration::from_secs(10);
/// How many times per grace a leased VM's quiet follower is probed: 5 s by default.
const LEASE_PROBES: u32 = 9;
/// Records the VM's `state_key`: `status`, `stop` and `logs` name it by `vm_id`.
const STATE_KEY_FILE: &str = "state_key";
/// What `worker::build` leaves in its `output_dir`: the image, then its build id.
const IMAGE_FILE: &str = "image.erofs";
const IMAGE_ID_FILE: &str = "image.id";
/// `<vm_id>/build-mode`, beside `root/` and out of the guest's reach: the VM's
/// data disk belongs to builds. Written by the first `worker::start` with `build`.
const BUILD_MODE_SEAL: &str = "build-mode";
/// Where the first start of a build keeps what an earlier adapter left in the VM
/// dir: a 0.26 overlay disk (`root/upper.ext4`) and a legacy rootfs clone (`root/`).
const PRE_BUILD_DISK: &str = "upper.ext4.pre-build";
const PRE_BUILD_ROOT: &str = "root.pre-build";
/// `<vm_id>/build-dir`, beside `root/` and out of the guest's reach: the build
/// directory the VM's scoped token claimed (see `claim_build_dir`).
const BUILD_DIR_RECORD: &str = "build-dir";
/// What the MAC of a `build-dir` record covers first, so it can never equal
/// another MAC of the project token over the same bytes.
const BUILD_DIR_DOMAIN: &[u8] = b"iii-sandbox/build-dir/v1\0";
/// A binary package runs only from a build: no prepare, no start without `build`.
const BINARY_NEEDS_A_BUILD: &str = "a binary source boots only a build: build it with \
     worker::build (with base_image), then start it with `build`";
/// A build's scratch dir in `<vm_id>/`: `build-<pid>-<seq>`.
const BUILD_SCRATCH_PREFIX: &str = "build-";
/// A build writes to its `logs` channel at least this often, a one-byte binary
/// frame when it has nothing to say, so a reader that left is noticed.
const BUILD_KEEPALIVE: Duration = Duration::from_secs(5);
/// The lines of build output an S507 carries.
const BUILD_TAIL_LINES: usize = 20;
/// `worker::build` and a `worker::start` of a build need the overlay rootfs.
const LEGACY_NO_BUILD: &str = "this adapter runs in the legacy rootfs mode \
     (III_ROOTFS_MODE=legacy, or a build without an embedded iii-init), which cannot boot a build";
/// A build's commit state, shared with its blocking image writer.
const BUILD_RUNNING: u8 = 0;
const BUILD_CANCELLED: u8 = 1;
const BUILD_COMMITTED: u8 = 2;

/// The worker VMs of one project, behind its token and host roots.
pub struct WorkerVms {
    #[cfg(test)]
    admission_context: Option<crate::worker_admission::AdmissionContext>,
    #[cfg(test)]
    cache_path: Option<PathBuf>,
    #[cfg(test)]
    start_fixture: Option<
        fn(
            &crate::worker_admission::Lease,
            &Path,
        ) -> Result<crate::cli::worker_manager::libkrun::VmCommand, String>,
    >,
    /// Per-VM state lives in `<base>/<vm_id>/`: the control files, and the VM's
    /// writable share `root/`, which a uid-0 guest controls.
    base: PathBuf,
    token: Option<String>,
    roots: Vec<PathBuf>,
    /// Keyed by `vm_id`. An exited VM stays, so `status` can report it.
    slots: Mutex<HashMap<String, Slot>>,
    /// Keep the original Arc across prepare/build/start. Restart recovery is
    /// separate; a same-process request never substitutes a fresh operation.
    operations: Mutex<HashMap<String, crate::worker_admission::Lease>>,
    prepared_caches: Mutex<HashMap<String, crate::cli::rootfs_cache::PreparedCache>>,
    retired_builds: Mutex<HashMap<String, crate::worker_admission::WriterRetirement>>,
    /// Only populated after the actual joined ImageWrite committed its pair.
    published_builds: Mutex<HashMap<String, PublishedBuild>>,
    /// Names each `start` call in its `Slot::Preparing` reservation.
    starts: AtomicU64,
    /// Notified as each `start` drops its reservation, for a `stop` that
    /// waits out the build it cancelled.
    unreserved: tokio::sync::Notify,
    /// The last instance of each `vm_id` this process destroyed, so its next
    /// boot still follows it and a stale stop gets S503.
    /// shortcut: in memory, so a daemon restart forgets it; persist it outside
    /// `<base>` if a stale stop can ever outlive the daemon.
    destroyed: Mutex<HashMap<String, u64>>,
    /// The `vm_id`s with a `worker::build` running: one at a time each (S509).
    /// Taken only while `slots` is held, as `destroyed` is.
    builds: Mutex<HashSet<String>>,
    /// Names each build's scratch dir and temporary files.
    build_seq: AtomicU64,
    /// Held while a scoped token claims a build directory, so two VMs never
    /// claim the same one.
    claims: Mutex<()>,
    /// How long a VM a scoped `start` booted may go without a log follower before
    /// it is stopped. A field so tests can shorten it.
    pub lease_grace: Duration,
}

#[derive(Clone)]
struct PublishedBuild {
    lease: crate::worker_admission::Lease,
    directory: PathBuf,
    build_id: String,
}
struct BuildReads {
    directory: crate::worker_admission::UseGuard,
    image: crate::worker_admission::UseGuard,
    id: crate::worker_admission::UseGuard,
}
impl PublishedBuild {
    fn read(&self) -> io::Result<BuildReads> {
        self.lease.revalidate()?;
        Ok(BuildReads {
            directory: self.lease.use_path(&self.directory)?,
            image: self.lease.use_path(&self.directory.join(IMAGE_FILE))?,
            id: self.lease.use_path(&self.directory.join(IMAGE_ID_FILE))?,
        })
    }
}

#[derive(Clone)]
enum Slot {
    /// `start` call `.1` is building the VM that will boot as instance `.0`;
    /// `.2` once `worker::stop` cancelled it. It stays until that build
    /// returns, so no other build or `destroy` of the key overlaps it.
    Preparing(u64, u64, bool),
    Booted(Arc<Vm>),
    /// `destroy` is removing the state of what was this instance.
    Destroying(u64),
}

struct Vm {
    instance: u64,
    /// Test fixtures only: the pidfd taken at spawn, before any waiter could
    /// reap the child, so a stop can never signal a reused PID.
    #[cfg(test)]
    fixture_pidfd: Option<std::os::fd::OwnedFd>,
    lifecycle: Option<Arc<crate::sandbox_daemon::adapters::RuntimeSession>>,
    admission: crate::worker_admission::Lease,
    retirement: Mutex<Option<Result<crate::worker_admission::WriterRetirement, String>>>,
    started_at_ms: u64,
    /// Set by `stop` before it signals, so the exit reads as `Stopped`.
    stopping: AtomicBool,
    /// Set once the waiter reaps the process: its pid may then be reused.
    #[cfg(test)]
    reaped: AtomicBool,
    /// `None` until the waiter records the exit, after the pumps drain.
    ended: watch::Sender<Option<Ended>>,
    /// The log its pumps append to; `worker::logs` reads it under this lock.
    console: Arc<Mutex<Console>>,
    /// How many `worker::logs` streams follow it now.
    followers: watch::Sender<usize>,
    /// Booted by a scoped `start`: stopped when nothing follows it (see `lease`).
    leased: AtomicBool,
}

#[derive(Clone, Copy)]
struct Ended {
    exit_code: Option<i32>,
    signal: Option<i32>,
    at_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct StateFile {
    instance: u64,
}

impl WorkerVms {
    pub fn new(base: PathBuf, token: Option<String>, roots: Vec<PathBuf>) -> Self {
        Self {
            #[cfg(test)]
            admission_context: None,
            #[cfg(test)]
            cache_path: None,
            #[cfg(test)]
            start_fixture: None,
            base,
            token,
            roots,
            slots: Mutex::default(),
            operations: Mutex::default(),
            prepared_caches: Mutex::default(),
            retired_builds: Mutex::default(),
            published_builds: Mutex::default(),
            starts: AtomicU64::new(0),
            unreserved: tokio::sync::Notify::new(),
            destroyed: Mutex::default(),
            builds: Mutex::default(),
            build_seq: AtomicU64::new(0),
            claims: Mutex::default(),
            lease_grace: LEASE_GRACE,
        }
    }

    fn admission(&self, key: Option<&str>) -> io::Result<crate::worker_admission::Lease> {
        let mut operations = self
            .operations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(key) = key {
            if let Some(lease) = operations.get(key) {
                return Ok(lease.clone());
            }
        }
        #[cfg(test)]
        let context = match &self.admission_context {
            Some(context) => context.clone(),
            None => crate::worker_admission::AdmissionContext::system()?,
        };
        #[cfg(not(test))]
        let context = crate::worker_admission::AdmissionContext::system()?;
        prepare_worker_base(&context, &self.base)?;
        let lease = context.worker(&self.base, key)?;
        let lease = match key {
            Some(key) => admit_control_files(lease, &self.base.join(vm_id(key)))?,
            None => lease,
        };
        if let Some(key) = key {
            operations.insert(key.to_string(), lease.clone());
        }
        Ok(lease)
    }

    fn cache_context(&self) -> io::Result<crate::worker_admission::AdmissionContext> {
        #[cfg(test)]
        if let Some(context) = &self.admission_context {
            return Ok(context.clone());
        }
        crate::worker_admission::AdmissionContext::system()
    }

    /// Only completed caller receipts: Vm::receipt is filled after all pumps
    /// and G settlement; retired_builds only after the full image job completes.
    fn cache_retirement(
        &self,
        key: &str,
    ) -> Result<crate::worker_admission::WriterRetirement, String> {
        let id = vm_id(key);
        let slot = self.slots().get(&id).cloned();
        if matches!(&slot, Some(Slot::Preparing(..) | Slot::Destroying(..))) {
            return Err("cache continuation is blocked by an active slot transition".into());
        }
        if let Some(receipt) = self
            .retired_builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .cloned()
        {
            return Ok(receipt);
        }
        match slot {
            Some(Slot::Booted(vm)) => vm.receipt().map_err(|e| e.to_string()),
            _ => Err(
                "new cache reference requires the current completed runtime retirement receipt"
                    .into(),
            ),
        }
    }

    fn project_cache_request(
        &self,
        key: Option<&str>,
        project: &WorkerProject,
        lease: crate::worker_admission::Lease,
        cache: &Path,
    ) -> Result<
        Result<crate::cli::rootfs_cache::PreparedCache, crate::cli::rootfs_cache::CacheRequest>,
        String,
    > {
        use crate::cli::rootfs_cache::CacheRequest;
        let reference = project.reference();
        let existing = key.and_then(|key| {
            self.prepared_caches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(key)
                .cloned()
        });
        if let Some(prepared) = &existing {
            if prepared.reference == reference {
                prepared
                    .validate(&reference)
                    .map_err(|e| format!("{e:#}"))?;
                return Ok(Ok(prepared.clone()));
            }
        }
        let has_lifecycle = key.is_some_and(|key| {
            self.slots().contains_key(&vm_id(key))
                || self
                    .retired_builds
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .contains_key(&vm_id(key))
        });
        let request = if existing.is_some() || has_lifecycle {
            let key = key.ok_or("cache continuation requires a keyed lifecycle")?;
            let receipt = self.cache_retirement(key)?;
            // No fallback: stale/foreign/consumed/missing receipt leaves A and
            // its original lease/claims untouched. Core validates identity.
            CacheRequest::from_retirement(lease, &receipt, &reference, cache)
        } else {
            CacheRequest::new(lease, &reference, cache)
        }
        .map_err(|e| format!("{e:#}"))?;
        Ok(Err(request))
    }

    fn remember_prepared(
        &self,
        key: Option<&str>,
        project: &mut WorkerProject,
        prepared: crate::cli::rootfs_cache::PreparedCache,
    ) -> crate::cli::rootfs_cache::PreparedCache {
        if let Some(key) = key {
            self.prepared_caches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key.to_string(), prepared.clone());
        }
        project.prepared = Some(prepared.clone());
        prepared
    }

    async fn prepare_project(
        &self,
        key: Option<&str>,
        project: &mut WorkerProject,
        lease: crate::worker_admission::Lease,
    ) -> Result<crate::cli::rootfs_cache::PreparedCache, String> {
        #[cfg(test)]
        let cache = self
            .cache_path
            .clone()
            .unwrap_or_else(crate::cli::rootfs_cache::cache_root);
        #[cfg(not(test))]
        let cache = crate::cli::rootfs_cache::cache_root();
        self.prepare_project_at(key, project, lease, &cache, |request| async move {
            request
                .resolve(true, || {})
                .await
                .map_err(|e| format!("{e:#}"))?
                .ok_or_else(|| "required cache was not prepared".to_string())
        })
        .await
    }

    async fn prepare_project_at<F, Fut>(
        &self,
        key: Option<&str>,
        project: &mut WorkerProject,
        lease: crate::worker_admission::Lease,
        cache: &Path,
        resolve: F,
    ) -> Result<crate::cli::rootfs_cache::PreparedCache, String>
    where
        F: FnOnce(crate::cli::rootfs_cache::CacheRequest) -> Fut,
        Fut: std::future::Future<Output = Result<crate::cli::rootfs_cache::PreparedCache, String>>,
    {
        let context = self.cache_context().map_err(|e| e.to_string())?;
        // A new continuation must not create a cache hierarchy before its
        // receipt is checked. First-use short setup is the existing H path.
        let previous = key.is_some_and(|key| {
            self.prepared_caches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key(key)
                || self.slots().contains_key(&vm_id(key))
                || self
                    .retired_builds
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .contains_key(&vm_id(key))
        });
        if !previous {
            crate::cli::rootfs_cache::prepare_cache_parent(&context, cache)
                .map_err(|e| e.to_string())?;
        }
        match self.project_cache_request(key, project, lease, cache)? {
            Ok(prepared) => {
                project.prepared = Some(prepared.clone());
                Ok(prepared)
            }
            Err(request) => {
                let prepared = resolve(request).await?;
                Ok(self.remember_prepared(key, project, prepared))
            }
        }
    }

    #[cfg(test)]
    fn with_admission_context(
        mut self,
        context: crate::worker_admission::AdmissionContext,
    ) -> Self {
        self.admission_context = Some(context);
        self
    }

    /// Compose sets the token and the roots on this container only.
    pub fn from_env() -> Self {
        let base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".iii/sandbox/workers");
        let token = std::env::var("III_SANDBOX_WORKER_TOKEN").ok();
        let roots = std::env::var_os("III_SANDBOX_HOST_ROOTS")
            .map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Self::new(base, token, roots)
    }

    /// Validate the source and pull its base image (and, under overlay, build
    /// the shared base erofs) so `worker::start` stays fast. Idempotent.
    pub async fn prepare(
        &self,
        req: WorkerPrepareRequest,
    ) -> Result<WorkerPrepareResponse, SandboxError> {
        self.authorize(&req.auth, req.state_key.as_deref())?;
        let source = Source::of(&req.source);
        if source.executable.is_some() {
            return Err(SandboxError::SourceRejected(
                BINARY_NEEDS_A_BUILD.to_string(),
            ));
        }
        let dir = self.admit(source.host_path)?;
        self.cache_context()
            .and_then(|context| context.reject_retained_path(Path::new(source.host_path)))
            .map_err(|e| SandboxError::PrepareFailed(e.to_string()))?;
        if std::fs::symlink_metadata(&self.base).is_err() {
            WorkerProject::resolve(
                &dir,
                source.package,
                source.run_override,
                source.executable,
                None,
                true,
            )
            .map_err(SandboxError::SourceRejected)?;
        }
        let _admission = self
            .admission(req.state_key.as_deref())
            .map_err(|e| SandboxError::PrepareFailed(e.to_string()))?;
        let _cache_hold = req
            .state_key
            .as_ref()
            .map(|key| self.hold_build(&vm_id(key)))
            .transpose()?;
        let _admission = if let Some(key) = &req.state_key {
            let vm = self.base.join(vm_id(key));
            crate::worker_admission::with_paths(
                _admission,
                &[crate::worker_admission::PathRequest {
                    path: &vm,
                    role: crate::worker_admission::PathRole::MutableTree,
                }],
            )
            .map_err(|e| SandboxError::PrepareFailed(e.to_string()))?
        } else {
            _admission
        };

        let _admission =
            crate::worker_admission::with_inputs(_admission, &[Path::new(source.host_path), &dir])
                .map_err(|e| SandboxError::PrepareFailed(e.to_string()))?;
        let _source_use = _admission
            .use_path(&dir)
            .map_err(|e| SandboxError::PrepareFailed(e.to_string()))?;
        let mut project = WorkerProject::resolve(
            &dir,
            source.package,
            source.run_override,
            source.executable,
            None,
            true,
        )
        .map_err(SandboxError::SourceRejected)?;
        drop(_source_use);
        self.prepare_project(req.state_key.as_deref(), &mut project, _admission.clone())
            .await
            .map_err(SandboxError::PrepareFailed)?;
        // For a client reattaching to a live boot: its `start` writes nothing, it gets S502.
        if let Some(state_key) = &req.state_key {
            let vm_dir = self.base.join(vm_id(state_key));
            write_control_admitted(&_admission, &vm_dir, STATE_KEY_FILE, state_key.as_bytes())
                .map_err(|e| SandboxError::PrepareFailed(format!("{}: {e}", vm_dir.display())))?;
        }
        Ok(WorkerPrepareResponse {})
    }

    /// Build the VM for `state_key` and boot it. S502 while that VM is
    /// preparing or running; a source `prepare` would reject is S501, as there;
    /// any other build error is S300.
    pub async fn start(
        &self,
        req: WorkerStartRequest,
    ) -> Result<WorkerStartResponse, SandboxError> {
        self.start_with_cache_admission(req, WorkerProject::admit_cache)
            .await
    }

    async fn start_with_cache_admission(
        &self,
        req: WorkerStartRequest,
        admit_cache: impl FnOnce(
            &WorkerProject,
            crate::worker_admission::Lease,
        ) -> Result<crate::worker_admission::Lease, String>,
    ) -> Result<WorkerStartResponse, SandboxError> {
        self.authorize(&req.auth, Some(&req.state_key))?;
        let source = Source::of(&req.source);
        if source.executable.is_some() && req.build.is_none() {
            return Err(SandboxError::SourceRejected(
                BINARY_NEEDS_A_BUILD.to_string(),
            ));
        }
        let dir = self.admit(source.host_path)?;
        if let Some(build) = &req.build {
            let build_dir = self.admit(&build.build_dir)?;
            let context = self
                .cache_context()
                .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
            for path in [
                PathBuf::from(&build.build_dir),
                build_dir.join(IMAGE_FILE),
                build_dir.join(IMAGE_ID_FILE),
            ] {
                context
                    .reject_retained_path(&path)
                    .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
            }
        }
        self.cache_context()
            .and_then(|context| context.reject_retained_path(Path::new(source.host_path)))
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        if std::fs::symlink_metadata(&self.base).is_err() {
            WorkerProject::resolve(
                &dir,
                source.package,
                source.run_override,
                source.executable,
                None,
                req.build.is_none(),
            )
            .map_err(SandboxError::SourceRejected)?;
        }
        let admission = self
            .admission(Some(&req.state_key))
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        // Compose presents the project token and supervises its VMs itself.
        let leased = !self.is_project_token(&req.auth);

        // Admit the supplied spellings as well as authorized canonical paths:
        // canonicalization must not hide symlinks or traversal from admission.
        let mut inputs = vec![PathBuf::from(source.host_path), dir.clone()];
        let id = vm_id(&req.state_key);
        let _cache_hold = self.hold_build(&id)?;
        let vm_dir = self.base.join(&id);
        inputs.push(vm_dir.join("root"));
        inputs.push(vm_dir.join("root").join(crate::cli::upper::UPPER_NAME));
        inputs.push(vm_dir.join(
            if req.build.is_some() || crate::cli::overlay::overlay_active() {
                "root/runtime"
            } else {
                "root/opt/iii"
            },
        ));
        inputs.push(vm_dir.join(crate::cli::upper::UPPER_NAME));
        inputs.push(vm_dir.clone());
        let mutable_end = inputs.len();
        let owned_build = req.build.as_ref().and_then(|build| {
            self.published_builds
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&id)
                .filter(|published| published.directory == Path::new(&build.build_dir))
                .cloned()
        });
        let owned_reads = owned_build
            .as_ref()
            .map(PublishedBuild::read)
            .transpose()
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        if let Some(build) = &req.build {
            if owned_build.is_none() {
                let build_dir = self.admit(&build.build_dir)?;
                inputs.push(PathBuf::from(&build.build_dir));
                inputs.push(build_dir.join(IMAGE_FILE));
                inputs.push(build_dir.join(IMAGE_ID_FILE));
                inputs.push(build_dir);
            }
        }
        let inputs: Vec<&Path> = inputs.iter().map(PathBuf::as_path).collect();
        let requests: Vec<_> = inputs
            .iter()
            .enumerate()
            .map(|(index, path)| crate::worker_admission::PathRequest {
                path,
                role: if index >= 2 && index < mutable_end {
                    if path
                        .file_name()
                        .is_some_and(|name| name == crate::cli::upper::UPPER_NAME)
                    {
                        crate::worker_admission::PathRole::MutableFile
                    } else {
                        crate::worker_admission::PathRole::MutableTree
                    }
                } else {
                    crate::worker_admission::PathRole::Strict
                },
            })
            .collect();
        let admission = crate::worker_admission::with_paths(admission, &requests)
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        if req.build.is_some() && !crate::cli::overlay::overlay_active() {
            return Err(SandboxError::BootFailed(LEGACY_NO_BUILD.to_string()));
        }
        let _source_use = admission
            .use_path(&dir)
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        let mut resolved = WorkerProject::resolve(
            &dir,
            source.package,
            source.run_override,
            source.executable,
            None,
            req.build.is_none(),
        )
        .map_err(SandboxError::SourceRejected)?;
        drop(_source_use);
        let admission = if req.build.is_none() {
            admit_cache(&resolved, admission).map_err(SandboxError::BootFailed)?
        } else {
            admission
        };
        if req.build.is_none() {
            self.prepare_project(Some(&req.state_key), &mut resolved, admission.clone())
                .await
                .map_err(SandboxError::BootFailed)?;
        }
        admission
            .revalidate()
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        let start = self.starts.fetch_add(1, Ordering::SeqCst);
        {
            let mut slots = self.slots();
            refuse_busy(&id, slots.get(&id))?;
            let next = self.next_instance(&id)?;
            slots.insert(id.clone(), Slot::Preparing(next, start, false));
        }
        let _unreserve = Unreserve(self, &id, start);
        // Under the reservation: no destroy runs while the claim writes the
        // VM's build-dir record, so none leaves it behind.
        let image = match &req.build {
            Some(build) => Some(self.admit_build_read(
                &req.auth,
                &req.state_key,
                build,
                admission.clone(),
                owned_build.as_ref(),
                owned_reads.as_ref(),
            )?),
            None => None,
        };
        // Use the project selected before reservation; do not re-read its manifest.
        let mut notes = Vec::new();
        let image = match image {
            Some(image) => {
                let vm_dir = self.base.join(&id);
                let (upper, kept) =
                    adopt_build_mode_admitted(&admission, &vm_dir, &req.worker_name).map_err(
                        |e| {
                            SandboxError::BootFailed(format!(
                                "cannot set up the data disk in {}: {e}",
                                vm_dir.display()
                            ))
                        },
                    )?;
                // Now: the seal is written, so no later start logs them again.
                for note in &kept {
                    tracing::warn!(vm_id = %id, "{note}");
                }
                notes = kept;
                Some(BootImage { image, upper })
            }
            None => None,
        };
        let over = VmOverride {
            state_dir: self.base.join(&id).join("root"),
            engine_url: &req.engine_url,
            extra_env: req.env.into_iter().collect(),
            image,
        };
        if let Some(reads) = &owned_reads {
            reads
                .directory
                .revalidate()
                .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
            reads
                .id
                .revalidate()
                .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        }
        if let Some(reads) = &owned_reads {
            let read = admission
                .use_path(reads.image.path())
                .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
            resolved = resolved.with_published_image(read);
        }
        #[cfg(test)]
        let fixture = self
            .start_fixture
            .map(|fixture| fixture(&admission, &self.base));
        #[cfg(not(test))]
        let fixture: Option<
            Result<crate::cli::worker_manager::libkrun::VmCommand, String>,
        > = None;
        let built = match fixture {
            Some(command) => command,
            None => {
                resolved
                    .start(
                        &req.worker_name,
                        &dir,
                        source.package,
                        over,
                        admission.clone(),
                    )
                    .await
            }
        }
        .map_err(|e| SandboxError::BootFailed(with_notes(e, &notes)))?;
        let started = self
            .launch_admitted(
                &req.state_key,
                built.command,
                built.env_stdin,
                Some(start),
                admission,
                built.lifecycle_leases,
                Some(built.runtime_paths),
                built.runtime_endpoints,
                owned_reads
                    .into_iter()
                    .flat_map(|reads| [reads.directory, reads.image, reads.id])
                    .collect(),
            )
            .map_err(|e| match e {
                SandboxError::BootFailed(e) => SandboxError::BootFailed(with_notes(e, &notes)),
                e => e,
            })?;
        if leased {
            self.arm_lease(&started.vm_id);
        }
        self.note(&started.vm_id, &notes);
        Ok(started)
    }

    /// Append `notes` to `vm_id`'s console, where its client reads them.
    fn note(&self, vm_id: &str, notes: &[String]) {
        if notes.is_empty() {
            return;
        }
        let Some(Slot::Booted(vm)) = self.slots().get(vm_id).cloned() else {
            return;
        };
        let mut console = vm.console.lock().unwrap_or_else(PoisonError::into_inner);
        for note in notes {
            let _ = console.append(&LogStream::Stderr, note.as_bytes());
        }
    }

    /// The image a start of `build` boots: S508 unless `build_dir/image.id` names
    /// `build_id` and the image is there. The image is not hashed. A scoped
    /// token boots no directory another VM recorded (S505, `claim_build_dir`),
    /// checked before any id is read.
    fn admit_build(
        &self,
        auth: &str,
        state_key: &str,
        build: &WorkerBuildRef,
    ) -> Result<PathBuf, SandboxError> {
        let admission = self
            .admission(Some(state_key))
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        self.admit_build_on(auth, state_key, build, admission)
    }

    fn admit_build_on(
        &self,
        auth: &str,
        state_key: &str,
        build: &WorkerBuildRef,
        admission: crate::worker_admission::Lease,
    ) -> Result<PathBuf, SandboxError> {
        self.admit_build_read(auth, state_key, build, admission, None, None)
    }

    fn admit_build_read(
        &self,
        auth: &str,
        state_key: &str,
        build: &WorkerBuildRef,
        admission: crate::worker_admission::Lease,
        published: Option<&PublishedBuild>,
        reads: Option<&BuildReads>,
    ) -> Result<PathBuf, SandboxError> {
        let dir = self.admit(&build.build_dir)?;
        // The start caller retains its operation through the waiter.
        // independent guard here too, through the claim and image inspection.
        let _admission = if let (Some(published), Some(reads)) = (published, reads) {
            if published.directory != dir || Path::new(&build.build_dir) != dir {
                return Err(SandboxError::BuildMismatch(
                    "published build path does not match exact output".into(),
                ));
            }
            reads
                .directory
                .revalidate()
                .and_then(|_| reads.image.revalidate())
                .and_then(|_| reads.id.revalidate())
                .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
            admission
        } else {
            crate::worker_admission::with_inputs(
                admission,
                &[
                    Path::new(&build.build_dir),
                    &dir,
                    &dir.join(IMAGE_FILE),
                    &dir.join(IMAGE_ID_FILE),
                ],
            )
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?
        };
        self.claim_build_dir_on(auth, state_key, &dir, "build_dir", Some(&_admission))?;
        let _directory_use = _admission
            .use_path(&dir)
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        let wanted = build.build_id.trim();
        if published.is_some_and(|published| published.build_id != wanted) {
            return Err(SandboxError::BuildMismatch(
                "requested id does not name this completed publication".into(),
            ));
        }
        match read_image_id(&dir) {
            Some(found) if found == wanted => {}
            Some(found) => {
                return Err(SandboxError::BuildMismatch(format!(
                    "{} names {found}, not {wanted}; build again",
                    dir.join(IMAGE_ID_FILE).display()
                )));
            }
            None => {
                return Err(SandboxError::BuildMismatch(format!(
                    "{} is missing; build again",
                    dir.join(IMAGE_ID_FILE).display()
                )));
            }
        }
        let image = dir.join(IMAGE_FILE);
        if !std::fs::symlink_metadata(&image).is_ok_and(|m| m.is_file()) {
            return Err(SandboxError::BuildMismatch(format!(
                "{} is missing; build again",
                image.display()
            )));
        }
        Ok(image)
    }

    /// Build the image of `state_key`'s VM into `output_dir`: a build VM on the
    /// base image runs `scripts.setup` and `scripts.install`, and its root becomes
    /// `image.erofs`, then `image.id`. The output streams into `logs`, and the
    /// build ends with the reader of `logs`: a closed channel, or a write that
    /// fails, stops the build VM, drops the temporary image and frees the key.
    /// S509 while another build of the key runs, S507 when a step fails, S505
    /// when a scoped token names a directory another VM recorded (`claim_build_dir`).
    pub async fn build(
        &self,
        iii: &IIIClient,
        req: WorkerBuildRequest,
    ) -> Result<WorkerBuildResponse, SandboxError> {
        self.build_with_cache_admission(iii, req, WorkerProject::admit_cache)
            .await
    }

    async fn build_with_cache_admission(
        &self,
        iii: &IIIClient,
        req: WorkerBuildRequest,
        admit_cache: impl FnOnce(
            &WorkerProject,
            crate::worker_admission::Lease,
        ) -> Result<crate::worker_admission::Lease, String>,
    ) -> Result<WorkerBuildResponse, SandboxError> {
        self.authorize(&req.auth, Some(&req.state_key))?;
        let source = Source::of(&req.source);
        let base_image = build_base_image(req.base_image.as_deref(), source.executable.is_some())?;
        let dir = self.admit(source.host_path)?;
        let planned = self.plan_output(&req.output_dir)?;
        let context = self
            .cache_context()
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        for path in [
            PathBuf::from(&req.output_dir),
            planned.join(IMAGE_FILE),
            planned.join(IMAGE_ID_FILE),
        ] {
            context
                .reject_retained_path(&path)
                .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        }
        self.cache_context()
            .and_then(|context| context.reject_retained_path(Path::new(source.host_path)))
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        if std::fs::symlink_metadata(&self.base).is_err() {
            WorkerProject::resolve(
                &dir,
                source.package,
                source.run_override,
                source.executable,
                base_image,
                true,
            )
            .map_err(SandboxError::SourceRejected)?;
        }
        let _admission = self
            .admission(Some(&req.state_key))
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        let id = vm_id(&req.state_key);
        let _held = self.hold_build(&id)?;
        use crate::worker_admission::{PathRequest, PathRole};
        let _admission = crate::worker_admission::with_paths(
            _admission,
            &[
                PathRequest {
                    path: Path::new(source.host_path),
                    role: PathRole::Strict,
                },
                PathRequest {
                    path: &dir,
                    role: PathRole::Strict,
                },
                PathRequest {
                    path: Path::new(&req.output_dir),
                    role: PathRole::MutableTree,
                },
                PathRequest {
                    path: &planned,
                    role: PathRole::MutableTree,
                },
                PathRequest {
                    path: &planned.join(IMAGE_FILE),
                    role: PathRole::MutableFile,
                },
                PathRequest {
                    path: &planned.join(IMAGE_ID_FILE),
                    role: PathRole::MutableFile,
                },
                PathRequest {
                    path: &planned.join("image.spool"),
                    role: PathRole::MutableFile,
                },
            ],
        )
        .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        let _source_use = _admission
            .use_path(&dir)
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        let mut resolved = WorkerProject::resolve(
            &dir,
            source.package,
            source.run_override,
            source.executable,
            base_image,
            true,
        )
        .map_err(SandboxError::SourceRejected)?;
        drop(_source_use);
        let _admission = admit_cache(&resolved, _admission).map_err(SandboxError::BuildFailed)?;
        if !crate::cli::overlay::overlay_active() {
            return Err(SandboxError::BuildFailed(LEGACY_NO_BUILD.to_string()));
        }
        let id = vm_id(&req.state_key);
        let vm_dir = self.base.join(&id);
        // A stable declared slot supports repeat builds without adding mutable
        // layout claims after A's retirement. Reclone preserves prior contents.
        let scratch = vm_dir.join(format!("{BUILD_SCRATCH_PREFIX}current"));
        let _admission = crate::worker_admission::with_paths(
            _admission,
            &[
                PathRequest {
                    path: &vm_dir,
                    role: PathRole::MutableTree,
                },
                PathRequest {
                    path: &scratch,
                    role: PathRole::MutableTree,
                },
                PathRequest {
                    path: &scratch.join("root"),
                    role: PathRole::MutableTree,
                },
                PathRequest {
                    path: &scratch.join("root/runtime"),
                    role: PathRole::MutableTree,
                },
                PathRequest {
                    path: &scratch.join(crate::cli::upper::UPPER_NAME),
                    role: PathRole::MutableFile,
                },
            ],
        )
        .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        self.prepare_project(Some(&req.state_key), &mut resolved, _admission.clone())
            .await
            .map_err(SandboxError::BuildFailed)?;
        let tag = format!(
            "{}-{}",
            std::process::id(),
            self.build_seq.fetch_add(1, Ordering::SeqCst)
        );
        let output_admission = _admission.clone();
        // The key stays held through the writer. V2 scratch/journal evidence
        // is not deleted by an unowned Drop or a PID-based sweep.
        // Under the hold, so no `destroy` removes the claim while this build
        // writes, and before the directory exists: a refused token creates nothing.
        self.claim_build_dir_on(
            &req.auth,
            &req.state_key,
            &planned,
            "output_dir",
            Some(&_admission),
        )?;
        let mut create = _admission
            .begin(crate::worker_admission::MutationPlan::Create {
                root: planned.clone(),
            })
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        create
            .create_dir_all(&planned, 0o700)
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        create
            .commit()
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        let out = planned.clone();
        // V2 stage/scratch evidence is core-journaled, not swept by PID. Legacy
        // sweep helpers remain for independent maintenance, not this transaction.
        let writer = ChannelWriter::new(iii.address(), &req.logs);
        let mut log = BuildLog::new(&writer);
        let state = Arc::new(AtomicU8::new(BUILD_RUNNING));
        let image_writer = WriterSlot::default();
        let done = Mutex::new(BuildDone::default());
        let job = BuildJob {
            worker_name: &req.worker_name,
            admission: _admission.clone(),
            output_admission,
            dir: &dir,
            resolved,
            package: source.package,
            out: &out,
            scratch: &scratch,
            tag: &tag,
            env: req.env.clone().into_iter().collect(),
            state: state.clone(),
            image_writer: &image_writer,
            done: &done,
        };
        self.retired_builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        self.published_builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        let mut built = tokio::select! {
            built = job.run(&mut log) => built,
            gone = reader_gone(&writer, BUILD_KEEPALIVE) => Err(SandboxError::BuildFailed(
                format!("build cancelled: the reader of `logs` is gone ({gone})")
            )),
        };
        if built.is_err() {
            // Await the blocking writer. Its uncommitted stages remain under
            // core recovery custody; cancellation never implies disposal.
            let written = cancel_writer(&image_writer, &state).await;
            // Unless the image was already in place: then the build is done.
            let mut done = done.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(response) = done.response.take() {
                built = Ok(response);
            } else if let (Some(Ok(build_id)), Some(base_image)) = (written, done.base_image.take())
            {
                built = Ok(WorkerBuildResponse {
                    build_id,
                    base_image,
                    built_at_ms: now_ms(),
                });
            }
        }
        if let Ok(response) = &built {
            self.remember_publication(&req.state_key, &_admission, &out, &response.build_id);
            if let Some(receipt) = done
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retirement
                .clone()
            {
                self.retired_builds
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(id.clone(), receipt);
            }
        }
        match &built {
            Ok(b) => {
                tracing::info!(vm_id = %id, worker = %req.worker_name, build_id = %b.build_id, "worker build done")
            }
            Err(e) => {
                tracing::info!(vm_id = %id, worker = %req.worker_name, error = %e, "worker build failed")
            }
        }
        // Connects even if nothing was sent, so the reader still sees the close.
        let _ = writer.write(&[]).await;
        let _ = writer.close().await;
        built
    }

    fn remember_publication(
        &self,
        key: &str,
        lease: &crate::worker_admission::Lease,
        out: &Path,
        id: &str,
    ) {
        self.published_builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                vm_id(key),
                PublishedBuild {
                    lease: lease.clone(),
                    directory: out.to_path_buf(),
                    build_id: id.to_string(),
                },
            );
    }

    /// Reserve `vm_id` for one build: S509 while another build holds it, S502
    /// while it is being destroyed.
    fn hold_build(&self, vm_id: &str) -> Result<BuildHold<'_>, SandboxError> {
        let slots = self.slots();
        if let Some(Slot::Booted(vm)) = slots.get(vm_id) {
            if vm.lifecycle.is_some() && vm.receipt().is_err() {
                return Err(SandboxError::StateBusy {
                    vm_id: vm_id.to_string(),
                    instance: vm.instance,
                });
            }
        }
        if let Some(Slot::Preparing(instance, ..) | Slot::Destroying(instance)) = slots.get(vm_id) {
            return Err(SandboxError::StateBusy {
                vm_id: vm_id.to_string(),
                instance: *instance,
            });
        }
        if !self.builds().insert(vm_id.to_string()) {
            return Err(SandboxError::BuildInProgress(vm_id.to_string()));
        }
        Ok(BuildHold(self, vm_id.to_string()))
    }

    fn builds(&self) -> MutexGuard<'_, HashSet<String>> {
        self.builds.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Where `output_dir` is, or will be once created: its nearest existing
    /// ancestor, canonical and under the host roots, joined with the components
    /// below it, which do not exist yet. Creates nothing. S501 for a relative
    /// path, `.` or `..`, an ancestor outside the roots, or an existing
    /// ancestor that is not a directory.
    fn plan_output(&self, output_dir: &str) -> Result<PathBuf, SandboxError> {
        use std::path::Component;
        let path = Path::new(output_dir);
        if !path.is_absolute()
            || path
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        {
            return Err(SandboxError::SourceRejected(format!(
                "output_dir {output_dir} must be an absolute path without `.` or `..`"
            )));
        }
        let mut existing = path;
        while std::fs::symlink_metadata(existing).is_err() {
            existing = existing.parent().ok_or_else(|| {
                SandboxError::SourceRejected(format!("output_dir {output_dir} has no parent"))
            })?;
        }
        let below = path
            .strip_prefix(existing)
            .expect("an ancestor is a prefix of its path");
        let existing = self.admit(existing)?;
        // Refused now, so a claim never moves the VM's record for a create
        // that must fail.
        if !existing.is_dir() {
            return Err(SandboxError::SourceRejected(
                if below.as_os_str().is_empty() {
                    format!("output_dir {output_dir} is not a directory")
                } else {
                    format!(
                        "output_dir {output_dir} cannot be created: {} is not a directory",
                        existing.display()
                    )
                },
            ));
        }
        Ok(if below.as_os_str().is_empty() {
            existing
        } else {
            existing.join(below)
        })
    }

    /// Create the directory `plan_output` planned for `output_dir`: S501 unless
    /// it is then that very directory, with no link put in its way since.
    fn create_output(&self, planned: &Path, output_dir: &str) -> Result<PathBuf, SandboxError> {
        std::fs::create_dir_all(planned).map_err(|e| {
            SandboxError::SourceRejected(format!("cannot create output_dir {output_dir}: {e}"))
        })?;
        let dir = self.admit(planned)?;
        if dir != planned || !dir.is_dir() {
            return Err(SandboxError::SourceRejected(format!(
                "output_dir {output_dir} is not a directory"
            )));
        }
        Ok(dir)
    }

    /// With a scoped token, `dir` (canonical, under the host roots, created or
    /// not yet) must not be the build directory another VM of this project
    /// recorded: a build writes there and a start boots from there only with
    /// that VM's token or the project token, so the token Compose hands one
    /// container cannot replace or boot an image another container's VM
    /// recorded. S505 otherwise, naming the record and the way out.
    ///
    /// A VM's build directory is the last one its scoped token presented, as
    /// `worker::build`'s `output_dir` or `worker::start`'s `build_dir`: another
    /// one moves it there and releases the previous one, as the VM's own
    /// `worker::destroy` would. The record, `<vm_id>/build-dir`, carries a MAC of
    /// the project token: it outlives an adapter restart, and one another project
    /// token wrote counts for nothing. Two paths name the same directory when
    /// they are equal or, both existing, share device and inode, so a second
    /// spelling (a bind mount, another case on a case-insensitive filesystem) is
    /// still the other VM's. Nothing reads a directory's name.
    ///
    /// The check and the write hold the claims mutex and an flock on `<base>`,
    /// so two claims of one directory, from two threads or two adapter
    /// processes, never both succeed; on a filesystem that cannot lock a
    /// directory, only the mutex, within this adapter process. The project token passes
    /// and records nothing. `field` names the request field in errors.
    fn claim_build_dir(
        &self,
        auth: &str,
        state_key: &str,
        dir: &Path,
        field: &str,
    ) -> Result<(), SandboxError> {
        self.claim_build_dir_on(auth, state_key, dir, field, None)
    }

    fn claim_build_dir_on(
        &self,
        auth: &str,
        state_key: &str,
        dir: &Path,
        field: &str,
        admission: Option<&crate::worker_admission::Lease>,
    ) -> Result<(), SandboxError> {
        if self.is_project_token(auth) {
            return Ok(());
        }
        // A scoped token that got here derives from a non-empty project token.
        let token = self.token.as_deref().unwrap_or("");
        let id = vm_id(state_key);
        let _claims = self.claims.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(lease) = admission {
            let mut create = lease
                .begin(crate::worker_admission::MutationPlan::Create {
                    root: self.base.join(&id),
                })
                .map_err(|e| SandboxError::FsIo(e.to_string()))?;
            create
                .create_dir_all(&self.base.join(&id), 0o700)
                .map_err(|e| SandboxError::FsIo(e.to_string()))?;
            create
                .commit()
                .map_err(|e| SandboxError::FsIo(e.to_string()))?;
        }
        let _lock = lock_build_dirs(&self.base).map_err(SandboxError::FsIo)?;
        let here = dir_identity(dir);
        let mut own = None;
        for (vm, recorded) in recorded_build_dirs(&self.base, token) {
            if recorded != dir && (here.is_none() || dir_identity(&recorded) != here) {
                continue;
            }
            if vm != id {
                return Err(SandboxError::BuildDirRefused(format!(
                    "{field} {} is the build directory of VM {vm}, recorded in {}, not of VM \
                     {id}; destroy VM {vm}, or delete that file, to use it for VM {id}",
                    dir.display(),
                    self.base.join(&vm).join(BUILD_DIR_RECORD).display()
                )));
            }
            own = Some(recorded);
        }
        if own.as_deref() == Some(dir) {
            return Ok(());
        }
        // A first claim, a move from another directory, or a new spelling.
        let stored = if let Some(lease) = admission {
            let record = BuildDirRecord {
                state_key: state_key.to_string(),
                dir: dir.to_path_buf(),
                mac: hex::encode(build_dir_mac(token, state_key, dir).finalize().into_bytes()),
            };
            serde_json::to_vec(&record)
                .map_err(io::Error::other)
                .and_then(|bytes| {
                    write_control_admitted(lease, &self.base.join(&id), BUILD_DIR_RECORD, &bytes)
                })
        } else {
            store_build_dir(&self.base, &id, state_key, dir, token)
        };
        stored.map_err(|e| {
            SandboxError::FsIo(format!("cannot record the build directory of VM {id}: {e}"))
        })
    }

    /// Spawn `cmd` as the next instance of `state_key`'s VM, its output pumped
    /// into `console.log`, and write `env_stdin` (the `--env-stdin` frame) to
    /// its stdin. `start` names the `start` call whose reservation this fills.
    #[cfg(test)]
    pub(crate) fn launch(
        &self,
        state_key: &str,
        cmd: Command,
        env_stdin: Vec<u8>,
        start: Option<u64>,
    ) -> Result<WorkerStartResponse, SandboxError> {
        let admission = self
            .admission(Some(state_key))
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        self.launch_admitted(
            state_key,
            cmd,
            env_stdin,
            start,
            admission,
            Vec::new(),
            None,
            Vec::new(),
            Vec::new(),
        )
    }

    fn launch_admitted(
        &self,
        state_key: &str,
        cmd: Command,
        env_stdin: Vec<u8>,
        start: Option<u64>,
        admission: crate::worker_admission::Lease,
        local_leases: Vec<crate::worker_admission::Lease>,
        runtime_paths: Option<Vec<(PathBuf, crate::worker_admission::RuntimePathKind)>>,
        runtime_endpoints: Vec<PathBuf>,
        image_reads: Vec<crate::worker_admission::UseGuard>,
    ) -> Result<WorkerStartResponse, SandboxError> {
        let control = self.base.join(vm_id(state_key));
        let admission = crate::worker_admission::with_paths(
            admission,
            &[crate::worker_admission::PathRequest {
                path: &control,
                role: crate::worker_admission::PathRole::MutableTree,
            }],
        )
        .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        admission
            .revalidate()
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        for lease in &local_leases {
            lease
                .revalidate()
                .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        }
        let id = vm_id(state_key);
        let dir = self.base.join(&id);
        // Held to the insert, so two launches of one key cannot both spawn.
        let mut slots = self.slots();
        match (slots.get(&id), start) {
            (slot, None) => refuse_busy(&id, slot)?,
            (Some(Slot::Preparing(_, held, false)), Some(start)) if *held == start => {}
            // `worker::stop` cancelled the reservation while the VM was being built.
            _ => {
                return Err(SandboxError::BootFailed(
                    "worker::stop cancelled this start before its VM booted".to_string(),
                ));
            }
        }
        let instance = self.next_instance(&id)?;
        let (console, lifeline, mut child, startup_error) =
            spawn_admitted(&admission, &dir, state_key, instance, cmd, runtime_paths).map_err(
                |e| SandboxError::BootFailed(format!("cannot launch VM in {}: {e}", dir.display())),
            )?;
        let console = Arc::new(Mutex::new(console));
        #[cfg(test)]
        let fixture_pidfd = child.take_fixture_pidfd();
        let stdin = child.take_stdin();
        let stdout = child.take_stdout().expect("stdout is piped");
        let stderr = child.take_stderr().expect("stderr is piped");
        let (lifecycle, completion) = child.supervise(admission.clone(), runtime_endpoints);
        let vm = Arc::new(Vm {
            instance,
            #[cfg(test)]
            fixture_pidfd,
            lifecycle,
            admission: admission.clone(),
            retirement: Mutex::new(None),
            started_at_ms: now_ms(),
            stopping: AtomicBool::new(false),
            #[cfg(test)]
            reaped: AtomicBool::new(false),
            ended: watch::Sender::new(None),
            console: console.clone(),
            followers: watch::Sender::new(0),
            leased: AtomicBool::new(false),
        });
        let feed = tokio::spawn(async move {
            crate::cli::worker_manager::libkrun::feed_env_stdin(stdin, &env_stdin).await
        });
        let pumps = [
            tokio::spawn(pump(stdout, LogStream::Stdout, console.clone())),
            tokio::spawn(pump(stderr, LogStream::Stderr, console)),
        ];
        let waited = vm.clone();
        let mut held = local_leases;
        held.push(admission);
        let writer_admission = crate::worker_admission::WriterLease::new(held);
        tokio::spawn(async move {
            let _image_reads = image_reads;
            let (status, mut retirement) = match completion.await {
                Ok(result) => result,
                Err(error) => (None, Err(format!("worker completion task: {error}"))),
            };
            #[cfg(test)]
            waited.reaped.store(status.is_some(), Ordering::SeqCst);
            drop(lifeline);
            let mut feed = feed;
            match tokio::time::timeout(DRAIN_GRACE, &mut feed).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(error))) => {
                    retirement = Err(format!("worker environment delivery failed: {error}"))
                }
                Ok(Err(error)) => {
                    retirement = Err(format!("worker environment task failed: {error}"))
                }
                Err(_) => {
                    feed.abort();
                    if let Err(error) = feed.await {
                        tracing::warn!(%error, "worker environment task joined after deadline");
                    }
                    retirement =
                        Err("worker environment delivery deadline; preserving lifecycle".into());
                }
            }
            for mut pump in pumps {
                match tokio::time::timeout(DRAIN_GRACE, &mut pump).await {
                    Ok(Ok(Ok(()))) => {}
                    Ok(Ok(Err(error))) => {
                        retirement = Err(format!("console pipe read failed: {error}"))
                    }
                    Ok(Err(error)) => retirement = Err(format!("console pump failed: {error}")),
                    Err(_) => {
                        pump.abort();
                        if let Err(error) = pump.await {
                            tracing::warn!(%error, "console pump joined after drain deadline");
                        }
                        retirement = Err("console pump did not drain; preserving lifecycle".into());
                    }
                }
            }
            // Every pump has joined (or was aborted and joined), so the console
            // owns the last writer clone: finish it before any receipt can reach
            // Remove or H1. A failed/unfinished log keeps the record uncertain.
            let finished = waited
                .console
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .finish();
            if let Err(error) = finished {
                retirement = Err(format!(
                    "console log did not finish; cleanup retained: {error}"
                ));
            }
            if let (Ok(Some(receipt)), Some(session)) = (&retirement, &waited.lifecycle) {
                if let Err(error) = session.finish_resources(receipt).await {
                    retirement = Err(format!("worker endpoint retirement failed: {error}"));
                }
            }
            if retirement.is_ok() {
                writer_admission.reaped();
            }
            *waited
                .retirement
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(retirement.and_then(|receipt| {
                receipt.ok_or_else(|| "fixture has no production retirement receipt".into())
            }));
            drop(_image_reads);
            waited.ended.send_replace(Some(Ended {
                exit_code: status.and_then(|s| s.code()),
                signal: status.and_then(|s| s.signal()),
                at_ms: now_ms(),
            }));
        });
        // Any actual later writer invalidates a build receipt. The core also
        // verifies this; never prefer an older successful build over this slot.
        self.retired_builds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        slots.insert(id.clone(), Slot::Booted(vm));
        if let Some(error) = startup_error {
            return Err(SandboxError::BootFailed(error));
        }
        Ok(WorkerStartResponse {
            vm_id: id,
            instance,
        })
    }

    /// S504 for an unknown `vm_id`, S503 for another instance.
    pub fn status(&self, req: WorkerStatusRequest) -> Result<WorkerStatus, SandboxError> {
        self.authorize_vm(&req.auth, &req.vm_id)?;
        Ok(self.lookup(&req.vm_id, req.instance)?.status(&req.vm_id))
    }

    /// SIGTERM, then SIGKILL after `grace_ms`; `grace_ms: 0` kills at once.
    /// Returns the final status; an exited VM returns it as is, and one still
    /// preparing is cancelled before it boots, once its build has ended.
    pub async fn stop(&self, req: WorkerStopRequest) -> Result<WorkerStatus, SandboxError> {
        self.authorize_vm(&req.auth, &req.vm_id)?;
        // A boot still preparing is cancelled: its `start` never launches, and
        // this returns once that build has ended, so neither the start retried
        // after it nor a `destroy` writes the VM's state alongside it.
        let cancelled = match self.slots().get_mut(&req.vm_id) {
            Some(Slot::Preparing(i, start, cancelled)) if *i == req.instance => {
                *cancelled = true;
                Some(*start)
            }
            _ => None,
        };
        if let Some(start) = cancelled {
            loop {
                let unreserved = self.unreserved.notified();
                if !matches!(self.slots().get(&req.vm_id), Some(Slot::Preparing(_, s, _)) if *s == start)
                {
                    break;
                }
                unreserved.await;
            }
            return Ok(WorkerStatus {
                vm_id: req.vm_id,
                instance: req.instance,
                state: State::Stopped,
                exit_code: None,
                signal: None,
                started_at_ms: 0,
                ended_at_ms: Some(now_ms()),
            });
        }
        let vm = booted(&req.vm_id, self.lookup(&req.vm_id, req.instance)?)?;
        vm.stop(Duration::from_millis(req.grace_ms))
            .await
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        Ok(Slot::Booted(vm).status(&req.vm_id))
    }

    /// Stream the VM's `console.log` into `req.channel` and return; a task
    /// pumps the records and closes the channel. S504 for an unknown `vm_id`.
    pub async fn logs(
        self: &Arc<Self>,
        iii: &IIIClient,
        mut req: WorkerLogsRequest,
    ) -> Result<WorkerLogsResponse, SandboxError> {
        self.authorize_vm(&req.auth, &req.vm_id)?;
        // Checked here: an error inside the task would reach no caller.
        let vm = booted(&req.vm_id, self.slot(&req.vm_id)?)?;
        let mut reply = WorkerLogsResponse::default();
        if req.from_end {
            let end = vm
                .console
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .end();
            req.cursor = Some(end);
            reply.cursor = (end > 0).then_some(end);
        }
        let probed = req.follow && vm.leased.load(Ordering::SeqCst);
        let every = self.lease_grace / LEASE_PROBES;
        let writer = ChannelWriter::new(iii.address(), &req.channel);
        let vms = self.clone();
        tokio::spawn(async move {
            // Connected now, not at the first record. When this adapter goes away, the engine
            // aborts a connected writer's stream, and that is how a supervisor learns its VM
            // went with it. An unattached writer would leave the reader open until the engine
            // sweeps the channel, minutes later. Bounded: `pump_logs` registers the follower a
            // lease waits for, so a stalled handshake must not hold it back past the grace. A
            // connect cut short here happens at the first record instead.
            let _ = tokio::time::timeout(every, writer.write(&[])).await;
            let channel = &writer;
            let pumped = tokio::select! {
                pumped = vms.pump_logs(&req, |record| async move {
                    let frame = serde_json::to_string(&record).map_err(|e| e.to_string())?;
                    channel
                        .send_message(&frame)
                        .await
                        .map_err(|e| e.to_string())
                }) => pumped,
                gone = reader_gone(channel, every), if probed => {
                    Err(SandboxError::FsChannelAborted(gone))
                }
            };
            if let Err(e) = pumped {
                tracing::info!(error = %e, vm_id = %req.vm_id, "worker log stream ended early");
            }
            // Connects even if nothing was sent, so the reader still sees the close.
            let _ = writer.write(&[]).await;
            let _ = writer.close().await;
        });
        Ok(reply)
    }

    /// Feed `sink` the instance's records from `req.cursor`. With `follow` it
    /// polls until the instance has exited and its log is drained; a sink error
    /// ends it too (S218). A `follow` holds the VM's lease for as long as it runs.
    pub(crate) async fn pump_logs<F, Fut>(
        &self,
        req: &WorkerLogsRequest,
        mut sink: F,
    ) -> Result<(), SandboxError>
    where
        F: FnMut(LogRecord) -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        // Looked up before the join: only ids `vm_id()` made are in `slots`.
        let vm = booted(&req.vm_id, self.slot(&req.vm_id)?)?;
        let _follower = req.follow.then(|| Follower::new(vm.clone()));
        let path = self.base.join(&req.vm_id).join(CONSOLE_LOG);
        let mut ended = vm.ended.subscribe();
        let mut cursor = req.cursor.unwrap_or(0);
        loop {
            // Sampled before the read: the waiter records the exit after the
            // log is complete, so a read that follows it is the last one.
            let exited = ended.borrow().is_some();
            // shortcut: a blocking read, bounded by twice `CONSOLE_LOG_CAP`;
            // move it to `spawn_blocking` if the cap grows.
            match read_records(&vm.console, cursor) {
                Ok((records, next)) => {
                    cursor = next;
                    for record in records {
                        sink(record).await.map_err(SandboxError::FsChannelAborted)?;
                    }
                }
                // `destroy` removed the dir of an exited VM: nothing left to read.
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(SandboxError::FsIo(format!("{}: {e}", path.display()))),
            }
            if exited || !req.follow {
                return Ok(());
            }
            let _ = tokio::time::timeout(FOLLOW_POLL, ended.wait_for(Option::is_some)).await;
        }
    }

    /// Remove `state_key`'s VM state. With `prune`, all of it; without, all but
    /// the VM's data ([`remove_runtime_state`]), which a later start of the same
    /// `state_key` boots over again; its build directory is free either way.
    /// `destroyed` says whether anything went (runtime state, without `prune`);
    /// `kept` lists the data a destroy without `prune` left. S502 while it is preparing, running or
    /// being destroyed. It blocks for the removal: call it off the async runtime.
    pub fn destroy(
        &self,
        req: WorkerDestroyRequest,
    ) -> Result<WorkerDestroyResponse, SandboxError> {
        self.authorize(&req.auth, Some(&req.state_key))?;
        let id = vm_id(&req.state_key);
        let dir = self.base.join(&id);
        // The marker, not the lock, keeps a `start` out of a half-removed dir:
        // a legacy rootfs clone takes seconds to remove.
        let (prev, last, admission, receipt) = {
            let mut slots = self.slots();
            refuse_busy(&id, slots.get(&id))?;
            // The build writes in `<vm_id>/`, so it must end first.
            if self.builds().contains(&id) {
                return Err(SandboxError::BuildInProgress(id));
            }
            let (admission, receipt, instance) = match slots.get(&id) {
                Some(Slot::Booted(vm)) => (
                    vm.admission.clone(),
                    vm.receipt()
                        .map_err(|e| SandboxError::FsIo(e.to_string()))?,
                    vm.instance,
                ),
                _ => {
                    let receipt = self.retired_builds.lock().unwrap_or_else(PoisonError::into_inner).get(&id).cloned()
                        .ok_or_else(|| SandboxError::FsIo("destroy has no original lifecycle retirement proof; retain for explicit recovery".into()))?;
                    let admission = self
                        .operations
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&req.state_key)
                        .cloned()
                        .ok_or_else(|| {
                            SandboxError::FsIo("retired build lost its original operation".into())
                        })?;
                    (admission, receipt, 0)
                }
            };
            // A daemon restarted since the last boot knows it only from disk.
            let last = instance.max(stored_instance(&dir).unwrap_or(0));
            (
                slots.insert(id.clone(), Slot::Destroying(instance)),
                last,
                admission,
                receipt,
            )
        };
        let removed = remove_runtime_state_admitted(&admission, &receipt, &dir, &id, req.prune);
        let mut slots = self.slots();
        slots.remove(&id);
        // Even on failure: the removal may have reached `state.json` already.
        if last > 0 {
            self.destroyed().insert(id.clone(), last);
        }
        let (destroyed, kept) = match removed {
            Ok((removed, kept)) => (removed, kept),
            Err(e) => {
                // Its state is still there, so keep reporting it.
                if let Some(prev) = prev {
                    slots.insert(id, prev);
                }
                return Err(SandboxError::FsIo(format!(
                    "cannot remove {}: {e}",
                    dir.display()
                )));
            }
        };
        // Keep the same lifecycle proof for same-process idempotent cleanup;
        // the core, not absence alone, decides if a consumed receipt permits it.
        if let Some(previous) = prev {
            slots.insert(id.clone(), previous);
        }
        let kept: Vec<String> = kept.iter().map(|p| p.display().to_string()).collect();
        for path in &kept {
            tracing::info!(vm_id = %id, path = %path, "worker::destroy kept the VM's data");
        }
        Ok(WorkerDestroyResponse { destroyed, kept })
    }

    /// Leave `vm_id`'s VM to its lease, from now on. Nothing is told when a client
    /// dies before it opens the logs, so the grace starts here.
    fn arm_lease(&self, vm_id: &str) {
        if let Some(Slot::Booted(vm)) = self.slots().get(vm_id) {
            vm.leased.store(true, Ordering::SeqCst);
            tokio::spawn(lease(vm.clone(), vm_id.to_string(), self.lease_grace));
        }
    }

    /// The instance the next boot of `vm_id` gets. Call it holding `slots`.
    fn next_instance(&self, vm_id: &str) -> Result<u64, SandboxError> {
        let floor = self.destroyed().get(vm_id).copied().unwrap_or(0);
        Ok(stored_instance(&self.base.join(vm_id))?.max(floor) + 1)
    }

    /// Taken only while `slots` is held, so the two locks cannot deadlock.
    fn destroyed(&self) -> MutexGuard<'_, HashMap<String, u64>> {
        self.destroyed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// `vm_id`'s entry: S504 when there is none, S503 when it is on another instance.
    fn lookup(&self, vm_id: &str, instance: u64) -> Result<Slot, SandboxError> {
        let slot = self.slot(vm_id)?;
        if slot.instance() != instance {
            return Err(SandboxError::InstanceMismatch {
                vm_id: vm_id.to_string(),
                expected: instance,
                found: slot.instance(),
            });
        }
        Ok(slot)
    }

    /// `vm_id`'s entry, whatever its instance: S504 when there is none.
    fn slot(&self, vm_id: &str) -> Result<Slot, SandboxError> {
        match self.slots().get(vm_id) {
            // Being destroyed reads as already destroyed.
            None | Some(Slot::Destroying(_)) => Err(SandboxError::VmNotFound(vm_id.to_string())),
            Some(slot) => Ok(slot.clone()),
        }
    }

    fn slots(&self) -> MutexGuard<'_, HashMap<String, Slot>> {
        // Every write under the lock is one insert or remove, so a poisoned map is consistent.
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Constant-time token check; no token (or an empty one) is never matched.
    fn is_project_token(&self, auth: &str) -> bool {
        let token = self.token.as_deref().unwrap_or("");
        let diff = token
            .bytes()
            .zip(auth.bytes())
            .fold(0, |d, (a, b)| d | (a ^ b));
        !token.is_empty() && token.len() == auth.len() && diff == 0
    }

    /// `auth` must be the project token or, for a call that names one, `state_key`'s scoped token.
    fn authorize(&self, auth: &str, state_key: Option<&str>) -> Result<(), SandboxError> {
        let token = self.token.as_deref().unwrap_or("");
        if self.is_project_token(auth)
            || state_key.is_some_and(|key| scoped_auth_matches(token, key, auth))
        {
            return Ok(());
        }
        Err(SandboxError::Unauthorized)
    }

    /// As `authorize`, for a call that names its VM by `vm_id`: a scoped token
    /// needs that VM's record of its state key.
    fn authorize_vm(&self, auth: &str, vm_id: &str) -> Result<(), SandboxError> {
        if self.is_project_token(auth) {
            return Ok(());
        }
        self.authorize(auth, stored_state_key(&self.base, vm_id).as_deref())
    }

    /// `host_path`, canonicalized, if it lies under a canonicalized root.
    fn admit(&self, host_path: impl AsRef<Path>) -> Result<PathBuf, SandboxError> {
        let host_path = host_path.as_ref();
        if let Ok(path) = host_path.canonicalize()
            && self
                .roots
                .iter()
                .filter_map(|root| root.canonicalize().ok())
                .any(|root| path.starts_with(root))
        {
            return Ok(path);
        }
        if self.roots.is_empty() {
            return Err(SandboxError::SourceRejected(format!(
                "no host roots are configured (III_SANDBOX_HOST_ROOTS is empty); \
                 refusing host_path {}",
                host_path.display()
            )));
        }
        let roots: Vec<_> = self.roots.iter().map(|r| r.display().to_string()).collect();
        let roots = roots.join(", ");
        if let Err(e) = host_path.canonicalize() {
            return Err(SandboxError::SourceRejected(format!(
                "host_path {} cannot be read ({e}); it must exist under the host roots \
                 [{roots}] (III_SANDBOX_HOST_ROOTS)",
                host_path.display()
            )));
        }
        Err(SandboxError::SourceRejected(format!(
            "host_path {} is outside the host roots [{roots}] (III_SANDBOX_HOST_ROOTS)",
            host_path.display()
        )))
    }
}

/// Holds a `vm_id` for one build; frees it when dropped, the build cancelled or not.
struct BuildHold<'a>(&'a WorkerVms, String);

impl Drop for BuildHold<'_> {
    fn drop(&mut self) {
        self.0.builds().remove(&self.1);
    }
}

/// One `worker::build`, from its build VM to the renamed image.
struct BuildJob<'a> {
    worker_name: &'a str,
    admission: crate::worker_admission::Lease,
    output_admission: crate::worker_admission::Lease,
    dir: &'a Path,
    resolved: WorkerProject,
    package: Option<&'a str>,
    out: &'a Path,
    scratch: &'a Path,
    /// Names the temporary image files: `<pid>-<seq>`.
    tag: &'a str,
    env: HashMap<String, String>,
    state: Arc<AtomicU8>,
    /// Where the blocking image writer runs, so a cancelled build can wait for it.
    image_writer: &'a WriterSlot,
    /// What the build did, for a caller that dropped it after the commit.
    done: &'a Mutex<BuildDone>,
}

/// What a build did before it may have been dropped: its base image once its
/// VM is set up, and its reply once the image is in place.
#[derive(Default)]
struct BuildDone {
    base_image: Option<BaseImageRef>,
    response: Option<WorkerBuildResponse>,
    retirement: Option<crate::worker_admission::WriterRetirement>,
}

impl BuildJob<'_> {
    async fn run(self, log: &mut BuildLog<'_>) -> Result<WorkerBuildResponse, SandboxError> {
        self.admission
            .revalidate()
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        let mut create = self
            .admission
            .begin(crate::worker_admission::MutationPlan::Reclone {
                root: self.scratch.to_path_buf(),
            })
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        let stage = create
            .stage_tree(self.scratch)
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        create
            .publish_tree(stage)
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        create
            .commit()
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        log.note(format!(
            "iii: building {} from {}",
            self.worker_name,
            self.dir.display()
        ))
        .await?;
        self.admission
            .revalidate()
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        let mut vm = self
            .resolved
            .build(
                self.dir,
                self.package,
                self.scratch,
                self.env,
                self.admission.clone(),
            )
            .await
            .map_err(SandboxError::BuildFailed)?;
        self.done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .base_image = Some(BaseImageRef {
            digest: crate::cli::rootfs_cache::cached_digest(&vm.reference),
            reference: vm.reference.clone(),
        });
        log.note(format!("iii: build VM on {}", vm.reference))
            .await?;
        // A dropped build future must not release source/output fences while
        // its VM may still write. Transfer the outer composite to WriterLease.
        vm.command
            .lifecycle_leases
            .extend([self.admission.clone(), self.output_admission.clone()]);
        let image_admissions = vm.command.lifecycle_leases.clone();
        let (status, receipt) = run_build_vm(vm.command, log).await?;
        self.done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retirement = Some(receipt);
        if !status.success() {
            let how = match (status.code(), status.signal()) {
                (Some(code), _) => format!("exited {code}"),
                (None, Some(signal)) => format!("was killed by signal {signal}"),
                _ => "ended".to_string(),
            };
            return Err(SandboxError::BuildFailed(format!(
                "the build of {} {how}; last output:\n{}",
                self.worker_name,
                log.tail()
            )));
        }
        log.note("iii: writing the image".to_string()).await?;
        for lease in &image_admissions {
            lease
                .revalidate()
                .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        }
        let export = open_build_export(&vm.root).map_err(|e| {
            SandboxError::BuildFailed(format!("the build VM left no image root: {e}"))
        })?;
        let image = ImageWrite {
            _admissions: image_admissions,
            export,
            out: self.out.to_path_buf(),
            spool: self.out.join("image.spool"),
            base_rootfs: vm.base_rootfs,
            tag: self.tag.to_string(),
            state: self.state.clone(),
        };
        let build_id = run_writer(self.image_writer, image).await.map_err(|e| {
            SandboxError::BuildFailed(format!(
                "cannot write the image of {} into {}: {e}",
                self.worker_name,
                self.out.display()
            ))
        })?;
        // The image is in place: the build is done whatever happens to `logs` now.
        let response = WorkerBuildResponse {
            build_id,
            base_image: self
                .done
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .base_image
                .clone()
                .expect("set once the build VM is"),
            built_at_ms: now_ms(),
        };
        self.done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .response = Some(response.clone());
        let _ = log
            .note(format!(
                "iii: built {} into {}",
                response.build_id,
                self.out.join(IMAGE_FILE).display()
            ))
            .await;
        Ok(response)
    }
}

/// The blocking image writer of a build. Awaited where the build runs, and,
/// once the build is cancelled, before its key is freed.
type WriterSlot = tokio::sync::Mutex<Option<tokio::task::JoinHandle<Result<String, String>>>>;

/// Run `image.write()` off the runtime, its handle in `slot` until it ends.
async fn run_writer(slot: &WriterSlot, image: ImageWrite) -> Result<String, String> {
    let mut slot = slot.lock().await;
    let handle = slot.insert(tokio::task::spawn_blocking(move || image.write()));
    let written = handle.await;
    // No await between the end and here: a dropped caller leaves the handle.
    *slot = None;
    written.map_err(|e| format!("the image writer died: {e}"))?
}

/// Mark the build cancelled and wait for its writer, if one runs, to stop and
/// remove its temporary files. Returns what that writer returned: the build id
/// when it renamed the image into place before the cancel.
async fn cancel_writer(slot: &WriterSlot, state: &AtomicU8) -> Option<Result<String, String>> {
    let _ = state.compare_exchange(
        BUILD_RUNNING,
        BUILD_CANCELLED,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
    let mut slot = slot.lock().await;
    let handle = slot.as_mut()?;
    let written = handle.await;
    *slot = None;
    Some(
        written
            .map_err(|e| format!("the image writer died: {e}"))
            .and_then(|w| w),
    )
}

/// Preserve temporary image evidence left by another or this process. There
/// is deliberately no PID-based removal authority in this compatibility hook.
fn sweep_image_partials(out: &Path) {
    tracing::warn!(path=%out.display(), "partial images retained: PID/name is not disposal authority");
}

/// The pid of a `<pid>-<seq>` build tag.
fn pid_seq(tag: &str) -> Option<&str> {
    let (pid, seq) = tag.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(pid) && digits(seq)).then_some(pid)
}

/// Spawn a build VM and stream its output. Cancellation retains the owned
/// supervisor; lifeline policy and core control, never Tokio raw-PID Drop,
/// determine process termination. No receipt is fabricated on cancellation.
async fn run_build_vm(
    vm: crate::cli::worker_manager::libkrun::VmCommand,
    log: &mut BuildLog<'_>,
) -> Result<
    (
        std::process::ExitStatus,
        crate::worker_admission::WriterRetirement,
    ),
    SandboxError,
> {
    // These local preparation guards supplement the outer worker build guard.
    // Cancellation may drop this future before Child::wait: retain their shared
    // leases conservatively unless successful wait proves the writer is gone.
    let runtime_endpoints = vm.runtime_endpoints;
    let lifecycle_leases = vm.lifecycle_leases;
    for lease in &lifecycle_leases {
        lease
            .revalidate()
            .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
    }
    let mut cmd = vm.command;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let admission = lifecycle_leases
        .first()
        .ok_or_else(|| SandboxError::BuildFailed("build has no runtime admission".into()))?;
    let paths: Vec<_> = vm
        .runtime_paths
        .iter()
        .map(|(path, kind)| crate::worker_admission::RuntimePath { path, kind: *kind })
        .collect();
    let launch = admission
        .prepare_runtime(&paths)
        .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
    let _lifeline = crate::daemon_exit::attach_lifeline(&mut cmd)
        .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
    let mut child = match launch.spawn(cmd) {
        Ok(child) => child,
        Err(error) => {
            let (cause, child) = error.into_parts();
            if let Some(mut child) = child {
                drop(child.take_stdin());
                let stdout = child.take_stdout();
                let stderr = child.take_stderr();
                let session = crate::sandbox_daemon::adapters::RuntimeSession::supervise(
                    child,
                    admission.clone(),
                    runtime_endpoints.clone(),
                );
                tokio::spawn(async move {
                    if let Some(mut stdout) = stdout {
                        if let Err(error) =
                            tokio::io::copy(&mut stdout, &mut tokio::io::sink()).await
                        {
                            tracing::warn!(%error, "failed-spawn stdout drain");
                        }
                    }
                });
                tokio::spawn(async move {
                    if let Some(mut stderr) = stderr {
                        if let Err(error) =
                            tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await
                        {
                            tracing::warn!(%error, "failed-spawn stderr drain");
                        }
                    }
                });
                tokio::spawn(async move {
                    let (_, result) = session.wait().await;
                    if let Err(error) = result {
                        tracing::error!(%error, "failed build spawn retained writer");
                    }
                });
            }
            return Err(SandboxError::BuildFailed(format!(
                "cannot launch build: {cause}"
            )));
        }
    };
    let stdin = child.take_stdin();
    let stdout = child.take_stdout().expect("stdout piped");
    let stderr = child.take_stderr().expect("stderr piped");
    let session = crate::sandbox_daemon::adapters::RuntimeSession::supervise(
        child,
        admission.clone(),
        runtime_endpoints,
    );
    let lifecycle_leases = crate::worker_admission::WriterLease::new(lifecycle_leases);
    let env_stdin = vm.env_stdin;
    let feed = tokio::spawn(async move {
        crate::cli::worker_manager::libkrun::feed_env_stdin(stdin, &env_stdin).await
    });
    let (lines, mut received) = tokio::sync::mpsc::unbounded_channel();
    let pumps = [
        tokio::spawn(forward_lines(stdout, LogStream::Stdout, lines.clone())),
        tokio::spawn(forward_lines(stderr, LogStream::Stderr, lines)),
    ];
    let waited = session.wait();
    tokio::pin!(waited);
    let (status, receipt) = loop {
        tokio::select! {
            Some((stream, line)) = received.recv() => log.record(stream, line).await?,
            completed = &mut waited => {
                let (status, retirement) = completed;
                let receipt = retirement.map_err(|e| SandboxError::BuildFailed(format!("build writer retirement: {e}")))?;
                break (status.ok_or_else(|| SandboxError::BuildFailed("build has no OS exit status".into()))?, receipt);
            },
        }
    };
    feed.await
        .map_err(|e| SandboxError::BuildFailed(format!("build env task: {e}")))?
        .map_err(|e| SandboxError::BuildFailed(format!("build env: {e}")))?;
    // What the VM printed last, unless a grandchild holds the pipes open.
    tokio::time::timeout(DRAIN_GRACE, async {
        while let Some((stream, line)) = received.recv().await {
            log.record(stream, line).await?;
        }
        Ok::<_, SandboxError>(())
    })
    .await
    .map_err(|_| {
        SandboxError::BuildFailed("build output writers did not drain; retaining evidence".into())
    })??;
    for pump in pumps {
        pump.await
            .map_err(|e| SandboxError::BuildFailed(format!("build pipe task: {e}")))?
            .map_err(|e| SandboxError::BuildFailed(format!("build pipe read: {e}")))?;
    }
    session
        .finish_resources(&receipt)
        .await
        .map_err(|e| SandboxError::BuildFailed(format!("build endpoint retirement: {e}")))?;
    lifecycle_leases.reaped();
    Ok((status, receipt))
}

/// Send one record per line of `out` to `lines`, as `pump` does for a VM's console.
async fn forward_lines(
    out: impl AsyncRead + Unpin,
    stream: LogStream,
    lines: tokio::sync::mpsc::UnboundedSender<(LogStream, String)>,
) -> io::Result<()> {
    let mut out = BufReader::new(out);
    let mut line = Vec::new();
    loop {
        line.clear();
        match (&mut out).take(MAX_LINE).read_until(b'\n', &mut line).await {
            Ok(0) => return Ok(()),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        let text = line.strip_suffix(b"\n").unwrap_or(&line);
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        let text = String::from_utf8_lossy(text).into_owned();
        if lines.send((stream.clone(), text)).is_err() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "build log receiver closed",
            ));
        }
    }
}

/// A build's `logs` channel: one `LogRecord` per text frame, and the last lines
/// for an S507.
struct BuildLog<'a> {
    channel: &'a ChannelWriter,
    cursor: u64,
    tail: VecDeque<String>,
}

impl<'a> BuildLog<'a> {
    fn new(channel: &'a ChannelWriter) -> Self {
        Self {
            channel,
            cursor: 0,
            tail: VecDeque::new(),
        }
    }

    /// One record; a write that fails ends the build.
    async fn record(&mut self, stream: LogStream, line: String) -> Result<(), SandboxError> {
        self.cursor += 1;
        if self.tail.len() == BUILD_TAIL_LINES {
            self.tail.pop_front();
        }
        self.tail.push_back(line.clone());
        let frame = serde_json::to_string(&LogRecord {
            stream,
            line,
            ts_ms: now_ms(),
            cursor: self.cursor,
        })
        .map_err(|e| SandboxError::BuildFailed(e.to_string()))?;
        self.channel.send_message(&frame).await.map_err(|e| {
            SandboxError::BuildFailed(format!(
                "build cancelled: its output cannot reach `logs` ({e})"
            ))
        })
    }

    /// A line of the adapter's own, on stderr.
    async fn note(&mut self, line: String) -> Result<(), SandboxError> {
        self.record(LogStream::Stderr, line).await
    }

    fn tail(&self) -> String {
        self.tail.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

/// The blocking part of a build: the exported root into an image in `out`.
struct ImageWrite {
    /// Survives a dropped async caller until the blocking writer has stopped.
    _admissions: Vec<crate::worker_admission::Lease>,
    /// The tar the build VM exported.
    export: File,
    out: PathBuf,
    spool: PathBuf,
    base_rootfs: PathBuf,
    tag: String,
    state: Arc<AtomicU8>,
}

impl ImageWrite {
    /// Write `image.erofs` and `image.id` as temporary files, then put them in
    /// place: the previous `image.id` goes, the image is renamed in, and its id
    /// last ([`swap_image`], [`commit_image_id`]). A build cancelled before that
    /// leaves the previous pair; returns the build id.
    fn write(self) -> Result<String, String> {
        if !self._admissions.is_empty() {
            return self.write_admitted();
        }
        #[cfg(test)]
        {
            return self.write_legacy();
        }
        #[cfg(not(test))]
        Err("image writer requires an admitted operation".into())
    }

    fn write_admitted(self) -> Result<String, String> {
        let admission = &self._admissions[0];
        let base = admission
            .use_path(&self.base_rootfs)
            .map_err(|e| e.to_string())?;
        let mutation = crate::cli::rootfs_cache::MutationWriter::begin(
            admission,
            crate::worker_admission::MutationPlan::ImagePair {
                directory: self.out.clone(),
            },
        )
        .map_err(|e| e.to_string())?;
        let mut image = mutation
            .apply(|g| g.stage_file(&self.out.join(IMAGE_FILE), 0o600))
            .map_err(|e| format!("{e:#}"))?;
        let mut id = mutation
            .apply(|g| g.stage_file(&self.out.join(IMAGE_ID_FILE), 0o600))
            .map_err(|e| format!("{e:#}"))?;
        let mut spool = mutation
            .apply(|g| g.stage_file(&self.spool, 0o600))
            .map_err(|e| format!("{e:#}"))?;
        let config = oci::read_rootfs_file_no_follow(base.path(), Path::new(".oci-config.json"));
        let extra: Vec<_> = config
            .ok()
            .map(|data| crate::cli::erofs::ImageFile {
                path: ".oci-config.json",
                data,
                mode: 0o644,
            })
            .into_iter()
            .collect();
        let cancelled = || self.state.load(Ordering::SeqCst) == BUILD_CANCELLED;
        crate::cli::erofs::build_erofs_from_tar_to_files(
            self.export,
            image.file_mut().try_clone().map_err(|e| e.to_string())?,
            spool.file_mut().try_clone().map_err(|e| e.to_string())?,
            &extra,
            &cancelled,
        )?;
        mutation.checkpoint.check().map_err(|e| e.to_string())?;
        image
            .file_mut()
            .seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut bytes = [0u8; 65536];
        loop {
            if cancelled() {
                return Err("cancelled".into());
            }
            let read = image
                .file_mut()
                .read(&mut bytes)
                .map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            hash.update(&bytes[..read]);
        }
        let build_id = format!("sha256:{}", hex::encode(hash.finalize()));
        id.file_mut()
            .write_all(format!("{build_id}\n").as_bytes())
            .map_err(|e| e.to_string())?;
        image.file_mut().sync_all().map_err(|e| e.to_string())?;
        id.file_mut().sync_all().map_err(|e| e.to_string())?;
        mutation
            .apply(|g| g.discard_file(spool))
            .map_err(|e| format!("{e:#}"))?;
        self.state
            .compare_exchange(
                BUILD_RUNNING,
                BUILD_COMMITTED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .map_err(|_| "cancelled".to_string())?;
        mutation
            .apply(|g| g.publish_file(image))
            .map_err(|e| format!("{e:#}"))?;
        mutation
            .apply(|g| g.publish_file(id))
            .map_err(|e| format!("{e:#}"))?;
        mutation.commit().map_err(|e| format!("{e:#}"))?;
        Ok(build_id)
    }

    #[cfg(test)]
    fn write_legacy(self) -> Result<String, String> {
        for lease in &self._admissions {
            lease.revalidate().map_err(|e| e.to_string())?;
        }
        let tmp_image = self.out.join(format!(".{IMAGE_FILE}.{}.partial", self.tag));
        let tmp_id = self
            .out
            .join(format!(".{IMAGE_ID_FILE}.{}.partial", self.tag));
        let _tmp = [
            RemoveOnDrop(tmp_image.clone()),
            RemoveOnDrop(tmp_id.clone()),
        ];
        // The run reads the base image's env from here: the host's copy, not
        // whatever the build left at that path.
        let config =
            oci::read_rootfs_file_no_follow(&self.base_rootfs, Path::new(".oci-config.json"));
        let extra: Vec<crate::cli::erofs::ImageFile<'_>> = config
            .ok()
            .map(|data| crate::cli::erofs::ImageFile {
                path: ".oci-config.json",
                data,
                mode: 0o644,
            })
            .into_iter()
            .collect();
        let state = &self.state;
        let cancelled = || state.load(Ordering::SeqCst) == BUILD_CANCELLED;
        crate::cli::erofs::build_erofs_from_tar(
            self.export,
            &tmp_image,
            &self.spool,
            &extra,
            &cancelled,
        )?;
        // The image's bytes reach the disk before a rename can publish them.
        File::open(&tmp_image)
            .and_then(|image| image.sync_all())
            .map_err(|e| format!("flush the image: {e}"))?;
        let build_id =
            sha256_file(&tmp_image, &cancelled).map_err(|e| format!("hash the image: {e}"))?;
        write_new_file(&tmp_id, format!("{build_id}\n").as_bytes())
            .map_err(|e| format!("{}: {e}", tmp_id.display()))?;
        // Past this point the build is no longer cancelled.
        if state
            .compare_exchange(
                BUILD_RUNNING,
                BUILD_COMMITTED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            return Err("cancelled".to_string());
        }
        swap_image(&self.out, &tmp_image)?;
        // A crash here leaves the new image without an id: S508 until a rebuild.
        commit_image_id(&self.out, &tmp_id)?;
        Ok(build_id)
    }
}

/// The first of the two steps that put a written image in place: remove
/// `out/image.id`, flush that removal to disk, rename `image` to
/// `out/image.erofs`, and flush that too. Until [`commit_image_id`] renames the
/// new id in, `out` holds an image and no id, which a start refuses with S508: a
/// crash, a failed flush or a failed rename between the two steps leaves a
/// rebuild to do, never the new image under the previous build's id. One
/// writer per `output_dir` at a time is assumed: two builds of one VM are S509,
/// and nothing serializes two VMs sharing a directory (README, Known limits).
fn swap_image(out: &Path, image: &Path) -> Result<(), String> {
    match std::fs::remove_file(out.join(IMAGE_ID_FILE)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            return Err(format!("remove the previous image.id: {e}"));
        }
        _ => {}
    }
    sync_dir(out).map_err(|e| format!("flush {}: {e}", out.display()))?;
    std::fs::rename(image, out.join(IMAGE_FILE))
        .map_err(|e| format!("rename the image into place: {e}"))?;
    // So the id's rename can never reach the disk without the image's.
    sync_dir(out).map_err(|e| format!("flush {}: {e}", out.display()))
}

/// The second step after [`swap_image`]: rename the image's id into place. The
/// build is done then, so a failed last flush is not an error.
fn commit_image_id(out: &Path, id: &Path) -> Result<(), String> {
    std::fs::rename(id, out.join(IMAGE_ID_FILE))
        .map_err(|e| format!("rename image.id into place: {e}"))?;
    let _ = sync_dir(out);
    Ok(())
}

/// Flush the entries of `dir` (a create, rename or removal in it) to disk. A
/// filesystem that cannot flush a directory ([`flush_unsupported`]) gets a
/// warning, not an error, as 0.4.0 did: the order of the operations still
/// holds against a crash of this process, only not against a power failure.
fn sync_dir(dir: &Path) -> io::Result<()> {
    match File::open(dir)?.sync_all() {
        Err(e) if flush_unsupported(&e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "this filesystem cannot flush a directory; going on without");
            Ok(())
        }
        flushed => flushed,
    }
}

/// EINVAL, ENOTSUP (EOPNOTSUPP) and EBADF from a directory's fsync: what
/// filesystems that do not flush directories answer (some network and
/// FAT-family volumes; `F_FULLFSYNC` on macOS).
fn flush_unsupported(e: &io::Error) -> bool {
    e.raw_os_error().is_some_and(|code| {
        [libc::EINVAL, libc::ENOTSUP, libc::EOPNOTSUPP, libc::EBADF].contains(&code)
    })
}

/// `sha256:<hex>` of a file's bytes; an error once `cancelled` returns true.
fn sha256_file(path: &Path, cancelled: &dyn Fn() -> bool) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancelled() {
            return Err(io::Error::other("cancelled"));
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

/// Create `path` (it must not exist), write `data` and flush it to disk.
fn write_new_file(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(data)?;
    file.sync_all()
}

/// The build id `dir/image.id` records, trimmed; `None` when it is missing,
/// not a regular file, or not a short line of text.
fn read_image_id(dir: &Path) -> Option<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(dir.join(IMAGE_ID_FILE))
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut id = String::new();
    file.take(256).read_to_string(&mut id).ok()?;
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// Removes a file or a directory tree when dropped, if it is still there.
struct RemoveOnDrop(PathBuf);

/// Remove `path` now, a directory tree or a file (a symlink itself, never its
/// target); a path already gone is no error.
fn remove_entry(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        match std::fs::symlink_metadata(&self.0) {
            Ok(m) if m.is_dir() => {
                let _ = std::fs::remove_dir_all(&self.0);
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&self.0);
            }
            Err(_) => {}
        }
    }
}

/// Preserve unresolved build scratch. Only stage disposal or a current exact
/// cleanup receipt may remove it; a filename/PID is diagnostic evidence only.
fn sweep_build_scratch(vm_dir: &Path) {
    tracing::warn!(path=%vm_dir.display(), "scratch retained: only owned stage discard or recovery may remove it");
}

/// Make `vm_dir` a VM that boots builds; returns its data disk and the lines to
/// log. The first time, marked by the seal `<vm_id>/build-mode`, it keeps what
/// an earlier adapter left: a 0.26 overlay disk `root/upper.ext4` moves to
/// `upper.ext4.pre-build`, and a legacy rootfs clone `root/` to
/// `root.pre-build/`, with a fresh `root/` in its place. The data disk is
/// `<vm_id>/upper.ext4`, outside `root/` (the guest's share), and outlives
/// every build; only `worker::destroy` removes it.
fn adopt_build_mode_admitted(
    lease: &crate::worker_admission::Lease,
    vm_dir: &Path,
    worker: &str,
) -> io::Result<(PathBuf, Vec<String>)> {
    let mut guard = lease.begin(crate::worker_admission::MutationPlan::Adopt {
        root: vm_dir.to_path_buf(),
    })?;
    guard.create_dir_all(vm_dir, 0o700)?;
    let mut notes = Vec::new();
    let seal = vm_dir.join(BUILD_MODE_SEAL);
    if std::fs::symlink_metadata(&seal).is_err() {
        let root = vm_dir.join("root");
        let old_upper = root.join(crate::cli::upper::UPPER_NAME);
        if std::fs::symlink_metadata(&old_upper).is_ok_and(|m| m.is_file()) {
            let kept = unused_name(vm_dir, PRE_BUILD_DISK);
            guard.rename_preserving(&old_upper, &kept)?;
            notes.push(format!(
                "{worker}: kept the pre-build data disk at {}",
                kept.display()
            ));
        }
        if is_rootfs_clone(&root) {
            let kept = unused_name(vm_dir, PRE_BUILD_ROOT);
            guard.rename_preserving(&root, &kept)?;
            guard.create_dir_all(&root, 0o700)?;
            notes.push(format!(
                "{worker}: kept the pre-build data disk at {}",
                kept.display()
            ));
        }
        let mut stage = guard.stage_file(&seal, 0o600)?;
        stage.file_mut().write_all(b"1\n")?;
        guard.publish_file(stage)?;
    }
    guard.commit()?;
    let upper = crate::cli::upper::ensure_upper_admitted(lease, vm_dir)?;
    Ok((upper, notes))
}

#[cfg(test)]
fn adopt_build_mode(vm_dir: &Path, worker: &str) -> io::Result<(PathBuf, Vec<String>)> {
    std::fs::create_dir_all(vm_dir)?;
    let mut notes = Vec::new();
    let seal = vm_dir.join(BUILD_MODE_SEAL);
    if std::fs::symlink_metadata(&seal).is_err() {
        let (kept, root_moved) = keep_pre_build(vm_dir, worker)?;
        if root_moved {
            std::fs::create_dir(vm_dir.join("root"))?;
        }
        notes = kept;
        write_new_file(&seal, b"1\n")?;
    }
    let upper = crate::cli::upper::ensure_upper_ext4(vm_dir)?;
    Ok((upper, notes))
}

/// Move the data an earlier adapter kept in `vm_dir/root/` beside it: a 0.26
/// overlay disk `root/upper.ext4` to `upper.ext4.pre-build`, a legacy rootfs
/// clone `root/` to `root.pre-build/` (or the first free `.<n>` of each).
/// Returns the lines to log, and whether `root/` itself moved.
#[cfg(test)]
fn keep_pre_build(vm_dir: &Path, worker: &str) -> io::Result<(Vec<String>, bool)> {
    let mut notes = Vec::new();
    let root = vm_dir.join("root");
    let old_upper = root.join(crate::cli::upper::UPPER_NAME);
    if std::fs::symlink_metadata(&old_upper).is_ok_and(|m| m.is_file()) {
        let kept = unused_name(vm_dir, PRE_BUILD_DISK);
        std::fs::rename(&old_upper, &kept)?;
        notes.push(format!(
            "{worker}: kept the pre-build data disk at {}",
            kept.display()
        ));
    }
    let root_moved = is_rootfs_clone(&root);
    if root_moved {
        let kept = unused_name(vm_dir, PRE_BUILD_ROOT);
        std::fs::rename(&root, &kept)?;
        notes.push(format!(
            "{worker}: kept the pre-build data disk at {}",
            kept.display()
        ));
    }
    Ok((notes, root_moved))
}

/// What `worker::destroy` without `prune` removes from `vm_dir`: the VM's
/// runtime state (`root/`, the console log, `state.json`, the state-key file,
/// build scratch) and its `build-dir` record, which frees its build directory.
/// What stays is the VM's data, the data disk `upper.ext4` and the pre-build
/// copies (`upper.ext4.pre-build[.<n>]`, `root.pre-build[.<n>]`), with the seal
/// `build-mode`: a later start with `build` boots over the same data disk.
/// Every destroy first moves the disk of a start without `build`
/// (`root/upper.ext4`, a 0.26 disk among them) or a legacy rootfs clone
/// (`root/`) to the next free pre-build name, sealed VM or not, so no destroy
/// without `prune` ever deletes what a guest wrote; a later start without
/// `build` begins with a new disk. A VM without data goes whole. Returns
/// whether runtime state went (`false` for a VM that is gone, or holds data
/// only), and the data paths kept.
fn remove_runtime_state_admitted(
    lease: &crate::worker_admission::Lease,
    receipt: &crate::worker_admission::WriterRetirement,
    vm_dir: &Path,
    worker: &str,
    prune: bool,
) -> io::Result<(bool, Vec<PathBuf>)> {
    use crate::worker_admission::MutationPlan;
    match std::fs::symlink_metadata(vm_dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let mut absent = receipt.begin_remove(lease, vm_dir, &[vm_dir])?;
            absent.remove_path(vm_dir)?;
            absent.commit()?;
            return Ok((false, Vec::new()));
        }
        metadata => {
            metadata?;
        }
    }
    if prune {
        // Only the original authenticated request selects this exact VM tree.
        let mut removal = receipt.begin_remove(lease, vm_dir, &[vm_dir])?;
        removal.remove_path(vm_dir)?;
        removal.commit()?;
        return Ok((true, Vec::new()));
    }
    let seal = vm_dir.join(BUILD_MODE_SEAL);
    let root = vm_dir.join("root");
    let old_upper = root.join(crate::cli::upper::UPPER_NAME);
    let move_upper = std::fs::symlink_metadata(&old_upper).is_ok_and(|m| m.is_file());
    let move_root = is_rootfs_clone(&root);
    if move_upper || move_root {
        let mut retain = lease.begin(MutationPlan::Adopt {
            root: vm_dir.to_path_buf(),
        })?;
        if move_upper {
            let kept = unused_name(vm_dir, PRE_BUILD_DISK);
            retain.rename_preserving(&old_upper, &kept)?;
            tracing::info!(
                "{worker}: kept the pre-build data disk at {}",
                kept.display()
            );
        }
        if move_root {
            let kept = unused_name(vm_dir, PRE_BUILD_ROOT);
            retain.rename_preserving(&root, &kept)?;
            tracing::info!("{worker}: kept the pre-build root at {}", kept.display());
        }
        retain.commit()?;
    }
    let mut kept = Vec::new();
    let mut targets = Vec::new();
    for (index, entry) in std::fs::read_dir(vm_dir)?.enumerate() {
        if index >= 4096 {
            return Err(io::Error::other("VM control entries exceed removal bound"));
        }
        let entry = entry?;
        if is_vm_data(entry.file_name().to_str().unwrap_or("")) {
            kept.push(entry.path());
        } else if entry.file_name() != BUILD_MODE_SEAL {
            targets.push(entry.path());
        }
    }
    // Roots and control files were declared before runtime. Unknown leftover
    // evidence is retained; never broaden a receipt from an enumeration.
    let permitted = [
        "root",
        STATE_FILE,
        STATE_KEY_FILE,
        BUILD_DIR_RECORD,
        CONSOLE_LOG,
        CONSOLE_LOG_OLD,
    ];
    let (mut allowed, unknown): (Vec<_>, Vec<_>) = targets.into_iter().partition(|path| {
        path == vm_dir
            || path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| permitted.contains(&n))
    });
    kept.extend(unknown);
    if kept.is_empty() {
        allowed = vec![vm_dir.to_path_buf()];
    } else if std::fs::symlink_metadata(&seal).is_err() {
        write_control_admitted(lease, vm_dir, BUILD_MODE_SEAL, b"1\n")?;
    }
    let removed = !allowed.is_empty();
    if removed {
        allowed.sort();
        let targets = allowed;
        let exact: Vec<&Path> = targets.iter().map(PathBuf::as_path).collect();
        let mut removal = receipt.begin_remove(lease, vm_dir, &exact)?;
        for target in targets {
            removal.remove_path(&target)?;
        }
        removal.commit()?;
    }
    kept.sort();
    Ok((removed, kept))
}

#[cfg(test)]
fn remove_runtime_state(vm_dir: &Path, worker: &str) -> io::Result<(bool, Vec<PathBuf>)> {
    match std::fs::symlink_metadata(vm_dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((false, Vec::new())),
        checked => checked?,
    };
    let seal = vm_dir.join(BUILD_MODE_SEAL);
    let sealed = std::fs::symlink_metadata(&seal).is_ok();
    let mut removed = false;
    for note in keep_pre_build(vm_dir, worker)?.0 {
        tracing::info!("{note}");
    }
    let mut kept = Vec::new();
    for entry in std::fs::read_dir(vm_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().unwrap_or("");
        if is_vm_data(name) {
            kept.push(entry.path());
        } else if name != BUILD_MODE_SEAL {
            remove_entry(&entry.path())?;
            removed = true;
        }
    }
    if kept.is_empty() {
        std::fs::remove_dir_all(vm_dir)?;
        return Ok((true, kept));
    }
    if !sealed {
        // What moved out of `root/` is pre-build data now: no start moves it again.
        write_new_file(&seal, b"1\n")?;
    }
    kept.sort();
    Ok((removed, kept))
}

/// A name in `<vm_id>/` that holds the VM's data: `upper.ext4`, and the
/// pre-build copies `upper.ext4.pre-build[.<n>]` and `root.pre-build[.<n>]`.
fn is_vm_data(name: &str) -> bool {
    name == crate::cli::upper::UPPER_NAME
        || [PRE_BUILD_DISK, PRE_BUILD_ROOT].iter().any(|kept| {
            name == *kept
                || name
                    .strip_prefix(kept)
                    .and_then(|rest| rest.strip_prefix('.'))
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
}

/// `error`, followed by the paths a migration kept: a boot that fails after
/// `adopt_build_mode` must still tell where the old disk went, as no later
/// start will.
fn with_notes(error: String, notes: &[String]) -> String {
    if notes.is_empty() {
        return error;
    }
    format!("{error} ({})", notes.join("; "))
}

/// A legacy-mode `root/` is a whole rootfs; an overlay trampoline has no `/bin`
/// or `/usr` of its own.
fn is_rootfs_clone(root: &Path) -> bool {
    ["bin", "usr"]
        .iter()
        .any(|d| std::fs::symlink_metadata(root.join(d)).is_ok())
}

/// `dir/name`, or `dir/name.<n>` for the first `n` not taken.
fn unused_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    (1..)
        .map(|n| dir.join(format!("{name}.{n}")))
        .find(|p| std::fs::symlink_metadata(p).is_err())
        .expect("an unused name")
}

/// Stable per `state_key`: the first 32 hex chars of its SHA-256.
pub fn vm_id(state_key: &str) -> String {
    hex::encode(Sha256::digest(state_key))[..32].to_string()
}

/// A request's source. `package` is the manifest name of a bundle or a binary
/// package; `executable`, set for a binary package only, its program relative
/// to `host_path`.
#[derive(Clone, Copy)]
struct Source<'a> {
    host_path: &'a str,
    package: Option<&'a str>,
    run_override: Option<&'a str>,
    executable: Option<&'a str>,
}

impl<'a> Source<'a> {
    fn of(source: &'a WorkerSource) -> Self {
        match source {
            WorkerSource::Bundle { package, host_path } => Self {
                host_path,
                package: Some(package),
                run_override: None,
                executable: None,
            },
            WorkerSource::Local {
                host_path,
                run_override,
            } => Self {
                host_path,
                package: None,
                run_override: run_override.as_deref(),
                executable: None,
            },
            WorkerSource::Binary {
                package,
                host_path,
                executable,
            } => Self {
                host_path,
                package: Some(package),
                run_override: None,
                executable: Some(executable),
            },
        }
    }
}

/// The base image a build request names, trimmed: S001 when it is not a
/// plausible OCI image reference, or when a binary package's build names none.
fn build_base_image(base_image: Option<&str>, binary: bool) -> Result<Option<&str>, SandboxError> {
    match base_image.map(str::trim) {
        Some(image) if crate::cli::project::is_plausible_image_ref(image) => Ok(Some(image)),
        Some(image) => Err(SandboxError::InvalidRequest(format!(
            "base_image {image:?} is not a plausible OCI image reference"
        ))),
        None if binary => Err(SandboxError::InvalidRequest(
            "a binary source needs base_image: its package names no base image".to_string(),
        )),
        None => Ok(None),
    }
}

/// The VM of `slot`; S502 while there is no process yet, and Compose retries.
fn booted(vm_id: &str, slot: Slot) -> Result<Arc<Vm>, SandboxError> {
    match slot {
        Slot::Booted(vm) => Ok(vm),
        Slot::Preparing(instance, ..) | Slot::Destroying(instance) => {
            Err(SandboxError::StateBusy {
                vm_id: vm_id.to_string(),
                instance,
            })
        }
    }
}

/// S502 while `slot` is preparing or running.
fn refuse_busy(vm_id: &str, slot: Option<&Slot>) -> Result<(), SandboxError> {
    match slot {
        Some(slot) if slot.busy() => Err(SandboxError::StateBusy {
            vm_id: vm_id.to_string(),
            instance: slot.instance(),
        }),
        _ => Ok(()),
    }
}

/// One following `worker::logs` stream of a VM, for as long as it lives.
struct Follower(Arc<Vm>);

impl Follower {
    fn new(vm: Arc<Vm>) -> Self {
        vm.followers.send_modify(|n| *n += 1);
        Self(vm)
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        self.0.followers.send_modify(|n| *n -= 1);
    }
}

/// Stops `vm` once `grace` passes with nothing following it, however that came
/// about: a client that never opened the logs, or the last stream's end. A VM
/// that exited ends this.
async fn lease(vm: Arc<Vm>, vm_id: String, grace: Duration) {
    let mut followers = vm.followers.subscribe();
    let mut ended = vm.ended.subscribe();
    loop {
        tokio::select! {
            _ = ended.wait_for(Option::is_some) => return,
            _ = followers.wait_for(|n| *n > 0) => {}
            () = tokio::time::sleep(grace) => break,
        }
        tokio::select! {
            _ = ended.wait_for(Option::is_some) => return,
            _ = followers.wait_for(|n| *n == 0) => {}
        }
    }
    tracing::warn!(%vm_id, instance = vm.instance, ?grace, "no client follows this worker VM; stopping it");
    if let Err(error) = vm.stop(LEASE_STOP_GRACE).await {
        tracing::error!(%error, %vm_id, "worker lease stop retained unresolved lifecycle");
    }
}

/// Resolves once a frame cannot reach `channel`'s reader. The engine closes a writer's
/// socket when its reader leaves, but `ChannelWriter` never reads that socket and a quiet
/// guest gives it nothing to fail on, so this writes a byte. Readers take records from text
/// frames only. The write after the one that meets the closed socket is the one that fails.
async fn reader_gone(channel: &ChannelWriter, every: Duration) -> String {
    loop {
        tokio::time::sleep(every).await;
        if let Err(e) = channel.write(&[0]).await {
            return e.to_string();
        }
    }
}

/// Drops `start`'s `Preparing` entry if it never launched (an error, a
/// cancel, or the call was dropped), so the key stays startable.
struct Unreserve<'a>(&'a WorkerVms, &'a str, u64);

impl Drop for Unreserve<'_> {
    fn drop(&mut self) {
        let mut slots = self.0.slots();
        if matches!(slots.get(self.1), Some(Slot::Preparing(_, start, _)) if *start == self.2) {
            slots.remove(self.1);
        }
        drop(slots);
        self.0.unreserved.notify_waiters();
    }
}

impl Slot {
    fn instance(&self) -> u64 {
        match self {
            Slot::Preparing(instance, ..) | Slot::Destroying(instance) => *instance,
            Slot::Booted(vm) => vm.instance,
        }
    }

    fn busy(&self) -> bool {
        match self {
            Slot::Preparing(..) | Slot::Destroying(_) => true,
            Slot::Booted(vm) => {
                vm.ended.borrow().is_none() || (vm.lifecycle.is_some() && vm.receipt().is_err())
            }
        }
    }

    fn status(&self, vm_id: &str) -> WorkerStatus {
        let (state, ended, started_at_ms) = match self {
            // Nothing has started yet. `lookup` never hands out `Destroying`.
            Slot::Preparing(..) | Slot::Destroying(_) => (State::Preparing, None, 0),
            Slot::Booted(vm) => {
                let observed = vm.lifecycle.as_ref().map(|session| session.observation());
                let ended = (*vm.ended.borrow()).or_else(|| {
                    observed
                        .as_ref()
                        .and_then(|o| o.os_status())
                        .map(|status| Ended {
                            exit_code: status.code(),
                            signal: status.signal(),
                            at_ms: now_ms(),
                        })
                });
                let state = match ended {
                    None => State::Running,
                    Some(_)
                        if vm.stopping.load(Ordering::SeqCst)
                            && (vm.receipt().is_ok() || cfg!(test) && vm.lifecycle.is_none()) =>
                    {
                        State::Stopped
                    }
                    Some(_) if observed.as_ref().is_some_and(|o| o.os_status().is_none()) => {
                        State::Running
                    }
                    Some(_) => State::Exited,
                };
                (state, ended, vm.started_at_ms)
            }
        };
        WorkerStatus {
            vm_id: vm_id.to_string(),
            instance: self.instance(),
            state,
            exit_code: ended.and_then(|e| e.exit_code),
            signal: ended.and_then(|e| e.signal),
            started_at_ms,
            ended_at_ms: ended.map(|e| e.at_ms),
        }
    }
}

impl Vm {
    fn receipt(&self) -> io::Result<crate::worker_admission::WriterRetirement> {
        match self
            .retirement
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            Some(Ok(receipt)) => Ok(receipt.clone()),
            Some(Err(error)) => Err(io::Error::other(error.clone())),
            None => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "runtime/host writers have not retired",
            )),
        }
    }
    /// Fixture-only stop signal through the spawn-time pidfd: after the child
    /// exited it yields ESRCH from that pidfd, never another process.
    #[cfg(test)]
    fn signal_fixture(&self, signal: libc::c_int) {
        if let Some(pidfd) = &self.fixture_pidfd {
            let _ = crate::sandbox_daemon::adapters::test_pidfd_signal(pidfd, signal);
        }
    }
    async fn stop(&self, grace: Duration) -> io::Result<()> {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(session) = &self.lifecycle {
            session.stop(grace).await?;
            let mut ended = self.ended.subscribe();
            tokio::time::timeout(
                DRAIN_GRACE + Duration::from_secs(5),
                ended.wait_for(Option::is_some),
            )
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "worker host-writer completion is unresolved",
                )
            })?
            .map_err(|_| io::Error::other("worker completion observer closed"))?;
            self.receipt()?;
            return Ok(());
        }
        #[cfg(test)]
        {
            if !self.reaped.load(Ordering::SeqCst) {
                if !grace.is_zero() {
                    self.signal_fixture(libc::SIGTERM);
                    let mut ended = self.ended.subscribe();
                    let _ = tokio::time::timeout(grace, ended.wait_for(Option::is_some)).await;
                }
                if !self.reaped.load(Ordering::SeqCst) {
                    self.signal_fixture(libc::SIGKILL);
                }
            }
            let mut ended = self.ended.subscribe();
            ended
                .wait_for(Option::is_some)
                .await
                .map_err(|_| io::Error::other("fixture waiter closed"))?;
            return Ok(());
        }
        #[cfg(not(test))]
        Err(io::Error::other(
            "worker instance lacks runtime control authority",
        ))
    }
}

/// Opens a control file with `O_NOFOLLOW`: a planted
/// symlink fails the open instead of redirecting it. A write truncates, mode 0600.
fn open_private(path: &Path, write: bool) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    if write {
        opts.write(true).create(true).truncate(true).mode(0o600);
    } else {
        opts.read(true);
    }
    opts.custom_flags(libc::O_NOFOLLOW).open(path)
}

/// The instance `<dir>/state.json` records: 0 before the first boot.
fn stored_instance(dir: &Path) -> Result<u64, SandboxError> {
    let path = dir.join(STATE_FILE);
    let read = match open_private(&path, false) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        file => file.and_then(|f| Ok(serde_json::from_reader::<_, StateFile>(f)?.instance)),
    };
    read.map_err(|e| SandboxError::BootFailed(format!("{}: {e}", path.display())))
}

/// Cold base creation is a short, exact operation, never part of a VM's
/// long-lived footprint. Existing ancestors are inspected, not repaired.
fn prepare_worker_base(
    context: &crate::worker_admission::AdmissionContext,
    base: &Path,
) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;
    if !base.is_absolute()
        || base
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "worker base must be absolute without traversal",
        ));
    }
    let uid = unsafe { libc::geteuid() };
    match std::fs::symlink_metadata(base) {
        Ok(meta) => {
            if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o7777 != 0o700 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "worker base is not owned/private; refusing chmod",
                ));
            }
            return Ok(());
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // Existing base children belong to independent lifecycle intervals. Do not
    // inspect/re-admit their union merely to use another state's existing base.
    // Cold setup has no children and must pass the full retained-path check.
    context.reject_retained_path(base)?;
    let mut first = base;
    for depth in 0..=64 {
        let parent = first
            .parent()
            .ok_or_else(|| io::Error::other("worker base has no existing parent"))?;
        match std::fs::symlink_metadata(parent) {
            Ok(meta) => {
                if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o022 != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "worker base ancestor is not owned/protected",
                    ));
                }
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound && depth < 64 => first = parent,
            Err(e) => return Err(e),
        }
    }
    let setup = context.local(first)?;
    let mut mutation = setup.begin(crate::worker_admission::MutationPlan::Create {
        root: first.to_path_buf(),
    })?;
    mutation.create_dir_all(base, 0o700)?;
    mutation.commit()?;
    setup.revalidate()?;
    drop(setup);
    Ok(())
}

fn admit_control_files(
    lease: crate::worker_admission::Lease,
    dir: &Path,
) -> io::Result<crate::worker_admission::Lease> {
    let paths: Vec<_> = [
        STATE_FILE,
        STATE_KEY_FILE,
        BUILD_DIR_RECORD,
        BUILD_MODE_SEAL,
        CONSOLE_LOG,
        CONSOLE_LOG_OLD,
    ]
    .iter()
    .map(|name| dir.join(name))
    .collect();
    let mut requests: Vec<_> = paths
        .iter()
        .map(|path| crate::worker_admission::PathRequest {
            path,
            role: crate::worker_admission::PathRole::MutableFile,
        })
        .collect();
    let root = dir.join("root");
    requests.push(crate::worker_admission::PathRequest {
        path: &root,
        role: crate::worker_admission::PathRole::MutableTree,
    });
    let runtime = root.join("runtime");
    let legacy_runtime = root.join("opt/iii");
    let root_upper = root.join(crate::cli::upper::UPPER_NAME);
    let build_upper = dir.join(crate::cli::upper::UPPER_NAME);
    requests.extend([
        crate::worker_admission::PathRequest {
            path: &runtime,
            role: crate::worker_admission::PathRole::MutableTree,
        },
        crate::worker_admission::PathRequest {
            path: &legacy_runtime,
            role: crate::worker_admission::PathRole::MutableTree,
        },
        crate::worker_admission::PathRequest {
            path: &root_upper,
            role: crate::worker_admission::PathRole::MutableFile,
        },
        crate::worker_admission::PathRequest {
            path: &build_upper,
            role: crate::worker_admission::PathRole::MutableFile,
        },
    ]);
    let scratch = dir.join(format!("{BUILD_SCRATCH_PREFIX}current"));
    let scratch_root = scratch.join("root");
    let scratch_runtime = scratch_root.join("runtime");
    let scratch_upper = scratch.join(crate::cli::upper::UPPER_NAME);
    requests.extend([
        crate::worker_admission::PathRequest {
            path: &scratch,
            role: crate::worker_admission::PathRole::MutableTree,
        },
        crate::worker_admission::PathRequest {
            path: &scratch_root,
            role: crate::worker_admission::PathRole::MutableTree,
        },
        crate::worker_admission::PathRequest {
            path: &scratch_runtime,
            role: crate::worker_admission::PathRole::MutableTree,
        },
        crate::worker_admission::PathRequest {
            path: &scratch_upper,
            role: crate::worker_admission::PathRole::MutableFile,
        },
    ]);
    crate::worker_admission::with_paths(lease, &requests)
}

fn write_control_admitted(
    lease: &crate::worker_admission::Lease,
    dir: &Path,
    name: &str,
    bytes: &[u8],
) -> io::Result<()> {
    if !dir.exists() {
        let mut create = lease.begin(crate::worker_admission::MutationPlan::Create {
            root: dir.to_path_buf(),
        })?;
        create.create_dir_all(dir, 0o700)?;
        create.commit()?;
    }
    let target = dir.join(name);
    let mut mutation = lease.begin(crate::worker_admission::MutationPlan::ControlFiles {
        files: vec![target.clone()],
    })?;
    let mut stage = mutation.stage_file(&target, 0o600)?;
    stage.file_mut().write_all(bytes)?;
    stage.file_mut().sync_all()?;
    mutation.publish_file(stage)?;
    mutation.commit()?;
    Ok(())
}

fn store_instance(dir: &Path, instance: u64) -> io::Result<()> {
    let tmp = dir.join("state.json.tmp");
    // A crash between the create and the rename leaves it behind.
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    serde_json::to_writer(&mut file, &StateFile { instance })?;
    file.sync_all()?;
    std::fs::rename(&tmp, dir.join(STATE_FILE))
}

/// Record `state_key` in `<dir>`, where `stored_state_key` reads it.
fn store_state_key(dir: &Path, state_key: &str) -> io::Result<()> {
    open_private(&dir.join(STATE_KEY_FILE), true)?.write_all(state_key.as_bytes())
}

/// The state key `<base>/<vm_id>/` records, if any. `vm_id` is whatever the caller sent,
/// not yet authorized, and `<base>/<vm_id>/root/` is the guest's: only an id `vm_id()`
/// could have made reaches the path.
fn stored_state_key(base: &Path, id: &str) -> Option<String> {
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut state_key = String::new();
    open_private(&base.join(id).join(STATE_KEY_FILE), false)
        .and_then(|mut file| file.read_to_string(&mut state_key))
        .ok()?;
    Some(state_key)
}

/// A `<vm_id>/build-dir` record: the build directory of the VM of `state_key`.
#[derive(Serialize, Deserialize)]
struct BuildDirRecord {
    state_key: String,
    dir: PathBuf,
    /// Hex HMAC-SHA256 keyed by the project token, see `build_dir_mac`.
    mac: String,
}

/// The MAC of a `build-dir` record: over [`BUILD_DIR_DOMAIN`], the state key's
/// length and bytes, then the directory's bytes.
fn build_dir_mac(token: &str, state_key: &str, dir: &Path) -> Hmac<Sha256> {
    // HMAC takes a key of any length; only fixed-size MACs can refuse one.
    let mut mac =
        Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC accepts a key of any length");
    mac.update(BUILD_DIR_DOMAIN);
    mac.update(&(state_key.len() as u64).to_be_bytes());
    mac.update(state_key.as_bytes());
    mac.update(dir.as_os_str().as_encoded_bytes());
    mac
}

/// The build directory `<base>/<id>/build-dir` records, when the record is whole,
/// names a state key whose `vm_id` is `id`, and carries `token`'s MAC. As in
/// `stored_state_key`, only an id `vm_id()` could have made reaches the path.
fn read_build_dir(base: &Path, id: &str, token: &str) -> Option<PathBuf> {
    if token.is_empty() || id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let file = open_private(&base.join(id).join(BUILD_DIR_RECORD), false).ok()?;
    let record: BuildDirRecord = serde_json::from_reader(file.take(64 * 1024)).ok()?;
    let tag = hex::decode(&record.mac).ok()?;
    (vm_id(&record.state_key) == id
        && build_dir_mac(token, &record.state_key, &record.dir)
            .verify_slice(&tag)
            .is_ok())
    .then_some(record.dir)
}

/// Record `dir` as the build directory of VM `id` (`state_key`'s) in
/// `<base>/<id>/build-dir`: a temp file flushed and renamed over the record, and
/// the directories flushed, so a crash or a power failure leaves the previous
/// record or this one, whole.
fn store_build_dir(
    base: &Path,
    id: &str,
    state_key: &str,
    dir: &Path,
    token: &str,
) -> io::Result<()> {
    let record = BuildDirRecord {
        state_key: state_key.to_string(),
        dir: dir.to_path_buf(),
        mac: hex::encode(build_dir_mac(token, state_key, dir).finalize().into_bytes()),
    };
    // A directory whose path is not UTF-8 fails here, and is never claimed.
    let data = serde_json::to_vec(&record)?;
    let vm_dir = base.join(id);
    std::fs::create_dir_all(&vm_dir)?;
    sync_dir(base)?;
    let tmp = vm_dir.join(format!("{BUILD_DIR_RECORD}.tmp"));
    // A crash between the create and the rename leaves it behind.
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    file.write_all(&data)?;
    file.sync_all()?;
    std::fs::rename(&tmp, vm_dir.join(BUILD_DIR_RECORD))?;
    sync_dir(&vm_dir)
}

/// Every build directory a VM of `token`'s project recorded under `base`, with
/// its VM: the records `read_build_dir` accepts.
fn recorded_build_dirs(base: &Path, token: &str) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().into_string().ok()?;
            let dir = read_build_dir(base, &id, token)?;
            Some((id, dir))
        })
        .collect()
}

/// A directory's identity, device and inode, when `path` is a directory now
/// (not through a final symlink): two spellings of one directory share it.
fn dir_identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|m| m.is_dir())
        .map(|m| (m.dev(), m.ino()))
}

/// Exclusive, blocking lock over the `build-dir` records of `base`, across
/// threads and processes: an flock on the `base` directory itself, so no lock
/// file outlives the last VM. Released when the handle drops or the process
/// exits. `None` where the filesystem cannot lock a directory
/// ([`lock_unsupported`], NFS among them): a warning, once, and the claims are
/// then serialized within this adapter process only, by its mutex.
fn lock_build_dirs(base: &Path) -> Result<Option<File>, String> {
    static WARNED: AtomicBool = AtomicBool::new(false);
    std::fs::create_dir_all(base).map_err(|e| format!("{}: {e}", base.display()))?;
    let dir = File::open(base).map_err(|e| format!("open {}: {e}", base.display()))?;
    match dir.lock() {
        Ok(()) => Ok(Some(dir)),
        Err(e) if lock_unsupported(&e) => {
            if !WARNED.swap(true, Ordering::SeqCst) {
                tracing::warn!(dir = %base.display(), error = %e, "this filesystem cannot lock a directory; build-dir claims are serialized within this adapter process only");
            }
            Ok(None)
        }
        Err(e) => Err(format!("lock {}: {e}", base.display())),
    }
}

/// ENOLCK, EBADF, ENOTSUP (EOPNOTSUPP) and EINVAL from an flock on a
/// directory: what filesystems that cannot lock one answer (NFS emulates
/// flock with POSIX locks, which a directory cannot hold).
fn lock_unsupported(e: &io::Error) -> bool {
    e.raw_os_error().is_some_and(|code| {
        [
            libc::ENOLCK,
            libc::EBADF,
            libc::ENOTSUP,
            libc::EOPNOTSUPP,
            libc::EINVAL,
        ]
        .contains(&code)
    })
}

/// Record `state_key` and `instance`, truncate `console.log` and spawn `cmd`
/// with piped stdin (the env frame) and output, and a lifeline.
enum LaunchedChild {
    Runtime(crate::worker_admission::RuntimeChild),
    /// A test-only plain child and the pidfd taken right after its spawn.
    #[cfg(test)]
    Fixture(Child, Option<std::os::fd::OwnedFd>),
}
impl LaunchedChild {
    fn id(&self) -> Option<u32> {
        match self {
            Self::Runtime(child) => child.id(),
            #[cfg(test)]
            Self::Fixture(child, _) => child.id(),
        }
    }
    /// The fixture's spawn-time pidfd (an owned runtime child is signalled
    /// only through the core's child-issued pidfd instead).
    #[cfg(test)]
    fn take_fixture_pidfd(&mut self) -> Option<std::os::fd::OwnedFd> {
        match self {
            Self::Runtime(_) => None,
            Self::Fixture(_, pidfd) => pidfd.take(),
        }
    }
    fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        match self {
            Self::Runtime(child) => child.take_stdin(),
            #[cfg(test)]
            Self::Fixture(child, _) => child.stdin.take(),
        }
    }
    fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        match self {
            Self::Runtime(child) => child.take_stdout(),
            #[cfg(test)]
            Self::Fixture(child, _) => child.stdout.take(),
        }
    }
    fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        match self {
            Self::Runtime(child) => child.take_stderr(),
            #[cfg(test)]
            Self::Fixture(child, _) => child.stderr.take(),
        }
    }
    fn supervise(
        self,
        lease: crate::worker_admission::Lease,
        endpoints: Vec<PathBuf>,
    ) -> (
        Option<Arc<crate::sandbox_daemon::adapters::RuntimeSession>>,
        tokio::task::JoinHandle<(
            Option<std::process::ExitStatus>,
            Result<Option<crate::worker_admission::WriterRetirement>, String>,
        )>,
    ) {
        match self {
            Self::Runtime(child) => {
                let session = crate::sandbox_daemon::adapters::RuntimeSession::supervise(
                    child, lease, endpoints,
                );
                let held = session.clone();
                let task = tokio::spawn(async move {
                    let (status, retirement) = held.wait().await;
                    (status, retirement.map(Some).map_err(|e| e.to_string()))
                });
                (Some(session), task)
            }
            #[cfg(test)]
            Self::Fixture(mut child, _pidfd) => (
                None,
                tokio::spawn(async move {
                    match child.wait().await {
                        Ok(status) => (Some(status), Ok(None)),
                        Err(error) => (None, Err(error.to_string())),
                    }
                }),
            ),
        }
    }
}

fn spawn_admitted(
    lease: &crate::worker_admission::Lease,
    dir: &Path,
    state_key: &str,
    instance: u64,
    mut cmd: Command,
    runtime_paths: Option<Vec<(PathBuf, crate::worker_admission::RuntimePathKind)>>,
) -> io::Result<(Console, Lifeline, LaunchedChild, Option<String>)> {
    write_control_admitted(lease, dir, STATE_KEY_FILE, state_key.as_bytes())?;
    write_control_admitted(
        lease,
        dir,
        STATE_FILE,
        &serde_json::to_vec(&StateFile { instance })?,
    )?;
    // The exact console pair gets ONE core log writer before launch; each new
    // instance explicitly resets its ordinary retention (offsets start at 0).
    let mut console = open_console(lease, dir)?;
    console.reset_instance()?;
    let launched = (|| -> io::Result<(Lifeline, LaunchedChild, Option<String>)> {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let lifeline = crate::daemon_exit::attach_lifeline(&mut cmd)?;
        let mut startup_error = None;
        let child = match runtime_paths {
            Some(paths) => {
                let paths: Vec<_> = paths
                    .iter()
                    .map(|(path, kind)| crate::worker_admission::RuntimePath { path, kind: *kind })
                    .collect();
                match lease.prepare_runtime(&paths)?.spawn(cmd) {
                    Ok(child) => LaunchedChild::Runtime(child),
                    Err(error) => {
                        let (cause, child) = error.into_parts();
                        match child {
                            Some(child) => {
                                startup_error = Some(cause.to_string());
                                LaunchedChild::Runtime(child)
                            }
                            None => return Err(cause),
                        }
                    }
                }
            }
            #[cfg(test)]
            None => {
                let mut child = cmd.spawn()?;
                // Taken while this Child is still owned and unwaited: the pidfd
                // can only name this process. Fixture stops never use a raw PID.
                let pidfd = child
                    .id()
                    .ok_or_else(|| io::Error::other("fixture child has no pid"))
                    .and_then(crate::sandbox_daemon::adapters::test_pidfd_open);
                match pidfd {
                    Ok(pidfd) => LaunchedChild::Fixture(child, Some(pidfd)),
                    Err(error) => {
                        // No signalable handle: reap it, never signal by PID.
                        tokio::spawn(async move {
                            let _ = child.wait().await;
                        });
                        return Err(error);
                    }
                }
            }
            #[cfg(not(test))]
            None => return Err(io::Error::other("runtime launch footprint is required")),
        };
        Ok((lifeline, child, startup_error))
    })();
    match launched {
        Ok((lifeline, child, startup_error)) => Ok((console, lifeline, child, startup_error)),
        // Nothing was spawned, so no pump owns the writer: settle it now so a
        // never-spawned rollback is not blocked by an unfinished log record.
        Err(error) => match console.finish() {
            Ok(()) => Err(error),
            Err(finish) => Err(io::Error::other(format!(
                "{error}; console log retained unfinished: {finish}"
            ))),
        },
    }
}

#[cfg(test)]
fn spawn(
    dir: &Path,
    state_key: &str,
    instance: u64,
    mut cmd: Command,
) -> io::Result<(File, Lifeline, Child)> {
    std::fs::create_dir_all(dir)?;
    let console = open_private(&dir.join(CONSOLE_LOG), true)?;
    store_state_key(dir, state_key)?;
    store_instance(dir, instance)?;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let lifeline = crate::daemon_exit::attach_lifeline(&mut cmd)?;
    Ok((console, lifeline, cmd.spawn()?))
}

/// A VM's `console.log`, rotated to `console.log.1` past `CONSOLE_LOG_CAP`.
///
/// A log offset counts every byte this instance logged, across rotations: the
/// current file holds the offsets from `base` on, the rotated one those from
/// `oldest` up to `base`. While the instance lives, every append, in-place
/// rotation and read goes through the ONE core-issued writer of the exact
/// declared pair (stable descriptors, no per-record stage, prior or journal).
/// Once that writer finished, the pair no longer changes and is read through
/// the last completed cursor only.
struct Console {
    dir: PathBuf,
    /// `None` once finished, or for a read-only view of a settled pair.
    writer: Option<crate::worker_admission::LogWriter>,
    /// Sticky once an append, read or finish outcome is not proven complete:
    /// the core keeps the record uncertain, so nothing retries from old offsets.
    failed: bool,
    oldest: u64,
    base: u64,
    len: u64,
}

/// The largest window one core log read returns (its record bound).
const CONSOLE_READ_CHUNK: usize = 1024 * 1024;

/// Create the exact console pair once and issue its one bounded log writer.
/// Only MISSING members are created, through one owned ControlFiles mutation;
/// an existing member is never replaced or truncated here, and the core proves
/// its admitted identity (or the finished ledger of this same operation).
fn open_console(lease: &crate::worker_admission::Lease, dir: &Path) -> io::Result<Console> {
    let current = dir.join(CONSOLE_LOG);
    let previous = dir.join(CONSOLE_LOG_OLD);
    let mut missing = Vec::new();
    for path in [&current, &previous] {
        match std::fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => missing.push(path.clone()),
            Err(error) => return Err(error),
        }
    }
    if !missing.is_empty() {
        let mut init = lease.begin(crate::worker_admission::MutationPlan::ControlFiles {
            files: missing.clone(),
        })?;
        for path in &missing {
            let stage = init.stage_file(path, 0o600)?;
            init.publish_file(stage)?;
        }
        init.commit()?;
    }
    let writer = lease.open_log(crate::worker_admission::LogSpec {
        current: &current,
        previous: Some(&previous),
        cap_bytes: CONSOLE_LOG_CAP,
    })?;
    let cursor = writer.cursor()?;
    let mut console = Console {
        dir: dir.to_path_buf(),
        writer: Some(writer),
        failed: false,
        oldest: 0,
        base: 0,
        len: 0,
    };
    console.track(&cursor);
    Ok(console)
}

impl Console {
    /// Cursors move only from a completed core outcome.
    fn track(&mut self, cursor: &crate::worker_admission::LogCursor) {
        self.oldest = cursor.previous_base;
        self.base = cursor.base;
        self.len = cursor.len;
    }

    fn settle<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn live(&self) -> io::Result<&crate::worker_admission::LogWriter> {
        if self.failed {
            return Err(io::Error::other(
                "console log outcome is uncertain; retained for explicit recovery",
            ));
        }
        self.writer
            .as_ref()
            .ok_or_else(|| io::Error::other("console log writer already finished"))
    }

    /// The log offset after the last record: whole records only, from the
    /// last completed core outcome.
    fn end(&self) -> u64 {
        self.base + self.len
    }

    /// Explicit ordinary-retention reset of the exact pair for a new instance.
    fn reset_instance(&mut self) -> io::Result<()> {
        let result = self.live()?.reset_instance();
        let cursor = self.settle(result)?;
        self.track(&cursor);
        Ok(())
    }

    /// Append one `{stream, line, ts_ms}` JSON line.
    fn append(&mut self, stream: &LogStream, line: &[u8]) -> io::Result<()> {
        let mut record = serde_json::to_vec(&serde_json::json!({
            "stream": stream,
            "line": String::from_utf8_lossy(line),
            "ts_ms": now_ms(),
        }))?;
        record.push(b'\n');
        let result = self.live()?.append(&record);
        let cursor = self.settle(result)?;
        self.track(&cursor);
        Ok(())
    }

    /// Finish the writer: only after every pump joined (this owner is the
    /// last clone). A failed writer is dropped unfinished, so the core keeps
    /// the record uncertain and Remove/H1 stay blocked; never reported as done.
    fn finish(&mut self) -> io::Result<()> {
        let Some(writer) = self.writer.take() else {
            if self.failed {
                return Err(io::Error::other("console log failed; record retained"));
            }
            return Ok(());
        };
        if self.failed {
            drop(writer);
            return Err(io::Error::other(
                "console log failed; record retained as uncertain",
            ));
        }
        let result = writer.finish();
        let cursor = self.settle(result)?;
        self.track(&cursor);
        Ok(())
    }

    /// At most `max` bytes from log offset `offset` (clamped to the oldest
    /// retained byte and the end): through the live writer, which serializes
    /// with appends and in-place rotation, else from the settled pair.
    fn read(&mut self, offset: u64, max: usize) -> io::Result<crate::worker_admission::LogRead> {
        if self.failed {
            return Err(io::Error::other(
                "console log outcome is uncertain; retained for explicit recovery",
            ));
        }
        if let Some(writer) = &self.writer {
            let result = writer.read(offset, max);
            return self.settle(result);
        }
        self.read_settled(offset, max)
    }

    /// A settled pair: no writer rotates it any more, so the last completed
    /// cursor names exactly which bytes of which file are this instance's.
    fn read_settled(
        &self,
        offset: u64,
        max: usize,
    ) -> io::Result<crate::worker_admission::LogRead> {
        use std::os::unix::fs::FileExt;
        let mut at = offset.max(self.oldest).min(self.end());
        let mut bytes = Vec::new();
        for (name, start, len) in [
            (CONSOLE_LOG_OLD, self.oldest, self.base - self.oldest),
            (CONSOLE_LOG, self.base, self.len),
        ] {
            if len == 0 || at >= start + len || bytes.len() >= max {
                continue;
            }
            if at < start {
                break;
            }
            let file = open_private(&self.dir.join(name), false)?;
            let from = at - start;
            let available = len.min(file.metadata()?.len()).saturating_sub(from);
            let n = available.min((max - bytes.len()) as u64) as usize;
            let old = bytes.len();
            bytes.resize(old + n, 0);
            file.read_exact_at(&mut bytes[old..], from)?;
            at += n as u64;
            if (n as u64) < len - from {
                break;
            }
        }
        Ok(crate::worker_admission::LogRead {
            bytes,
            next: at,
            end: self.end(),
            oldest: self.oldest,
        })
    }
}

/// Copy one output stream into `console.log`, a record per line. It keeps
/// reading after a write error, so a full disk cannot stall the VM on a full pipe.
async fn pump(
    out: impl AsyncRead + Unpin,
    stream: LogStream,
    console: Arc<Mutex<Console>>,
) -> io::Result<()> {
    let mut out = BufReader::new(out);
    let mut line = Vec::new();
    let mut warned = false;
    loop {
        line.clear();
        match (&mut out).take(MAX_LINE).read_until(b'\n', &mut line).await {
            Ok(0) => return Ok(()),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        let text = line.strip_suffix(b"\n").unwrap_or(&line);
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        let written = console
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .append(&stream, text);
        if let Err(e) = written
            && !warned
        {
            warned = true;
            tracing::warn!(error = %e, "cannot write the worker VM's console.log; dropping its output");
        }
    }
}

/// What `Console::append` writes, minus the cursor.
#[derive(Deserialize)]
struct Entry {
    stream: LogStream,
    line: String,
    ts_ms: u64,
}

/// The records from log offset `from` on (see `Console`), and the offset after
/// the last whole line read. Offsets survive a rotation: a cursor into the
/// rotated file reads its rest, then the new file. A cursor past the end is
/// another instance's, so it reads from the oldest record kept. A partial last
/// line waits for its newline; a line that does not parse is skipped. Each
/// bounded window is read under the console lock through the live core writer
/// (never an independent path reader racing its in-place rotation).
fn read_records(console: &Mutex<Console>, from: u64) -> io::Result<(Vec<LogRecord>, u64)> {
    let read = |offset: u64| {
        console
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .read(offset, CONSOLE_READ_CHUNK)
    };
    let first = read(from)?;
    let mut window = if from > first.end { read(0)? } else { first };
    let mut at = window.next - window.bytes.len() as u64;
    let mut end = at;
    let mut pending = Vec::new();
    let mut records = Vec::new();
    loop {
        let start = window.next - window.bytes.len() as u64;
        if start != at {
            // Retention moved past the unread bytes between two windows; the
            // oldest kept byte is a record boundary (rotation is per record).
            pending.clear();
            end = start;
        }
        pending.extend_from_slice(&window.bytes);
        at = window.next;
        let mut consumed = 0;
        while let Some(n) = pending[consumed..].iter().position(|&b| b == b'\n') {
            let raw = &pending[consumed..consumed + n + 1];
            consumed += n + 1;
            end += raw.len() as u64;
            if let Ok(Entry {
                stream,
                line,
                ts_ms,
            }) = serde_json::from_slice(raw)
            {
                records.push(LogRecord {
                    stream,
                    line,
                    ts_ms,
                    cursor: end,
                });
            }
        }
        pending.drain(..consumed);
        if window.bytes.is_empty() || at >= window.end {
            return Ok((records, end));
        }
        window = read(at)?;
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
#[path = "worker_vm/compat_admission_tests.rs"]
mod compat_admission_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::scoped_auth_token;
    use futures::StreamExt;
    use iii_sdk::channels::{ChannelDirection, StreamChannelRef};
    use tokio::sync::{Notify, mpsc};
    use tokio_tungstenite::tungstenite::Message;

    /// Umask-independent fixtures: every MISSING component of `path` is created
    /// private (0700), as admitted worker bases and VM dirs are; an existing
    /// ancestor is never chmodded or repaired.
    fn private_dirs(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut missing = Vec::new();
        let mut cursor = Some(path);
        while let Some(dir) = cursor {
            if dir.exists() {
                break;
            }
            missing.push(dir.to_path_buf());
            cursor = dir.parent();
        }
        for dir in missing.into_iter().rev() {
            std::fs::create_dir(&dir).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }

    /// A private (0600) fixture file, independent of the host umask.
    fn write_private(path: &Path, bytes: impl AsRef<[u8]>) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, bytes).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn vms(t: &Path, token: Option<&str>, roots: Vec<PathBuf>) -> WorkerVms {
        use std::os::unix::fs::PermissionsExt;
        let catalog = t.join("admission-catalog");
        std::fs::create_dir_all(&catalog).unwrap();
        std::fs::set_permissions(&catalog, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&catalog).unwrap();
        WorkerVms::new(t.join("workers"), token.map(str::to_string), roots)
            .with_admission_context(context)
    }

    fn local(host_path: &Path) -> WorkerSource {
        WorkerSource::Local {
            host_path: host_path.display().to_string(),
            run_override: None,
        }
    }

    async fn prepare_err(vms: &WorkerVms, auth: &str, source: WorkerSource) -> SandboxError {
        vms.prepare(WorkerPrepareRequest {
            auth: auth.to_string(),
            source,
            state_key: None,
        })
        .await
        .expect_err("prepare must be refused")
    }

    fn sh(script: &str) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    fn code<T: std::fmt::Debug>(result: Result<T, SandboxError>) -> &'static str {
        result.expect_err("must be refused").code().as_str()
    }

    fn start_req(auth: &str, state_key: &str, source: WorkerSource) -> WorkerStartRequest {
        WorkerStartRequest {
            auth: auth.to_string(),
            state_key: state_key.to_string(),
            worker_name: "w".to_string(),
            source,
            env: Default::default(),
            engine_url: "ws://127.0.0.1:49134".to_string(),
            build: None,
        }
    }

    fn status_req(auth: &str, vm_id: &str, instance: u64) -> WorkerStatusRequest {
        WorkerStatusRequest {
            auth: auth.to_string(),
            vm_id: vm_id.to_string(),
            instance,
        }
    }

    fn stop_req(auth: &str, vm_id: &str, instance: u64, grace_ms: u64) -> WorkerStopRequest {
        WorkerStopRequest {
            auth: auth.to_string(),
            vm_id: vm_id.to_string(),
            instance,
            grace_ms,
        }
    }

    fn logs_req(auth: &str, vm_id: &str, follow: bool) -> WorkerLogsRequest {
        WorkerLogsRequest {
            auth: auth.to_string(),
            vm_id: vm_id.to_string(),
            cursor: None,
            follow,
            channel: StreamChannelRef {
                channel_id: "c".to_string(),
                access_key: "k".to_string(),
                direction: ChannelDirection::Write,
            },
            from_end: false,
        }
    }

    /// What `pump_logs` sends for `req`.
    async fn pumped(vms: &WorkerVms, req: &WorkerLogsRequest) -> Vec<LogRecord> {
        let sent = std::cell::RefCell::new(Vec::new());
        vms.pump_logs(req, |record| {
            sent.borrow_mut().push(record);
            async { Ok(()) }
        })
        .await
        .unwrap();
        sent.into_inner()
    }

    fn lines(records: &[LogRecord]) -> Vec<&str> {
        records.iter().map(|r| r.line.as_str()).collect()
    }

    /// A destroy with `prune`: all the VM's state goes, as before contract 1.4.
    fn destroy_req(auth: &str, state_key: &str) -> WorkerDestroyRequest {
        WorkerDestroyRequest {
            auth: auth.to_string(),
            state_key: state_key.to_string(),
            prune: true,
        }
    }

    /// A destroy without `prune`: the VM's data stays.
    fn keep_req(auth: &str, state_key: &str) -> WorkerDestroyRequest {
        WorkerDestroyRequest {
            prune: false,
            ..destroy_req(auth, state_key)
        }
    }

    async fn wait_ended(vms: &WorkerVms, vm_id: &str, instance: u64) -> WorkerStatus {
        for _ in 0..200 {
            let status = vms.status(status_req("t", vm_id, instance)).unwrap();
            if status.ended_at_ms.is_some() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("VM {vm_id} did not exit within 10 s");
    }

    async fn wait_ended_within(vms: &WorkerVms, vm_id: &str, within: Duration) -> WorkerStatus {
        tokio::time::timeout(within, async {
            loop {
                let status = vms.status(status_req("t", vm_id, 1)).unwrap();
                if status.ended_at_ms.is_some() {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the VM outlived its bound")
    }

    /// The first `console.log` record whose line is `line`.
    async fn wait_record(dir: &Path, line: &str) -> serde_json::Value {
        for _ in 0..200 {
            let text = std::fs::read_to_string(dir.join("console.log")).unwrap_or_default();
            if let Some(record) = text
                .lines()
                .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
                .find(|r| r["line"] == line)
            {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no {line:?} record in {}", dir.display());
    }

    #[tokio::test]
    async fn worker_ops_answer_s505_without_the_token() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let iii = iii_sdk::IIIClient::new("ws://127.0.0.1:9");
        // An adapter started without a token (or with an empty one) must refuse
        // an empty `auth` too, which the length and byte checks alone would pass.
        for (token, auth) in [(None, "u"), (Some("t"), "u"), (None, ""), (Some(""), "")] {
            let vms = Arc::new(vms(t, token, vec![t.into()]));
            let err = prepare_err(&vms, auth, local(t)).await;
            assert_eq!(err.code().as_str(), "S505", "{err}");

            // A known VM: without the token, a stale instance is S505, not S503.
            let known = vms
                .launch("k", sh("exit 0"), Vec::new(), None)
                .unwrap()
                .vm_id;
            assert_eq!(
                code(vms.start(start_req(auth, "k", local(t))).await),
                "S505"
            );
            for vm_id in ["unknown", known.as_str()] {
                assert_eq!(code(vms.status(status_req(auth, vm_id, 9))), "S505");
                assert_eq!(code(vms.stop(stop_req(auth, vm_id, 9, 0)).await), "S505");
                assert_eq!(
                    code(vms.logs(&iii, logs_req(auth, vm_id, true)).await),
                    "S505"
                );
            }
            assert_eq!(code(vms.destroy(destroy_req(auth, "k"))), "S505");
        }
    }

    #[tokio::test]
    async fn start_status_stop_destroy_cycle() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![]);
        let started = vms
            .launch("k", sh("echo hi; sleep 30"), Vec::new(), None)
            .unwrap();
        let id = started.vm_id;
        assert_eq!((id.as_str(), started.instance), (vm_id("k").as_str(), 1));

        let status = vms.status(status_req("t", &id, 1)).unwrap();
        assert!(matches!(status.state, State::Running), "{status:?}");
        assert_eq!(status.instance, 1);
        let dir = t.join("workers").join(&id);
        assert_eq!(wait_record(&dir, "hi").await["stream"], "stdout");

        let status = vms.stop(stop_req("t", &id, 1, 2000)).await.unwrap();
        assert!(matches!(status.state, State::Stopped), "{status:?}");
        assert_eq!(status.signal, Some(15), "{status:?}");

        assert!(vms.destroy(destroy_req("t", "k")).unwrap().destroyed);
        assert!(!dir.exists());
    }

    /// A stop delayed past a destroy and a new boot must not reach that boot,
    /// even when the destroying daemon never booted the VM itself.
    #[tokio::test]
    async fn instance_survives_a_destroy() {
        let t = tempfile::tempdir().unwrap();
        let first = vms(t.path(), Some("t"), vec![]);
        let id = first
            .launch("k", sh("exit 0"), Vec::new(), None)
            .unwrap()
            .vm_id;
        wait_ended(&first, &id, 1).await;
        drop(first);

        let vms = vms(t.path(), Some("t"), vec![]);
        assert!(vms.destroy(destroy_req("t", "k")).unwrap().destroyed);
        assert!(!t.path().join("workers").join(&id).exists());
        let again = vms.launch("k", sh("sleep 30"), Vec::new(), None).unwrap();
        assert_eq!(again.instance, 2);
        assert_eq!(code(vms.stop(stop_req("t", &id, 1, 0)).await), "S503");
        vms.stop(stop_req("t", &id, 2, 0)).await.unwrap();
    }

    #[tokio::test]
    async fn instance_survives_an_adapter_restart() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let first = vms(t, Some("t"), vec![]);
        let id = first
            .launch("k", sh("sleep 30"), Vec::new(), None)
            .unwrap()
            .vm_id;
        first.stop(stop_req("t", &id, 1, 0)).await.unwrap();
        drop(first);

        let vms = vms(t, Some("t"), vec![]);
        assert_eq!(
            vms.launch("k", sh("exit 0"), Vec::new(), None)
                .unwrap()
                .instance,
            2
        );
        assert_eq!(code(vms.status(status_req("t", &id, 1))), "S503");
    }

    #[tokio::test]
    async fn stop_escalates_to_sigkill_after_grace() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![]);
        let id = vms
            .launch(
                "k",
                sh("trap '' TERM; echo ready; sleep 30"),
                Vec::new(),
                None,
            )
            .unwrap()
            .vm_id;
        // The trap is set once `ready` is out.
        wait_record(&t.join("workers").join(&id), "ready").await;

        let begun = std::time::Instant::now();
        let status = vms.stop(stop_req("t", &id, 1, 300)).await.unwrap();
        assert!(begun.elapsed() < Duration::from_secs(5));
        assert!(matches!(status.state, State::Stopped), "{status:?}");
        assert_eq!(status.signal, Some(9), "{status:?}");
    }

    #[tokio::test]
    async fn a_running_vm_refuses_start_and_destroy() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let id = vms
            .launch("k", sh("sleep 30"), Vec::new(), None)
            .unwrap()
            .vm_id;

        let busy = vms.launch("k", sh("exit 0"), Vec::new(), None).unwrap_err();
        assert_eq!(busy.code().as_str(), "S502");
        assert_eq!(
            busy.to_payload()["fix"],
            serde_json::json!({"vm_id": id, "instance": 1})
        );
        assert_eq!(code(vms.start(start_req("t", "k", local(t))).await), "S502");
        assert_eq!(code(vms.destroy(destroy_req("t", "k"))), "S502");

        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    /// Compose's S502 stop-and-retry against a start still preparing (one whose
    /// caller is gone): the stop cancels it and returns only once its build
    /// has ended, so the retry never builds into the same state alongside it.
    #[tokio::test]
    async fn stop_cancels_a_start_still_preparing() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vm_id("k");
        // Start call 7, still building.
        vms.slots().insert(id.clone(), Slot::Preparing(1, 7, false));
        let build = Unreserve(&vms, &id, 7);
        assert_eq!(
            code(vms.launch("k", sh("exit 0"), Vec::new(), None)),
            "S502"
        );

        let stop = vms.stop(stop_req("t", &id, 1, 0));
        tokio::pin!(stop);
        let early = tokio::time::timeout(Duration::from_millis(200), &mut stop).await;
        assert!(early.is_err(), "stop returned while the build still ran");
        // Meanwhile neither a retry nor a destroy touches the state.
        assert_eq!(
            code(vms.launch("k", sh("exit 0"), Vec::new(), None)),
            "S502"
        );
        assert_eq!(code(vms.destroy(destroy_req("t", "k"))), "S502");

        // The cancelled build ends without a VM.
        let cancelled = vms.launch("k", sh("sleep 30"), Vec::new(), Some(7));
        assert_eq!(code(cancelled), "S300");
        drop(build);
        let stopped = stop.await.unwrap();
        assert!(matches!(stopped.state, State::Stopped), "{stopped:?}");
        assert_eq!(stopped.instance, 1);

        let retried = vms.launch("k", sh("sleep 30"), Vec::new(), None).unwrap();
        assert_eq!(retried.instance, 1);
        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    #[tokio::test]
    async fn a_vm_being_destroyed_is_busy_to_start_and_gone_to_status() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let id = vm_id("k");
        vms.slots().insert(id.clone(), Slot::Destroying(1));

        assert_eq!(
            code(vms.launch("k", sh("exit 0"), Vec::new(), None)),
            "S502"
        );
        assert_eq!(code(vms.start(start_req("t", "k", local(t))).await), "S502");
        assert_eq!(code(vms.destroy(destroy_req("t", "k"))), "S502");
        assert_eq!(code(vms.status(status_req("t", &id, 1))), "S504");
        assert_eq!(code(vms.stop(stop_req("t", &id, 1, 0)).await), "S504");
    }

    #[tokio::test]
    async fn a_failed_destroy_keeps_the_exited_vm() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![]);
        let id = vms
            .launch("k", sh("exit 7"), Vec::new(), None)
            .unwrap()
            .vm_id;
        wait_ended(&vms, &id, 1).await;
        // A file where the base dir was: ENOTDIR, which root cannot bypass.
        let base = t.join("workers");
        std::fs::rename(&base, t.join("moved")).unwrap();
        std::fs::write(&base, "").unwrap();

        assert_eq!(code(vms.destroy(destroy_req("t", "k"))), "S216");
        let status = vms.status(status_req("t", &id, 1)).unwrap();
        assert_eq!(status.exit_code, Some(7), "{status:?}");
    }

    /// `remove_dir_all` stops at the first entry it cannot remove, possibly
    /// after `state.json`: the next boot must still follow the old one.
    #[tokio::test]
    async fn a_partly_failed_destroy_keeps_the_instance_order() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vms
            .launch("k", sh("exit 0"), Vec::new(), None)
            .unwrap()
            .vm_id;
        wait_ended(&vms, &id, 1).await;
        // A non-empty read-only dir in the guest's share: unremovable unless root.
        let stuck = t.path().join("workers").join(&id).join("root").join("ro");
        private_dirs(&stuck);
        write_private(&stuck.join("f"), "");
        std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o555)).unwrap();

        let destroyed = vms.destroy(destroy_req("t", "k"));
        if stuck.exists() {
            assert_eq!(code(destroyed), "S216");
            std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::remove_dir_all(&stuck).unwrap();
        }
        let again = vms.launch("k", sh("exit 0"), Vec::new(), None).unwrap();
        assert_eq!(again.instance, 2);
    }

    #[tokio::test]
    async fn unknown_vm_is_s504() {
        let t = tempfile::tempdir().unwrap();
        let vms = Arc::new(vms(t.path(), Some("t"), vec![]));
        let iii = iii_sdk::IIIClient::new("ws://127.0.0.1:9");
        assert_eq!(code(vms.status(status_req("t", "nope", 1))), "S504");
        assert_eq!(code(vms.stop(stop_req("t", "nope", 1, 0)).await), "S504");
        assert_eq!(
            code(vms.logs(&iii, logs_req("t", "nope", false)).await),
            "S504"
        );
    }

    #[tokio::test]
    async fn exit_code_is_recorded() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vms
            .launch("k", sh("exit 7"), Vec::new(), None)
            .unwrap()
            .vm_id;
        let status = wait_ended(&vms, &id, 1).await;
        assert!(matches!(status.state, State::Exited), "{status:?}");
        assert_eq!((status.exit_code, status.signal), (Some(7), None));
    }

    #[tokio::test]
    async fn a_rejected_source_is_s501_and_leaves_the_key_startable() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let empty = t.join("empty");
        std::fs::create_dir(&empty).unwrap();
        let vms = vms(t, Some("t"), vec![t.into()]);
        for _ in 0..2 {
            assert_eq!(
                code(vms.start(start_req("t", "k", local(&empty))).await),
                "S501"
            );
        }
    }

    #[tokio::test]
    async fn launch_refuses_a_symlinked_console_log() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let dir = t.join("workers").join(vm_id("k"));
        private_dirs(&dir);
        std::fs::write(t.join("target"), "keep").unwrap();
        std::os::unix::fs::symlink(t.join("target"), dir.join("console.log")).unwrap();

        let vms = vms(t, Some("t"), vec![]);
        assert_eq!(
            code(vms.launch("k", sh("echo pwned"), Vec::new(), None)),
            "S300"
        );
        assert_eq!(std::fs::read_to_string(t.join("target")).unwrap(), "keep");
    }

    /// `state.json` is replaced, never rewritten in place: a write cut short
    /// must leave the previous instance readable, not an empty file.
    #[tokio::test]
    async fn state_json_is_replaced_not_rewritten_in_place() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vms
            .launch("k", sh("exit 0"), Vec::new(), None)
            .unwrap()
            .vm_id;
        wait_ended(&vms, &id, 1).await;
        let dir = t.path().join("workers").join(&id);
        // Same inode as state.json: an in-place rewrite would show through it.
        std::fs::hard_link(dir.join(STATE_FILE), dir.join("old")).unwrap();
        // A temp file left by a crash mid-write must not block the next boot.
        std::fs::write(dir.join("state.json.tmp"), r#"{"inst"#).unwrap();

        let again = vms.launch("k", sh("exit 0"), Vec::new(), None).unwrap();
        assert_eq!(again.instance, 2);
        assert_eq!(stored_instance(&dir).unwrap(), 2);
        let old: StateFile =
            serde_json::from_slice(&std::fs::read(dir.join("old")).unwrap()).unwrap();
        assert_eq!(old.instance, 1);
        assert!(!dir.join("state.json.tmp").exists());
    }

    #[tokio::test]
    async fn console_log_rotates_past_the_cap() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![]);
        let id = vms
            .launch(
                "k",
                // ~12 MB in 200 whole lines: past the cap with every record durable.
                sh(r#"yes "$(printf %060000d 0)" | head -n 200"#),
                Vec::new(),
                None,
            )
            .unwrap()
            .vm_id;
        wait_ended(&vms, &id, 1).await;

        let dir = t.join("workers").join(&id);
        assert!(dir.join("console.log.1").exists());
        let len = std::fs::metadata(dir.join("console.log")).unwrap().len();
        assert!(len < CONSOLE_LOG_CAP, "{len}");
    }

    #[tokio::test]
    async fn host_paths_outside_the_roots_answer_s501() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        std::os::unix::fs::symlink("/etc", t.join("esc")).unwrap();
        let t_str = t.display().to_string();
        let cases = [
            (vec![t.to_path_buf()], PathBuf::from("/etc"), t_str.as_str()),
            (vec![t.to_path_buf()], t.join("../etc"), &t_str),
            (vec![t.to_path_buf()], t.join("esc"), &t_str),
            (vec![], t.to_path_buf(), "no host roots are configured"),
        ];
        for (roots, host_path, needle) in cases {
            let err = prepare_err(&vms(t, Some("t"), roots), "t", local(&host_path)).await;
            let msg = err.to_string();
            assert_eq!(err.code().as_str(), "S501", "{msg}");
            assert!(msg.contains("III_SANDBOX_HOST_ROOTS"), "{msg}");
            assert!(msg.contains(needle), "{msg}");
        }
    }

    #[tokio::test]
    async fn a_missing_host_path_says_so_not_outside_the_roots() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let err = prepare_err(
            &vms(t, Some("t"), vec![t.into()]),
            "t",
            local(&t.join("gone")),
        )
        .await;
        let msg = err.to_string();
        assert_eq!(err.code().as_str(), "S501", "{msg}");
        assert!(msg.contains("cannot be read"), "{msg}");
        assert!(!msg.contains("is outside"), "{msg}");
    }

    #[test]
    fn host_roots_accept_a_path_under_a_symlinked_root() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        std::fs::create_dir_all(t.join("real/w")).unwrap();
        std::os::unix::fs::symlink(t.join("real"), t.join("link")).unwrap();
        let vms = vms(t, Some("t"), vec![t.join("link")]);
        for path in [t.join("real/w"), t.join("link/w")] {
            assert!(vms.admit(path.display().to_string()).is_ok(), "{path:?}");
        }

        // macOS: /tmp is a symlink to /private/tmp.
        #[cfg(target_os = "macos")]
        {
            let tmp = tempfile::tempdir_in("/tmp").unwrap();
            std::fs::create_dir(tmp.path().join("w")).unwrap();
            let vms = WorkerVms::new(t.join("workers"), None, vec![tmp.path().into()]);
            let path = tmp.path().join("w").display().to_string();
            assert!(vms.admit(&path).is_ok(), "{path}");
        }
    }

    #[tokio::test]
    async fn a_bundle_naming_another_package_is_s501() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let bundle = t.join("bundle");
        std::fs::create_dir(&bundle).unwrap();
        std::fs::write(
            bundle.join("iii.worker.yaml"),
            "name: other\nscripts:\n  start: node bundle.js\n",
        )
        .unwrap();
        let source = WorkerSource::Bundle {
            package: "probe".to_string(),
            host_path: bundle.display().to_string(),
        };

        let err = prepare_err(&vms(t, Some("t"), vec![t.into()]), "t", source).await;
        assert_eq!(err.code().as_str(), "S501", "{err}");
        assert!(err.to_string().contains("\"probe\""), "{err}");
    }

    #[test]
    fn records_resume_from_a_cursor() {
        let t = tempfile::tempdir().unwrap();
        let text = concat!(
            r#"{"stream":"stdout","line":"a","ts_ms":1}"#,
            "\n",
            r#"{"stream":"stderr","line":"b","ts_ms":2}"#,
            "\n",
            r#"{"stream":"stdout","line":"c","ts_ms":3}"#,
            "\n",
        );
        // An earlier instance's rotated file is not this one's.
        std::fs::write(t.path().join(CONSOLE_LOG_OLD), record("stale")).unwrap();
        let console = console_over(t.path(), "", text);
        let len = text.len() as u64;

        let (all, end) = read_records(&console, 0).unwrap();
        assert_eq!(lines(&all), ["a", "b", "c"]);
        assert_eq!(all[1].ts_ms, 2);
        assert!(matches!(all[1].stream, LogStream::Stderr));
        assert!(all.windows(2).all(|w| w[0].cursor < w[1].cursor));
        assert_eq!((all[2].cursor, end), (len, len));

        let (rest, _) = read_records(&console, all[0].cursor).unwrap();
        assert_eq!(lines(&rest), ["b", "c"]);
        assert!(read_records(&console, len).unwrap().0.is_empty());

        // A cursor past the end is another instance's: read from the start.
        let (other, _) = read_records(&console, len + 1).unwrap();
        assert_eq!(lines(&other), ["a", "b", "c"]);
    }

    #[test]
    fn a_partial_last_line_is_left_for_the_next_read() {
        let t = tempfile::tempdir().unwrap();
        let whole = r#"{"stream":"stdout","line":"a","ts_ms":1}"#;
        let console = console_over(t.path(), "", &format!("{whole}\n{{\"stream\":\"std"));

        let (recs, end) = read_records(&console, 0).unwrap();
        assert_eq!(lines(&recs), ["a"]);
        assert_eq!(end, whole.len() as u64 + 1);
    }

    /// One `console.log` line holding `line`.
    fn record(line: &str) -> String {
        let record = serde_json::json!({"stream": "stdout", "line": line, "ts_ms": 1});
        format!("{record}\n")
    }

    /// A settled `Console` over `dir` that rotated out `rotated` (never, when
    /// empty) and holds `current` now: no writer, the completed cursor only.
    fn console_over(dir: &Path, rotated: &str, current: &str) -> Mutex<Console> {
        if !rotated.is_empty() {
            std::fs::write(dir.join(CONSOLE_LOG_OLD), rotated).unwrap();
        }
        std::fs::write(dir.join(CONSOLE_LOG), current).unwrap();
        Mutex::new(Console {
            dir: dir.to_path_buf(),
            writer: None,
            failed: false,
            oldest: 0,
            base: rotated.len() as u64,
            len: current.len() as u64,
        })
    }

    #[tokio::test]
    async fn retirement_survives_final_console_and_retention_without_deleting_data() {
        let t = tempfile::tempdir().unwrap();
        let (lease, dir) = admitted_console_lease(t.path(), b"");
        let control = t.path().join("endpoints");
        std::fs::create_dir(&control).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = control.join("worker.sock");
        let lease = crate::worker_admission::with_paths(
            lease,
            &[crate::worker_admission::PathRequest {
                path: &endpoint,
                role: crate::worker_admission::PathRole::Endpoint,
            }],
        )
        .unwrap();
        let upper = dir.join("root").join(crate::cli::upper::UPPER_NAME);
        let mut create = lease
            .begin(crate::worker_admission::MutationPlan::Create { root: dir.clone() })
            .unwrap();
        let mut image = create.stage_file(&upper, 0o600).unwrap();
        image.file_mut().write_all(b"guest-data").unwrap();
        create.publish_file(image).unwrap();
        create.commit().unwrap();
        // The launch path issues the console writer before the runtime starts.
        let mut console = open_console(&lease, &dir).unwrap();
        let session = crate::sandbox_daemon::adapters::runtime_test_session_resources(
            lease.clone(),
            &t.path().join("catalog"),
            false,
            crate::sandbox_daemon::adapters::RuntimeTestResources {
                reads: vec![dir.join("root")],
                endpoints: vec![endpoint.clone()],
                exit_without_drop: true,
                ..Default::default()
            },
        )
        .await;
        let receipt = session.wait().await.1.unwrap();
        console
            .append(&LogStream::Stdout, b"last line after OS wait")
            .unwrap();
        // As the waiter does once every pump joined: finish the last clone
        // before endpoint retirement and the receipt's Remove.
        console.finish().unwrap();
        session.finish_resources(&receipt).await.unwrap();
        assert!(!endpoint.exists());
        let (removed, kept) =
            remove_runtime_state_admitted(&lease, &receipt, &dir, "console", false).unwrap();
        assert!(removed);
        assert!(kept.contains(&dir.join(PRE_BUILD_DISK)));
        assert_eq!(
            std::fs::read(dir.join(PRE_BUILD_DISK)).unwrap(),
            b"guest-data"
        );
        assert!(!dir.join(CONSOLE_LOG).exists());
        assert!(dir.join(BUILD_MODE_SEAL).exists());
    }

    /// A cursor into the rotated file reads its rest, then the new file, even
    /// once the new file has grown past the cursor's position in the old one.
    #[test]
    fn a_cursor_survives_a_rotation() {
        let t = tempfile::tempdir().unwrap();
        let old = ["a", "b", "c"].map(record).concat();
        let new = ["d", "e", "f", "g", "h"].map(record).concat();
        let console = console_over(t.path(), &old, &new);

        let (all, end) = read_records(&console, 0).unwrap();
        assert_eq!(lines(&all), ["a", "b", "c", "d", "e", "f", "g", "h"]);
        assert!(all.windows(2).all(|w| w[0].cursor < w[1].cursor));
        assert_eq!(end, (old.len() + new.len()) as u64);

        let (rest, _) = read_records(&console, all[0].cursor).unwrap();
        assert_eq!(lines(&rest), ["b", "c", "d", "e", "f", "g", "h"]);
        let (rest, _) = read_records(&console, all[2].cursor).unwrap();
        assert_eq!(lines(&rest), ["d", "e", "f", "g", "h"]);
        assert!(read_records(&console, end).unwrap().0.is_empty());
        // Another instance's cursor reads from the oldest record kept.
        assert_eq!(read_records(&console, end + 1).unwrap().0.len(), 8);
    }

    /// The real per-key operation over `<base>/<vm_id>/` with its declared
    /// control files; `initial` pre-exists as `console.log`, pinned at admission.
    fn admitted_console_lease(
        t: &Path,
        initial: &[u8],
    ) -> (crate::worker_admission::Lease, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let catalog = t.join("catalog");
        std::fs::create_dir(&catalog).unwrap();
        std::fs::set_permissions(&catalog, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&catalog).unwrap();
        let base = t.join("workers");
        let dir = base.join(vm_id("console"));
        private_dirs(&dir.join("root"));
        // As `write_control_admitted` creates it: a private log parent.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(dir.join(CONSOLE_LOG), initial).unwrap();
        std::fs::set_permissions(
            dir.join(CONSOLE_LOG),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let lease =
            admit_control_files(context.worker(&base, Some("console")).unwrap(), &dir).unwrap();
        (lease, dir)
    }

    /// The exact pair the launch path opens, through the same `open_console`.
    fn admitted_console_fixture(
        t: &Path,
        initial: &[u8],
    ) -> (crate::worker_admission::Lease, Console) {
        let (lease, dir) = admitted_console_lease(t, initial);
        let console = open_console(&lease, &dir).unwrap();
        (lease, console)
    }

    // Explicit regression budgets, not new production limits. A bounded log
    // capability may use a small fixed ledger, never a journal/path per line.
    fn r1_catalog_inventory(namespace: &Path) -> (usize, usize, u64) {
        let bytes = std::fs::read(namespace.join("catalog.json")).unwrap();
        let envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let catalog: serde_json::Value =
            serde_json::from_str(envelope["payload"].as_str().unwrap()).unwrap();
        let operations = catalog["operations"].as_array().unwrap();
        let entries = operations
            .iter()
            .map(|op| op["entries"].as_array().unwrap().len())
            .sum();
        let journals = operations
            .iter()
            .map(|op| op["journals"].as_array().unwrap().len())
            .sum();
        (entries, journals, bytes.len() as u64)
    }
    fn r1_assert_bounded(initial: (usize, usize, u64), now: (usize, usize, u64)) {
        assert!(
            now.0 <= initial.0 + 32,
            "live path cardinality grew per log record: {initial:?} -> {now:?}"
        );
        assert!(
            now.1 <= initial.1 + 16,
            "live journal count grew per log record: {initial:?} -> {now:?}"
        );
        assert!(
            now.2 <= initial.2 + 64 * 1024,
            "live catalog bytes grew per log record: {initial:?} -> {now:?}"
        );
    }

    #[tokio::test]
    async fn r1_three_hundred_console_records_rotate_and_retire_with_bounded_history() {
        let t = tempfile::tempdir().unwrap();
        let (lease, dir) = admitted_console_lease(t.path(), b"");
        let upper = dir.join("root").join(crate::cli::upper::UPPER_NAME);
        let mut create = lease
            .begin(crate::worker_admission::MutationPlan::Create { root: dir.clone() })
            .unwrap();
        let mut stage = create.stage_file(&upper, 0o600).unwrap();
        stage
            .file_mut()
            .write_all(b"OLD guest data must survive")
            .unwrap();
        create.publish_file(stage).unwrap();
        create.commit().unwrap();
        // The launch path issues the console writer before the runtime starts.
        let console = open_console(&lease, &dir).unwrap();
        let evidence = t.path().join(".iii-prior-unrelated-evidence");
        std::fs::write(&evidence, b"not log retention").unwrap();
        let session = crate::sandbox_daemon::adapters::runtime_test_session(
            lease.clone(),
            &t.path().join("catalog"),
            false,
        )
        .await;
        let receipt = session.wait().await.1.unwrap();
        session.finish_resources(&receipt).await.unwrap();
        let console = Mutex::new(console);
        let initial = r1_catalog_inventory(&t.path().join("catalog"));
        let mut seen_cursor = 0;
        for n in 0..320 {
            console
                .lock()
                .unwrap()
                .append(&LogStream::Stdout, format!("short-{n:04}").as_bytes())
                .unwrap_or_else(|error| panic!("normal admitted line {n} failed: {error}"));
            if n == 31 {
                seen_cursor = read_records(&console, 0).unwrap().1;
            }
        }
        let (rest, end) = read_records(&console, seen_cursor).unwrap();
        assert_eq!(rest.len(), 288);
        assert_eq!(rest.first().unwrap().line, "short-0032");
        assert_eq!(rest.last().unwrap().line, "short-0319");
        assert!(end > seen_cursor && rest.windows(2).all(|pair| pair[0].cursor < pair[1].cursor));
        r1_assert_bounded(initial, r1_catalog_inventory(&t.path().join("catalog")));
        let payload = vec![b'x'; MAX_LINE as usize - 1024];
        for _ in 0..200 {
            if console.lock().unwrap().base > 0 {
                break;
            }
            console
                .lock()
                .unwrap()
                .append(&LogStream::Stderr, &payload)
                .unwrap();
        }
        assert!(
            console.lock().unwrap().base > 0,
            "normal bounded rotation never occurred"
        );
        console
            .lock()
            .unwrap()
            .append(&LogStream::Stdout, b"after-rotation")
            .unwrap();
        let (after_rotation, final_cursor) = read_records(&console, end).unwrap();
        assert_eq!(after_rotation.last().unwrap().line, "after-rotation");
        assert!(
            after_rotation
                .windows(2)
                .all(|pair| pair[0].cursor < pair[1].cursor)
        );
        assert!(final_cursor > end);
        r1_assert_bounded(initial, r1_catalog_inventory(&t.path().join("catalog")));
        let data_bytes: u64 = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let meta = entry.metadata().unwrap();
                if meta.is_file() { meta.len() } else { 0 }
            })
            .sum();
        assert!(
            data_bytes <= 2 * CONSOLE_LOG_CAP + 4 * MAX_LINE,
            "ordinary log history is not bounded on disk: {data_bytes}"
        );
        console.lock().unwrap().finish().unwrap(); // last writer finished before H1/cleanup
        let cache = t.path().join("unrelated-cache");
        private_dirs(&cache); // a cache parent must be owned/protected
        receipt
            .probe_cache(
                &lease,
                &[crate::worker_admission::CacheEntry {
                    root: &cache.join("next"),
                }],
            )
            .unwrap()
            .cancel()
            .unwrap();
        drop(console); // all readable log handles closed before data cleanup
        let (removed, kept) =
            remove_runtime_state_admitted(&lease, &receipt, &dir, "console", false).unwrap();
        assert!(removed);
        assert!(kept.contains(&dir.join(PRE_BUILD_DISK)));
        assert_eq!(
            std::fs::read(dir.join(PRE_BUILD_DISK)).unwrap(),
            b"OLD guest data must survive"
        );
        assert_eq!(std::fs::read(evidence).unwrap(), b"not log retention");
        assert!(!dir.join(CONSOLE_LOG).exists());
        assert!(!dir.join(CONSOLE_LOG_OLD).exists());
    }

    #[test]
    fn r1_concurrent_valid_console_writers_preserve_all_records_and_bound_catalog() {
        let t = tempfile::tempdir().unwrap();
        let (_lease, console) = admitted_console_fixture(t.path(), b"");
        let console = Arc::new(Mutex::new(console));
        let initial = r1_catalog_inventory(&t.path().join("catalog"));
        std::thread::scope(|scope| {
            let tasks: Vec<_> = (0..4)
                .map(|writer| {
                    let console = console.clone();
                    scope.spawn(move || {
                        for n in 0..80 {
                            console
                                .lock()
                                .unwrap()
                                .append(&LogStream::Stdout, format!("{writer}:{n}").as_bytes())
                                .unwrap();
                        }
                    })
                })
                .collect();
            for task in tasks {
                task.join().unwrap();
            }
        });
        let (records, _) = read_records(&console, 0).unwrap();
        let lines: HashSet<_> = records.iter().map(|record| record.line.clone()).collect();
        assert_eq!(records.len(), 320);
        assert_eq!(lines.len(), 320);
        assert!(
            records
                .windows(2)
                .all(|pair| pair[0].cursor < pair[1].cursor)
        );
        r1_assert_bounded(initial, r1_catalog_inventory(&t.path().join("catalog")));
    }

    /// A real owned runtime child for `launch_admitted`: after an optional
    /// release file it prints `lines` stdout records (and every odd one on
    /// stderr), then activates as the runtime fixture and exits or holds.
    fn owned_console_child(
        lease: &crate::worker_admission::Lease,
        namespace: &Path,
        lines: usize,
        release: Option<&Path>,
        live: bool,
    ) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(
                r#"
            binary=$1; count=$2; release=$3; shift 3
            while [ "$#" -gt 0 ]; do
                case "$1" in
                  --admission-fd) export III_TEST_ADMISSION_FD="$2"; shift 2;;
                  --runtime-write-fd) export III_TEST_RUNTIME_FD="$2"; shift 2;;
                  *) exit 91;;
                esac
            done
            if [ -n "$release" ]; then
                while [ ! -e "$release" ]; do sleep 0.05; done
                echo after-release
            fi
            i=0
            while [ "$i" -lt "$count" ]; do
                echo "console-line-$i"
                [ $((i % 2)) -eq 0 ] || echo "console-err-$i" >&2
                i=$((i + 1))
            done
            exec "$binary" --exact sandbox_daemon::adapters::tests::runtime_owned_child_fixture --nocapture
        "#,
            )
            .arg("owned-console-fixture")
            .arg(std::env::current_exe().unwrap())
            .arg(lines.to_string())
            .arg(release.map_or_else(String::new, |p| p.display().to_string()));
        command
            .env("III_TEST_ADMISSION_NAMESPACE", namespace)
            .env("III_TEST_RUNTIME_LIVE", if live { "1" } else { "0" })
            .env(
                "III_TEST_RUNTIME_RESOURCES",
                serde_json::to_string(&crate::sandbox_daemon::adapters::RuntimeTestResources {
                    exit_without_drop: true,
                    ..Default::default()
                })
                .unwrap(),
            );
        crate::cli::worker_manager::libkrun::attach_child_admission(&mut command, lease).unwrap();
        command
    }

    fn owned_launch(
        vms: &WorkerVms,
        admission: &crate::worker_admission::Lease,
        command: tokio::process::Command,
    ) -> Result<WorkerStartResponse, SandboxError> {
        vms.launch_admitted(
            "k",
            command,
            Vec::new(),
            None,
            admission.clone(),
            Vec::new(),
            Some(Vec::new()),
            Vec::new(),
            Vec::new(),
        )
    }

    fn console_lines(records: &[LogRecord], prefix: &str) -> Vec<String> {
        records
            .iter()
            .filter(|record| record.line.starts_with(prefix))
            .map(|record| record.line.clone())
            .collect()
    }

    /// R1 through the production launch/waiter/destroy path: more records
    /// than the old 256-path/journal bound, read while live, then stop and
    /// destroy succeed because the waiter finished the one writer.
    #[tokio::test]
    async fn r1_routed_launch_logs_beyond_256_lines_then_stop_and_destroy_succeed() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let namespace = t.path().join("admission-catalog");
        let admission = vms.admission(Some("k")).unwrap();
        let initial = r1_catalog_inventory(&namespace);
        let child = owned_console_child(&admission, &namespace, 300, None, true);
        let started = owned_launch(&vms, &admission, child).unwrap();
        let id = started.vm_id.clone();
        let dir = t.path().join("workers").join(&id);
        let expected: Vec<_> = (0..300).map(|i| format!("console-line-{i}")).collect();
        let mut seen = Vec::new();
        for _ in 0..400 {
            seen = pumped(&vms, &logs_req("t", &id, false)).await;
            if console_lines(&seen, "console-line-").len() == 300 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(console_lines(&seen, "console-line-"), expected);
        assert_eq!(console_lines(&seen, "console-err-").len(), 150);
        assert!(seen.windows(2).all(|pair| pair[0].cursor < pair[1].cursor));
        r1_assert_bounded(initial, r1_catalog_inventory(&namespace));

        let status = vms
            .stop(stop_req("t", &id, started.instance, 0))
            .await
            .unwrap();
        assert!(matches!(status.state, State::Stopped), "{status:?}");
        wait_ended(&vms, &id, started.instance).await;
        // The settled pair still serves every record through its cursor.
        let settled = pumped(&vms, &logs_req("t", &id, false)).await;
        assert_eq!(console_lines(&settled, "console-line-"), expected);
        assert!(!std::fs::read_dir(&dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".iii-")
        }));
        r1_assert_bounded(initial, r1_catalog_inventory(&namespace));
        assert!(vms.destroy(destroy_req("t", "k")).unwrap().destroyed);
        assert!(!dir.exists());
    }

    /// R1 fault: a foreign writer swaps the admitted console inode while the
    /// VM runs. The append fails closed, the waiter cannot finish the log, no
    /// successful receipt exists, and destroy keeps the tree and foreign bytes.
    #[tokio::test]
    async fn r1_routed_foreign_console_replacement_fails_closed_and_retains_cleanup() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let namespace = t.path().join("admission-catalog");
        let admission = vms.admission(Some("k")).unwrap();
        let release = t.path().join("release");
        let child = owned_console_child(&admission, &namespace, 4, Some(&release), false);
        let started = owned_launch(&vms, &admission, child).unwrap();
        let dir = t.path().join("workers").join(&started.vm_id);
        let current = dir.join(CONSOLE_LOG);
        std::fs::rename(&current, dir.join("held-console")).unwrap();
        write_private(&current, b"FOREIGN\n");
        std::fs::write(&release, b"").unwrap();
        wait_ended(&vms, &started.vm_id, started.instance).await;
        assert_eq!(std::fs::read(&current).unwrap(), b"FOREIGN\n");
        assert!(
            std::fs::read(dir.join("held-console")).unwrap().is_empty(),
            "no record may land in the displaced inode after the fault"
        );
        assert!(vms.destroy(destroy_req("t", "k")).is_err());
        assert!(dir.exists());
        assert_eq!(std::fs::read(&current).unwrap(), b"FOREIGN\n");
    }

    /// R1 gate (expected FAIL until the core gate in the routing report is
    /// delivered): after destroy removed the exact pair, a relaunch on the SAME
    /// operation must re-issue one writer for the re-created pair instead of
    /// refusing on the finished ledger's old identities.
    #[tokio::test]
    async fn r1_gate_relaunch_after_destroy_reissues_the_console_writer() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let namespace = t.path().join("admission-catalog");
        let admission = vms.admission(Some("k")).unwrap();
        let child = owned_console_child(&admission, &namespace, 3, None, false);
        let first = owned_launch(&vms, &admission, child).unwrap();
        wait_ended(&vms, &first.vm_id, first.instance).await;
        assert!(vms.destroy(destroy_req("t", "k")).unwrap().destroyed);
        let child = owned_console_child(&admission, &namespace, 3, None, false);
        let again = owned_launch(&vms, &admission, child).unwrap();
        wait_ended(&vms, &again.vm_id, again.instance).await;
        let records = pumped(&vms, &logs_req("t", &again.vm_id, false)).await;
        assert_eq!(
            console_lines(&records, "console-line-"),
            ["console-line-0", "console-line-1", "console-line-2"]
        );
    }

    #[test]
    fn admitted_console_exact_files_do_not_conflict_with_prepared_guest_tree() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (lease, mut console) = admitted_console_fixture(t.path(), b"");
        let guest = console.dir.join("root");
        let runtime = lease
            .prepare_runtime(&[crate::worker_admission::RuntimePath {
                path: &guest,
                kind: crate::worker_admission::RuntimePathKind::GuestTree,
            }])
            .unwrap();
        console.append(&LogStream::Stdout, b"first").unwrap();
        assert_eq!(console.base, 0);
        assert!(console.len > 0);
        assert_eq!(
            std::fs::metadata(console.dir.join(CONSOLE_LOG))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        // The live writer is the pair's only effect: no second mutation of the
        // exact files is issued while it holds them.
        assert!(
            lease
                .begin(crate::worker_admission::MutationPlan::ControlFiles {
                    files: vec![
                        console.dir.join(CONSOLE_LOG),
                        console.dir.join(CONSOLE_LOG_OLD),
                    ],
                })
                .is_err()
        );
        let third = console.dir.join("not-declared");
        let mut exact = lease
            .begin(crate::worker_admission::MutationPlan::ControlFiles {
                files: vec![console.dir.join(STATE_FILE)],
            })
            .unwrap();
        assert!(exact.stage_file(&third, 0o600).is_err());
        assert!(!third.exists());
        drop(exact);
        drop(runtime);
        // Prepared-runtime conflict coverage, not a claim of actual guest execution.
    }

    #[test]
    fn admitted_console_rotation_publishes_private_files_before_advancing_cursor() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let t = tempfile::tempdir().unwrap();
        let initial = vec![b'x'; CONSOLE_LOG_CAP as usize];
        let (_lease, mut console) = admitted_console_fixture(t.path(), &initial);
        let inodes = [CONSOLE_LOG, CONSOLE_LOG_OLD]
            .map(|name| std::fs::metadata(console.dir.join(name)).unwrap().ino());
        console.append(&LogStream::Stderr, b"rotate").unwrap();
        let old = std::fs::read(console.dir.join(CONSOLE_LOG_OLD)).unwrap();
        assert!(old.starts_with(&initial));
        assert!(old.ends_with(b"\n"));
        assert_eq!(console.base, old.len() as u64);
        assert_eq!(console.len, 0);
        assert!(!console.failed);
        assert!(
            std::fs::read(console.dir.join(CONSOLE_LOG))
                .unwrap()
                .is_empty()
        );
        for (name, inode) in [CONSOLE_LOG, CONSOLE_LOG_OLD].into_iter().zip(inodes) {
            let meta = std::fs::metadata(console.dir.join(name)).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
            // In-place bounded rotation: the exact admitted inodes, no new
            // publication and no `.iii-*` stage/prior evidence per record.
            assert_eq!(meta.ino(), inode);
        }
        assert!(!std::fs::read_dir(&console.dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".iii-")
        }));
        console.finish().unwrap();
    }

    #[test]
    fn admitted_console_failures_never_advance_cursor_or_swap_the_readable_handle() {
        use std::os::unix::fs::MetadataExt;
        // Real faults at the caller boundary: a foreign current inode, a
        // foreign rotation target, and bytes written outside the owned writer.
        for fault in ["current", "previous", "external-bytes"] {
            let t = tempfile::tempdir().unwrap();
            let initial = vec![b'x'; CONSOLE_LOG_CAP as usize];
            let (_lease, mut console) = admitted_console_fixture(t.path(), &initial);
            let current = console.dir.join(CONSOLE_LOG);
            let previous = console.dir.join(CONSOLE_LOG_OLD);
            let held = console.dir.join("held");
            let inode = std::fs::metadata(&current).unwrap().ino();
            match fault {
                "current" => {
                    std::fs::rename(&current, &held).unwrap();
                    write_private(&current, b"");
                }
                "previous" => {
                    std::fs::rename(&previous, &held).unwrap();
                    write_private(&previous, b"");
                }
                _ => {
                    std::fs::OpenOptions::new()
                        .append(true)
                        .open(&current)
                        .unwrap()
                        .write_all(b"foreign\n")
                        .unwrap();
                }
            }
            assert!(
                console.append(&LogStream::Stderr, b"line").is_err(),
                "{fault}"
            );
            assert_eq!(console.len, CONSOLE_LOG_CAP, "{fault}");
            assert_eq!(console.base, 0, "{fault}");
            assert!(console.failed, "{fault}");
            // The admitted inode was neither written nor swapped for another.
            let (kept, expected) = match fault {
                "current" => (held.clone(), initial.clone()),
                "previous" => (current.clone(), initial.clone()),
                _ => (current.clone(), [&initial[..], &b"foreign\n"[..]].concat()),
            };
            assert_eq!(std::fs::metadata(&kept).unwrap().ino(), inode, "{fault}");
            assert_eq!(std::fs::read(&kept).unwrap(), expected, "{fault}");
            if fault == "previous" {
                assert!(std::fs::read(&held).unwrap().is_empty());
            }
            assert!(
                console
                    .append(&LogStream::Stdout, b"must not retry")
                    .is_err()
            );
            assert!(console.finish().is_err(), "{fault}");
        }
    }

    #[test]
    fn admitted_console_refuses_replacement_without_copying_into_it() {
        let t = tempfile::tempdir().unwrap();
        let (_lease, mut console) = admitted_console_fixture(t.path(), b"old\n");
        let path = console.dir.join(CONSOLE_LOG);
        std::fs::rename(&path, console.dir.join("held-old")).unwrap();
        write_private(&path, b"external\n");
        assert!(console.append(&LogStream::Stdout, b"forbidden").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"external\n");
        assert_eq!(
            std::fs::read(console.dir.join("held-old")).unwrap(),
            b"old\n"
        );
        assert_eq!(console.len, 4);
        assert_eq!(console.base, 0);
        assert!(console.failed);
        assert!(
            console.finish().is_err(),
            "a refused writer is never reported finished"
        );
    }

    /// Through `Console::append`: a reader a whole rotation behind still gets
    /// every record after its cursor, in order.
    #[test]
    fn a_reader_behind_a_rotation_loses_no_record() {
        let t = tempfile::tempdir().unwrap();
        let (_lease, console) = admitted_console_fixture(t.path(), b"");
        let console = Mutex::new(console);
        let append = |line: &str| {
            let mut console = console.lock().unwrap();
            console.append(&LogStream::Stdout, line.as_bytes()).unwrap();
        };
        append("first");
        let (_, cursor) = read_records(&console, 0).unwrap();
        // About 12 MB of whole records: past CONSOLE_LOG_CAP once. Fewer,
        // larger lines keep every record durable through the core writer.
        let filler = "x".repeat(30_000);
        let n = 400;
        for i in 0..n {
            append(&format!("{i} {filler}"));
        }
        assert!(console.lock().unwrap().base > 0, "the log did not rotate");

        let (recs, _) = read_records(&console, cursor).unwrap();
        assert_eq!(recs.len(), n);
        assert!(recs[0].line.starts_with("0 "));
        assert!(recs[n - 1].line.starts_with(&format!("{} ", n - 1)));
        console.lock().unwrap().finish().unwrap();
    }

    #[tokio::test]
    async fn follow_streams_until_the_instance_ends() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vms
            .launch("k", sh("echo a; sleep 0.3; echo b >&2"), Vec::new(), None)
            .unwrap()
            .vm_id;

        let recs = pumped(&vms, &logs_req("t", &id, true)).await;
        assert_eq!(lines(&recs), ["a", "b"]);
        let streams: Vec<_> = recs.iter().map(|r| format!("{:?}", r.stream)).collect();
        assert_eq!(streams, ["Stdout", "Stderr"]);
        let status = vms.status(status_req("t", &id, 1)).unwrap();
        assert!(status.ended_at_ms.is_some(), "{status:?}");
    }

    #[tokio::test]
    async fn follow_on_an_exited_instance_drains_and_returns() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vms
            .launch("k", sh("echo a"), Vec::new(), None)
            .unwrap()
            .vm_id;
        let status = wait_ended(&vms, &id, 1).await;
        assert!(matches!(status.state, State::Exited), "{status:?}");

        let req = logs_req("t", &id, true);
        let recs = tokio::time::timeout(Duration::from_secs(2), pumped(&vms, &req))
            .await
            .expect("follow on an exited instance must return");
        assert_eq!(lines(&recs), ["a"]);
    }

    /// The env reaches `__vm-boot` on stdin, never argv (MOT-5492).
    #[tokio::test]
    async fn launch_writes_the_env_frame_to_the_vm_stdin() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let env = vec![("API_KEY".to_string(), "sk-secret".to_string())];
        let frame = iii_supervisor::env_channel::encode_frame(&env).unwrap();
        // Drops the 4-byte length prefix; ends only at EOF, so the pipe was closed.
        let id = vms
            .launch("k", sh("tail -c +5; echo"), frame, None)
            .unwrap()
            .vm_id;
        wait_ended(&vms, &id, 1).await;
        let recs = pumped(&vms, &logs_req("t", &id, false)).await;
        assert_eq!(lines(&recs), [r#"[["API_KEY","sk-secret"]]"#]);
    }

    #[tokio::test]
    async fn a_pump_resumes_after_the_cursor_and_ends_without_follow() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vms
            .launch("k", sh("echo a; echo b; sleep 30"), Vec::new(), None)
            .unwrap()
            .vm_id;
        wait_record(&t.path().join("workers").join(&id), "b").await;

        let all = pumped(&vms, &logs_req("t", &id, false)).await;
        assert_eq!(lines(&all), ["a", "b"]);
        let mut req = logs_req("t", &id, false);
        req.cursor = Some(all[0].cursor);
        assert_eq!(lines(&pumped(&vms, &req).await), ["b"]);

        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    #[tokio::test]
    async fn a_sink_error_ends_the_pump() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vms
            .launch("k", sh("echo a; sleep 30"), Vec::new(), None)
            .unwrap()
            .vm_id;
        wait_record(&t.path().join("workers").join(&id), "a").await;

        let ended = vms
            .pump_logs(&logs_req("t", &id, true), |_| async {
                Err("channel closed".to_string())
            })
            .await;
        assert_eq!(code(ended), "S218");

        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    #[tokio::test]
    async fn a_scoped_token_controls_its_vm_and_no_other() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = Arc::new(vms(t, Some("t"), vec![t.into()]));
        let iii = IIIClient::new("ws://127.0.0.1:9");
        let mine = vms
            .launch("mine", sh("sleep 30"), Vec::new(), None)
            .unwrap()
            .vm_id;
        let other = vms
            .launch("other", sh("sleep 30"), Vec::new(), None)
            .unwrap()
            .vm_id;
        let token = scoped_auth_token("t", "mine");

        assert_eq!(code(vms.status(status_req(&token, &other, 1))), "S505");
        assert_eq!(code(vms.stop(stop_req(&token, &other, 1, 0)).await), "S505");
        assert_eq!(
            code(vms.logs(&iii, logs_req(&token, &other, false)).await),
            "S505"
        );
        assert_eq!(
            code(vms.start(start_req(&token, "other", local(t))).await),
            "S505"
        );
        assert_eq!(code(vms.destroy(destroy_req(&token, "other"))), "S505");

        // S502, not S505: it got past the token.
        assert_eq!(
            code(vms.start(start_req(&token, "mine", local(t))).await),
            "S502"
        );
        assert_eq!(code(vms.destroy(destroy_req(&token, "mine"))), "S502");
        let status = vms.status(status_req(&token, &mine, 1)).unwrap();
        assert!(matches!(status.state, State::Running), "{status:?}");
        vms.logs(&iii, logs_req(&token, &mine, false))
            .await
            .unwrap();
        let stopped = vms.stop(stop_req(&token, &mine, 1, 0)).await.unwrap();
        assert!(matches!(stopped.state, State::Stopped), "{stopped:?}");
        assert!(vms.destroy(destroy_req(&token, "mine")).unwrap().destroyed);

        // The project token still opens every VM.
        let status = vms.status(status_req("t", &other, 1)).unwrap();
        assert!(matches!(status.state, State::Running), "{status:?}");
        vms.stop(stop_req("t", &other, 1, 0)).await.unwrap();
    }

    /// An empty project token's MAC anyone can compute.
    #[tokio::test]
    async fn no_token_derives_from_an_absent_or_empty_project_token() {
        let t = tempfile::tempdir().unwrap();
        for token in [None, Some("")] {
            let vms = vms(t.path(), token, vec![t.path().into()]);
            let id = vms
                .launch("k", sh("exit 0"), Vec::new(), None)
                .unwrap()
                .vm_id;
            let forged = scoped_auth_token("", "k");
            assert_eq!(code(vms.status(status_req(&forged, &id, 1))), "S505");
            assert_eq!(code(vms.destroy(destroy_req(&forged, "k"))), "S505");
            assert_eq!(code(vms.status(status_req("", &id, 1))), "S505");
        }
    }

    #[tokio::test]
    async fn a_scoped_prepare_needs_its_own_state_key() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let empty = t.join("empty");
        std::fs::create_dir(&empty).unwrap();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let prepare = |auth: String, state_key: Option<&str>| {
            vms.prepare(WorkerPrepareRequest {
                auth,
                source: local(&empty),
                state_key: state_key.map(str::to_string),
            })
        };
        let token = scoped_auth_token("t", "k");
        assert_eq!(code(prepare(token.clone(), None).await), "S505");
        assert_eq!(code(prepare(token.clone(), Some("other")).await), "S505");
        assert_eq!(code(prepare(String::new(), Some("k")).await), "S505");
        // S501, not S505: it got past the token, to the empty source.
        assert_eq!(code(prepare(token, Some("k")).await), "S501");
        assert_eq!(code(prepare("t".to_string(), None).await), "S501");
        assert_eq!(code(prepare("t".to_string(), Some("k")).await), "S501");
    }

    /// `status`, `stop` and `logs` name the VM by `vm_id`, so only the record
    /// `launch` wrote ties a scoped token to it.
    #[tokio::test]
    async fn a_scoped_token_without_the_vms_record_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vms
            .launch("k", sh("exit 0"), Vec::new(), None)
            .unwrap()
            .vm_id;
        wait_ended(&vms, &id, 1).await;
        let token = scoped_auth_token("t", "k");
        vms.status(status_req(&token, &id, 1)).unwrap();

        let record = t.path().join("workers").join(&id).join(STATE_KEY_FILE);
        let mode = std::fs::metadata(&record).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_file(&record).unwrap();
        assert_eq!(code(vms.status(status_req(&token, &id, 1))), "S505");
        // The project token needs no record.
        vms.status(status_req("t", &id, 1)).unwrap();
    }

    /// `vm_id` reaches a path before anything authorized the caller, and `root/`
    /// is the guest's: nothing planted there may open a call, or even be read.
    #[tokio::test]
    async fn a_scoped_token_reads_no_record_outside_a_vms_own_dir() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let share = t.path().join("workers").join(vm_id("k")).join("root");
        std::fs::create_dir_all(&share).unwrap();
        std::fs::write(share.join(STATE_KEY_FILE), "k").unwrap();
        let token = scoped_auth_token("t", "k");
        let planted = format!("{}/root", vm_id("k"));
        assert_eq!(code(vms.status(status_req(&token, &planted, 1))), "S505");
    }

    /// A stand-in for the engine's channel endpoint. It passes on what each writer
    /// sends, and when notified closes the writer's socket, as the engine does
    /// once the reader has left.
    async fn channel_engine() -> (IIIClient, mpsc::UnboundedReceiver<Message>, Arc<Notify>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let iii = IIIClient::new(&format!("ws://{}", listener.local_addr().unwrap()));
        let (frames, received) = mpsc::unbounded_channel();
        let leave = Arc::new(Notify::new());
        let left = leave.clone();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                loop {
                    tokio::select! {
                        frame = ws.next() => match frame {
                            Some(Ok(frame)) => { let _ = frames.send(frame); }
                            _ => break,
                        },
                        () = left.notified() => {
                            let _ = ws.close(None).await;
                            break;
                        }
                    }
                }
            }
        });
        (iii, received, leave)
    }

    async fn next_frame(frames: &mut mpsc::UnboundedReceiver<Message>) -> Message {
        tokio::time::timeout(Duration::from_secs(10), frames.recv())
            .await
            .expect("no frame within 10 s")
            .expect("the engine stand-in ended")
    }

    /// The next `LogRecord` frame, skipping any other.
    async fn next_record(frames: &mut mpsc::UnboundedReceiver<Message>) -> LogRecord {
        loop {
            if let Message::Text(text) = next_frame(frames).await {
                return serde_json::from_str(&text).unwrap();
            }
        }
    }

    /// `sh` that prints `before`, waits for `go` to exist, then prints `after`
    /// and stays up, so a test decides what the log holds when it asks.
    fn gated(t: &Path, before: &str, after: &str) -> (tokio::process::Command, PathBuf) {
        let go = t.join("go");
        let script = format!(
            "{before}; while [ ! -e {go} ]; do sleep 0.02; done; echo {after}; sleep 30",
            go = go.display()
        );
        (sh(&script), go)
    }

    /// A follower's writer connects before the VM prints anything. The engine ends a connected
    /// writer's stream when this adapter goes away, and a supervisor waits on that end.
    #[tokio::test]
    async fn a_quiet_vm_connects_its_followers_writer_at_once() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = Arc::new(vms(t, Some("t"), vec![]));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let iii = IIIClient::new(&format!("ws://{}", listener.local_addr().unwrap()));
        let id = vms
            .launch("k", sh("sleep 30"), Vec::new(), None)
            .unwrap()
            .vm_id;
        vms.logs(&iii, logs_req("t", &id, true)).await.unwrap();
        let connected = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await;
        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
        assert!(
            connected.is_ok(),
            "the writer waited for a record to connect"
        );
    }

    /// A stalled channel handshake does not hold the follower back: a VM's lease waits for
    /// the follower, not for the engine to answer the upgrade.
    #[tokio::test]
    async fn a_stalled_channel_handshake_does_not_hold_the_follower_back() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let mut vms = vms(t, Some("t"), vec![]);
        vms.lease_grace = Duration::from_millis(900);
        let vms = Arc::new(vms);
        // Takes connections into its backlog and never answers the upgrade.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let iii = IIIClient::new(&format!("ws://{}", listener.local_addr().unwrap()));
        let id = vms
            .launch("k", sh("sleep 30"), Vec::new(), None)
            .unwrap()
            .vm_id;
        vms.logs(&iii, logs_req("t", &id, true)).await.unwrap();
        let Some(Slot::Booted(vm)) = vms.slots().get(&id).cloned() else {
            panic!("{id} is not booted");
        };
        tokio::time::sleep(Duration::from_millis(500)).await;
        let following = *vm.followers.borrow();
        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
        assert_eq!(
            following, 1,
            "the stalled connect kept the follower from registering"
        );
    }

    #[tokio::test]
    async fn from_end_starts_after_the_last_record_and_names_it() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = Arc::new(vms(t, Some("t"), vec![]));
        let (iii, mut frames, _) = channel_engine().await;
        let (cmd, go) = gated(t, "echo old", "new");
        let id = vms.launch("k", cmd, Vec::new(), None).unwrap().vm_id;
        let dir = t.join("workers").join(&id);
        wait_record(&dir, "old").await;
        let old = pumped(&vms, &logs_req("t", &id, false)).await;

        let mut req = logs_req("t", &id, true);
        req.from_end = true;
        // Ignored: honoured, it would replay the old record.
        req.cursor = Some(0);
        let reply = vms.logs(&iii, req).await.unwrap();
        assert_eq!(reply.cursor, Some(old[0].cursor));

        std::fs::write(&go, "").unwrap();
        let first = next_record(&mut frames).await;
        assert_eq!(first.line, "new", "the old record was replayed");

        // A stream reopened at the reply's cursor neither replays nor skips.
        let mut reopened = logs_req("t", &id, false);
        reopened.cursor = reply.cursor;
        assert_eq!(lines(&pumped(&vms, &reopened).await), ["new"]);

        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    /// An empty log has no last record, and what comes next is the stream's first.
    #[tokio::test]
    async fn from_end_on_an_empty_log_names_nothing_and_loses_nothing() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = Arc::new(vms(t, Some("t"), vec![]));
        let (iii, mut frames, _) = channel_engine().await;
        let (cmd, go) = gated(t, ":", "first");
        let id = vms.launch("k", cmd, Vec::new(), None).unwrap().vm_id;

        let mut req = logs_req("t", &id, true);
        req.from_end = true;
        let reply = vms.logs(&iii, req).await.unwrap();
        assert_eq!(reply.cursor, None);
        assert_eq!(serde_json::to_value(&reply).unwrap(), serde_json::json!({}));

        std::fs::write(&go, "").unwrap();
        assert_eq!(next_record(&mut frames).await.line, "first");
        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    /// Past a rotation the end still counts every byte, so it names the same
    /// record `read_records` does.
    #[test]
    fn the_end_of_a_rotated_log_is_a_read_cursor() {
        let t = tempfile::tempdir().unwrap();
        let console = console_over(t.path(), &record("a"), &record("b"));
        let (all, _) = read_records(&console, 0).unwrap();
        let end = console.lock().unwrap().end();
        assert_eq!(end, all[1].cursor);
        assert!(read_records(&console, end).unwrap().0.is_empty());
    }

    #[tokio::test]
    async fn without_from_end_the_stream_honours_the_cursor_and_names_nothing() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = Arc::new(vms(t, Some("t"), vec![]));
        let (iii, mut frames, _) = channel_engine().await;
        let id = vms
            .launch("k", sh("echo a; echo b; sleep 30"), Vec::new(), None)
            .unwrap()
            .vm_id;
        wait_record(&t.join("workers").join(&id), "b").await;

        let reply = vms.logs(&iii, logs_req("t", &id, true)).await.unwrap();
        assert_eq!(reply.cursor, None);
        assert_eq!(next_record(&mut frames).await.line, "a");
        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    /// A VM left to its lease, with `grace`.
    fn leased(t: &Path, script: &str, grace: Duration) -> (Arc<WorkerVms>, String) {
        let mut vms = vms(t, Some("t"), vec![]);
        vms.lease_grace = grace;
        let id = vms.launch("k", sh(script), Vec::new(), None).unwrap().vm_id;
        vms.arm_lease(&id);
        (Arc::new(vms), id)
    }

    /// Follows the VM, sinking records without fail, for `held`.
    async fn follow_for(vms: &WorkerVms, id: &str, held: Duration) {
        let req = logs_req("t", id, true);
        let pump = vms.pump_logs(&req, |_| async { Ok(()) });
        let _ = tokio::time::timeout(held, pump).await;
    }

    #[tokio::test]
    async fn a_lease_stops_a_vm_nothing_follows() {
        let t = tempfile::tempdir().unwrap();
        let (vms, id) = leased(t.path(), "sleep 30", Duration::from_millis(200));
        let status = wait_ended(&vms, &id, 1).await;
        assert!(matches!(status.state, State::Stopped), "{status:?}");
        assert_eq!(status.signal, Some(15), "{status:?}");
    }

    /// A guest that ignores SIGTERM cannot outlive its lease.
    #[tokio::test]
    async fn a_lease_kills_a_vm_that_ignores_its_stop() {
        let t = tempfile::tempdir().unwrap();
        let (vms, id) = leased(
            t.path(),
            "trap '' TERM; echo ready; sleep 60",
            Duration::from_millis(200),
        );
        let status = wait_ended_within(&vms, &id, LEASE_STOP_GRACE + Duration::from_secs(10)).await;
        assert!(matches!(status.state, State::Stopped), "{status:?}");
        assert_eq!(status.signal, Some(9), "{status:?}");
    }

    #[tokio::test]
    async fn a_follower_holds_a_lease_and_its_end_starts_the_grace() {
        let t = tempfile::tempdir().unwrap();
        let (vms, id) = leased(t.path(), "sleep 30", Duration::from_millis(300));
        follow_for(&vms, &id, Duration::from_millis(1000)).await;
        let status = vms.status(status_req("t", &id, 1)).unwrap();
        assert!(matches!(status.state, State::Running), "{status:?}");

        let status = wait_ended(&vms, &id, 1).await;
        assert!(matches!(status.state, State::Stopped), "{status:?}");
    }

    /// The last of several followers ending is what starts the grace.
    #[tokio::test]
    async fn a_lease_waits_for_the_last_follower() {
        let t = tempfile::tempdir().unwrap();
        let (vms, id) = leased(t.path(), "sleep 30", Duration::from_millis(300));
        tokio::join!(
            follow_for(&vms, &id, Duration::from_millis(100)),
            follow_for(&vms, &id, Duration::from_millis(1000)),
        );
        let status = vms.status(status_req("t", &id, 1)).unwrap();
        assert!(matches!(status.state, State::Running), "{status:?}");
        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    #[tokio::test]
    async fn a_reattach_within_the_grace_keeps_the_vm() {
        let t = tempfile::tempdir().unwrap();
        let (vms, id) = leased(t.path(), "sleep 30", Duration::from_millis(1000));
        follow_for(&vms, &id, Duration::from_millis(100)).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        // Well past the grace of the first follower's end.
        follow_for(&vms, &id, Duration::from_millis(1500)).await;
        let status = vms.status(status_req("t", &id, 1)).unwrap();
        assert!(matches!(status.state, State::Running), "{status:?}");
        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    #[tokio::test]
    async fn a_failing_sink_counts_as_a_gone_follower() {
        let t = tempfile::tempdir().unwrap();
        let (vms, id) = leased(t.path(), "echo a; sleep 30", Duration::from_millis(300));
        wait_record(&t.path().join("workers").join(&id), "a").await;
        let ended = vms
            .pump_logs(&logs_req("t", &id, true), |_| async {
                Err("channel closed".to_string())
            })
            .await;
        assert_eq!(code(ended), "S218");
        let status = wait_ended(&vms, &id, 1).await;
        assert!(matches!(status.state, State::Stopped), "{status:?}");
    }

    /// The case the lease exists for: the client was killed while the guest has
    /// nothing to say, so no record ever fails to send. The engine closes the
    /// writer's socket, and only a write can find out.
    #[tokio::test]
    async fn a_quiet_follower_whose_reader_left_counts_as_gone() {
        let t = tempfile::tempdir().unwrap();
        let (vms, id) = leased(t.path(), "sleep 30", Duration::from_millis(900));
        let (iii, mut frames, reader_left) = channel_engine().await;
        vms.logs(&iii, logs_req("t", &id, true)).await.unwrap();

        // Connected, and past the grace: the probes reach a live reader.
        next_frame(&mut frames).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let status = vms.status(status_req("t", &id, 1)).unwrap();
        assert!(matches!(status.state, State::Running), "{status:?}");

        reader_left.notify_one();
        let status = wait_ended(&vms, &id, 1).await;
        assert!(matches!(status.state, State::Stopped), "{status:?}");
    }

    /// Only records reach a stream of a VM without a lease, whatever it waits for.
    #[tokio::test]
    async fn a_vm_without_a_lease_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        let mut vms = vms(t.path(), Some("t"), vec![]);
        vms.lease_grace = Duration::from_millis(100);
        let vms = Arc::new(vms);
        let id = vms
            .launch("k", sh("sleep 30"), Vec::new(), None)
            .unwrap()
            .vm_id;
        let (iii, mut frames, reader_left) = channel_engine().await;
        vms.logs(&iii, logs_req("t", &id, true)).await.unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(frames.try_recv().is_err(), "a frame reached the reader");
        reader_left.notify_one();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let status = vms.status(status_req("t", &id, 1)).unwrap();
        assert!(matches!(status.state, State::Running), "{status:?}");
        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    /// An exit that needed no lease reads as the guest's own.
    #[tokio::test]
    async fn a_lease_leaves_an_exited_vm_alone() {
        let t = tempfile::tempdir().unwrap();
        let (vms, id) = leased(t.path(), "exit 3", Duration::from_millis(100));
        let status = wait_ended(&vms, &id, 1).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(matches!(status.state, State::Exited), "{status:?}");
        let status = vms.status(status_req("t", &id, 1)).unwrap();
        assert!(matches!(status.state, State::Exited), "{status:?}");
        assert_eq!(status.exit_code, Some(3), "{status:?}");
    }

    /// A 0.26 overlay VM dir: its disk in the trampoline `root/`.
    #[test]
    fn a_pre_build_overlay_disk_is_kept_beside_root() {
        let t = tempfile::tempdir().unwrap();
        let vm_dir = t.path().join("vm");
        std::fs::create_dir_all(vm_dir.join("root/runtime")).unwrap();
        std::fs::write(vm_dir.join("root/upper.ext4"), "0.26 data").unwrap();

        let (upper, notes) = adopt_build_mode(&vm_dir, "w").unwrap();

        let kept = vm_dir.join(PRE_BUILD_DISK);
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "0.26 data");
        assert_eq!(
            notes,
            [format!(
                "w: kept the pre-build data disk at {}",
                kept.display()
            )]
        );
        assert_eq!(upper, vm_dir.join("upper.ext4"));
        assert!(upper.is_file() && !vm_dir.join("root/upper.ext4").exists());
        // The trampoline stays: it is no rootfs.
        assert!(vm_dir.join("root/runtime").is_dir());
        assert!(!vm_dir.join(PRE_BUILD_ROOT).exists());
        assert!(vm_dir.join(BUILD_MODE_SEAL).is_file());
        assert!(!vm_dir.join("root").join(BUILD_MODE_SEAL).exists());

        // Sealed: the next start keeps the data disk and moves nothing.
        std::fs::write(vm_dir.join("root/upper.ext4"), "planted").unwrap();
        let len = std::fs::metadata(&upper).unwrap().len();
        let (again, notes) = adopt_build_mode(&vm_dir, "w").unwrap();
        assert_eq!((again, notes.len()), (upper.clone(), 0));
        assert_eq!(std::fs::metadata(&upper).unwrap().len(), len);
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "0.26 data");
    }

    /// A legacy VM dir: `root/` is the rootfs clone holding the guest's data.
    #[test]
    fn a_legacy_rootfs_clone_is_kept_as_root_pre_build() {
        let t = tempfile::tempdir().unwrap();
        let vm_dir = t.path().join("vm");
        for dir in ["root/bin", "root/usr/lib", "root/var/lib/app"] {
            std::fs::create_dir_all(vm_dir.join(dir)).unwrap();
        }
        std::fs::write(vm_dir.join("root/var/lib/app/db"), "rows").unwrap();

        let (upper, notes) = adopt_build_mode(&vm_dir, "w").unwrap();

        let kept = vm_dir.join(PRE_BUILD_ROOT);
        assert_eq!(
            std::fs::read_to_string(kept.join("var/lib/app/db")).unwrap(),
            "rows"
        );
        assert_eq!(
            notes,
            [format!(
                "w: kept the pre-build data disk at {}",
                kept.display()
            )]
        );
        let root: Vec<_> = std::fs::read_dir(vm_dir.join("root")).unwrap().collect();
        assert!(root.is_empty(), "root/ is not fresh");
        assert!(upper.is_file() && vm_dir.join(BUILD_MODE_SEAL).is_file());
    }

    /// A name already taken is never overwritten.
    #[test]
    fn a_kept_disk_never_replaces_another() {
        let t = tempfile::tempdir().unwrap();
        let vm_dir = t.path().join("vm");
        std::fs::create_dir_all(vm_dir.join("root")).unwrap();
        std::fs::write(vm_dir.join(PRE_BUILD_DISK), "older").unwrap();
        std::fs::write(vm_dir.join("root/upper.ext4"), "newer").unwrap();
        let (_, notes) = adopt_build_mode(&vm_dir, "w").unwrap();
        assert_eq!(
            std::fs::read_to_string(vm_dir.join(PRE_BUILD_DISK)).unwrap(),
            "older"
        );
        let second = vm_dir.join(format!("{PRE_BUILD_DISK}.1"));
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "newer");
        assert!(notes[0].ends_with(&second.display().to_string()));
    }

    fn build_ref(dir: &Path, build_id: &str) -> WorkerBuildRef {
        WorkerBuildRef {
            build_id: build_id.to_string(),
            build_dir: dir.display().to_string(),
        }
    }

    #[test]
    fn a_start_of_a_build_checks_image_id_and_refuses_with_s508() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let dir = t.join(".build/w");
        std::fs::create_dir_all(&dir).unwrap();
        let id = format!("sha256:{}", "a".repeat(64));

        // No image.id at all.
        assert_eq!(
            code(vms.admit_build("t", "k", &build_ref(&dir, &id))),
            "S508"
        );
        std::fs::write(dir.join(IMAGE_FILE), "image").unwrap();
        assert_eq!(
            code(vms.admit_build("t", "k", &build_ref(&dir, &id))),
            "S508"
        );
        // Another build's id.
        std::fs::write(
            dir.join(IMAGE_ID_FILE),
            format!("sha256:{}\n", "b".repeat(64)),
        )
        .unwrap();
        let err = vms
            .admit_build("t", "k", &build_ref(&dir, &id))
            .unwrap_err();
        assert_eq!(err.code().as_str(), "S508");
        assert!(err.to_string().contains(&"b".repeat(64)), "{err}");
        // A planted symlink is no id.
        std::fs::remove_file(dir.join(IMAGE_ID_FILE)).unwrap();
        let elsewhere = t.join("id");
        std::fs::write(&elsewhere, &id).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join(IMAGE_ID_FILE)).unwrap();
        assert_eq!(
            code(vms.admit_build("t", "k", &build_ref(&dir, &id))),
            "S300"
        );
        std::fs::remove_file(dir.join(IMAGE_ID_FILE)).unwrap();
        // The matching id, but no image.
        std::fs::write(dir.join(IMAGE_ID_FILE), format!("{id}\n")).unwrap();
        std::fs::remove_file(dir.join(IMAGE_FILE)).unwrap();
        assert_eq!(
            code(vms.admit_build("t", "k", &build_ref(&dir, &id))),
            "S508"
        );
        std::fs::write(dir.join(IMAGE_FILE), "image").unwrap();
        let image = vms.admit_build("t", "k", &build_ref(&dir, &id)).unwrap();
        assert_eq!(image, dir.canonicalize().unwrap().join(IMAGE_FILE));
        // Outside the host roots it is S501, before any id is read.
        let outside = vms_without_roots(t);
        assert_eq!(
            code(outside.admit_build("t", "k", &build_ref(&dir, &id))),
            "S501"
        );
    }

    fn vms_without_roots(t: &Path) -> WorkerVms {
        vms(t, Some("t"), vec![t.join("elsewhere")])
    }

    #[test]
    fn one_build_per_state_key_and_no_destroy_under_it() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vm_id("k");
        let held = vms.hold_build(&id).unwrap();
        assert_eq!(code(vms.hold_build(&id).map(drop)), "S509");
        assert_eq!(code(vms.destroy(destroy_req("t", "k"))), "S509");
        // Another key builds alongside.
        let other = vms.hold_build(&vm_id("other")).unwrap();
        drop(held);
        let again = vms.hold_build(&id).unwrap();
        drop((again, other));
        assert!(!vms.destroy(destroy_req("t", "k")).unwrap().destroyed);
    }

    /// `plan_output` creates nothing and refuses what is not a directory under
    /// the roots; `create_output` creates the planned directory, and refuses
    /// one a link redirected since.
    #[test]
    fn an_output_dir_is_planned_under_the_host_roots_and_created_after() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let project = t.join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(t.join("elsewhere")).unwrap();
        std::fs::write(project.join("file"), "").unwrap();
        let vms = vms(t, Some("t"), vec![project.clone()]);
        let out = project.join(".build/w");
        let planned = vms.plan_output(&out.display().to_string()).unwrap();
        assert_eq!(planned, project.canonicalize().unwrap().join(".build/w"));
        assert!(!project.join(".build").exists(), "a plan creates nothing");
        let got = vms.create_output(&planned, "w").unwrap();
        assert_eq!(got, out.canonicalize().unwrap());
        assert_eq!(vms.plan_output(&out.display().to_string()).unwrap(), got);
        for refused in [
            t.join("elsewhere/.build/w").display().to_string(),
            "relative/.build/w".to_string(),
            format!("{}/../escape", project.display()),
            project.join("file").display().to_string(),
            project.join("file/w").display().to_string(),
        ] {
            assert_eq!(code(vms.plan_output(&refused)), "S501", "{refused}");
        }
        assert!(!t.join("elsewhere/.build").exists() && !t.join("escape").exists());
        // A link put in the planned path between the plan and the create.
        let planned = vms
            .plan_output(&project.join("late/w").display().to_string())
            .unwrap();
        std::os::unix::fs::symlink(t.join("elsewhere"), project.join("late")).unwrap();
        assert_eq!(code(vms.create_output(&planned, "late/w")), "S501");
    }

    fn claim(vms: &WorkerVms, auth: &str, key: &str, dir: &Path) -> Result<(), SandboxError> {
        vms.claim_build_dir(auth, key, dir, "output_dir")
    }

    /// `<t>/project/build/<name>` for each name, created and canonical.
    fn build_dirs<const N: usize>(t: &Path, names: [&str; N]) -> [PathBuf; N] {
        names.map(|name| {
            let dir = t.join("project/build").join(name);
            private_dirs(&dir);
            dir.canonicalize().unwrap()
        })
    }

    fn record_of(t: &Path, key: &str) -> PathBuf {
        t.join("workers").join(vm_id(key)).join(BUILD_DIR_RECORD)
    }

    /// A scoped token never takes a directory another VM of the project
    /// recorded: S505, naming the record and the way out. The project token is
    /// never checked and records nothing.
    #[test]
    fn a_scoped_token_never_takes_a_dir_another_vm_recorded() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let [dir_a, dir_b, dir_c] = build_dirs(t, ["a", "b", "c"]);
        let [a, b, c, d, f] = ["a", "b", "c", "d", "f"].map(|key| scoped_auth_token("t", key));
        let first = vms(t, Some("t"), vec![t.into()]);

        claim(&first, &a, "a", &dir_a).unwrap();
        claim(&first, &b, "b", &dir_b).unwrap();
        claim(&first, &a, "a", &dir_a).unwrap();
        let err = claim(&first, &a, "a", &dir_b).unwrap_err();
        assert_eq!(err.code().as_str(), "S505", "{err}");
        let message = err.to_string();
        for part in [
            vm_id("b"),
            record_of(t, "b").display().to_string(),
            "destroy VM".to_string(),
            "delete that file".to_string(),
        ] {
            assert!(message.contains(&part), "{part}: {message}");
        }
        // C, which has recorded nothing, cannot take B's either.
        assert_eq!(code(claim(&first, &c, "c", &dir_b)), "S505");
        // The project token goes anywhere and records nothing.
        claim(&first, "t", "a", &dir_b).unwrap();
        claim(&first, "t", "c", &dir_c).unwrap();
        assert!(!record_of(t, "c").exists());

        // The records outlive the adapter process, private.
        let again = vms(t, Some("t"), vec![t.into()]);
        assert_eq!(code(claim(&again, &a, "a", &dir_b)), "S505");
        let mode = std::fs::metadata(record_of(t, "a"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        // A destroyed VM's directory is free again.
        assert!(again.destroy(destroy_req("t", "b")).unwrap().destroyed);
        claim(&again, &d, "d", &dir_b).unwrap();

        // A record the project token did not sign counts for nothing: under a
        // new token nobody holds a directory, and a forged record holds none.
        let rotated = vms(t, Some("u"), vec![t.into()]);
        claim(&rotated, &scoped_auth_token("u", "c"), "c", &dir_a).unwrap();
        store_build_dir(&t.join("workers"), &vm_id("e"), "e", &dir_c, "not-t").unwrap();
        claim(&again, &f, "f", &dir_c).unwrap();
    }

    /// Another directory moves the VM there and frees the one it leaves, even
    /// when that one stays on disk (Compose moved the container's build
    /// directory and kept the old one).
    #[test]
    fn a_vm_moves_to_another_dir_and_releases_the_old_one() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let [old, new] = build_dirs(t, ["old", "new"]);
        let (a, b) = (scoped_auth_token("t", "a"), scoped_auth_token("t", "b"));
        let vms = vms(t, Some("t"), vec![t.into()]);
        claim(&vms, &a, "a", &old).unwrap();
        claim(&vms, &a, "a", &new).unwrap();
        assert!(old.is_dir());
        assert_eq!(
            read_build_dir(&t.join("workers"), &vm_id("a"), "t"),
            Some(new.clone())
        );
        claim(&vms, &b, "b", &old).unwrap();
        assert_eq!(code(claim(&vms, &a, "a", &old)), "S505");
        claim(&vms, &a, "a", &new).unwrap();
    }

    /// A second spelling of one directory, same device and inode, is still the
    /// VM's that recorded it; the VM itself is recorded again under its own.
    #[cfg(target_os = "linux")]
    #[test]
    fn another_spelling_of_a_recorded_dir_is_still_that_vms() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let [dir_b] = build_dirs(t, ["b"]);
        let alias = PathBuf::from(format!("/proc/self/root{}", dir_b.display()));
        assert_ne!(alias, dir_b);
        let base = t.join("workers");
        store_build_dir(&base, &vm_id("b"), "b", &alias, "t").unwrap();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let a = scoped_auth_token("t", "a");
        assert_eq!(code(claim(&vms, &a, "a", &dir_b)), "S505");
        let b = scoped_auth_token("t", "b");
        claim(&vms, &b, "b", &dir_b).unwrap();
        assert_eq!(read_build_dir(&base, &vm_id("b"), "t"), Some(dir_b.clone()));
        assert_eq!(code(claim(&vms, &a, "a", &alias)), "S505");
    }

    /// A record counts only in its own VM's dir, and only as a regular file: a
    /// valid record copied into another VM's dir, or a `build-dir` that is a
    /// symlink to one, holds nothing, and a claim replaces the link itself.
    #[test]
    fn a_transplanted_or_linked_record_counts_for_nothing() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let [dir_a, dir_b, dir_c, dir_x] = build_dirs(t, ["a", "b", "c", "x"]);
        let [a, b, c, d] = ["a", "b", "c", "d"].map(|key| scoped_auth_token("t", key));
        let vms = vms(t, Some("t"), vec![t.into()]);
        let base = t.join("workers");

        claim(&vms, &b, "b", &dir_b).unwrap();
        private_dirs(&base.join(vm_id("a")));
        std::fs::copy(record_of(t, "b"), record_of(t, "a")).unwrap();
        assert_eq!(read_build_dir(&base, &vm_id("a"), "t"), None);
        // Read as VM a's, the copy would refuse B its own directory.
        claim(&vms, &b, "b", &dir_b).unwrap();
        assert_eq!(code(claim(&vms, &a, "a", &dir_b)), "S505");
        claim(&vms, &a, "a", &dir_a).unwrap();

        // C's record, valid, moved out and linked back.
        store_build_dir(&base, &vm_id("c"), "c", &dir_c, "t").unwrap();
        let target = t.join("elsewhere-record");
        std::fs::rename(record_of(t, "c"), &target).unwrap();
        std::os::unix::fs::symlink(&target, record_of(t, "c")).unwrap();
        let kept = std::fs::read(&target).unwrap();
        assert_eq!(read_build_dir(&base, &vm_id("c"), "t"), None);
        claim(&vms, &d, "d", &dir_c).unwrap();
        claim(&vms, &c, "c", &dir_x).unwrap();
        let record = std::fs::symlink_metadata(record_of(t, "c")).unwrap();
        assert!(record.is_file(), "{record:?}");
        assert_eq!(std::fs::read(&target).unwrap(), kept);
        assert_eq!(read_build_dir(&base, &vm_id("c"), "t"), Some(dir_x));
    }

    /// A scoped `worker::destroy` deletes its own VM's record only: another
    /// VM's directory stays out of reach.
    #[test]
    fn a_scoped_destroy_frees_only_its_own_dir() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let [dir_a, dir_b] = build_dirs(t, ["a", "b"]);
        let (a, b) = (scoped_auth_token("t", "a"), scoped_auth_token("t", "b"));
        let vms = vms(t, Some("t"), vec![t.into()]);
        claim(&vms, &a, "a", &dir_a).unwrap();
        claim(&vms, &b, "b", &dir_b).unwrap();
        assert!(vms.destroy(destroy_req(&a, "a")).unwrap().destroyed);
        assert!(!record_of(t, "a").exists());
        assert_eq!(code(claim(&vms, &a, "a", &dir_b)), "S505");
        claim(&vms, &a, "a", &dir_a).unwrap();
        // Nothing of the claims, lock included, outlives the last VM.
        for key in ["a", "b"] {
            assert!(vms.destroy(destroy_req("t", key)).unwrap().destroyed);
        }
        assert_eq!(std::fs::read_dir(t.join("workers")).unwrap().count(), 0);
    }

    /// Two VMs claim one directory at once, through two adapters on one base
    /// (two processes' worth: separate mutexes, one flock): one gets it, the
    /// other S505, every time.
    #[test]
    fn two_claims_of_one_dir_at_once_give_it_to_one_vm() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let first = vms(t, Some("t"), vec![t.into()]);
        let second = vms(t, Some("t"), vec![t.into()]);
        for round in 0..25 {
            let [dir] = build_dirs(t, [&format!("race-{round}")]);
            let keys = [format!("x{round}"), format!("y{round}")];
            let barrier = std::sync::Barrier::new(2);
            let results: Vec<_> = std::thread::scope(|scope| {
                let racers: Vec<_> = [&first, &second]
                    .into_iter()
                    .zip(&keys)
                    .map(|(vms, key)| {
                        let (barrier, dir) = (&barrier, &dir);
                        scope.spawn(move || {
                            let token = scoped_auth_token("t", key);
                            barrier.wait();
                            claim(vms, &token, key, dir)
                        })
                    })
                    .collect();
                racers.into_iter().map(|r| r.join().unwrap()).collect()
            });
            let won = results.iter().filter(|r| r.is_ok()).count();
            assert_eq!(won, 1, "round {round}: {results:?}");
            for lost in results.iter().filter_map(|r| r.as_ref().err()) {
                assert_eq!(lost.code().as_str(), "S505", "round {round}: {lost}");
            }
        }
    }

    /// `worker::start` takes `build_dir` through the same claim, before it
    /// reads image.id: another VM's image is S505, whatever the id. Aliases
    /// (a symlink, `..`) are refused by input admission before the claim.
    #[test]
    fn a_scoped_start_boots_no_other_vms_build_dir() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let [dir_a, dir_b] = build_dirs(t, ["a", "b"]);
        let id = format!("sha256:{}", "a".repeat(64));
        std::fs::write(dir_b.join(IMAGE_FILE), "image").unwrap();
        std::fs::write(dir_b.join(IMAGE_ID_FILE), format!("{id}\n")).unwrap();
        let link = t.join("link-to-b");
        std::os::unix::fs::symlink(&dir_b, &link).unwrap();
        let dotdot = PathBuf::from(format!("{}/../b", dir_a.display()));
        let (a, b) = (scoped_auth_token("t", "a"), scoped_auth_token("t", "b"));

        vms.admit_build(&b, "b", &build_ref(&dir_b, &id)).unwrap();
        assert_eq!(
            code(vms.admit_build(&a, "a", &build_ref(&dir_b, &id))),
            "S505"
        );
        // Admission now rejects aliases/traversal before claims, without
        // canonicalizing them into a spelling that evades the input fence.
        for path in [&link, &dotdot] {
            assert_eq!(
                code(vms.admit_build(&a, "a", &build_ref(path, &id))),
                "S300"
            );
        }
        assert_eq!(
            code(vms.admit_build(&a, "a", &build_ref(&dir_b, "sha256:00"))),
            "S505"
        );
        // S508, not S505: its own directory got past the claim.
        assert_eq!(
            code(vms.admit_build(&a, "a", &build_ref(&dir_a, &id))),
            "S508"
        );
        // The project token boots any directory under the roots, as before.
        vms.admit_build("t", "a", &build_ref(&dir_b, &id)).unwrap();
    }

    /// Claims still reject another VM with S505 after complete preflight.
    /// Use the claim helper directly: an empty source now refuses BEFORE a claim.
    #[test]
    fn a_scoped_build_claim_still_refuses_another_vms_directory() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![t.path().into()]);
        let out = t.path().join("output");
        std::fs::create_dir(&out).unwrap();
        let (a, b) = (scoped_auth_token("t", "a"), scoped_auth_token("t", "b"));
        let admission = vms.admission(Some("b")).unwrap();
        let admission = crate::worker_admission::with_inputs(admission, &[&out]).unwrap();
        claim(&vms, &b, "b", &out).unwrap();
        assert_eq!(code(claim(&vms, &a, "a", &out)), "S505");
        assert!(!vms.base.join(vm_id("a")).join(BUILD_DIR_RECORD).exists());
        assert_eq!(
            read_build_dir(&vms.base, &vm_id("b"), "t"),
            Some(out.clone())
        );
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
        drop(admission);
    }

    /// A VM dir as a VM that booted builds leaves it, the earlier adapter's
    /// disks kept beside it, and its runtime state.
    fn vm_with_data(t: &Path, vms: &WorkerVms, key: &str) -> PathBuf {
        let vm = t.join("workers").join(vm_id(key));
        private_dirs(&vm.join("root/opt/iii"));
        private_dirs(&vm.join("root.pre-build/bin"));
        private_dirs(&vm.join("build-1-2/root"));
        for (name, body) in [
            ("upper.ext4", "data"),
            ("upper.ext4.pre-build", "0.26 disk"),
            ("upper.ext4.pre-build.1", "older 0.26 disk"),
            ("build-mode", "1\n"),
            ("console.log", "log"),
            ("console.log.1", "old log"),
            ("state.json", r#"{"instance":3}"#),
            ("state_key", key),
            ("root/opt/iii/run.sh", "script"),
        ] {
            write_private(&vm.join(name), body);
        }
        let dir = t.join("project/build").join(key);
        private_dirs(&dir);
        claim(
            vms,
            &scoped_auth_token("t", key),
            key,
            &dir.canonicalize().unwrap(),
        )
        .unwrap();
        vm
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// Without `prune`, destroy removes the runtime state and the build-dir
    /// record, and keeps the data: the data disk, the earlier adapter's disks
    /// and the seal. The next start of the key boots over the same disk, and
    /// the build directory is free.
    #[test]
    fn a_destroy_without_prune_keeps_the_data_for_the_next_start() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let vm = vm_with_data(t, &vms, "k");
        let outside = t.join("outside");
        std::fs::write(&outside, "keep me").unwrap();
        std::os::unix::fs::symlink(&outside, vm.join("planted")).unwrap();

        let reply = vms.destroy(keep_req("t", "k")).unwrap();
        assert!(reply.destroyed);
        assert_eq!(
            reply.kept,
            [
                "root.pre-build",
                "upper.ext4",
                "upper.ext4.pre-build",
                "upper.ext4.pre-build.1"
            ]
            .map(|name| vm.join(name).display().to_string())
        );
        assert_eq!(
            names_in(&vm),
            [
                "build-mode",
                "root.pre-build",
                "upper.ext4",
                "upper.ext4.pre-build",
                "upper.ext4.pre-build.1"
            ]
        );
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep me");
        assert!(vm.join("root.pre-build/bin").is_dir());

        // The next start: the seal stands, nothing moves, the same disk.
        let (upper, notes) = adopt_build_mode(&vm, "w").unwrap();
        assert_eq!(upper, vm.join("upper.ext4"));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(std::fs::read_to_string(&upper).unwrap(), "data");
        // The build directory is free.
        let dir = t.join("project/build/k").canonicalize().unwrap();
        claim(&vms, &scoped_auth_token("t", "other"), "other", &dir).unwrap();
        // A second destroy finds data only: it keeps it, removes nothing, and
        // says what it kept.
        let reply = vms.destroy(keep_req("t", "k")).unwrap();
        assert!(!reply.destroyed);
        assert_eq!(reply.kept.len(), 4, "{:?}", reply.kept);
        assert!(vm.join("upper.ext4").is_file());
    }

    /// With `prune`, destroy leaves nothing, the data and the record included.
    #[test]
    fn a_destroy_with_prune_leaves_nothing() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let vm = vm_with_data(t, &vms, "k");
        let reply = vms.destroy(destroy_req("t", "k")).unwrap();
        assert!(reply.destroyed && reply.kept.is_empty(), "{reply:?}");
        assert!(!vm.exists());
        assert_eq!(std::fs::read_dir(t.join("workers")).unwrap().count(), 0);
        assert!(!vms.destroy(destroy_req("t", "k")).unwrap().destroyed);
        assert!(!vms.destroy(keep_req("t", "k")).unwrap().destroyed);
    }

    /// A VM with no data goes whole without `prune` too, seal included; a VM an
    /// earlier adapter ran keeps its disk as a pre-build copy, as its first
    /// build start would have.
    #[tokio::test]
    async fn a_destroy_without_prune_keeps_only_what_holds_data() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let id = vms
            .launch("empty", sh("exit 0"), Vec::new(), None)
            .unwrap()
            .vm_id;
        wait_ended(&vms, &id, 1).await;
        assert!(vms.destroy(keep_req("t", "empty")).unwrap().destroyed);
        assert!(!t.join("workers").join(&id).exists());
        // The same key boots on, as after a destroy with prune.
        let again = vms.launch("empty", sh("exit 0"), Vec::new(), None).unwrap();
        assert_eq!(again.instance, 2);
        wait_ended(&vms, &again.vm_id, 2).await;

        // A 0.26 overlay VM: its disk is `root/upper.ext4`, no seal yet.
        let old = t.join("workers").join(vm_id("old"));
        private_dirs(&old.join("root"));
        write_private(&old.join("root/upper.ext4"), "0.26 data");
        write_private(&old.join("console.log"), "log");
        assert!(vms.destroy(keep_req("t", "old")).unwrap().destroyed);
        assert_eq!(names_in(&old), ["build-mode", "upper.ext4.pre-build"]);
        assert_eq!(
            std::fs::read_to_string(old.join("upper.ext4.pre-build")).unwrap(),
            "0.26 data"
        );
        // A legacy rootfs clone: `root/` is the data.
        let legacy = t.join("workers").join(vm_id("legacy"));
        private_dirs(&legacy.join("root/bin"));
        write_private(&legacy.join("root/bin/x"), "x");
        // Nothing but data there: the clone moves aside, and no runtime state goes.
        assert!(!vms.destroy(keep_req("t", "legacy")).unwrap().destroyed);
        assert_eq!(names_in(&legacy), ["build-mode", "root.pre-build"]);
        assert!(legacy.join("root.pre-build/bin/x").is_file());
    }

    /// A binary package: `bin/worker`, executable, and optionally a manifest.
    fn binary_package(dir: &Path) -> WorkerSource {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let exe = dir.join("bin/worker");
        std::fs::write(&exe, "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        WorkerSource::Binary {
            package: "pkg".to_string(),
            host_path: dir.display().to_string(),
            executable: "bin/worker".to_string(),
        }
    }

    /// The executable must be a relative path of plain components naming an
    /// executable regular file inside the package, through no symlink; a
    /// manifest is optional, and one the package ships passes the bundle gates.
    #[test]
    fn a_binary_package_runs_its_executable_and_nothing_else() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let pkg = t.path().join("pkg");
        binary_package(&pkg);
        let project = validate_binary(&pkg, "pkg", "bin/worker").unwrap();
        assert_eq!(project.run_cmd, "exec '/workspace/bin/worker'");
        assert_eq!(
            (project.setup_cmd.as_str(), project.install_cmd.as_str()),
            ("", "")
        );
        assert!(project.env.is_empty() && project.base_image.is_none());
        let run = crate::cli::local_worker::build_image_run_script(&project, true);
        let last = run.lines().last().unwrap();
        assert!(
            last.starts_with("exec /bin/sh -c ") && last.contains("/workspace/bin/worker"),
            "{run}"
        );
        assert!(run.contains("code from the build image"), "{run}");
        let build = crate::cli::local_worker::build_image_script(&project, true);
        assert!(build.contains("--exclude=.iii"), "{build}");

        std::fs::write(pkg.join("data"), "").unwrap();
        std::fs::set_permissions(pkg.join("data"), std::fs::Permissions::from_mode(0o644)).unwrap();
        std::os::unix::fs::symlink("bin/worker", pkg.join("link")).unwrap();
        let outside = t.path().join("outside");
        std::fs::write(&outside, "").unwrap();
        std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o755)).unwrap();
        for (refused, why) in [
            ("", "relative path"),
            ("/bin/sh", "relative path"),
            ("../outside", "relative path"),
            ("./bin/worker", "relative path"),
            ("bin/missing", "cannot be read"),
            ("bin", "not a regular file"),
            ("data", "not executable"),
            ("link", "symlink"),
        ] {
            let err = validate_binary(&pkg, "pkg", refused)
                .err()
                .expect("refused");
            assert!(err.contains(why), "{refused:?}: {err}");
        }

        // A manifest: its install and env, under the bundle gates.
        std::fs::write(
            pkg.join("iii.worker.yaml"),
            "name: pkg\nscripts:\n  install: \"echo installed\"\nenv:\n  MODE: vm\n",
        )
        .unwrap();
        let project = validate_binary(&pkg, "pkg", "bin/worker").unwrap();
        assert_eq!(project.install_cmd, "echo installed");
        assert_eq!(project.env.get("MODE").map(String::as_str), Some("vm"));
        let err = validate_binary(&pkg, "other", "bin/worker")
            .err()
            .expect("refused");
        assert!(err.contains("manifest"), "{err}");
        std::fs::write(
            pkg.join("iii.worker.yaml"),
            "name: pkg\nscripts:\n  setup: \"x\"\n",
        )
        .unwrap();
        assert!(validate_binary(&pkg, "pkg", "bin/worker").is_err());
    }

    /// A build's base image: trimmed, a plausible reference, and required for a
    /// binary package; S001 otherwise.
    #[test]
    fn a_builds_base_image_is_checked_and_required_for_a_binary() {
        assert_eq!(build_base_image(None, false).unwrap(), None);
        assert_eq!(
            build_base_image(Some(" docker.io/iiidev/python:latest "), true).unwrap(),
            Some("docker.io/iiidev/python:latest")
        );
        assert_eq!(code(build_base_image(None, true)), "S001");
        assert_eq!(code(build_base_image(Some(""), false)), "S001");
        assert_eq!(
            code(build_base_image(Some("no spaces allowed"), false)),
            "S001"
        );
    }

    /// A binary package runs only from a build: prepare, and a start without
    /// `build`, are S501 before anything is read.
    #[tokio::test]
    async fn a_binary_package_is_neither_prepared_nor_started_without_a_build() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let source = binary_package(&t.join("pkg"));
        let err = vms
            .prepare(WorkerPrepareRequest {
                auth: "t".to_string(),
                source: source.clone(),
                state_key: None,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code().as_str(), "S501", "{err}");
        assert!(err.to_string().contains("boots only a build"), "{err}");
        let err = vms.start(start_req("t", "k", source)).await.unwrap_err();
        assert_eq!(err.code().as_str(), "S501", "{err}");
        assert!(err.to_string().contains("boots only a build"), "{err}");
    }

    /// `worker::build` of a binary package without a base image is S001 before
    /// anything is created; with one, the build goes on to the package checks.
    #[tokio::test]
    async fn a_binary_build_needs_a_base_image() {
        if !crate::cli::overlay::overlay_active() {
            eprintln!("[skip] a build needs the overlay rootfs (an embedded iii-init)");
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let iii = IIIClient::new("ws://127.0.0.1:9");
        let pkg = t.join("pkg");
        binary_package(&pkg);
        let build = |executable: &str, base_image: Option<&str>| {
            vms.build(
                &iii,
                WorkerBuildRequest {
                    auth: "t".to_string(),
                    state_key: "k".to_string(),
                    worker_name: "w".to_string(),
                    source: WorkerSource::Binary {
                        package: "pkg".to_string(),
                        host_path: pkg.display().to_string(),
                        executable: executable.to_string(),
                    },
                    output_dir: t.join("out").display().to_string(),
                    env: Default::default(),
                    logs: StreamChannelRef {
                        channel_id: "c".to_string(),
                        access_key: "k".to_string(),
                        direction: ChannelDirection::Write,
                    },
                    base_image: base_image.map(str::to_string),
                },
            )
        };
        let err = build("bin/worker", None).await.unwrap_err();
        assert_eq!(err.code().as_str(), "S001", "{err}");
        assert!(err.to_string().contains("needs base_image"), "{err}");
        assert!(!t.join("out").exists());
        let err = build("bin/missing", Some("docker.io/iiidev/python:latest"))
            .await
            .unwrap_err();
        assert_eq!(err.code().as_str(), "S501", "{err}");
        assert!(err.to_string().contains("bin/missing"), "{err}");
    }

    /// A base image named by the request, or a start of a build, lets a local
    /// worker's manifest leave `runtime.base_image` out; nothing else does.
    #[test]
    fn a_local_manifest_needs_no_base_image_when_the_request_names_one() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("w");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("iii.worker.yaml"),
            "name: w\nruntime:\n  kind: python\nscripts:\n  install: \"\"\n  start: \"python main.py\"\n",
        )
        .unwrap();
        let err = validate_source(&dir, None, None).err().expect("refused");
        assert!(err.contains("runtime.base_image"), "{err}");
        let (project, _) = validate_source_with(&dir, None, None, false).unwrap();
        assert_eq!(project.base_image, None);
    }

    /// A start without `build` boots on `root/upper.ext4`, a new disk once a
    /// destroy kept the previous one: a second destroy without `prune` moves it
    /// to the next pre-build name and lists it, never deletes it.
    #[tokio::test]
    async fn a_second_destroy_keeps_what_a_start_without_build_wrote() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let vm = vm_with_data(t, &vms, "k");
        assert!(vms.destroy(keep_req("t", "k")).unwrap().destroyed);

        let disk = vm.join("root").join(crate::cli::upper::UPPER_NAME);
        let script = format!(
            "mkdir -p '{}' && echo guest-wrote > '{}'",
            vm.join("root").display(),
            disk.display()
        );
        let started = vms.launch("k", sh(&script), Vec::new(), None).unwrap();
        wait_ended(&vms, &started.vm_id, started.instance).await;
        assert!(disk.is_file());

        let reply = vms.destroy(keep_req("t", "k")).unwrap();
        assert!(reply.destroyed);
        // `upper.ext4.pre-build` and `.1` were taken already.
        let moved = vm.join("upper.ext4.pre-build.2");
        assert!(
            reply.kept.contains(&moved.display().to_string()),
            "{:?}",
            reply.kept
        );
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "guest-wrote\n");
        assert!(!vm.join("root").exists());
        assert_eq!(
            std::fs::read_to_string(vm.join("upper.ext4")).unwrap(),
            "data"
        );
    }

    /// In the legacy mode every start clones a rootfs into `root/`: each
    /// destroy without `prune` moves the clone it finds to the next pre-build
    /// name, so no clone is ever deleted.
    #[test]
    fn a_destroy_without_prune_in_legacy_mode_keeps_every_clone() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let vm = t.join("workers").join(vm_id("legacy"));
        let clone = |body: &str| {
            private_dirs(&vm.join("root/bin"));
            write_private(&vm.join("root/bin/data"), body);
            write_private(&vm.join("console.log"), "log");
        };
        clone("first");
        let reply = vms.destroy(keep_req("t", "legacy")).unwrap();
        assert_eq!(
            reply.kept,
            [vm.join("root.pre-build").display().to_string()]
        );
        clone("second");
        let reply = vms.destroy(keep_req("t", "legacy")).unwrap();
        assert!(reply.destroyed);
        assert_eq!(
            reply.kept,
            ["root.pre-build", "root.pre-build.1"].map(|n| vm.join(n).display().to_string())
        );
        for (name, body) in [("root.pre-build", "first"), ("root.pre-build.1", "second")] {
            let data = std::fs::read_to_string(vm.join(name).join("bin/data")).unwrap();
            assert_eq!(data, body);
        }
    }

    /// A start reserves its VM before it claims the build directory: one that
    /// meets a destroy gets S502 and leaves no `build-dir` record behind.
    #[tokio::test]
    async fn a_start_during_a_destroy_records_no_build_dir() {
        if !crate::cli::overlay::overlay_active() {
            eprintln!("[skip] a start of a build needs the overlay rootfs (an embedded iii-init)");
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let [dir] = build_dirs(t, ["b"]);
        let id = format!("sha256:{}", "a".repeat(64));
        std::fs::write(dir.join(IMAGE_FILE), "image").unwrap();
        std::fs::write(dir.join(IMAGE_ID_FILE), format!("{id}\n")).unwrap();
        vms.slots().insert(vm_id("b"), Slot::Destroying(1));
        let req = WorkerStartRequest {
            build: Some(build_ref(&dir, &id)),
            ..start_req(&scoped_auth_token("t", "b"), "b", local(t))
        };
        assert_eq!(code(vms.start(req).await), "S502");
        assert!(!record_of(t, "b").exists());
    }

    /// An executable reached through a symlinked directory, inside the package
    /// or out of it, or under `.iii` in any case, is S501 material.
    #[test]
    fn an_executable_through_a_linked_dir_or_under_iii_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let pkg = t.path().join("pkg");
        binary_package(&pkg);
        let outside = t.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::copy(pkg.join("bin/worker"), outside.join("worker")).unwrap();
        std::fs::set_permissions(
            outside.join("worker"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::os::unix::fs::symlink(pkg.join("bin"), pkg.join("inner")).unwrap();
        std::os::unix::fs::symlink(&outside, pkg.join("out")).unwrap();
        for (refused, why) in [
            ("inner/worker", "symlink"),
            ("out/worker", "symlink"),
            (".iii/worker", "under `.iii`"),
            ("bin/.III/worker", "under `.iii`"),
            (".Iii", "under `.iii`"),
        ] {
            let err = validate_binary(&pkg, "pkg", refused)
                .err()
                .expect("refused");
            assert!(err.contains(why), "{refused:?}: {err}");
        }
        validate_binary(&pkg, "pkg", "bin/worker").unwrap();
    }

    /// A binary package's manifest is read as a bundle's: never through a
    /// symlink, and never past 64 KiB.
    #[test]
    fn a_binary_manifest_is_a_small_regular_file() {
        let t = tempfile::tempdir().unwrap();
        let pkg = t.path().join("pkg");
        binary_package(&pkg);
        let elsewhere = t.path().join("manifest.yaml");
        std::fs::write(&elsewhere, "name: pkg\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, pkg.join("iii.worker.yaml")).unwrap();
        let err = validate_binary(&pkg, "pkg", "bin/worker")
            .err()
            .expect("a linked manifest");
        assert!(err.contains("manifest"), "{err}");
        std::fs::remove_file(pkg.join("iii.worker.yaml")).unwrap();
        let big = format!("name: pkg\n#{}\n", "x".repeat(64 * 1024));
        std::fs::write(pkg.join("iii.worker.yaml"), big).unwrap();
        let err = validate_binary(&pkg, "pkg", "bin/worker")
            .err()
            .expect("an oversize manifest");
        assert!(err.contains("manifest"), "{err}");
    }

    /// A binary source outside a build is refused before its `host_path` is
    /// read: missing, or outside the host roots, it still gets that S501.
    #[tokio::test]
    async fn a_binary_outside_a_build_is_refused_before_its_path_is_read() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms_without_roots(t);
        let real = t.join("pkg");
        binary_package(&real);
        for host_path in [real.clone(), t.join("missing")] {
            let source = WorkerSource::Binary {
                package: "pkg".to_string(),
                host_path: host_path.display().to_string(),
                executable: "bin/worker".to_string(),
            };
            let err = vms
                .prepare(WorkerPrepareRequest {
                    auth: "t".to_string(),
                    source: source.clone(),
                    state_key: None,
                })
                .await
                .unwrap_err();
            assert_eq!(err.code().as_str(), "S501", "{err}");
            assert!(err.to_string().contains("boots only a build"), "{err}");
            let err = vms.start(start_req("t", "k", source)).await.unwrap_err();
            assert_eq!(err.code().as_str(), "S501", "{err}");
            assert!(err.to_string().contains("boots only a build"), "{err}");
        }
    }

    /// What iii-init's export writes: "./" and a file.
    fn export_tar(path: &Path, content: &str) {
        let header = |kind, mode, size| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(kind);
            h.set_mode(mode);
            h.set_uid(0);
            h.set_gid(0);
            h.set_mtime(0);
            h.set_size(size);
            h
        };
        let mut tar = tar::Builder::new(File::create(path).unwrap());
        let mut h = header(tar::EntryType::Directory, 0o755, 0);
        tar.append_data(&mut h, "./", io::empty()).unwrap();
        let mut h = header(tar::EntryType::Regular, 0o644, content.len() as u64);
        tar.append_data(&mut h, "version", content.as_bytes())
            .unwrap();
        tar.finish().unwrap();
    }

    fn image_write(t: &Path, content: &str, state: u8) -> (ImageWrite, Arc<AtomicU8>) {
        let tar = t.join(format!("root-{content}.tar"));
        export_tar(&tar, content);
        let export = File::open(&tar).unwrap();
        let base = t.join("base");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(
            base.join(".oci-config.json"),
            r#"{"config":{"Env":["A=1"]}}"#,
        )
        .unwrap();
        let out = t.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let state = Arc::new(AtomicU8::new(state));
        let write = ImageWrite {
            _admissions: Vec::new(),
            export,
            out,
            spool: t.join("spool"),
            base_rootfs: base,
            tag: content.to_string(),
            state: state.clone(),
        };
        (write, state)
    }

    #[test]
    fn an_image_lands_with_its_id_and_a_cancelled_one_leaves_the_previous_pair() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let (write, state) = image_write(t, "v1", BUILD_RUNNING);
        let id = write.write().unwrap();
        let out = t.join("out");
        let image = std::fs::read(out.join(IMAGE_FILE)).unwrap();
        assert_eq!(
            id,
            format!("sha256:{}", hex::encode(Sha256::digest(&image)))
        );
        assert_eq!(read_image_id(&out), Some(id.clone()));
        assert_eq!(state.load(Ordering::SeqCst), BUILD_COMMITTED);
        let reader = crate::cli::erofs::reader::read_file_from_erofs;
        assert_eq!(reader(&out.join(IMAGE_FILE), "/version").unwrap(), b"v1");
        assert_eq!(
            reader(&out.join(IMAGE_FILE), "/.oci-config.json").unwrap(),
            br#"{"config":{"Env":["A=1"]}}"#
        );

        // Cancelled before the renames: the previous pair stays, no temp file.
        let (write, _) = image_write(t, "v2", BUILD_CANCELLED);
        assert!(write.write().is_err());
        assert_eq!(std::fs::read(out.join(IMAGE_FILE)).unwrap(), image);
        assert_eq!(read_image_id(&out), Some(id));
        let names: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(!t.join("spool").exists());
    }

    /// A crash between the two renames, simulated by running the first step
    /// alone: the new image is in place without an id, so a start of the
    /// previous build or of the new one is S508, until a rebuild completes.
    #[test]
    fn a_crash_between_the_renames_leaves_no_id_and_a_start_gets_s508() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let (write, _) = image_write(t, "v1", BUILD_RUNNING);
        let previous = write.write().unwrap();
        let out = t.join("out");
        let vms = vms(t, Some("t"), vec![t.into()]);
        vms.admit_build("t", "k", &build_ref(&out, &previous))
            .unwrap();

        // The next build's two temporary files, its first step, and the crash.
        let image = out.join(format!(".{IMAGE_FILE}.v2.partial"));
        let id = out.join(format!(".{IMAGE_ID_FILE}.v2.partial"));
        std::fs::write(&image, "v2").unwrap();
        let next = format!("sha256:{}", hex::encode(Sha256::digest(b"v2")));
        std::fs::write(&id, format!("{next}\n")).unwrap();
        swap_image(&out, &image).unwrap();

        assert_eq!(std::fs::read(out.join(IMAGE_FILE)).unwrap(), b"v2");
        assert_eq!(read_image_id(&out), None);
        for build_id in [&previous, &next] {
            let err = vms
                .admit_build("t", "k", &build_ref(&out, build_id))
                .unwrap_err();
            assert_eq!(err.code().as_str(), "S508", "{err}");
            assert!(err.to_string().contains("is missing; build again"), "{err}");
        }

        // A rebuild puts a whole pair back.
        let (write, _) = image_write(t, "v3", BUILD_RUNNING);
        let rebuilt = write.write().unwrap();
        vms.admit_build("t", "k", &build_ref(&out, &rebuilt))
            .unwrap();
        assert_eq!(
            code(vms.admit_build("t", "k", &build_ref(&out, &previous))),
            "S508"
        );
    }

    /// A filesystem that cannot flush a directory gets a warning, not a failed
    /// build; an I/O error is still an error.
    #[test]
    fn a_directory_flush_the_filesystem_refuses_is_a_warning() {
        for unsupported in [libc::EINVAL, libc::ENOTSUP, libc::EOPNOTSUPP, libc::EBADF] {
            assert!(flush_unsupported(&io::Error::from_raw_os_error(
                unsupported
            )));
        }
        for failed in [libc::EIO, libc::ENOSPC, libc::EACCES] {
            assert!(!flush_unsupported(&io::Error::from_raw_os_error(failed)));
        }
    }

    /// A filesystem that cannot lock a directory (NFS) leaves the claims to the
    /// in-process mutex instead of failing them; other errors still fail.
    #[test]
    fn a_directory_lock_the_filesystem_refuses_falls_back_to_the_mutex() {
        for unsupported in [
            libc::ENOLCK,
            libc::EBADF,
            libc::ENOTSUP,
            libc::EOPNOTSUPP,
            libc::EINVAL,
        ] {
            assert!(lock_unsupported(&io::Error::from_raw_os_error(unsupported)));
        }
        for failed in [libc::EIO, libc::EACCES, libc::EINTR] {
            assert!(!lock_unsupported(&io::Error::from_raw_os_error(failed)));
        }
    }

    /// The build's run is dropped once its image is renamed into place, before
    /// it saw its writer end: the cancel gets the writer's build id, so the
    /// build reports the image it installed.
    #[tokio::test]
    async fn a_build_dropped_after_its_commit_still_has_its_build_id() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let (write, state) = image_write(t, "v1", BUILD_RUNNING);
        let slot = WriterSlot::default();
        let id_file = t.join("out").join(IMAGE_ID_FILE);
        // One poll puts the writer in its slot and leaves the run waiting on it.
        let mut run = Box::pin(run_writer(&slot, write));
        let first = std::future::poll_fn(|cx| std::task::Poll::Ready(run.as_mut().poll(cx))).await;
        assert!(
            first.is_pending(),
            "the writer ended within one poll: {first:?}"
        );
        // The writer commits while the run is never polled again, as when the
        // build's caller left: then the run goes. The writer flushes the image
        // and the directory, which a loaded disk can take a while to do.
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        while !id_file.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        drop(run);
        let written = cancel_writer(&slot, &state).await;
        assert!(id_file.exists(), "the writer never committed: {written:?}");
        assert_eq!(state.load(Ordering::SeqCst), BUILD_COMMITTED);
        let id = written.expect("the writer was still in its slot").unwrap();
        assert_eq!(read_image_id(&t.join("out")), Some(id));
    }

    #[test]
    fn build_scratch_is_preserved_without_owned_removal_authority() {
        let t = tempfile::tempdir().unwrap();
        let mine = t.path().join(format!("build-{}-0", std::process::id()));
        let stale = t.path().join("build-1-0");
        let root = t.path().join("root");
        let other = t.path().join("build-cache");
        for dir in [&mine, &stale, &root, &other] {
            std::fs::create_dir_all(dir).unwrap();
        }
        // The seal shares the prefix and is no scratch dir.
        std::fs::write(t.path().join(BUILD_MODE_SEAL), "1\n").unwrap();
        std::fs::write(t.path().join("build-2-0"), "a file").unwrap();
        sweep_build_scratch(t.path());
        assert!(mine.is_dir() && root.is_dir() && other.is_dir() && stale.exists());
        assert!(t.path().join(BUILD_MODE_SEAL).is_file());
        assert!(t.path().join("build-2-0").is_file());
    }

    /// A prior adapter's partial image is unresolved evidence, not permission
    /// for the next process to remove it by filename or diagnostic PID.
    #[test]
    fn the_next_build_preserves_unrelated_partial_image_evidence() {
        let t = tempfile::tempdir().unwrap();
        let mine = format!("{}-3", std::process::id());
        let names = [
            ".image.erofs.1-0.partial".to_string(),
            ".image.id.1-0.partial".to_string(),
            format!(".image.erofs.{mine}.partial"),
            format!(".image.id.{mine}.partial"),
            ".image.erofs.x-0.partial".to_string(),
            "image.erofs".to_string(),
            "image.id".to_string(),
            "build.json".to_string(),
        ];
        for name in &names {
            std::fs::write(t.path().join(name), "x").unwrap();
        }
        sweep_image_partials(t.path());
        for name in &names {
            assert!(
                t.path().join(name).exists(),
                "PID-only sweep must preserve {name}"
            );
        }
    }

    /// What `export_tar` writes, with `files` files of `size` bytes each.
    fn big_export_tar(path: &Path, files: usize, size: usize) {
        let mut tar = tar::Builder::new(File::create(path).unwrap());
        let data = vec![7u8; size];
        for i in 0..files {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Regular);
            h.set_mode(0o644);
            h.set_uid(0);
            h.set_gid(0);
            h.set_mtime(0);
            h.set_size(size as u64);
            tar.append_data(&mut h, format!("blob-{i}"), &data[..])
                .unwrap();
        }
        tar.finish().unwrap();
    }

    /// The build's run is dropped (its reader left) while its image is being
    /// written: the cancel waits for the writer, which stops and removes its
    /// temporary files, so nothing is left once the key is free, and the
    /// previous pair stays.
    #[tokio::test]
    async fn a_build_cancelled_while_writing_its_image_waits_for_the_writer() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let (write, _) = image_write(t, "v1", BUILD_RUNNING);
        let id = write.write().unwrap();
        let out = t.join("out");
        let image = std::fs::read(out.join(IMAGE_FILE)).unwrap();

        let (mut write, state) = image_write(t, "v2", BUILD_RUNNING);
        let tar = t.join("big.tar");
        big_export_tar(&tar, 8, 4 << 20);
        write.export = File::open(&tar).unwrap();
        let slot = WriterSlot::default();
        let partial = out.join(format!(".{IMAGE_FILE}.v2.partial"));
        let writing = async {
            while !partial.exists() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        tokio::select! {
            done = run_writer(&slot, write) => panic!("the image was written before the cancel: {done:?}"),
            () = writing => {}
        }
        let written = cancel_writer(&slot, &state).await;
        assert!(matches!(written, Some(Err(_))), "{written:?}");

        assert_eq!(state.load(Ordering::SeqCst), BUILD_CANCELLED);
        let mut left: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, [IMAGE_FILE, IMAGE_ID_FILE]);
        assert_eq!(std::fs::read(out.join(IMAGE_FILE)).unwrap(), image);
        assert_eq!(read_image_id(&out), Some(id));
        assert!(!t.join("spool").exists());
    }

    #[test]
    fn vm_id_is_stable_and_32_hex() {
        assert_eq!(vm_id("a"), vm_id("a"));
        assert_eq!(vm_id("a").len(), 32);
        assert!(vm_id("a").bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(vm_id("a"), vm_id("b"));
    }
}
