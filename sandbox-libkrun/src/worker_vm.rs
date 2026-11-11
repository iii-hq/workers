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
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::watch;

use crate::cli::local_worker::{
    BootImage, VmOverride, bundle_vm_command, image_build_command, local_vm_command,
    open_build_export, validate_source,
};
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
    /// Per-VM state lives in `<base>/<vm_id>/`: the control files, and the VM's
    /// writable share `root/`, which a uid-0 guest controls.
    base: PathBuf,
    token: Option<String>,
    roots: Vec<PathBuf>,
    /// Keyed by `vm_id`. An exited VM stays, so `status` can report it.
    slots: Mutex<HashMap<String, Slot>>,
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
    pid: Pid,
    started_at_ms: u64,
    /// Set by `stop` before it signals, so the exit reads as `Stopped`.
    stopping: AtomicBool,
    /// Set once the waiter reaps the process: its pid may then be reused.
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
            base,
            token,
            roots,
            slots: Mutex::default(),
            starts: AtomicU64::new(0),
            unreserved: tokio::sync::Notify::new(),
            destroyed: Mutex::default(),
            builds: Mutex::default(),
            build_seq: AtomicU64::new(0),
            claims: Mutex::default(),
            lease_grace: LEASE_GRACE,
        }
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
        let (package, host_path, run_override) = source_parts(&req.source);
        let dir = self.admit(host_path)?;
        let (project, kind) =
            validate_source(&dir, package, run_override).map_err(SandboxError::SourceRejected)?;
        let rootfs = oci::prepare_rootfs(&kind, project.base_image.as_deref())
            .await
            .map_err(|e| SandboxError::PrepareFailed(format!("{e:#}")))?;
        if crate::cli::overlay::overlay_active() {
            tokio::task::spawn_blocking(move || crate::cli::erofs::ensure_base_erofs(&rootfs))
                .await
                .map_err(|e| SandboxError::PrepareFailed(e.to_string()))?
                .map_err(SandboxError::PrepareFailed)?;
        }
        // For a client reattaching to a live boot: its `start` writes nothing, it gets S502.
        if let Some(state_key) = &req.state_key {
            let vm_dir = self.base.join(vm_id(state_key));
            std::fs::create_dir_all(&vm_dir)
                .and_then(|()| store_state_key(&vm_dir, state_key))
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
        self.authorize(&req.auth, Some(&req.state_key))?;
        // Compose presents the project token and supervises its VMs itself.
        let leased = !self.is_project_token(&req.auth);
        let (package, host_path, run_override) = source_parts(&req.source);
        let dir = self.admit(host_path)?;
        let image = match &req.build {
            Some(_) if !crate::cli::overlay::overlay_active() => {
                return Err(SandboxError::BootFailed(LEGACY_NO_BUILD.to_string()));
            }
            Some(build) => Some(self.admit_build(&req.auth, &req.state_key, build)?),
            None => None,
        };
        let id = vm_id(&req.state_key);
        let start = self.starts.fetch_add(1, Ordering::SeqCst);
        {
            let mut slots = self.slots();
            refuse_busy(&id, slots.get(&id))?;
            let next = self.next_instance(&id)?;
            slots.insert(id.clone(), Slot::Preparing(next, start, false));
        }
        let _unreserve = Unreserve(self, &id, start);
        validate_source(&dir, package, run_override).map_err(SandboxError::SourceRejected)?;
        let mut notes = Vec::new();
        let image = match image {
            Some(image) => {
                let vm_dir = self.base.join(&id);
                let (upper, kept) = adopt_build_mode(&vm_dir, &req.worker_name).map_err(|e| {
                    SandboxError::BootFailed(format!(
                        "cannot set up the data disk in {}: {e}",
                        vm_dir.display()
                    ))
                })?;
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
        let built = match package {
            Some(package) => bundle_vm_command(package, &req.worker_name, &dir, over).await,
            None => local_vm_command(&req.worker_name, &dir, run_override, over).await,
        }
        .map_err(|e| SandboxError::BootFailed(with_notes(e, &notes)))?;
        let started = self
            .launch(&req.state_key, built.command, built.env_stdin, Some(start))
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
        let dir = self.admit(&build.build_dir)?;
        self.claim_build_dir(auth, state_key, &dir, "build_dir")?;
        let wanted = build.build_id.trim();
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
        self.authorize(&req.auth, Some(&req.state_key))?;
        if !crate::cli::overlay::overlay_active() {
            return Err(SandboxError::BuildFailed(LEGACY_NO_BUILD.to_string()));
        }
        let (package, host_path, run_override) = source_parts(&req.source);
        let dir = self.admit(host_path)?;
        let planned = self.plan_output(&req.output_dir)?;
        let id = vm_id(&req.state_key);
        // Dropped last of the locals below: the key is free only once the
        // writer stopped and the scratch dir is gone.
        let _held = self.hold_build(&id)?;
        // Under the hold, so no `destroy` removes the claim while this build
        // writes, and before the directory exists: a refused token creates nothing.
        self.claim_build_dir(&req.auth, &req.state_key, &planned, "output_dir")?;
        let out = self.create_output(&planned, &req.output_dir)?;
        validate_source(&dir, package, run_override).map_err(SandboxError::SourceRejected)?;
        let vm_dir = self.base.join(&id);
        sweep_build_scratch(&vm_dir);
        sweep_image_partials(&out);
        let tag = format!(
            "{}-{}",
            std::process::id(),
            self.build_seq.fetch_add(1, Ordering::SeqCst)
        );
        let scratch = vm_dir.join(format!("{BUILD_SCRATCH_PREFIX}{tag}"));
        let _scratch = RemoveOnDrop(scratch.clone());
        let writer = ChannelWriter::new(iii.address(), &req.logs);
        let mut log = BuildLog::new(&writer);
        let state = Arc::new(AtomicU8::new(BUILD_RUNNING));
        let image_writer = WriterSlot::default();
        let done = Mutex::new(BuildDone::default());
        let job = BuildJob {
            worker_name: &req.worker_name,
            dir: &dir,
            package,
            run_override,
            out: &out,
            scratch: &scratch,
            tag: &tag,
            env: req.env.clone().into_iter().collect(),
            state: state.clone(),
            image_writer: &image_writer,
            done: &done,
        };
        let mut built = tokio::select! {
            built = job.run(&mut log) => built,
            gone = reader_gone(&writer, BUILD_KEEPALIVE) => Err(SandboxError::BuildFailed(
                format!("build cancelled: the reader of `logs` is gone ({gone})")
            )),
        };
        if built.is_err() {
            // The build VM died with the dropped run; a writer still in its
            // blocking part stops, renames nothing and removes its files.
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

    /// Reserve `vm_id` for one build: S509 while another build holds it, S502
    /// while it is being destroyed.
    fn hold_build(&self, vm_id: &str) -> Result<BuildHold<'_>, SandboxError> {
        let slots = self.slots();
        if let Some(Slot::Destroying(instance)) = slots.get(vm_id) {
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
    /// path, `.` or `..`, an ancestor outside the roots, or a file.
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
        let planned = self.admit(existing)?.join(below);
        if below.as_os_str().is_empty() && !planned.is_dir() {
            return Err(SandboxError::SourceRejected(format!(
                "output_dir {output_dir} is not a directory"
            )));
        }
        Ok(planned)
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
    /// The check and the write hold the claims mutex and an flock on
    /// `<base>/.build-dir.lock`, so two claims of one directory, from two threads
    /// or two adapter processes, never both succeed. The project token passes
    /// and records nothing. `field` names the request field in errors.
    fn claim_build_dir(
        &self,
        auth: &str,
        state_key: &str,
        dir: &Path,
        field: &str,
    ) -> Result<(), SandboxError> {
        if self.is_project_token(auth) {
            return Ok(());
        }
        // A scoped token that got here derives from a non-empty project token.
        let token = self.token.as_deref().unwrap_or("");
        let id = vm_id(state_key);
        let _claims = self.claims.lock().unwrap_or_else(PoisonError::into_inner);
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
        store_build_dir(&self.base, &id, state_key, dir, token).map_err(|e| {
            SandboxError::FsIo(format!("cannot record the build directory of VM {id}: {e}"))
        })
    }

    /// Spawn `cmd` as the next instance of `state_key`'s VM, its output pumped
    /// into `console.log`, and write `env_stdin` (the `--env-stdin` frame) to
    /// its stdin. `start` names the `start` call whose reservation this fills.
    pub(crate) fn launch(
        &self,
        state_key: &str,
        cmd: Command,
        env_stdin: Vec<u8>,
        start: Option<u64>,
    ) -> Result<WorkerStartResponse, SandboxError> {
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
        let (console, lifeline, mut child) =
            spawn(&dir, state_key, instance, cmd).map_err(|e| {
                SandboxError::BootFailed(format!("cannot launch VM in {}: {e}", dir.display()))
            })?;
        let console = Arc::new(Mutex::new(Console {
            dir,
            file: console,
            len: 0,
            base: 0,
        }));
        let vm = Arc::new(Vm {
            instance,
            pid: Pid::from_raw(child.id().expect("a just-spawned child has a pid") as i32),
            started_at_ms: now_ms(),
            stopping: AtomicBool::new(false),
            reaped: AtomicBool::new(false),
            ended: watch::Sender::new(None),
            console: console.clone(),
            followers: watch::Sender::new(0),
            leased: AtomicBool::new(false),
        });
        let stdin = child.stdin.take();
        tokio::spawn(async move {
            // On failure the VM gets a truncated frame and exits by itself,
            // with the reason in its console.log.
            if let Err(e) =
                crate::cli::worker_manager::libkrun::feed_env_stdin(stdin, &env_stdin).await
            {
                tracing::warn!(error = %e, "cannot send the worker environment to the VM");
            }
        });
        let pumps = [
            tokio::spawn(pump(
                child.stdout.take().expect("stdout is piped"),
                LogStream::Stdout,
                console.clone(),
            )),
            tokio::spawn(pump(
                child.stderr.take().expect("stderr is piped"),
                LogStream::Stderr,
                console,
            )),
        ];
        let waited = vm.clone();
        tokio::spawn(async move {
            let status = child.wait().await.ok();
            waited.reaped.store(true, Ordering::SeqCst);
            drop(lifeline);
            // So an exited VM's log is complete.
            let _ = tokio::time::timeout(DRAIN_GRACE, futures::future::join_all(pumps)).await;
            waited.ended.send_replace(Some(Ended {
                exit_code: status.and_then(|s| s.code()),
                signal: status.and_then(|s| s.signal()),
                at_ms: now_ms(),
            }));
        });
        slots.insert(id.clone(), Slot::Booted(vm));
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
        vm.stop(Duration::from_millis(req.grace_ms)).await;
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

    /// Remove `state_key`'s VM state. S502 while it is preparing, running or
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
        let (prev, last) = {
            let mut slots = self.slots();
            refuse_busy(&id, slots.get(&id))?;
            // The build writes in `<vm_id>/`, so it must end first.
            if self.builds().contains(&id) {
                return Err(SandboxError::BuildInProgress(id));
            }
            let instance = slots.get(&id).map_or(0, Slot::instance);
            // A daemon restarted since the last boot knows it only from disk.
            let last = instance.max(stored_instance(&dir).unwrap_or(0));
            (slots.insert(id.clone(), Slot::Destroying(instance)), last)
        };
        let removed = std::fs::remove_dir_all(&dir);
        let mut slots = self.slots();
        slots.remove(&id);
        // Even on failure: the removal may have reached `state.json` already.
        if last > 0 {
            self.destroyed().insert(id.clone(), last);
        }
        let destroyed = match removed {
            Ok(()) => true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => false,
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
        Ok(WorkerDestroyResponse { destroyed })
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
    dir: &'a Path,
    package: Option<&'a str>,
    run_override: Option<&'a str>,
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
}

impl BuildJob<'_> {
    async fn run(self, log: &mut BuildLog<'_>) -> Result<WorkerBuildResponse, SandboxError> {
        std::fs::create_dir_all(self.scratch).map_err(|e| {
            SandboxError::BuildFailed(format!("cannot create {}: {e}", self.scratch.display()))
        })?;
        log.note(format!(
            "iii: building {} from {}",
            self.worker_name,
            self.dir.display()
        ))
        .await?;
        let vm = image_build_command(
            self.dir,
            self.package,
            self.run_override,
            self.scratch,
            self.env,
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
        let status = run_build_vm(vm.command, log).await?;
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
        let export = open_build_export(&vm.root).map_err(|e| {
            SandboxError::BuildFailed(format!("the build VM left no image root: {e}"))
        })?;
        let image = ImageWrite {
            export,
            out: self.out.to_path_buf(),
            spool: self.scratch.join("image.spool"),
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

/// The temporary image files a build of an earlier adapter process left in
/// `out` (it died while writing): `.image.erofs.<pid>-<n>.partial` and
/// `.image.id.<pid>-<n>.partial` of another pid. This process's own may be a
/// running build's.
fn sweep_image_partials(out: &Path) {
    let Ok(entries) = std::fs::read_dir(out) else {
        return;
    };
    let mine = std::process::id().to_string();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(tag) = name.to_str().and_then(|name| {
            [IMAGE_FILE, IMAGE_ID_FILE].iter().find_map(|file| {
                name.strip_prefix(&format!(".{file}."))?
                    .strip_suffix(".partial")
            })
        }) else {
            continue;
        };
        if pid_seq(tag).is_some_and(|pid| pid != mine) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The pid of a `<pid>-<seq>` build tag.
fn pid_seq(tag: &str) -> Option<&str> {
    let (pid, seq) = tag.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(pid) && digits(seq)).then_some(pid)
}

/// Spawn a build VM and stream its output into `log` until it exits. Dropped
/// early, it kills the VM.
async fn run_build_vm(
    vm: crate::cli::worker_manager::libkrun::VmCommand,
    log: &mut BuildLog<'_>,
) -> Result<std::process::ExitStatus, SandboxError> {
    let mut cmd = vm.command;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let spawned = crate::daemon_exit::attach_lifeline(&mut cmd)
        .and_then(|lifeline| Ok((lifeline, cmd.spawn()?)));
    let (_lifeline, mut child) = spawned
        .map_err(|e| SandboxError::BuildFailed(format!("cannot launch the build VM: {e}")))?;
    let stdin = child.stdin.take();
    let env_stdin = vm.env_stdin;
    tokio::spawn(async move {
        if let Err(e) = crate::cli::worker_manager::libkrun::feed_env_stdin(stdin, &env_stdin).await
        {
            tracing::warn!(error = %e, "cannot send the build environment to the VM");
        }
    });
    let (lines, mut received) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(forward_lines(
        child.stdout.take().expect("stdout is piped"),
        LogStream::Stdout,
        lines.clone(),
    ));
    tokio::spawn(forward_lines(
        child.stderr.take().expect("stderr is piped"),
        LogStream::Stderr,
        lines,
    ));
    let status = loop {
        tokio::select! {
            Some((stream, line)) = received.recv() => log.record(stream, line).await?,
            status = child.wait() => break status.map_err(|e| {
                SandboxError::BuildFailed(format!("cannot wait for the build VM: {e}"))
            })?,
        }
    };
    // What the VM printed last, unless a grandchild holds the pipes open.
    tokio::time::timeout(DRAIN_GRACE, async {
        while let Some((stream, line)) = received.recv().await {
            log.record(stream, line).await?;
        }
        Ok::<_, SandboxError>(())
    })
    .await
    .unwrap_or(Ok(()))?;
    Ok(status)
}

/// Send one record per line of `out` to `lines`, as `pump` does for a VM's console.
async fn forward_lines(
    out: impl AsyncRead + Unpin,
    stream: LogStream,
    lines: tokio::sync::mpsc::UnboundedSender<(LogStream, String)>,
) {
    let mut out = BufReader::new(out);
    let mut line = Vec::new();
    loop {
        line.clear();
        match (&mut out).take(MAX_LINE).read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = line.strip_suffix(b"\n").unwrap_or(&line);
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        let text = String::from_utf8_lossy(text).into_owned();
        if lines.send((stream.clone(), text)).is_err() {
            return;
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

/// Remove the scratch dirs (`build-<pid>-<seq>`) that a build of an earlier
/// adapter process left in `vm_dir`; this process's own builds remove theirs.
/// Nothing else matches: not the `build-mode` seal.
fn sweep_build_scratch(vm_dir: &Path) {
    let mine = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir(vm_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let stale = name
            .to_str()
            .and_then(|name| name.strip_prefix(BUILD_SCRATCH_PREFIX))
            .and_then(pid_seq)
            .is_some_and(|pid| pid != mine);
        if stale && entry.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Make `vm_dir` a VM that boots builds; returns its data disk and the lines to
/// log. The first time, marked by the seal `<vm_id>/build-mode`, it keeps what
/// an earlier adapter left: a 0.26 overlay disk `root/upper.ext4` moves to
/// `upper.ext4.pre-build`, and a legacy rootfs clone `root/` to
/// `root.pre-build/`, with a fresh `root/` in its place. The data disk is
/// `<vm_id>/upper.ext4`, outside `root/` (the guest's share), and outlives
/// every build; only `worker::destroy` removes it.
fn adopt_build_mode(vm_dir: &Path, worker: &str) -> io::Result<(PathBuf, Vec<String>)> {
    std::fs::create_dir_all(vm_dir)?;
    let mut notes = Vec::new();
    let seal = vm_dir.join(BUILD_MODE_SEAL);
    if std::fs::symlink_metadata(&seal).is_err() {
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
        if is_rootfs_clone(&root) {
            let kept = unused_name(vm_dir, PRE_BUILD_ROOT);
            std::fs::rename(&root, &kept)?;
            std::fs::create_dir(&root)?;
            notes.push(format!(
                "{worker}: kept the pre-build data disk at {}",
                kept.display()
            ));
        }
        write_new_file(&seal, b"1\n")?;
    }
    let upper = crate::cli::upper::ensure_upper_ext4(vm_dir)?;
    Ok((upper, notes))
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

/// `(package, host_path, run_override)`; `package` is `Some` for a bundle.
fn source_parts(source: &WorkerSource) -> (Option<&str>, &str, Option<&str>) {
    match source {
        WorkerSource::Bundle { package, host_path } => (Some(package), host_path, None),
        WorkerSource::Local {
            host_path,
            run_override,
        } => (None, host_path, run_override.as_deref()),
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
    vm.stop(LEASE_STOP_GRACE).await;
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
            Slot::Booted(vm) => vm.ended.borrow().is_none(),
        }
    }

    fn status(&self, vm_id: &str) -> WorkerStatus {
        let (state, ended, started_at_ms) = match self {
            // Nothing has started yet. `lookup` never hands out `Destroying`.
            Slot::Preparing(..) | Slot::Destroying(_) => (State::Preparing, None, 0),
            Slot::Booted(vm) => {
                let ended = *vm.ended.borrow();
                let state = match ended {
                    None => State::Running,
                    Some(_) if vm.stopping.load(Ordering::SeqCst) => State::Stopped,
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
    /// Signal the VM unless it was reaped already, since its pid may name another process by then.
    fn signal(&self, signal: Signal) {
        if !self.reaped.load(Ordering::SeqCst) {
            let _ = kill(self.pid, signal);
        }
    }

    /// SIGTERM, then SIGKILL after `grace`; a zero `grace` kills at once. Returns once
    /// the exit is recorded. A VM that already exited is left as it is.
    async fn stop(&self, grace: Duration) {
        if self.ended.borrow().is_some() {
            return;
        }
        self.stopping.store(true, Ordering::SeqCst);
        let mut ended = self.ended.subscribe();
        let gone = !grace.is_zero() && {
            self.signal(Signal::SIGTERM);
            tokio::time::timeout(grace, ended.wait_for(Option::is_some))
                .await
                .is_ok()
        };
        if !gone {
            self.signal(Signal::SIGKILL);
            let _ = ended.wait_for(Option::is_some).await;
        }
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

/// Replace `<dir>/state.json` with one recording `instance`. Written to a
/// temp file and renamed over it, so a crash or a full disk mid-write leaves
/// the previous file whole: a truncated one failed every later start.
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
/// threads and processes: an flock on `<base>/.build-dir.lock`, released on
/// drop or process exit (the pattern of the erofs build lock).
fn lock_build_dirs(base: &Path) -> Result<fslock::LockFile, String> {
    let path = base.join(".build-dir.lock");
    std::fs::create_dir_all(base).map_err(|e| format!("{}: {e}", base.display()))?;
    let mut lock = fslock::LockFile::open(&path)
        .map_err(|e| format!("open the build-dir lock {}: {e}", path.display()))?;
    lock.lock()
        .map_err(|e| format!("take the build-dir lock {}: {e}", path.display()))?;
    Ok(lock)
}

/// Record `state_key` and `instance`, truncate `console.log` and spawn `cmd`
/// with piped stdin (the env frame) and output, and a lifeline.
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
/// current file holds the offsets from `base` on, the rotated one those below.
struct Console {
    dir: PathBuf,
    file: File,
    len: u64,
    base: u64,
}

impl Console {
    /// The log offset after the last record: `append` writes whole lines under this lock.
    fn end(&self) -> u64 {
        self.base + self.len
    }

    /// Append one `{stream, line, ts_ms}` JSON line.
    fn append(&mut self, stream: &LogStream, line: &[u8]) -> io::Result<()> {
        let mut record = serde_json::to_vec(&serde_json::json!({
            "stream": stream,
            "line": String::from_utf8_lossy(line),
            "ts_ms": now_ms(),
        }))?;
        record.push(b'\n');
        self.file.write_all(&record)?;
        self.len += record.len() as u64;
        if self.len > CONSOLE_LOG_CAP {
            let path = self.dir.join(CONSOLE_LOG);
            std::fs::rename(&path, self.dir.join(CONSOLE_LOG_OLD))?;
            self.file = open_private(&path, true)?;
            self.base += self.len;
            self.len = 0;
        }
        Ok(())
    }
}

/// Copy one output stream into `console.log`, a record per line. It keeps
/// reading after a write error, so a full disk cannot stall the VM on a full pipe.
async fn pump(out: impl AsyncRead + Unpin, stream: LogStream, console: Arc<Mutex<Console>>) {
    let mut out = BufReader::new(out);
    let mut line = Vec::new();
    let mut warned = false;
    loop {
        line.clear();
        match (&mut out).take(MAX_LINE).read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return,
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
/// line waits for its newline; a line that does not parse is skipped.
fn read_records(console: &Mutex<Console>, from: u64) -> io::Result<(Vec<LogRecord>, u64)> {
    // Opened under the lock, so no rotation falls between the opens and `base`.
    let (rotated, current, base) = {
        let console = console.lock().unwrap_or_else(PoisonError::into_inner);
        let rotated = (console.base > 0)
            .then(|| open_private(&console.dir.join(CONSOLE_LOG_OLD), false))
            .transpose()?;
        let current = open_private(&console.dir.join(CONSOLE_LOG), false)?;
        (rotated, current, console.base)
    };
    let mut from = if from > base + current.metadata()?.len() {
        0
    } else {
        from
    };
    let mut records = Vec::new();
    if let Some(rotated) = rotated
        && from < base
    {
        let start = base.saturating_sub(rotated.metadata()?.len());
        read_lines(rotated, start, from.max(start), &mut records)?;
        // Nothing more is written to a rotated file: its rest is done.
        from = base;
    }
    let end = read_lines(current, base, from.max(base), &mut records)?;
    Ok((records, end))
}

/// Append to `records` the whole lines of `file`, which holds the log offsets
/// from `start` on, from offset `from`; returns the offset after the last one.
fn read_lines(
    mut file: File,
    start: u64,
    from: u64,
    records: &mut Vec<LogRecord>,
) -> io::Result<u64> {
    file.seek(SeekFrom::Start(from - start))?;
    // Bounded: `Console` rotates the file past `CONSOLE_LOG_CAP`.
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut end = from;
    for raw in bytes.split_inclusive(|&b| b == b'\n') {
        if !raw.ends_with(b"\n") {
            break;
        }
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
    Ok(end)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::scoped_auth_token;
    use futures::StreamExt;
    use iii_sdk::channels::{ChannelDirection, StreamChannelRef};
    use tokio::sync::{Notify, mpsc};
    use tokio_tungstenite::tungstenite::Message;

    fn vms(t: &Path, token: Option<&str>, roots: Vec<PathBuf>) -> WorkerVms {
        WorkerVms::new(t.join("workers"), token.map(str::to_string), roots)
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

    fn destroy_req(auth: &str, state_key: &str) -> WorkerDestroyRequest {
        WorkerDestroyRequest {
            auth: auth.to_string(),
            state_key: state_key.to_string(),
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
        std::fs::create_dir_all(&stuck).unwrap();
        std::fs::write(stuck.join("f"), "").unwrap();
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
        std::fs::create_dir_all(&dir).unwrap();
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
                sh(r#"yes "$(printf %01000d 0)" | head -n 11000"#),
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
            assert!(vms.admit(&path.display().to_string()).is_ok(), "{path:?}");
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

    /// A `Console` over `dir` that rotated out `rotated` (never, when empty)
    /// and holds `current` now.
    fn console_over(dir: &Path, rotated: &str, current: &str) -> Mutex<Console> {
        if !rotated.is_empty() {
            std::fs::write(dir.join(CONSOLE_LOG_OLD), rotated).unwrap();
        }
        std::fs::write(dir.join(CONSOLE_LOG), current).unwrap();
        Mutex::new(Console {
            dir: dir.to_path_buf(),
            file: File::open(dir.join(CONSOLE_LOG)).unwrap(),
            len: current.len() as u64,
            base: rotated.len() as u64,
        })
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

    /// Through `Console::append`: a reader a whole rotation behind still gets
    /// every record after its cursor, in order.
    #[test]
    fn a_reader_behind_a_rotation_loses_no_record() {
        let t = tempfile::tempdir().unwrap();
        let console = Mutex::new(Console {
            dir: t.path().to_path_buf(),
            file: open_private(&t.path().join(CONSOLE_LOG), true).unwrap(),
            len: 0,
            base: 0,
        });
        let append = |line: &str| {
            let mut console = console.lock().unwrap();
            console.append(&LogStream::Stdout, line.as_bytes()).unwrap();
        };
        append("first");
        let (_, cursor) = read_records(&console, 0).unwrap();
        // About 11.5 MB: past CONSOLE_LOG_CAP once.
        let filler = "x".repeat(1000);
        let n = 11_000;
        for i in 0..n {
            append(&format!("{i} {filler}"));
        }
        assert!(console.lock().unwrap().base > 0, "the log did not rotate");

        let (recs, _) = read_records(&console, cursor).unwrap();
        assert_eq!(recs.len(), n);
        assert!(recs[0].line.starts_with("0 "));
        assert!(recs[n - 1].line.starts_with(&format!("{} ", n - 1)));
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
            "S508"
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
            std::fs::create_dir_all(&dir).unwrap();
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
    /// when that one stays on disk (Compose moved `.build/<c>` to
    /// `.iii/build/<c>` and kept the old one).
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
        std::fs::create_dir_all(base.join(vm_id("a"))).unwrap();
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
    /// reads image.id: another VM's image is S505, whatever the id, and so is
    /// any other path to it (a symlink, `..`).
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
        for path in [&dir_b, &link, &dotdot] {
            assert_eq!(
                code(vms.admit_build(&a, "a", &build_ref(path, &id))),
                "S505"
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

    /// `worker::build` claims `output_dir` under its key's hold, before it
    /// creates it or reads the source: a refused token creates nothing, by any
    /// path to the other VM's directory, even one that is gone from disk.
    #[tokio::test]
    async fn a_scoped_build_writes_no_other_vms_build_dir() {
        if !crate::cli::overlay::overlay_active() {
            eprintln!("[skip] a build needs the overlay rootfs (an embedded iii-init)");
            return;
        }
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let empty = t.join("empty");
        std::fs::create_dir(&empty).unwrap();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let iii = IIIClient::new("ws://127.0.0.1:9");
        let build = |auth: &str, state_key: &str, out: &Path| {
            vms.build(
                &iii,
                WorkerBuildRequest {
                    auth: auth.to_string(),
                    state_key: state_key.to_string(),
                    worker_name: "w".to_string(),
                    source: local(&empty),
                    output_dir: out.display().to_string(),
                    env: Default::default(),
                    logs: StreamChannelRef {
                        channel_id: "c".to_string(),
                        access_key: "k".to_string(),
                        direction: ChannelDirection::Write,
                    },
                },
            )
        };
        let (out_a, out_b) = (t.join("project/a"), t.join("project/b"));
        let link = t.join("project/link-to-b");
        let (a, b) = (scoped_auth_token("t", "a"), scoped_auth_token("t", "b"));
        // S501, not S505: past the claim, to the empty source.
        assert_eq!(code(build(&b, "b", &out_b).await), "S501");
        std::os::unix::fs::symlink(&out_b, &link).unwrap();
        assert_eq!(code(build(&a, "a", &out_b).await), "S505");
        assert_eq!(code(build(&a, "a", &link).await), "S505");
        // `..` is refused before any claim.
        let dotdot = PathBuf::from(format!("{}/../b", out_a.display()));
        assert_eq!(code(build(&a, "a", &dotdot).await), "S501");
        assert!(!out_a.exists());
        assert_eq!(std::fs::read_dir(&out_b).unwrap().count(), 0);
        // B's directory, deleted, is still B's: A gets S505 and creates nothing.
        std::fs::remove_dir(&out_b).unwrap();
        assert_eq!(code(build(&a, "a", &out_b).await), "S505");
        assert!(!out_b.exists());
        assert_eq!(code(build(&a, "a", &out_a).await), "S501");
        assert!(out_a.is_dir());
        assert_eq!(code(build("t", "a", &out_b).await), "S501");
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
        // build's caller left: then the run goes.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !id_file.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the writer never committed"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        drop(run);
        let written = cancel_writer(&slot, &state).await;
        assert_eq!(state.load(Ordering::SeqCst), BUILD_COMMITTED);
        let id = written.expect("the writer was still in its slot").unwrap();
        assert_eq!(read_image_id(&t.join("out")), Some(id));
    }

    #[test]
    fn only_an_earlier_processs_build_scratch_is_swept() {
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
        assert!(mine.is_dir() && root.is_dir() && other.is_dir() && !stale.exists());
        assert!(t.path().join(BUILD_MODE_SEAL).is_file());
        assert!(t.path().join("build-2-0").is_file());
    }

    /// A build whose adapter died while writing left its temporary image in the
    /// project; the next build removes it, and only it.
    #[test]
    fn the_next_build_sweeps_an_earlier_processs_partial_image() {
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
            let gone = name.contains(".1-0.");
            assert_eq!(!t.path().join(name).exists(), gone, "{name}");
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
