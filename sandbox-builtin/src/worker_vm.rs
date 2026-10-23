// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! `sandbox-builtin::worker::*`: worker VMs that Compose supervises.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iii_sdk::IIIClient;
use iii_sdk::channels::ChannelWriter;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::watch;

use crate::cli::local_worker::{VmOverride, bundle_vm_command, local_vm_command, validate_source};
use crate::cli::worker_manager::oci;
use crate::daemon_exit::Lifeline;
use crate::sandbox_daemon::SandboxError;
use crate::wire::{
    LogRecord, LogStream, State, WorkerDestroyRequest, WorkerDestroyResponse, WorkerLogsRequest,
    WorkerLogsResponse, WorkerPrepareRequest, WorkerPrepareResponse, WorkerSource,
    WorkerStartRequest, WorkerStartResponse, WorkerStatus, WorkerStatusRequest, WorkerStopRequest,
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
const STATE_FILE: &str = "state.json";

/// The worker VMs of one project, behind its token and host roots.
pub struct WorkerVms {
    /// Per-VM state lives in `<base>/<vm_id>/`: the control files, and the VM's
    /// writable share `root/`, which a uid-0 guest controls.
    base: PathBuf,
    token: Option<String>,
    roots: Vec<PathBuf>,
    /// Keyed by `vm_id`. An exited VM stays, so `status` can report it.
    slots: Mutex<HashMap<String, Slot>>,
}

#[derive(Clone)]
enum Slot {
    /// `start` is building the VM that will boot as this instance.
    Preparing(u64),
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
        self.authorize(&req.auth)?;
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
        Ok(WorkerPrepareResponse {})
    }

    /// Build the VM for `state_key` and boot it. S502 while that VM is
    /// preparing or running; a source `prepare` would reject is S501, as there;
    /// any other build error is S300.
    pub async fn start(
        &self,
        req: WorkerStartRequest,
    ) -> Result<WorkerStartResponse, SandboxError> {
        self.authorize(&req.auth)?;
        let (package, host_path, run_override) = source_parts(&req.source);
        let dir = self.admit(host_path)?;
        let id = vm_id(&req.state_key);
        {
            let mut slots = self.slots();
            refuse_busy(&id, slots.get(&id))?;
            let next = stored_instance(&self.base.join(&id))? + 1;
            slots.insert(id.clone(), Slot::Preparing(next));
        }
        let _unreserve = Unreserve(self, &id);
        validate_source(&dir, package, run_override).map_err(SandboxError::SourceRejected)?;
        let over = VmOverride {
            state_dir: self.base.join(&id).join("root"),
            engine_url: &req.engine_url,
            extra_env: req.env.into_iter().collect(),
        };
        let built = match package {
            Some(package) => bundle_vm_command(package, &req.worker_name, &dir, over).await,
            None => local_vm_command(&req.worker_name, &dir, run_override, over).await,
        }
        .map_err(SandboxError::BootFailed)?;
        self.launch(&req.state_key, built.command)
    }

    /// Spawn `cmd` as the next instance of `state_key`'s VM, its output pumped
    /// into `console.log`.
    pub(crate) fn launch(
        &self,
        state_key: &str,
        cmd: Command,
    ) -> Result<WorkerStartResponse, SandboxError> {
        let id = vm_id(state_key);
        let dir = self.base.join(&id);
        // Held to the insert, so two launches of one key cannot both spawn.
        let mut slots = self.slots();
        // A `Preparing` entry is the reservation of the `start` calling this.
        if !matches!(slots.get(&id), Some(Slot::Preparing(_))) {
            refuse_busy(&id, slots.get(&id))?;
        }
        let instance = stored_instance(&dir)? + 1;
        let (console, lifeline, mut child) = spawn(&dir, instance, cmd).map_err(|e| {
            SandboxError::BootFailed(format!("cannot launch VM in {}: {e}", dir.display()))
        })?;
        let vm = Arc::new(Vm {
            instance,
            pid: Pid::from_raw(child.id().expect("a just-spawned child has a pid") as i32),
            started_at_ms: now_ms(),
            stopping: AtomicBool::new(false),
            reaped: AtomicBool::new(false),
            ended: watch::Sender::new(None),
        });
        let console = Arc::new(Mutex::new(Console {
            dir,
            file: console,
            len: 0,
        }));
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
        self.authorize(&req.auth)?;
        Ok(self.lookup(&req.vm_id, req.instance)?.status(&req.vm_id))
    }

    /// SIGTERM, then SIGKILL after `grace_ms`; `grace_ms: 0` kills at once.
    /// Returns the final status; an exited VM returns it as is.
    pub async fn stop(&self, req: WorkerStopRequest) -> Result<WorkerStatus, SandboxError> {
        self.authorize(&req.auth)?;
        let vm = booted(&req.vm_id, self.lookup(&req.vm_id, req.instance)?)?;
        if vm.ended.borrow().is_none() {
            vm.stopping.store(true, Ordering::SeqCst);
            let mut ended = vm.ended.subscribe();
            let gone = req.grace_ms > 0 && {
                vm.signal(Signal::SIGTERM);
                let grace = Duration::from_millis(req.grace_ms);
                tokio::time::timeout(grace, ended.wait_for(Option::is_some))
                    .await
                    .is_ok()
            };
            if !gone {
                vm.signal(Signal::SIGKILL);
                let _ = ended.wait_for(Option::is_some).await;
            }
        }
        Ok(Slot::Booted(vm).status(&req.vm_id))
    }

    /// Stream the VM's `console.log` into `req.channel` and return; a task
    /// pumps the records and closes the channel. S504 for an unknown `vm_id`.
    pub async fn logs(
        self: &Arc<Self>,
        iii: &IIIClient,
        req: WorkerLogsRequest,
    ) -> Result<WorkerLogsResponse, SandboxError> {
        self.authorize(&req.auth)?;
        // Checked here: an error inside the task would reach no caller.
        booted(&req.vm_id, self.slot(&req.vm_id)?)?;
        let writer = ChannelWriter::new(iii.address(), &req.channel);
        let vms = self.clone();
        tokio::spawn(async move {
            let channel = &writer;
            let pumped = vms
                .pump_logs(&req, |record| async move {
                    let frame = serde_json::to_string(&record).map_err(|e| e.to_string())?;
                    channel
                        .send_message(&frame)
                        .await
                        .map_err(|e| e.to_string())
                })
                .await;
            if let Err(e) = pumped {
                tracing::info!(error = %e, vm_id = %req.vm_id, "worker log stream ended early");
            }
            // Connects even if nothing was sent, so the reader still sees the close.
            let _ = writer.write(&[]).await;
            let _ = writer.close().await;
        });
        Ok(WorkerLogsResponse {})
    }

    /// Feed `sink` the instance's records from `req.cursor`. With `follow` it
    /// polls until the instance has exited and its log is drained; a sink error
    /// ends it too (S218).
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
        let path = self.base.join(&req.vm_id).join(CONSOLE_LOG);
        let mut ended = vm.ended.subscribe();
        let mut cursor = req.cursor.unwrap_or(0);
        loop {
            // Sampled before the read: the waiter records the exit after the
            // log is complete, so a read that follows it is the last one.
            let exited = ended.borrow().is_some();
            // shortcut: a blocking read, bounded by `CONSOLE_LOG_CAP`; move it
            // to `spawn_blocking` if the cap grows.
            match read_records(&path, cursor) {
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
        self.authorize(&req.auth)?;
        let id = vm_id(&req.state_key);
        // The marker, not the lock, keeps a `start` out of a half-removed dir:
        // a legacy rootfs clone takes seconds to remove.
        let prev = {
            let mut slots = self.slots();
            refuse_busy(&id, slots.get(&id))?;
            let instance = slots.get(&id).map_or(0, Slot::instance);
            slots.insert(id.clone(), Slot::Destroying(instance))
        };
        let dir = self.base.join(&id);
        let removed = std::fs::remove_dir_all(&dir);
        let mut slots = self.slots();
        slots.remove(&id);
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

    /// Constant-time token check; no token (or an empty one) refuses every call.
    fn authorize(&self, auth: &str) -> Result<(), SandboxError> {
        let token = self.token.as_deref().unwrap_or("");
        let diff = token
            .bytes()
            .zip(auth.bytes())
            .fold(0, |d, (a, b)| d | (a ^ b));
        if token.is_empty() || token.len() != auth.len() || diff != 0 {
            return Err(SandboxError::Unauthorized);
        }
        Ok(())
    }

    /// `host_path`, canonicalized, if it lies under a canonicalized root.
    fn admit(&self, host_path: &str) -> Result<PathBuf, SandboxError> {
        if let Ok(path) = Path::new(host_path).canonicalize()
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
                 refusing host_path {host_path}"
            )));
        }
        let roots: Vec<_> = self.roots.iter().map(|r| r.display().to_string()).collect();
        Err(SandboxError::SourceRejected(format!(
            "host_path {host_path} is outside the host roots [{}] (III_SANDBOX_HOST_ROOTS)",
            roots.join(", ")
        )))
    }
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
        Slot::Preparing(instance) | Slot::Destroying(instance) => Err(SandboxError::StateBusy {
            vm_id: vm_id.to_string(),
            instance,
        }),
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

/// Drops `start`'s `Preparing` entry if it never launched (an error, or the
/// call was dropped), so the key stays startable.
struct Unreserve<'a>(&'a WorkerVms, &'a str);

impl Drop for Unreserve<'_> {
    fn drop(&mut self) {
        let mut slots = self.0.slots();
        if matches!(slots.get(self.1), Some(Slot::Preparing(_))) {
            slots.remove(self.1);
        }
    }
}

impl Slot {
    fn instance(&self) -> u64 {
        match self {
            Slot::Preparing(instance) | Slot::Destroying(instance) => *instance,
            Slot::Booted(vm) => vm.instance,
        }
    }

    fn busy(&self) -> bool {
        match self {
            Slot::Preparing(_) | Slot::Destroying(_) => true,
            Slot::Booted(vm) => vm.ended.borrow().is_none(),
        }
    }

    fn status(&self, vm_id: &str) -> WorkerStatus {
        let (state, ended, started_at_ms) = match self {
            // Nothing has started yet. `lookup` never hands out `Destroying`.
            Slot::Preparing(_) | Slot::Destroying(_) => (State::Preparing, None, 0),
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
}

/// Opens a control file with `O_NOFOLLOW`, as `cli::pidfile` does: a planted
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

/// Record `instance` in `state.json`, truncate `console.log` and spawn `cmd`
/// with null stdin, piped output and a lifeline.
fn spawn(dir: &Path, instance: u64, mut cmd: Command) -> io::Result<(File, Lifeline, Child)> {
    std::fs::create_dir_all(dir)?;
    let console = open_private(&dir.join(CONSOLE_LOG), true)?;
    serde_json::to_writer(
        open_private(&dir.join(STATE_FILE), true)?,
        &StateFile { instance },
    )?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let lifeline = crate::daemon_exit::attach_lifeline(&mut cmd)?;
    Ok((console, lifeline, cmd.spawn()?))
}

/// A VM's `console.log`, rotated to `console.log.1` past `CONSOLE_LOG_CAP`.
struct Console {
    dir: PathBuf,
    file: File,
    len: u64,
}

impl Console {
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
            std::fs::rename(&path, self.dir.join("console.log.1"))?;
            self.file = open_private(&path, true)?;
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

/// The records of `path` from byte offset `from`, and the offset after the last
/// whole line read. A cursor past the end means the file rotated, so it reads
/// from 0. A partial last line waits for its newline; a line that does not parse
/// (a cursor into the middle of one) is skipped, not an error.
pub(crate) fn read_records(path: &Path, from: u64) -> io::Result<(Vec<LogRecord>, u64)> {
    let mut file = open_private(path, false)?;
    let from = if from > file.metadata()?.len() {
        0
    } else {
        from
    };
    file.seek(SeekFrom::Start(from))?;
    // Bounded: `Console` rotates the file past `CONSOLE_LOG_CAP`.
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut records = Vec::new();
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
    Ok((records, end))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iii_sdk::channels::{ChannelDirection, StreamChannelRef};

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
            let known = vms.launch("k", sh("exit 0")).unwrap().vm_id;
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
        let started = vms.launch("k", sh("echo hi; sleep 30")).unwrap();
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

    #[tokio::test]
    async fn instance_survives_an_adapter_restart() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let first = vms(t, Some("t"), vec![]);
        let id = first.launch("k", sh("sleep 30")).unwrap().vm_id;
        first.stop(stop_req("t", &id, 1, 0)).await.unwrap();
        drop(first);

        let vms = vms(t, Some("t"), vec![]);
        assert_eq!(vms.launch("k", sh("exit 0")).unwrap().instance, 2);
        assert_eq!(code(vms.status(status_req("t", &id, 1))), "S503");
    }

    #[tokio::test]
    async fn stop_escalates_to_sigkill_after_grace() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![]);
        let id = vms
            .launch("k", sh("trap '' TERM; echo ready; sleep 30"))
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
        let id = vms.launch("k", sh("sleep 30")).unwrap().vm_id;

        let busy = vms.launch("k", sh("exit 0")).unwrap_err();
        assert_eq!(busy.code().as_str(), "S502");
        assert_eq!(
            busy.to_payload()["fix"],
            serde_json::json!({"vm_id": id, "instance": 1})
        );
        assert_eq!(code(vms.start(start_req("t", "k", local(t))).await), "S502");
        assert_eq!(code(vms.destroy(destroy_req("t", "k"))), "S502");

        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    #[tokio::test]
    async fn a_vm_being_destroyed_is_busy_to_start_and_gone_to_status() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![t.into()]);
        let id = vm_id("k");
        vms.slots().insert(id.clone(), Slot::Destroying(1));

        assert_eq!(code(vms.launch("k", sh("exit 0"))), "S502");
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
        let id = vms.launch("k", sh("exit 7")).unwrap().vm_id;
        wait_ended(&vms, &id, 1).await;
        // A file where the base dir was: ENOTDIR, which root cannot bypass.
        let base = t.join("workers");
        std::fs::rename(&base, t.join("moved")).unwrap();
        std::fs::write(&base, "").unwrap();

        assert_eq!(code(vms.destroy(destroy_req("t", "k"))), "S216");
        let status = vms.status(status_req("t", &id, 1)).unwrap();
        assert_eq!(status.exit_code, Some(7), "{status:?}");
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
        let id = vms.launch("k", sh("exit 7")).unwrap().vm_id;
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
        assert_eq!(code(vms.launch("k", sh("echo pwned"))), "S300");
        assert_eq!(std::fs::read_to_string(t.join("target")).unwrap(), "keep");
    }

    #[tokio::test]
    async fn console_log_rotates_past_the_cap() {
        let t = tempfile::tempdir().unwrap();
        let t = t.path();
        let vms = vms(t, Some("t"), vec![]);
        let id = vms
            .launch("k", sh(r#"yes "$(printf %01000d 0)" | head -n 11000"#))
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
        let path = t.path().join(CONSOLE_LOG);
        let text = concat!(
            r#"{"stream":"stdout","line":"a","ts_ms":1}"#,
            "\n",
            r#"{"stream":"stderr","line":"b","ts_ms":2}"#,
            "\n",
            r#"{"stream":"stdout","line":"c","ts_ms":3}"#,
            "\n",
        );
        std::fs::write(&path, text).unwrap();
        let len = text.len() as u64;

        let (all, end) = read_records(&path, 0).unwrap();
        assert_eq!(lines(&all), ["a", "b", "c"]);
        assert_eq!(all[1].ts_ms, 2);
        assert!(matches!(all[1].stream, LogStream::Stderr));
        assert!(all.windows(2).all(|w| w[0].cursor < w[1].cursor));
        assert_eq!((all[2].cursor, end), (len, len));

        let (rest, _) = read_records(&path, all[0].cursor).unwrap();
        assert_eq!(lines(&rest), ["b", "c"]);
        assert!(read_records(&path, len).unwrap().0.is_empty());

        // A cursor past the end means the file rotated: read the new file from its start.
        let (rotated, _) = read_records(&path, len + 1).unwrap();
        assert_eq!(lines(&rotated), ["a", "b", "c"]);
    }

    #[test]
    fn a_partial_last_line_is_left_for_the_next_read() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join(CONSOLE_LOG);
        let whole = r#"{"stream":"stdout","line":"a","ts_ms":1}"#;
        std::fs::write(&path, format!("{whole}\n{{\"stream\":\"std")).unwrap();

        let (recs, end) = read_records(&path, 0).unwrap();
        assert_eq!(lines(&recs), ["a"]);
        assert_eq!(end, whole.len() as u64 + 1);
    }

    #[tokio::test]
    async fn follow_streams_until_the_instance_ends() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vms
            .launch("k", sh("echo a; sleep 0.3; echo b >&2"))
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
        let id = vms.launch("k", sh("echo a")).unwrap().vm_id;
        let status = wait_ended(&vms, &id, 1).await;
        assert!(matches!(status.state, State::Exited), "{status:?}");

        let req = logs_req("t", &id, true);
        let recs = tokio::time::timeout(Duration::from_secs(2), pumped(&vms, &req))
            .await
            .expect("follow on an exited instance must return");
        assert_eq!(lines(&recs), ["a"]);
    }

    #[tokio::test]
    async fn a_pump_resumes_after_the_cursor_and_ends_without_follow() {
        let t = tempfile::tempdir().unwrap();
        let vms = vms(t.path(), Some("t"), vec![]);
        let id = vms
            .launch("k", sh("echo a; echo b; sleep 30"))
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
        let id = vms.launch("k", sh("echo a; sleep 30")).unwrap().vm_id;
        wait_record(&t.path().join("workers").join(&id), "a").await;

        let ended = vms
            .pump_logs(&logs_req("t", &id, true), |_| async {
                Err("channel closed".to_string())
            })
            .await;
        assert_eq!(code(ended), "S218");

        vms.stop(stop_req("t", &id, 1, 0)).await.unwrap();
    }

    #[test]
    fn vm_id_is_stable_and_32_hex() {
        assert_eq!(vm_id("a"), vm_id("a"));
        assert_eq!(vm_id("a").len(), 32);
        assert!(vm_id("a").bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(vm_id("a"), vm_id("b"));
    }
}
