use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine;
use iii_shell_client::{RequestSpec, Session, VecSink};

use crate::sandbox_daemon::create::{BootHandle, BootParams, VmLauncher};
use crate::sandbox_daemon::errors::SandboxError;
use crate::sandbox_daemon::exec::{ExecRequest, ExecResponse, ShellRunner};
use crate::sandbox_daemon::stop::VmStopper;

/// How often to stat the shell socket while waiting for __vm-boot's
/// `shell_relay` to bind it. Unix socket existence is a cheap fs stat,
/// so polling aggressively is harmless; the previous 500ms interval
/// added up to ~499ms of pure lag after the socket actually bound.
const BOOT_SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(10);
const BOOT_SOCKET_TIMEOUT: Duration = Duration::from_secs(30);
/// Default per-exec deadline when the caller omits `timeout_ms`.
///
/// Sized for the common agent workload: `npm install`, `pip install`,
/// `cargo build`, `apt-get install` — first-run cold-cache cases that
/// regularly exceed a minute. The previous 30s default surfaced as the
/// opaque engine-gate `gate_unavailable` denial (no S200, no
/// `timed_out: true`) because the engine's invocation deadline tracks
/// this value, so a daemon-side default that's too tight is the same
/// as a missing structured timeout response.
///
/// Calls that should fail fast (probes, version checks, "is this
/// service up") should still pass an explicit `timeout_ms`.
pub const DEFAULT_EXEC_TIMEOUT_MS: u64 = 300_000;
/// How long a create waits for the guest once the relay is up: the default
/// exec deadline, which covered a slow boot when the first exec paid for it.
/// A VM that dies closes the relay, so it fails the create at once.
const GUEST_READY_TIMEOUT: Duration = Duration::from_millis(DEFAULT_EXEC_TIMEOUT_MS);
/// Grace between SIGTERM and SIGKILL. Kept short because the sandbox VM
/// is ephemeral — nothing needs to flush to persistent storage. Values
/// larger than a few hundred ms directly translate into user-visible
/// `sandbox run` latency.
const STOP_GRACE_MS: u64 = 200;
/// Cap sandbox output at 1 MiB per stream (same as vm_client).
const OUTPUT_CAP: usize = 1_048_576;

/// Set once the first boot has completed its per-host provisioning
/// (codesign entitlement + libkrunfw dylib placement). Subsequent boots
/// skip the work, which is idempotent but touches the filesystem on
/// every call.
static PROVISION_DONE: AtomicBool = AtomicBool::new(false);

/// Refuse to boot a sandbox unless this binary carries an embedded iii-init.
///
/// Only an embedded init is guaranteed to understand the overlay boot
/// (`III_BLOCK_ROOT_*`); an on-disk init (`III_INIT_PATH`) may predate it
/// (see `cli::overlay`). Without one the isolated overlay root cannot be
/// assembled, and the sandbox has no other isolated boot shape, so fail
/// closed.
pub(crate) fn require_overlay_capable(has_embedded_init: bool) -> Result<(), SandboxError> {
    if has_embedded_init {
        return Ok(());
    }
    Err(SandboxError::BootFailed(
        "sandbox isolation requires a sandbox-libkrun built with an embedded iii-init \
         (official release builds, or run BUILD_TARGET=<host target triple> \
         scripts/build-init.sh and build with \
         --features embed-init locally); without it the \
         sandbox root cannot be isolated from the shared image cache"
            .to_string(),
    ))
}

/// Build (or reuse) the read-only erofs image of `base_rootfs`, the guest's
/// overlay lower. Written next to the cache dir (`<dir>.erofs`) and shared
/// with worker VMs on the same image; see `cli::erofs::ensure_base_erofs`.
#[cfg(test)]
pub(crate) async fn prepare_overlay_lower(base_rootfs: &Path) -> Result<PathBuf, SandboxError> {
    require_overlay_capable(iii_filesystem::init::has_init())?;
    let base = base_rootfs.to_path_buf();
    tokio::task::spawn_blocking(move || crate::cli::erofs::ensure_base_erofs(&base))
        .await
        .map_err(|e| SandboxError::BootFailed(format!("erofs build task failed: {e}")))?
        .map_err(|e| SandboxError::BootFailed(format!("build read-only base erofs: {e}")))
}

/// `__vm-boot` arguments for one sandbox.
///
/// Boot shape: `--rootfs` is the per-sandbox trampoline, served
/// read-only (`--rootfs-readonly`); the image arrives as the read-only erofs
/// `--rootfs-lower` under `--rootfs-mode overlay`. There is no
/// `--rootfs-upper`, so iii-init uses an in-guest tmpfs upper that dies with
/// the VM. The shared cache directory itself is never passed.
///
/// Flag names follow `VmBootArgs`, and that is not cosmetic: it declares
/// `vcpus` / `ram` / `exec`, so the older `--cpus` / `--memory-mb` and missing
/// `--exec` caused clap to reject the args, the child to exit instantly, and a
/// 30s `shell.sock` wait that ended in an opaque S300.
pub(crate) fn vm_boot_args(params: &BootParams, rootfs_lower: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "--rootfs".into(),
        params.rootfs.clone().into(),
        "--rootfs-readonly".into(),
        "--rootfs-mode".into(),
        "overlay".into(),
        "--rootfs-lower".into(),
        rootfs_lower.into(),
        "--exec".into(),
        "/bin/sh".into(),
        // Keep PID 1 alive; iii-init supervisor serves every
        // `sb.exec()` through `shell.sock` independently of PID 1's
        // foreground command.
        "--arg".into(),
        "-c".into(),
        "--arg".into(),
        "exec sleep infinity".into(),
        // `params.workdir` is a host path (the sandbox's overlay
        // merged dir) and does NOT exist inside the VM. iii-init's
        // supervisor chdir's here before spawning PID-1, producing
        // `spawn_initial: No such file or directory (os error 2)`.
        // The VM's rootfs defines its own semantics; pass `/` as a
        // universally-valid cwd and let sandbox::exec requests carry
        // their own `cwd` when callers care.
        "--workdir".into(),
        "/".into(),
        "--vcpus".into(),
        params.cpus.to_string().into(),
        "--ram".into(),
        params.memory_mb.to_string().into(),
        "--shell-sock".into(),
        params.shell_sock.clone().into(),
    ];
    if params.network {
        args.push("--network".into());
        // Idle-reaper beacon: the smoltcp stack lives in the child
        // spawned here, and this file's mtime is its only channel back
        // to the daemon. Without it a sandbox serving pure network
        // traffic reads as idle and is reaped mid-service.
        args.push("--net-activity-file".into());
        args.push(crate::sandbox_daemon::registry::net_activity_path(&params.shell_sock).into());
    }
    // Env values go on stdin (`env_stdin_frame`), never argv, where `ps`
    // shows them to every local user (MOT-5492).
    args.push("--env-stdin".into());
    args
}

/// `params.env` (`KEY=VALUE` entries) as the `__vm-boot --env-stdin` frame.
/// Entries without `=` are dropped, as `__vm-boot` always did.
pub(crate) fn env_stdin_frame(params: &BootParams) -> Result<Vec<u8>, SandboxError> {
    let pairs: Vec<(String, String)> = params
        .env
        .iter()
        .filter_map(|kv| kv.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    iii_supervisor::env_channel::encode_frame(&pairs)
        .map_err(|e| SandboxError::BootFailed(format!("sandbox env: {e}")))
}

/// In-memory lifecycle owner. No constructor accepts PID/status as authority.
/// The original lease and core receipt survive through logs and exact cleanup.
pub(crate) struct RuntimeSession {
    lease: crate::worker_admission::Lease,
    control: crate::worker_admission::RuntimeControl,
    /// Exact launch configuration; never inferred from disk or reduced on error.
    endpoints: Vec<PathBuf>,
    #[cfg(test)]
    diagnostic_pid: Option<u32>,
    completion: tokio::sync::watch::Receiver<Option<RuntimeResult>>,
    host_writers: tokio::sync::Mutex<Vec<tokio::task::JoinHandle<std::io::Result<()>>>>,
    /// Exact host log writers (sandbox stderr). Finished only after every host
    /// writer joined and before endpoint retirement or the receipt's Remove.
    logs: Mutex<Vec<crate::worker_admission::LogWriter>>,
    host_failure: Mutex<Option<String>>,
}
#[derive(Clone)]
struct RuntimeResult {
    status: Option<std::process::ExitStatus>,
    retirement:
        Result<crate::worker_admission::WriterRetirement, crate::worker_admission::RuntimeFailure>,
}
impl RuntimeSession {
    pub(crate) fn supervise(
        child: crate::worker_admission::RuntimeChild,
        lease: crate::worker_admission::Lease,
        endpoints: Vec<PathBuf>,
    ) -> std::sync::Arc<Self> {
        let control = child.control();
        let (done, completion) = tokio::sync::watch::channel(None);
        let session = std::sync::Arc::new(Self {
            lease,
            control,
            endpoints,
            #[cfg(test)]
            diagnostic_pid: child.id(),
            completion,
            host_writers: tokio::sync::Mutex::new(Vec::new()),
            logs: Mutex::new(Vec::new()),
            host_failure: Mutex::new(None),
        });
        let held = session.clone();
        // Install the actual child owner before the caller's first await.
        tokio::spawn(async move {
            let completion = child.wait().await;
            let result = RuntimeResult {
                status: completion.os_status(),
                retirement: completion.retirement().cloned().map_err(Clone::clone),
            };
            if let Err(error) = &result.retirement {
                tracing::error!(%error, "runtime retirement failed; preserving operation");
            }
            done.send_replace(Some(result));
            drop(held);
        });
        session
    }
    /// The stderr pump owns ONE clone of the exact log writer and drops it
    /// before it reports; the session keeps the original for `finish`.
    fn pump_stderr(
        self: &std::sync::Arc<Self>,
        stderr: Option<tokio::process::ChildStderr>,
        log: crate::worker_admission::LogWriter,
    ) {
        let Some(stderr) = stderr else {
            self.adopt_log(log);
            return;
        };
        self.adopt_log(log.clone());
        let task = tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            let mut failure = None;
            let mut read = Ok(());
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if failure.is_none() {
                            if let Err(error) = append_stderr(&log, format!("{line}\n").as_bytes())
                            {
                                failure = Some(error);
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        read = Err(error);
                        break;
                    }
                }
                // Drain even after a failed append: never block the guest on
                // stderr, but cleanup must observe the failure at completion.
            }
            drop(log);
            read?;
            match failure {
                Some(error) => Err(error),
                None => Ok(()),
            }
        });
        self.host_writers
            .try_lock()
            .expect("writers attached synchronously before publishing session")
            .push(task);
    }
    /// Retain an exact log writer until every host writer joined; it is
    /// finished (or kept uncertain on failure) by `finish_host_writers`.
    pub(crate) fn adopt_log(&self, log: crate::worker_admission::LogWriter) {
        self.logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(log);
    }
    #[cfg(test)]
    pub(crate) async fn add_test_host_writer(
        &self,
        task: tokio::task::JoinHandle<std::io::Result<()>>,
    ) {
        self.host_writers.lock().await.push(task);
    }

    pub(crate) async fn finish_host_writers(&self) -> std::io::Result<()> {
        let mut tasks = self.host_writers.lock().await;
        if let Some(error) = self
            .host_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return Err(std::io::Error::other(error));
        }
        // Keep handles in the slot if cancelled; remove only an observed result.
        while let Some(task) = tasks.last_mut() {
            let result = tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "sandbox stderr writer has not drained",
                    )
                })?
                .map_err(|e| std::io::Error::other(format!("stderr task: {e}")))
                .and_then(|r| r);
            tasks.pop();
            if let Err(error) = result {
                *self
                    .host_failure
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(error.to_string());
                return Err(error);
            }
        }
        // Every pump joined and returned its writer clone, so each retained
        // original is the last one: finish it before endpoint retirement or
        // the receipt's Remove. A finish failure is sticky and never hidden.
        let logs = std::mem::take(&mut *self.logs.lock().unwrap_or_else(PoisonError::into_inner));
        let mut first = None;
        for log in logs {
            if let Err(error) = log.finish() {
                first.get_or_insert(error);
            }
        }
        if let Some(error) = first {
            *self
                .host_failure
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(error.to_string());
            return Err(error);
        }
        Ok(())
    }
    /// Called after actual retirement and after caller-owned pumps have joined.
    /// Failure may leave an exact partial close; never clear or shrink the set.
    pub(crate) async fn finish_resources(
        &self,
        receipt: &crate::worker_admission::WriterRetirement,
    ) -> std::io::Result<()> {
        self.finish_host_writers().await?;
        let paths: Vec<&Path> = self.endpoints.iter().map(PathBuf::as_path).collect();
        receipt.retire_runtime_endpoints(&self.lease, &paths)
    }

    #[cfg(test)]
    pub(crate) fn test_diagnostic_pid(&self) -> Option<u32> {
        self.diagnostic_pid
    }

    #[cfg(test)]
    pub(crate) async fn test_imported_close_and_retain(
        &self,
    ) -> std::io::Result<crate::worker_admission::WriterRetirement> {
        let receipt = self.stop(Duration::ZERO).await?;
        self.finish_resources(&receipt).await?;
        Ok(receipt)
    }

    pub(crate) fn lease(&self) -> &crate::worker_admission::Lease {
        &self.lease
    }
    pub(crate) fn observation(&self) -> crate::worker_admission::RuntimeObservation {
        self.control.observation()
    }
    pub(crate) fn signal(
        &self,
        signal: crate::worker_admission::RuntimeSignal,
    ) -> std::io::Result<()> {
        match self.control.signal(signal) {
            Ok(()) => Ok(()),
            Err(_error) if self.control.observation().os_status().is_some() => Ok(()),
            Err(error) => Err(error),
        }
    }
    pub(crate) async fn wait(
        &self,
    ) -> (
        Option<std::process::ExitStatus>,
        std::io::Result<crate::worker_admission::WriterRetirement>,
    ) {
        let mut done = self.completion.clone();
        loop {
            if let Some(result) = done.borrow().clone() {
                return (
                    result.status,
                    result.retirement.map_err(|e| e.to_io_error()),
                );
            }
            if done.changed().await.is_err() {
                let observed = self.control.terminal().await;
                return (
                    observed.os_status(),
                    Err(observed.failure().map_or_else(
                        || {
                            std::io::Error::other(
                                "owned runtime supervisor closed without completion",
                            )
                        },
                        |failure| failure.to_io_error(),
                    )),
                );
            }
        }
    }
    pub(crate) fn retirement(&self) -> std::io::Result<crate::worker_admission::WriterRetirement> {
        match self.completion.borrow().clone() {
            Some(result) => result.retirement.map_err(|e| e.to_io_error()),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "runtime retirement is not complete",
            )),
        }
    }
    pub(crate) async fn stop(
        &self,
        grace: Duration,
    ) -> std::io::Result<crate::worker_admission::WriterRetirement> {
        use crate::worker_admission::RuntimeSignal;
        if self.observation().os_status().is_none() {
            if !grace.is_zero() {
                self.signal(RuntimeSignal::Terminate)?;
                let _elapsed = tokio::time::timeout(grace, self.control.terminal()).await;
            }
            if self.observation().os_status().is_none() {
                self.signal(RuntimeSignal::Kill)?;
            }
        }
        self.wait().await.1
    }
}
impl std::fmt::Debug for RuntimeSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeSession")
            .field("phase", &self.observation().phase())
            .finish_non_exhaustive()
    }
}

fn initialize_stderr(
    lease: &crate::worker_admission::Lease,
    path: &Path,
) -> std::io::Result<crate::worker_admission::LogWriter> {
    use crate::worker_admission::{PathRequest, PathRole};
    // The rotation target is declared exactly beside the stderr log; it is
    // ordinary bounded retention, never `.iii-prior-*` migration evidence.
    let previous = stderr_previous(path)?;
    let lease = crate::worker_admission::with_paths(
        lease.clone(),
        &[PathRequest {
            path: &previous,
            role: PathRole::MutableFile,
        }],
    )?;
    let mut missing = Vec::new();
    for member in [path, previous.as_path()] {
        match std::fs::symlink_metadata(member) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(member.to_path_buf())
            }
            Err(error) => return Err(error),
        }
    }
    // Created once per exact pair; an existing member is proven by the core.
    if !missing.is_empty() {
        let mut guard = lease.begin(crate::worker_admission::MutationPlan::ControlFiles {
            files: missing.clone(),
        })?;
        for member in &missing {
            let file = guard.stage_file(member, 0o600)?;
            guard.publish_file(file)?;
        }
        guard.commit()?;
    }
    let log = lease.open_log(crate::worker_admission::LogSpec {
        current: path,
        previous: Some(&previous),
        cap_bytes: STDERR_LOG_CAP,
    })?;
    // Each launch starts its own stderr, as the old empty replacement did.
    log.reset_instance()?;
    Ok(log)
}

/// Ordinary sandbox stderr retention: past this the log rotates IN PLACE into
/// its declared `.1` sibling (one rotated window kept), as the worker console.
const STDERR_LOG_CAP: u64 = 10 * 1024 * 1024;

fn stderr_previous(path: &Path) -> std::io::Result<PathBuf> {
    let mut name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("stderr log has no file name"))?
        .to_os_string();
    name.push(".1");
    Ok(path.with_file_name(name))
}

/// One record through the exact core log writer; no stage, prior or journal.
fn append_stderr(log: &crate::worker_admission::LogWriter, bytes: &[u8]) -> std::io::Result<()> {
    log.append(bytes).map(|_| ())
}

/// A writer whose child was never spawned has no pump: finish it now so a
/// never-spawned rollback is not blocked, and never hide a finish failure.
fn settle_unspawned_stderr(log: crate::worker_admission::LogWriter, cause: String) -> String {
    match log.finish() {
        Ok(_) => cause,
        Err(error) => format!("{cause}; stderr log retained unfinished: {error}"),
    }
}

pub struct VmBootLauncher;

#[async_trait::async_trait]
impl VmLauncher for VmBootLauncher {
    fn preflight(&self) -> Result<(), SandboxError> {
        require_overlay_capable(iii_filesystem::init::has_init())
    }

    async fn boot(&self, _params: &BootParams) -> Result<BootHandle, SandboxError> {
        Err(SandboxError::BootFailed(
            "direct sandbox boot requires an admitted registry lifecycle".into(),
        ))
    }

    async fn boot_admitted(
        &self,
        params: &BootParams,
        admission: crate::worker_admission::Lease,
        registry: &super::registry::SandboxRegistry,
        id: uuid::Uuid,
    ) -> Result<BootHandle, SandboxError> {
        let t_boot_start = Instant::now();
        admission
            .revalidate()
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        // Defense in depth: `handle_create` already ran `preflight`;
        // re-check before libkrunfw provisioning for any other caller.
        require_overlay_capable(iii_filesystem::init::has_init())?;
        // We're running inside the sandbox-libkrun binary ourselves, so the
        // path to fork+exec for __vm-boot is our own executable. More
        // reliable than a PATH lookup: current_exe() is guaranteed to
        // resolve (unlike PATH, which can be empty in service managers)
        // and cannot disagree with the version we compiled against.
        let bin = std::env::current_exe()
            .map_err(|e| SandboxError::BootFailed(format!("current_exe() failed: {e}")))?;

        // Per-host provisioning: codesign (macOS Hypervisor entitlement)
        // and libkrunfw dylib placement. Both are idempotent but touch
        // the filesystem, and this runs on every sandbox::create --
        // when the user is issuing a stream of short runs, that's
        // wasteful lag on the hot path. Cache completion in an atomic
        // so only the first boot of each daemon lifetime pays for it.
        //
        // If the cached bit is missing (first boot, or restart after
        // the user wiped ~/.iii/lib/), we run both steps. If it's set,
        // we trust the previous boot's side-effects — if the user
        // manually deletes the dylib mid-session, __vm-boot will surface
        // a `libkrunfw: load` error which is diagnostic enough.
        if !PROVISION_DONE.load(Ordering::Acquire) {
            #[cfg(target_os = "macos")]
            {
                let t0 = Instant::now();
                if let Err(e) =
                    crate::cli::worker_manager::platform::ensure_macos_entitlements(&bin)
                {
                    tracing::warn!(error = %e, "failed to codesign sandbox-libkrun for Hypervisor entitlement");
                }
                tracing::info!(
                    ms = t0.elapsed().as_millis() as u64,
                    "boot_phase: codesign (first boot)"
                );
            }
        } else {
            tracing::debug!("boot_phase: codesign (skipped, cached)");
        }

        // Ensure `libkrunfw.<soname>` is discoverable by the loader, as
        // local_worker does for worker VMs. When sandbox-libkrun ships
        // with embed-libkrunfw, this extracts the embedded bytes to
        // ~/.iii/lib/ on first use; otherwise it needs a local libkrunfw
        // (III_LIBKRUNFW_PATH, ~/.iii/lib or a system path). Without
        // this step, dlopen(libkrunfw.X.dylib) fails inside the spawned
        // __vm-boot subprocess, libkrun's vm.enter() returns "build
        // error: libkrunfw: load", shell_relay binds shell.sock briefly
        // and then exits, and the caller sees the classic S300
        // "Connection refused" on the socket that should still be live.
        if !PROVISION_DONE.load(Ordering::Acquire) {
            let t_fw = Instant::now();
            // Hard-fail: without libkrunfw the child `__vm-boot` will
            // briefly bind shell.sock during init, then exit when
            // `vm.enter()` can't dlopen the dylib. Our outer wait loop
            // sees the stale socket and returns "boot OK", then the
            // next `sandbox::exec` hits Connection refused — a very
            // expensive way to surface a missing dependency. Bail now
            // with a real error the user can act on.
            crate::cli::firmware::provision::ensure_libkrunfw()
                .await
                .map_err(|e| {
                    SandboxError::BootFailed(format!(
                        "ensure_libkrunfw failed (vm.enter would crash with \
                         dlopen error): {e}"
                    ))
                })?;
            tracing::info!(
                ms = t_fw.elapsed().as_millis() as u64,
                "boot_phase: ensure_libkrunfw (first boot)"
            );
            // Mark provisioning done only after libkrunfw is in place.
            // Codesign above is macOS-only and soft-failing; guarding
            // this flag on libkrunfw is the correctness-critical step.
            // With the hard-fail on libkrunfw, we never set this on a
            // partially-provisioned host.
            PROVISION_DONE.store(true, Ordering::Release);
        } else {
            tracing::debug!("boot_phase: ensure_libkrunfw (skipped, cached)");
        }

        // The guest root is this sandbox's read-only trampoline
        // (`params.rootfs`) plus the cached image as a read-only erofs
        // lower; the shared cache itself is never handed to the guest, and
        // nothing is written into it (the embedded init serves /init.krun
        // from memory). Deliberately does NOT consult `III_ROOTFS_MODE`:
        // the worker VMs' legacy opt-out would re-open the shared
        // writable root.
        let t_erofs = Instant::now();
        let prepared = params.prepared.as_ref().ok_or_else(|| {
            SandboxError::BootFailed("sandbox lower was not prepared before launch".into())
        })?;
        prepared
            .validate(&prepared.reference)
            .map_err(|e| SandboxError::BootFailed(format!("{e:#}")))?;
        if prepared.root != params.base_rootfs {
            return Err(SandboxError::BootFailed(
                "prepared cache selection differs from boot params".into(),
            ));
        }
        let rootfs_lower = prepared.lower.clone();
        tracing::info!(
            ms = t_erofs.elapsed().as_millis() as u64,
            lower = %rootfs_lower.display(),
            "boot_phase: read-only base erofs"
        );

        admission
            .revalidate()
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        let _root_use = admission
            .use_path(&params.rootfs)
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        let _lower_use = admission
            .use_path(&rootfs_lower)
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        let env_stdin = env_stdin_frame(params)?;
        let mut cmd = tokio::process::Command::new(&bin);
        cmd.arg("__vm-boot")
            .args(vm_boot_args(params, &rootfs_lower))
            .stdin(std::process::Stdio::piped());

        // Capture stderr to a per-sandbox log. Dropping to /dev/null
        // masked clap parse errors and libkrun panics as opaque 30s
        // timeouts. A new file only: never follow a planted symlink.
        let log_path = params
            .rootfs
            .parent()
            .ok_or_else(|| SandboxError::BootFailed("sandbox root has no layout parent".into()))?
            .join("vm-boot.stderr.log");

        // Lifeline: the registry entry will hold the write end; if this
        // daemon dies (any way, SIGKILL included) or the sandbox entry is
        // dropped, the VM's __vm-boot watcher sees EOF and self-terminates —
        // VMs no longer outlive the daemon as orphaned session leaders.
        let lifeline = crate::daemon_exit::attach_lifeline(&mut cmd)
            .map_err(|e| SandboxError::BootFailed(format!("lifeline pipe: {e}")))?;

        let t_spawn = Instant::now();
        crate::cli::worker_manager::libkrun::attach_child_admission(&mut cmd, &admission)
            .map_err(SandboxError::BootFailed)?;
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let activity = super::registry::net_activity_path(&params.shell_sock);
        let runtime_paths: Vec<_> = if params.network {
            vec![crate::worker_admission::RuntimePath {
                path: &activity,
                kind: crate::worker_admission::RuntimePathKind::ActivityStamp,
            }]
        } else {
            Vec::new()
        };
        drop(_root_use);
        drop(_lower_use);
        // Capture stderr through a pipe into ONE exact core log writer (stable
        // descriptors, bounded in-place rotation into `.1`), issued right
        // before the runtime; the child never receives a log descriptor.
        let stderr_log = initialize_stderr(&admission, &log_path)
            .map_err(|e| SandboxError::BootFailed(e.to_string()))?;
        let launch = match admission.prepare_runtime(&runtime_paths) {
            Ok(launch) => launch,
            Err(error) => {
                return Err(SandboxError::BootFailed(settle_unspawned_stderr(
                    stderr_log,
                    error.to_string(),
                )));
            }
        };
        let mut child = match launch.spawn(cmd) {
            Ok(child) => child,
            Err(error) => {
                let (cause, child) = error.into_parts();
                if let Some(mut child) = child {
                    drop(child.take_stdin());
                    let stderr = child.take_stderr();
                    let session = RuntimeSession::supervise(
                        child,
                        admission.clone(),
                        vec![params.shell_sock.clone()],
                    );
                    session.pump_stderr(stderr, stderr_log);
                    registry.attach_runtime(id, session).await?;
                    return Err(SandboxError::BootFailed(format!("spawn sandbox: {cause}")));
                }
                return Err(SandboxError::BootFailed(settle_unspawned_stderr(
                    stderr_log,
                    format!("spawn sandbox: {cause}"),
                )));
            }
        };
        let vm_pid = child.id();
        let stdin = child.take_stdin();
        let stderr = child.take_stderr();
        let session =
            RuntimeSession::supervise(child, admission.clone(), vec![params.shell_sock.clone()]);
        // Ownership is installed synchronously above, before registry or stdin awaits.
        session.pump_stderr(stderr, stderr_log);
        registry.attach_runtime(id, session.clone()).await?;
        let vm_pid =
            vm_pid.ok_or_else(|| SandboxError::BootFailed("missing child diagnostic id".into()))?;
        crate::cli::worker_manager::libkrun::feed_env_stdin(stdin, &env_stdin)
            .await
            .map_err(|e| SandboxError::BootFailed(format!("send sandbox env: {e}")))?;
        tracing::info!(
            ms = t_spawn.elapsed().as_millis() as u64,
            pid = vm_pid,
            "boot_phase: spawn __vm-boot"
        );

        // Wait up to 30 s for the shell socket to appear AND accept a
        // connection. File-existence alone is not enough: the previous
        // implementation broke out of the loop the instant shell_relay
        // called `bind()`, before it had a chance to die on a missing
        // libkrunfw. That surfaced downstream as S300 "Connection
        // refused" on the next `sandbox::exec`.
        //
        // Three ways this loop exits:
        //   1. a test connect() succeeds — the relay is up; go on to the guest.
        //   2. the child PID is no longer alive — VM died; bail with
        //      the tail of the stderr log so the user can see why.
        //   3. 30 s elapsed — bail with a useful error.
        let t_sock = Instant::now();
        let sock = params.shell_sock.clone();
        let deadline = Instant::now() + BOOT_SOCKET_TIMEOUT;
        loop {
            if sock.exists() {
                // Try to connect. If the listener is actually
                // accepting, we're done.
                match tokio::net::UnixStream::connect(&sock).await {
                    Ok(_stream) => break,
                    // Connection refused = socket file present but no
                    // listener, likely because the relay bound then
                    // crashed. Keep looping only if the child PID is
                    // still alive; otherwise bail fast.
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                        if session.observation().is_terminal()
                            || session.observation().os_status().is_some()
                        {
                            let hint = read_stderr_tail(&log_path);
                            return Err(SandboxError::BootFailed(format!(
                                "__vm-boot child {vm_pid} exited before the shell relay \
                                 accepted connections. Last stderr lines:\n{hint}"
                            )));
                        }
                    }
                    // Any other connect error is unusual — treat it
                    // the same as a timeout so the caller sees a
                    // specific error instead of an opaque hang.
                    Err(e) => {
                        let hint = read_stderr_tail(&log_path);
                        return Err(SandboxError::BootFailed(format!(
                            "connect({}) failed unexpectedly: {e}. Last stderr:\n{hint}",
                            sock.display()
                        )));
                    }
                }
            } else if session.observation().is_terminal()
                || session.observation().os_status().is_some()
            {
                let hint = read_stderr_tail(&log_path);
                return Err(SandboxError::BootFailed(format!(
                    "__vm-boot child {vm_pid} exited before binding shell socket {}. \
                     Last stderr lines:\n{hint}",
                    sock.display()
                )));
            }
            if Instant::now() >= deadline {
                let hint = read_stderr_tail(&log_path);
                return Err(SandboxError::BootFailed(format!(
                    "shell socket {} did not start accepting within {:?}. Last stderr:\n{hint}",
                    sock.display(),
                    BOOT_SOCKET_TIMEOUT
                )));
            }
            tokio::time::sleep(BOOT_SOCKET_POLL_INTERVAL).await;
        }
        tracing::info!(
            ms = t_sock.elapsed().as_millis() as u64,
            total_ms = t_boot_start.elapsed().as_millis() as u64,
            "boot_phase: shell_sock_wait (boot total)"
        );

        // The relay binds `shell.sock` before the VM is even built: wait for
        // the guest itself, so the first exec's deadline does not pay for the
        // boot and a guest that never comes up fails the create.
        let t_guest = Instant::now();
        guest_answers(&sock, GUEST_READY_TIMEOUT)
            .await
            .map_err(|e| {
                let hint = read_stderr_tail(&log_path);
                SandboxError::BootFailed(format!(
                    "the guest behind {} did not answer: {e}. Last stderr:\n{hint}",
                    sock.display()
                ))
            })?;
        tracing::info!(
            ms = t_guest.elapsed().as_millis() as u64,
            total_ms = t_boot_start.elapsed().as_millis() as u64,
            "boot_phase: guest_ready"
        );

        Ok(BootHandle {
            vm_pid,
            lifeline: Some(std::sync::Arc::new(lifeline)),
        })
    }
}

/// One fs round trip through the relay at `sock`, within `within`: proof the
/// guest is up, which a connect to the relay is not.
async fn guest_answers(sock: &Path, within: Duration) -> Result<(), String> {
    let round_trip = async {
        let session = Session::connect(sock).await?;
        session
            .fs_call(iii_shell_proto::FsOp::Stat {
                path: "/".to_string(),
            })
            .await
    };
    match tokio::time::timeout(within, round_trip).await {
        // An fs error is an answer too: the guest is up.
        Ok(Ok(_)) | Ok(Err(iii_shell_client::VmClientError::FsError { .. })) => Ok(()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err(format!("no answer within {within:?}")),
    }
}

/// Diagnostic text only (never a cursor or authority): the rotated window,
/// then the current one, so a recent rotation does not hide the failure.
fn read_stderr_tail(log_path: &std::path::Path) -> String {
    const TAIL_LINES: usize = 32;
    const MAX_BYTES: usize = 4096;
    let rotated = stderr_previous(log_path)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    let content = match std::fs::read_to_string(log_path).map(|current| rotated + &current) {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => return format!("  (stderr log at {} is empty)", log_path.display()),
        Err(e) => return format!("  (could not read stderr log {}: {e})", log_path.display()),
    };
    let trimmed: String = content
        .lines()
        .rev()
        .take(TAIL_LINES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    // Cap size so a runaway log doesn't blow up the error message.
    if trimmed.len() > MAX_BYTES {
        let start = trimmed.len() - MAX_BYTES;
        format!("  ...(truncated)\n{}", &trimmed[start..])
    } else {
        trimmed
    }
}

/// Detect dispatcher errors that represent in-VM `execve()` failures
/// (POSIX ENOENT/ENOTDIR/EACCES) and convert them into a synthetic
/// `ExecResponse` carrying the canonical POSIX shell exit code (127 for
/// "not found", 126 for "permission denied"). Returns `None` for any
/// other dispatcher error so the caller preserves `BootFailed` (S300)
/// semantics.
///
/// **Why substring matching:** the in-VM dispatcher (iii-init's
/// `shell_dispatcher.rs:193`) builds its `Error { message }` frame as
/// `format!("spawn: {e}")` where `e` is a `std::io::Error` from
/// `Command::spawn()`. By the time the host receives this through
/// `VmClientError::DispatcherError(String)`, the structured `ErrorKind`
/// has been flattened to its `Display` representation. The strings we
/// match here are stable POSIX `strerror(3)` messages plus libstd's
/// `(os error N)` suffix, which are stable across every Linux target
/// the sandbox runs on. Promoting this to a structured wire type
/// (e.g. carrying `errno: i32` end-to-end through the proto) would be
/// the right next step if these strings ever drift; today the brittle
/// substring check is the smallest correct change.
///
/// `cmd` is folded into the synthetic stderr the way `/bin/sh` does
/// (`exec: <cmd>: not found`) so users see what failed without
/// scrolling through `code: S300` chrome that no longer applies.
fn classify_dispatcher_spawn_error(msg: &str, cmd: &str, duration_ms: u64) -> Option<ExecResponse> {
    // `os error N` is the canonical libstd suffix on Unix; we anchor on
    // it to avoid catching unrelated dispatcher errors that happen to
    // mention "permission" in some other context.
    let is_enoent = msg.contains("No such file or directory") || msg.contains("(os error 2)");
    let is_enotdir = msg.contains("Not a directory") || msg.contains("(os error 20)");
    let is_eacces = msg.contains("Permission denied") || msg.contains("(os error 13)");

    if is_enoent || is_enotdir {
        return Some(ExecResponse {
            stdout: String::new(),
            stderr: format!("exec: {cmd}: not found\n"),
            exit_code: Some(127),
            timed_out: false,
            duration_ms,
            success: false,
            stdout_truncated: false,
            stderr_truncated: false,
        });
    }
    if is_eacces {
        return Some(ExecResponse {
            stdout: String::new(),
            stderr: format!("exec: {cmd}: permission denied\n"),
            exit_code: Some(126),
            timed_out: false,
            duration_ms,
            success: false,
            stdout_truncated: false,
            stderr_truncated: false,
        });
    }
    None
}

/// Translate the wire-level `stdin: Option<String>` into the byte-level
/// payload `iii-shell-client::Session::run` expects.
///
/// `Some(vec![])` -> open stdin, write 0 bytes, send EOF (lib.rs:448).
/// `None` -> don't touch stdin; child stays blocked on read.
///
/// Both `Some("")` and missing-stdin must map to `Some(vec![])` so an
/// exec that reads from stdin (e.g. `for line in sys.stdin:`) gets EOF
/// instead of timing out.
fn decode_stdin(stdin: Option<&str>) -> Result<Option<Vec<u8>>, SandboxError> {
    match stdin {
        Some("") => Ok(Some(Vec::new())),
        Some(s) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(s.as_bytes())
                .map_err(|e| SandboxError::InvalidRequest(format!("stdin base64 decode: {e}")))?;
            Ok(Some(bytes))
        }
        None => Ok(Some(Vec::new())),
    }
}

/// Uses `tokio::task::spawn_blocking` + an inner `current_thread`
/// runtime so the `!Send` `&mut dyn OutputSink` reference never crosses
/// the outer multi-thread runtime's Send boundary.
pub struct ShellProtoRunner {
    /// Ceiling applied to every caller-supplied `timeout_ms`. See
    /// `SandboxConfig::max_exec_timeout_ms`: the reaper exempts busy
    /// sandboxes from reclamation, so an unbounded deadline would let one
    /// exec pin a VM forever.
    max_timeout_ms: u64,
}

impl ShellProtoRunner {
    pub fn new(max_timeout_ms: u64) -> Self {
        Self {
            max_timeout_ms: max_timeout_ms.max(1),
        }
    }
}

impl Default for ShellProtoRunner {
    fn default() -> Self {
        Self::new(DEFAULT_EXEC_TIMEOUT_MS)
    }
}

#[async_trait::async_trait]
impl ShellRunner for ShellProtoRunner {
    async fn run(&self, sock: PathBuf, req: &ExecRequest) -> Result<ExecResponse, SandboxError> {
        let cmd = req.cmd.clone();
        let args = req.args.clone();
        // `handle_exec` always normalises req.env to `EnvShape::Vec(_)` before
        // dispatching, so `into_kv_vec()` here can never hit the Map-validation
        // path. If a future caller bypasses handle_exec, the error surfaces
        // up the stack via the `?`.
        let env = req.env.clone().into_kv_vec()?;
        let cwd = req.workdir.clone();
        // Clamped, not trusted: `timeout_ms` is caller-supplied and the
        // reaper protects busy sandboxes, so an unbounded deadline pins a VM.
        let timeout_ms = req
            .timeout_ms
            .unwrap_or(DEFAULT_EXEC_TIMEOUT_MS)
            .min(self.max_timeout_ms);

        let stdin: Option<Vec<u8>> = decode_stdin(req.stdin.as_deref())?;

        let join: Result<ExecResponse, SandboxError> = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| SandboxError::BootFailed(format!("inner runtime build: {e}")))?;
            rt.block_on(async move {
                let started = Instant::now();

                let t_connect = Instant::now();
                let session = Session::connect(&sock)
                    .await
                    .map_err(|e| SandboxError::BootFailed(format!("shell connect: {e}")))?;
                tracing::info!(
                    ms = t_connect.elapsed().as_millis() as u64,
                    "exec_phase: shell_connect"
                );

                // Keep a copy of `cmd` for the spawn-failure error path
                // (`classify_dispatcher_spawn_error`) so the synthetic
                // stderr can name the binary the user tried to exec.
                // `RequestSpec` consumes the original, so without this
                // we'd have nothing to render after the move.
                let cmd_for_err = cmd.clone();
                let spec = RequestSpec {
                    cmd,
                    args,
                    cwd,
                    env,
                    stdin,
                };

                let mut sink = VecSink::with_cap(OUTPUT_CAP);

                // Use `tokio::time::timeout` as the SOLE timeout
                // authority and pass `None` to `session.run` so
                // iii-shell-client does not add its own post-timeout
                // kill+grace tail (WRITE_TIMEOUT 5s + POST_KILL_GRACE
                // 1s at iii-shell-client/src/lib.rs:69,79). Without
                // this, a timed-out exec holds `exec_in_progress` on
                // the registry for up to ~1.5s after the user's
                // deadline, causing every subsequent sandbox::exec
                // on the same handle to return S003 ("concurrent
                // exec"). When the outer timeout fires, the
                // `session.run` future is dropped; its UnixStream
                // drops; the host-side socket closes; the relay
                // tears down its virtio-console view. `handle_exec`
                // then calls `end_exec` immediately, freeing the
                // sandbox for the next request.
                let t_run = Instant::now();
                let outer = tokio::time::timeout(
                    Duration::from_millis(timeout_ms),
                    session.run(spec, &mut sink, None),
                )
                .await;
                tracing::info!(
                    ms = t_run.elapsed().as_millis() as u64,
                    "exec_phase: shell_run (in-VM)"
                );

                let outcome = match outer {
                    Ok(Ok(o)) => o,
                    Ok(Err(e)) => {
                        // In-VM `execve()` failures (binary missing, not
                        // executable, intermediate path component is a
                        // file) arrive as `DispatcherError`. POSIX shells
                        // surface these as exit 127 / 126; do the same so
                        // callers don't see S300 BootFailed for what is
                        // really a per-exec spawn failure on a healthy
                        // VM. See `classify_dispatcher_spawn_error` for
                        // the substring rationale. All other dispatcher
                        // errors fall through to the original
                        // BootFailed mapping.
                        if let iii_shell_client::VmClientError::DispatcherError(msg) = &e
                            && let Some(resp) = classify_dispatcher_spawn_error(
                                msg,
                                &cmd_for_err,
                                started.elapsed().as_millis() as u64,
                            )
                        {
                            return Ok(resp);
                        }
                        return Err(SandboxError::BootFailed(format!("shell run: {e}")));
                    }
                    Err(_) => {
                        // Timeouts are expected outcomes of
                        // sandbox::exec (user commands can legitimately
                        // exceed the deadline). Kept at info because
                        // returning S200 already signals the caller —
                        // this log is only for operator awareness,
                        // not an error.
                        tracing::info!(timeout_ms, "exec timed out; session dropped, forcing S200");
                        return Err(SandboxError::ExecTimedOut { timeout_ms });
                    }
                };

                let duration_ms = started.elapsed().as_millis() as u64;
                let timed_out = outcome.status.timed_out;
                let exit_code = outcome.status.code;
                let success = exit_code == Some(0) && !timed_out;

                // `Session::run` can still surface timed_out=true if
                // iii-shell-client decides the stream hit some other
                // deadline; map it to S200 for caller parity.
                if timed_out {
                    return Err(SandboxError::ExecTimedOut { timeout_ms });
                }

                Ok(ExecResponse {
                    stdout: String::from_utf8_lossy(&sink.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&sink.stderr).into_owned(),
                    exit_code,
                    timed_out,
                    duration_ms,
                    success,
                    stdout_truncated: outcome.stdout_truncated,
                    stderr_truncated: outcome.stderr_truncated,
                })
            })
        })
        .await
        .map_err(|e| SandboxError::BootFailed(format!("spawn_blocking join: {e}")))?;

        join
    }
}

/// Test-only race-free process handle: `pidfd_open` of a child this process
/// still owns and has NOT reaped, taken right after spawn and before any
/// waiter can reap it, so it can only ever name that process.
#[cfg(test)]
pub(crate) fn test_pidfd_open(pid: u32) -> std::io::Result<std::os::fd::OwnedFd> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::FromRawFd;
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as std::os::fd::RawFd) })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "fixture signals require a Linux pidfd",
        ))
    }
}

/// Signal only through a spawn-time pidfd; never `kill(pid)`. ESRCH (the
/// process already exited) is success: the pidfd cannot reach a reused PID.
#[cfg(test)]
pub(crate) fn test_pidfd_signal(
    pidfd: &std::os::fd::OwnedFd,
    signal: libc::c_int,
) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let sent = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if sent < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pidfd, signal);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "fixture signals require a Linux pidfd",
        ))
    }
}

/// The VMs this process spawned and has not reaped yet, each with the pidfd
/// taken before its waiter existed. The stopper signals only these, and only
/// through that pidfd. Global because every handler builds its own
/// `SignalStopper`.
#[cfg(test)]
static UNREAPED_VMS: Mutex<Vec<(u32, std::os::fd::OwnedFd)>> = Mutex::new(Vec::new());

#[cfg(test)]
fn unreaped_vms() -> MutexGuard<'static, Vec<(u32, std::os::fd::OwnedFd)>> {
    UNREAPED_VMS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Hand the VM `child` (pid `pid`) to a task that reaps it on exit, and keep
/// it signalable through its pidfd until then. The pidfd is opened here,
/// while `child` is still owned and unwaited, so it names exactly this child;
/// if it cannot be opened the VM is simply not signalable (no PID fallback).
///
/// Admission-v2 integration boundary: RuntimeChild::wait performs OS reap and
/// THEN fallible/possibly waiting catalog finalization. Substituting that wait
/// here would leave a reused PID signalable during finalization. It requires a
/// core-owned reap observation synchronized with signal_vm, before catalog work;
/// neither a caller bool nor a final wait error can supply that evidence.
#[cfg(test)]
fn watch_vm(mut child: tokio::process::Child, pid: u32) {
    if child.id() == Some(pid) {
        if let Ok(pidfd) = test_pidfd_open(pid) {
            unreaped_vms().push((pid, pidfd));
        }
    }
    tokio::spawn(async move {
        let _ = child.wait().await;
        unreaped_vms().retain(|(p, _)| *p != pid);
    });
}

/// Send `signal` to the VM `pid` names if it is a VM not reaped yet, through
/// its spawn-time pidfd. A VM reaped but not yet removed from the list only
/// yields ESRCH from its own pidfd; no other process can be reached.
#[cfg(test)]
fn signal_vm(pid: u32, signal: nix::sys::signal::Signal) {
    let vms = unreaped_vms();
    if let Some((_, pidfd)) = vms.iter().find(|(p, _)| *p == pid) {
        let _ = test_pidfd_signal(pidfd, signal as libc::c_int);
    }
}

pub struct SignalStopper;

#[async_trait::async_trait]
impl VmStopper for SignalStopper {
    async fn stop(&self, vm_pid: u32) -> Result<(), SandboxError> {
        #[cfg(not(test))]
        {
            let _ = vm_pid;
            return Err(SandboxError::BootFailed(
                "PID-only stop has no runtime control authority".into(),
            ));
        }
        #[cfg(test)]
        {
            self.stop_fixture(vm_pid).await
        }
    }
}
#[cfg(test)]
impl SignalStopper {
    async fn stop_fixture(&self, vm_pid: u32) -> Result<(), SandboxError> {
        use nix::sys::signal::Signal;

        let t_stop = Instant::now();

        // SIGTERM first so iii-init/libkrun can close cleanly when they
        // do respond. The grace here is intentionally short: the sandbox
        // VM's upper layer is tmpfs and discarded on reap, so "allow
        // flush" isn't a real use case. Every millisecond of grace is
        // visible in `sandbox run` end-to-end latency.
        signal_vm(vm_pid, Signal::SIGTERM);

        tokio::time::sleep(Duration::from_millis(STOP_GRACE_MS)).await;

        signal_vm(vm_pid, Signal::SIGKILL);

        tracing::info!(
            ms = t_stop.elapsed().as_millis() as u64,
            pid = vm_pid,
            "stop_phase: SIGTERM+grace+SIGKILL"
        );
        Ok(())
    }
}

#[cfg(test)]
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct RuntimeTestResources {
    pub reads: Vec<PathBuf>,
    pub endpoints: Vec<PathBuf>,
    pub exit_without_drop: bool,
    pub skip_bind: bool,
    pub descendant_release: Option<PathBuf>,
}

#[cfg(test)]
pub(crate) async fn runtime_test_session(
    lease: crate::worker_admission::Lease,
    namespace: &Path,
    keep_running: bool,
) -> std::sync::Arc<RuntimeSession> {
    runtime_test_session_resources(
        lease,
        namespace,
        keep_running,
        RuntimeTestResources::default(),
    )
    .await
}

#[cfg(test)]
pub(crate) async fn runtime_test_session_resources(
    lease: crate::worker_admission::Lease,
    namespace: &Path,
    keep_running: bool,
    resources: RuntimeTestResources,
) -> std::sync::Arc<RuntimeSession> {
    runtime_test_session_resources_inner(lease, namespace, keep_running, resources, None)
        .await
        .expect("owned fixture activation failed after child was reaped and drained")
}

#[cfg(test)]
pub(crate) async fn runtime_test_session_registered(
    lease: crate::worker_admission::Lease,
    namespace: &Path,
    registry: &super::registry::SandboxRegistry,
    id: uuid::Uuid,
    resources: RuntimeTestResources,
) -> std::io::Result<std::sync::Arc<RuntimeSession>> {
    runtime_test_session_resources_inner(lease, namespace, true, resources, Some((registry, id)))
        .await
}

#[cfg(test)]
async fn runtime_test_session_resources_inner(
    lease: crate::worker_admission::Lease,
    namespace: &Path,
    keep_running: bool,
    resources: RuntimeTestResources,
    attach: Option<(&super::registry::SandboxRegistry, uuid::Uuid)>,
) -> std::io::Result<std::sync::Arc<RuntimeSession>> {
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg("-c").arg(r#"
        binary=$1; shift
        while [ "$#" -gt 0 ]; do
            case "$1" in
              --admission-fd) export III_TEST_ADMISSION_FD="$2"; shift 2;;
              --runtime-write-fd) export III_TEST_RUNTIME_FD="$2"; shift 2;;
              *) exit 91;;
            esac
        done
        exec "$binary" --exact sandbox_daemon::adapters::tests::runtime_owned_child_fixture --nocapture
    "#).arg("owned-runtime-fixture").arg(std::env::current_exe().unwrap());
    command.env("III_TEST_ADMISSION_NAMESPACE", namespace).env(
        "III_TEST_RUNTIME_LIVE",
        if keep_running { "1" } else { "0" },
    );
    command.env(
        "III_TEST_RUNTIME_RESOURCES",
        serde_json::to_string(&resources).unwrap(),
    );
    crate::cli::worker_manager::libkrun::attach_child_admission(&mut command, &lease).unwrap();
    command.stdout(std::process::Stdio::piped());
    let mut child = lease
        .prepare_runtime(&[])
        .unwrap()
        .spawn(command)
        .unwrap_or_else(|e| panic!("runtime fixture spawn: {}", e.error()));
    let stdout = child.take_stdout().unwrap();
    let session = RuntimeSession::supervise(child, lease, resources.endpoints);
    let attached = if let Some((registry, id)) = attach {
        registry
            .attach_runtime(id, session.clone())
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))
    } else {
        Ok(())
    };
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let activated = if attached.is_ok() {
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(line) = lines.next_line().await? {
                if line.contains("RUNTIME_READY") {
                    return Ok(());
                }
            }
            Err(std::io::Error::other(
                "runtime fixture exited without activation acknowledgement",
            ))
        })
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "fixture activation timed out")
        })
        .and_then(|result| result)
    } else {
        attached
    };
    let drain = tokio::spawn(async move {
        while lines.next_line().await?.is_some() {}
        Ok(())
    });
    session.host_writers.lock().await.push(drain);
    if let Err(error) = activated {
        // Report failure only after this actual owned child has exited and the
        // pipe has drained. Never delete its namespace or signal a raw PID.
        let signal_error = if session.observation().os_status().is_none() {
            session
                .signal(crate::worker_admission::RuntimeSignal::Kill)
                .err()
        } else {
            None
        };
        let (status, retirement) = session.wait().await;
        let drained = session.finish_host_writers().await;
        return Err(std::io::Error::other(format!(
            "{error}; signal={signal_error:?}; status={status:?}; retirement={:?}; drain={drained:?}",
            retirement.err()
        )));
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_owned_child_fixture() {
        use std::os::fd::{FromRawFd, OwnedFd};
        let Some(namespace) = std::env::var_os("III_TEST_ADMISSION_NAMESPACE") else {
            return;
        };
        let fd = |key| std::env::var(key).unwrap().parse::<i32>().unwrap();
        let context =
            crate::worker_admission::testing::open_context(Path::new(&namespace)).unwrap();
        let lease = context
            .import_child(unsafe { OwnedFd::from_raw_fd(fd("III_TEST_ADMISSION_FD")) })
            .unwrap();
        let writer = crate::worker_admission::import_runtime_writer(&lease, unsafe {
            OwnedFd::from_raw_fd(fd("III_TEST_RUNTIME_FD"))
        })
        .unwrap();
        writer.checkpoint().unwrap();
        let resources: RuntimeTestResources =
            serde_json::from_str(&std::env::var("III_TEST_RUNTIME_RESOURCES").unwrap()).unwrap();
        let reads: Vec<_> = resources
            .reads
            .iter()
            .map(|path| writer.use_path(path).unwrap())
            .collect();
        let endpoints: Vec<_> = if resources.skip_bind {
            Vec::new()
        } else {
            resources
                .endpoints
                .iter()
                .map(|path| writer.endpoint(path).unwrap().bind().unwrap())
                .collect()
        };
        if let Some(release) = &resources.descendant_release {
            // The guards, not this variable, must retain the dedicated writer.
            drop(writer);
            let child = unsafe { libc::fork() };
            assert!(child >= 0);
            if child == 0 {
                let deadline = Instant::now() + Duration::from_secs(10);
                while !release.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                unsafe { libc::_exit(0) }
            }
            println!("RUNTIME_READY");
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            unsafe { libc::_exit(0) }
        }
        println!("RUNTIME_READY");
        std::io::Write::flush(&mut std::io::stdout()).unwrap();
        if std::env::var("III_TEST_RUNTIME_LIVE").unwrap() == "1" {
            std::thread::sleep(Duration::from_secs(30));
        }
        // Test-only: the exit status of this owned runtime child after its
        // activation, for a scenario that must tell its own exit apart.
        let exit_code = std::env::var("III_TEST_RUNTIME_EXIT_CODE")
            .ok()
            .map(|code| code.parse::<i32>().unwrap());
        if resources.exit_without_drop {
            // Deliberately bypass every Rust Drop; only real wait + G settlement
            // may retire these imported runtime-owned reads/endpoints.
            unsafe { libc::_exit(exit_code.unwrap_or(0)) }
        }
        for endpoint in endpoints {
            endpoint.close().unwrap();
        }
        drop(reads);
        drop(writer);
        drop(lease);
        if let Some(code) = exit_code {
            unsafe { libc::_exit(code) }
        }
    }

    #[tokio::test]
    async fn r2_initialized_stderr_one_line_then_owned_retirement_allows_safe_cleanup() {
        use crate::worker_admission::{PathRequest, PathRole};
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let (lease, root, endpoints) = resource_fixture(data.path(), &context, 1);
        let stderr = root.join("vm-boot.stderr.log");
        // As the sandbox layout: the guest-read tree is a sibling of the log.
        let guest = root.join("rootfs");
        let lease = crate::worker_admission::with_paths(
            lease,
            &[PathRequest {
                path: &stderr,
                role: PathRole::MutableFile,
            }],
        )
        .unwrap();
        let mut create = lease
            .begin(crate::worker_admission::MutationPlan::Create { root: root.clone() })
            .unwrap();
        create.create_dir_all(&guest, 0o700).unwrap();
        create.commit().unwrap();
        let protected = data.path().join(".iii-prior-independent-evidence");
        std::fs::write(&protected, b"OLD independent evidence").unwrap();
        let log = initialize_stderr(&lease, &stderr).unwrap();
        append_stderr(&log, b"normal sandbox startup line\n").unwrap();
        assert_eq!(
            std::fs::read(&stderr).unwrap(),
            b"normal sandbox startup line\n"
        );
        let session = runtime_test_session_resources(
            lease.clone(),
            namespace.path(),
            false,
            RuntimeTestResources {
                reads: vec![guest.clone()],
                endpoints: endpoints.clone(),
                exit_without_drop: true,
                ..Default::default()
            },
        )
        .await;
        // The launch hands the session the original writer (the pump owns a
        // clone); `finish_resources` finishes it after every host writer.
        session.adopt_log(log);
        let (status, retirement) = session.wait().await;
        assert!(status.unwrap().success());
        let receipt = retirement.unwrap();
        session.finish_resources(&receipt).await.unwrap();
        for path in &endpoints {
            assert!(!path.exists());
        }
        // Normal stderr retention must not leave migration-like debris that
        // makes every routine stop fail. Authority is the original receipt.
        assert!(!std::fs::read_dir(&root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".iii-")
        }));
        let mut remove = receipt.begin_remove(&lease, &root, &[&root]).unwrap();
        remove.remove_path(&root).unwrap();
        remove.commit().unwrap();
        assert!(!root.exists());
        assert_eq!(
            std::fs::read(&protected).unwrap(),
            b"OLD independent evidence"
        );
    }

    #[tokio::test]
    async fn r2_logged_cleanup_does_not_treat_unrelated_prior_evidence_as_disposable() {
        use crate::worker_admission::{PathRequest, PathRole};
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let (lease, root, _) = resource_fixture(data.path(), &context, 0);
        let protected = root.join(".iii-prior-unrelated-evidence");
        // The entry exists before runtime issuance; no proof says it is log retention.
        std::fs::write(&protected, b"OLD must not disappear").unwrap();
        // Owner-private evidence: Remove must refuse it as evidence, not by mode.
        std::fs::set_permissions(
            &protected,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .unwrap();
        let stderr = root.join("vm-boot.stderr.log");
        let lease = crate::worker_admission::with_paths(
            lease,
            &[PathRequest {
                path: &stderr,
                role: PathRole::MutableFile,
            }],
        )
        .unwrap();
        let log = initialize_stderr(&lease, &stderr).unwrap();
        append_stderr(&log, b"ordinary line\n").unwrap();
        let session = runtime_test_session(lease.clone(), namespace.path(), false).await;
        session.adopt_log(log);
        let receipt = session.wait().await.1.unwrap();
        // The log is Finished here, so the refusal below is the evidence's.
        session.finish_resources(&receipt).await.unwrap();
        let refused = match receipt.begin_remove(&lease, &root, &[&root]) {
            Err(_) => true,
            Ok(mut removal) => removal.remove_path(&root).is_err(),
        };
        assert!(
            refused,
            "log cleanup must not authorize genuine prior evidence"
        );
        assert_eq!(
            std::fs::read(&protected).unwrap(),
            b"OLD must not disappear"
        );
    }

    fn r2_stderr_fixture(
        namespace: &Path,
        data: &Path,
    ) -> (crate::worker_admission::Lease, PathBuf, PathBuf) {
        use crate::worker_admission::{PathRequest, PathRole};
        let context = crate::worker_admission::testing::provision_context(namespace).unwrap();
        let (lease, root, _) = resource_fixture(data, &context, 0);
        let stderr = root.join("vm-boot.stderr.log");
        let lease = crate::worker_admission::with_paths(
            lease,
            &[PathRequest {
                path: &stderr,
                role: PathRole::MutableFile,
            }],
        )
        .unwrap();
        (lease, root, stderr)
    }

    fn r2_stderr_writer(script: &str) -> tokio::process::Child {
        tokio::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }

    /// R2 through the real pump: a piped writer emits ~12 MB, the log rotates
    /// IN PLACE into its declared `.1` (same inodes, no `.iii-*` debris), the
    /// session finishes the writer and the receipt's Remove consumes the base.
    #[tokio::test]
    async fn r2_routed_stderr_pump_rotates_in_place_and_cleanup_consumes_the_receipt() {
        use std::os::unix::fs::MetadataExt;
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let (lease, root, stderr) = r2_stderr_fixture(namespace.path(), data.path());
        let rotated = stderr_previous(&stderr).unwrap();
        let log = initialize_stderr(&lease, &stderr).unwrap();
        let inodes = [&stderr, &rotated].map(|path| std::fs::metadata(path).unwrap().ino());
        let session = runtime_test_session(lease.clone(), namespace.path(), false).await;
        let mut writer = r2_stderr_writer(
            "i=0; while [ $i -lt 12 ]; do head -c 1000000 /dev/zero | tr '\\0' x; echo; i=$((i + 1)); done >&2",
        );
        session.pump_stderr(writer.stderr.take(), log);
        assert!(writer.wait().await.unwrap().success());
        let receipt = session.wait().await.1.unwrap();
        session.finish_resources(&receipt).await.unwrap();
        // Rotation happens after the whole 11th record crossed the cap.
        let record = 1_000_001usize;
        assert_eq!(std::fs::read(&rotated).unwrap().len(), 11 * record);
        assert_eq!(std::fs::read(&stderr).unwrap().len(), record);
        for (path, inode) in [&stderr, &rotated].into_iter().zip(inodes) {
            assert_eq!(std::fs::metadata(path).unwrap().ino(), inode);
        }
        assert!(!std::fs::read_dir(&root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".iii-")
        }));
        let mut remove = receipt.begin_remove(&lease, &root, &[&root]).unwrap();
        remove.remove_path(&root).unwrap();
        remove.commit().unwrap();
        assert!(!root.exists());
    }

    /// R2 fault: a foreign writer swaps the admitted stderr inode after the
    /// guest was reaped. The late record fails closed, the failure is sticky in
    /// `finish_resources`, and the receipt cannot remove the retained base.
    #[tokio::test]
    async fn r2_routed_foreign_stderr_replacement_fails_closed_and_keeps_cleanup_blocked() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let (lease, root, stderr) = r2_stderr_fixture(namespace.path(), data.path());
        let log = initialize_stderr(&lease, &stderr).unwrap();
        let session = runtime_test_session(lease.clone(), namespace.path(), false).await;
        let receipt = session.wait().await.1.unwrap();
        let held = root.join("held-stderr");
        std::fs::rename(&stderr, &held).unwrap();
        std::fs::write(&stderr, b"FOREIGN\n").unwrap();
        // Owner-private like the original: refused by identity, not mode.
        std::fs::set_permissions(&stderr, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .unwrap();
        let mut writer = r2_stderr_writer("echo late-after-reap >&2");
        session.pump_stderr(writer.stderr.take(), log);
        assert!(writer.wait().await.unwrap().success());
        assert!(session.finish_resources(&receipt).await.is_err());
        assert!(
            session.finish_resources(&receipt).await.is_err(),
            "a host writer failure is sticky, never cleared by a retry"
        );
        assert!(receipt.begin_remove(&lease, &root, &[&root]).is_err());
        assert_eq!(std::fs::read(&stderr).unwrap(), b"FOREIGN\n");
        assert!(std::fs::read(&held).unwrap().is_empty());
        assert!(root.exists());
    }

    /// R2 negative: only an exact declared MutableFile can become a log; the
    /// payload's MutableTree root is not a log grant and nothing is created.
    #[test]
    fn r2_undeclared_stderr_path_is_refused_before_any_file_exists() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let (lease, root, _) = resource_fixture(data.path(), &context, 0);
        let stderr = root.join("vm-boot.stderr.log");
        assert!(initialize_stderr(&lease, &stderr).is_err());
        assert!(!stderr.exists());
        assert!(!stderr_previous(&stderr).unwrap().exists());
    }

    fn resource_fixture(
        t: &Path,
        context: &crate::worker_admission::AdmissionContext,
        count: usize,
    ) -> (crate::worker_admission::Lease, PathBuf, Vec<PathBuf>) {
        use crate::worker_admission::{PathRequest, PathRole};
        use std::os::unix::fs::PermissionsExt;
        let root = t.join("payload");
        let parent = t.join("endpoints");
        std::fs::create_dir(&parent).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        let paths: Vec<_> = (0..count)
            .map(|i| parent.join(format!("shell-{i}.sock")))
            .collect();
        let declarations: Vec<_> = paths
            .iter()
            .map(|path| PathRequest {
                path,
                role: PathRole::Endpoint,
            })
            .collect();
        let lease =
            crate::worker_admission::with_paths(context.local(&root).unwrap(), &declarations)
                .unwrap();
        let mut create = lease
            .begin(crate::worker_admission::MutationPlan::Create { root: root.clone() })
            .unwrap();
        create.create_dir_all(&root, 0o700).unwrap();
        create.commit().unwrap();
        (lease, root, paths)
    }

    #[tokio::test]
    async fn runtime_read_and_endpoints_settle_after_no_drop_exit_or_explicit_close() {
        for explicit_close in [false, true] {
            let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
            let data = crate::worker_admission::testing::private_tempdir().unwrap();
            let context =
                crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
            let (lease, root, paths) = resource_fixture(data.path(), &context, 2);
            let parent_read = lease.use_path(&root).unwrap();
            let session = runtime_test_session_resources(
                lease.clone(),
                namespace.path(),
                false,
                RuntimeTestResources {
                    reads: vec![root.clone()],
                    endpoints: paths.clone(),
                    exit_without_drop: !explicit_close,
                    ..Default::default()
                },
            )
            .await;
            let (status, receipt) = session.wait().await;
            assert!(status.unwrap().success());
            let receipt = receipt.unwrap();
            session.finish_resources(&receipt).await.unwrap();
            session.finish_resources(&receipt).await.unwrap(); // exact idempotence before Remove
            for path in &paths {
                assert!(!path.exists());
            }
            assert!(
                lease
                    .begin(crate::worker_admission::MutationPlan::Reclone { root: root.clone() })
                    .is_err(),
                "parent read must survive child retirement without consuming Remove authority"
            );
            parent_read.revalidate().unwrap();
            drop(parent_read);
            let mut removal = receipt.begin_remove(&lease, &root, &[&root]).unwrap();
            removal.remove_path(&root).unwrap();
            removal.commit().unwrap();
            assert!(!root.exists());
        }
    }

    #[tokio::test]
    async fn expected_endpoint_missing_or_replaced_never_becomes_an_empty_set() {
        for skip_bind in [false, true] {
            let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
            let data = crate::worker_admission::testing::private_tempdir().unwrap();
            let context =
                crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
            let (lease, root, paths) = resource_fixture(data.path(), &context, 2);
            let session = runtime_test_session_resources(
                lease.clone(),
                namespace.path(),
                false,
                RuntimeTestResources {
                    reads: vec![root.clone()],
                    endpoints: paths.clone(),
                    exit_without_drop: true,
                    skip_bind,
                    ..Default::default()
                },
            )
            .await;
            let receipt = session.wait().await.1.unwrap();
            let replacement = if skip_bind {
                None
            } else {
                std::fs::rename(&paths[1], paths[1].with_extension("held")).unwrap();
                Some(std::os::unix::net::UnixListener::bind(&paths[1]).unwrap())
            };
            assert!(session.finish_resources(&receipt).await.is_err());
            assert_eq!(session.endpoints, paths);
            assert!(root.exists());
            if !skip_bind {
                assert!(
                    paths[0].exists(),
                    "prevalidate all paths before the first close"
                );
                assert!(paths[1].exists());
            }
            drop(replacement);
        }
    }

    #[tokio::test]
    async fn host_drain_failure_blocks_endpoint_settlement_and_removal() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let (lease, root, paths) = resource_fixture(data.path(), &context, 1);
        let session = runtime_test_session_resources(
            lease.clone(),
            namespace.path(),
            false,
            RuntimeTestResources {
                endpoints: paths.clone(),
                exit_without_drop: true,
                ..Default::default()
            },
        )
        .await;
        let receipt = session.wait().await.1.unwrap();
        session.host_writers.lock().await.push(tokio::spawn(async {
            Err(std::io::Error::other("host drain fault"))
        }));
        assert!(session.finish_resources(&receipt).await.is_err());
        assert!(session.finish_resources(&receipt).await.is_err());
        assert!(paths[0].exists());
        assert!(root.exists());
        assert!(receipt.begin_remove(&lease, &root, &[&root]).is_err());
    }

    #[tokio::test]
    async fn completed_endpoint_subset_stays_visible_when_a_later_endpoint_refuses() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let (lease, root, paths) = resource_fixture(data.path(), &context, 2);
        let session = runtime_test_session_resources(
            lease.clone(),
            namespace.path(),
            false,
            RuntimeTestResources {
                endpoints: paths.clone(),
                exit_without_drop: true,
                ..Default::default()
            },
        )
        .await;
        let receipt = session.wait().await.1.unwrap();
        session.finish_host_writers().await.unwrap();
        receipt
            .retire_runtime_endpoints(&lease, &[&paths[0]])
            .unwrap();
        std::fs::rename(&paths[1], paths[1].with_extension("held")).unwrap();
        let replacement = std::os::unix::net::UnixListener::bind(&paths[1]).unwrap();
        assert!(session.finish_resources(&receipt).await.is_err());
        assert!(!paths[0].exists());
        assert!(paths[1].exists());
        assert!(root.exists());
        assert!(receipt.begin_remove(&lease, &root, &[&root]).is_err());
        drop(replacement);
    }

    #[tokio::test]
    async fn descendant_read_or_listener_keeps_runtime_retirement_uncertain() {
        for listener in [false, true] {
            let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
            let data = crate::worker_admission::testing::private_tempdir().unwrap();
            let context =
                crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
            let (lease, root, paths) =
                resource_fixture(data.path(), &context, usize::from(listener));
            let release = data.path().join("release-descendant");
            let session = runtime_test_session_resources(
                lease.clone(),
                namespace.path(),
                false,
                RuntimeTestResources {
                    reads: if listener {
                        Vec::new()
                    } else {
                        vec![root.clone()]
                    },
                    endpoints: paths.clone(),
                    descendant_release: Some(release.clone()),
                    ..Default::default()
                },
            )
            .await;
            let (status, retirement) = session.wait().await;
            assert!(status.unwrap().success());
            assert!(
                retirement.is_err(),
                "surviving guard is actual writer ownership"
            );
            assert!(root.exists());
            for path in paths {
                assert!(path.exists());
            }
            std::fs::write(release, b"release").unwrap();
            session.finish_host_writers().await.unwrap();
            assert!(
                session.retirement().is_err(),
                "later descriptor closure does not erase the original failed completion"
            );
        }
    }

    #[tokio::test]
    async fn os_exit_without_runtime_activation_is_not_cleanup_authority() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let lease = context.local(data.path()).unwrap();
        let command = tokio::process::Command::new("/bin/true");
        let child = lease
            .prepare_runtime(&[])
            .unwrap()
            .spawn(command)
            .unwrap_or_else(|e| panic!("{}", e.error()));
        let control = child.control();
        let session = RuntimeSession::supervise(child, lease, Vec::new());
        let (status, retirement) = session.wait().await;
        assert!(status.unwrap().success());
        assert!(
            retirement.is_err(),
            "OS exit cannot activate/retire a writer"
        );
        assert_eq!(
            control.observation().phase(),
            crate::worker_admission::RuntimePhase::RetirementFailed
        );
        assert!(
            control
                .signal(crate::worker_admission::RuntimeSignal::Kill)
                .is_err()
        );
        assert!(data.path().exists());
    }

    #[tokio::test]
    async fn retired_process_with_unclosed_endpoint_cannot_remove_payload() {
        use crate::worker_admission::{PathRequest, PathRole};
        use std::os::unix::fs::PermissionsExt;
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
        let root = t.path().join("payload");
        let control = t.path().join("control");
        std::fs::create_dir(&control).unwrap();
        std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = control.join("shell.sock");
        let lease = crate::worker_admission::with_paths(
            context.local(&root).unwrap(),
            &[PathRequest {
                path: &socket,
                role: PathRole::Endpoint,
            }],
        )
        .unwrap();
        let mut create = lease
            .begin(crate::worker_admission::MutationPlan::Create { root: root.clone() })
            .unwrap();
        create.create_dir_all(&root, 0o700).unwrap();
        create.commit().unwrap();
        let endpoint = lease.endpoint(&socket).unwrap().bind().unwrap();
        let session = runtime_test_session(lease.clone(), namespace.path(), false).await;
        let receipt = session.wait().await.1.unwrap();
        session.finish_host_writers().await.unwrap();
        assert!(
            receipt
                .retire_runtime_endpoints(&lease, &[&socket])
                .is_err(),
            "parent-owned endpoint is not the child's settlement authority"
        );
        endpoint.close().unwrap();
        let mut removal = receipt.begin_remove(&lease, &root, &[&root]).unwrap();
        removal.remove_path(&root).unwrap();
        removal.commit().unwrap();
        assert!(!root.exists());
        assert!(!socket.exists());
        // Parent-owned endpoints remain independent. Child-owned endpoints use
        // the exact G settlement route exercised by the cross-exec tests.
    }

    #[tokio::test]
    async fn dropping_owned_child_publishes_abandoned_without_successful_cleanup() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let lease = context.local(data.path()).unwrap();
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "sleep 0.1"]);
        let child = lease
            .prepare_runtime(&[])
            .unwrap()
            .spawn(command)
            .unwrap_or_else(|e| panic!("{}", e.error()));
        let control = child.control();
        drop(child);
        let observed = tokio::time::timeout(Duration::from_secs(1), control.terminal())
            .await
            .unwrap();
        assert_eq!(
            observed.phase(),
            crate::worker_admission::RuntimePhase::Abandoned
        );
        assert!(observed.retirement().is_none());
        assert!(observed.failure().is_some());
        assert!(data.path().exists());
    }

    #[tokio::test]
    async fn failed_spawn_never_becomes_never_spawned_cleanup_receipt() {
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        let data = crate::worker_admission::testing::private_tempdir().unwrap();
        let lease = context.local(data.path()).unwrap();
        let command = tokio::process::Command::new(data.path().join("absent-executable"));
        let error = match lease.prepare_runtime(&[]).unwrap().spawn(command) {
            Ok(_) => panic!("missing executable unexpectedly spawned"),
            Err(error) => error,
        };
        let (cause, child) = error.into_parts();
        assert_eq!(cause.kind(), std::io::ErrorKind::NotFound);
        assert!(child.is_none());
        assert!(data.path().exists());
        assert!(
            lease.prepare_runtime(&[]).is_err(),
            "failed spawn retains uncertain state"
        );
    }

    #[tokio::test]
    async fn actual_runtime_completion_keeps_os_exit_separate_from_removal_authority() {
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
        let ns = t.path().join("catalog");
        std::fs::create_dir(&ns).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ns, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&ns).unwrap();
        let root = t.path().join("payload");
        let lease = context.local(&root).unwrap();
        let mut create = lease
            .begin(crate::worker_admission::MutationPlan::Create { root: root.clone() })
            .unwrap();
        create.create_dir_all(&root, 0o700).unwrap();
        create.commit().unwrap();
        let session = runtime_test_session(lease.clone(), &ns, false).await;
        let (status, receipt) = session.wait().await;
        assert!(status.unwrap().success());
        let receipt = receipt.unwrap();
        session.finish_resources(&receipt).await.unwrap(); // validated no-relay set
        assert!(
            session
                .signal(crate::worker_admission::RuntimeSignal::Kill)
                .is_ok()
        );
        let foreign = context.local(&t.path().join("foreign")).unwrap();
        assert!(receipt.begin_remove(&foreign, &root, &[&root]).is_err());
        let mut remove = receipt.begin_remove(&lease, &root, &[&root]).unwrap();
        remove.remove_path(&root).unwrap();
        remove.commit().unwrap();
        assert!(!root.exists());
    }

    // `Some(vec![])` is the iii-shell-client trigger for "open stdin,
    // write 0 bytes, send EOF" (lib.rs:448-468). `None` skips stdin
    // entirely and leaves the child blocked on read. So the wire-level
    // `Some("")` and missing-stdin must both decode to `Some(vec![])`,
    // not `None`.
    #[test]
    fn decode_stdin_empty_string_is_eof_not_none() {
        let got = decode_stdin(Some("")).unwrap();
        assert_eq!(
            got,
            Some(Vec::<u8>::new()),
            "stdin: \"\" must produce Some(empty) so the child sees EOF"
        );
    }

    #[test]
    fn decode_stdin_missing_is_eof_not_none() {
        let got = decode_stdin(None).unwrap();
        assert_eq!(
            got,
            Some(Vec::<u8>::new()),
            "missing stdin must produce Some(empty) so the child sees EOF"
        );
    }

    #[test]
    fn decode_stdin_non_empty_decodes_base64() {
        // base64("hi\n") = "aGkK"
        let got = decode_stdin(Some("aGkK")).unwrap();
        assert_eq!(got, Some(b"hi\n".to_vec()));
    }

    /// Output past OUTPUT_CAP is cut, and the reply says which stream was, so
    /// a caller can tell a cut stream from a complete one (contract 1.1).
    #[tokio::test]
    async fn exec_reply_flags_the_stream_cut_at_the_output_cap() {
        use iii_shell_proto::{ShellMessage, encode_frame, flags::FLAG_TERMINAL};
        use std::io::{Read, Write};
        use std::os::unix::fs::PermissionsExt;

        let dir = crate::worker_admission::testing::private_tempdir().unwrap();
        let sock = dir.path().join("shell.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600)).unwrap();
        // Relay and guest in one: hand out id range 0, read the request,
        // answer with 64 KiB more stdout than the cap and a little stderr.
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(&0u32.to_be_bytes()).unwrap();
            let mut len = [0u8; 4];
            s.read_exact(&mut len).unwrap();
            let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
            s.read_exact(&mut body).unwrap();
            let b64 = base64::engine::general_purpose::STANDARD;
            let chunk = b64.encode(vec![b'o'; 64 * 1024]);
            for _ in 0..=OUTPUT_CAP / (64 * 1024) {
                let out = ShellMessage::Stdout {
                    data_b64: chunk.clone(),
                };
                s.write_all(&encode_frame(1, 0, &out).unwrap()).unwrap();
            }
            let err = ShellMessage::Stderr {
                data_b64: b64.encode(b"warn\n"),
            };
            s.write_all(&encode_frame(1, 0, &err).unwrap()).unwrap();
            let exited = ShellMessage::Exited { code: 0 };
            s.write_all(&encode_frame(1, FLAG_TERMINAL, &exited).unwrap())
                .unwrap();
        });

        let req: ExecRequest =
            serde_json::from_value(serde_json::json!({"sandbox_id": "x", "cmd": "cat"})).unwrap();
        let resp = ShellProtoRunner::default().run(sock, &req).await.unwrap();
        assert_eq!(resp.stdout.len(), OUTPUT_CAP);
        assert!(resp.stdout_truncated);
        assert_eq!(resp.stderr, "warn\n");
        assert!(!resp.stderr_truncated);
        let wire = serde_json::to_value(&resp).unwrap();
        assert_eq!(wire["stdout_truncated"], true);
        assert_eq!(wire["stderr_truncated"], false);
    }

    /// The stopper signals only a VM this process spawned and has not reaped:
    /// any other pid, such as a reaped VM's reused one, is left alone.
    #[tokio::test]
    async fn the_stopper_signals_only_unreaped_vms() {
        let mut other = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        // Own unwaited child: its pidfd, taken now, is the only signal path.
        let other_pidfd = test_pidfd_open(other.id()).unwrap();
        SignalStopper.stop(other.id()).await.unwrap();
        let untouched = other.try_wait().unwrap().is_none();
        test_pidfd_signal(&other_pidfd, libc::SIGKILL).unwrap();
        other.wait().unwrap();
        assert!(untouched, "signaled a pid that is no live VM");

        let vm = tokio::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = vm.id().unwrap();
        watch_vm(vm, pid);
        SignalStopper.stop(pid).await.unwrap();
        for _ in 0..100 {
            if !unreaped_vms().iter().any(|(p, _)| *p == pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the VM was not stopped and reaped");
    }

    /// A relay on `shell.sock` that hands out id range 0 and reads one
    /// request; `reply` then answers it, or the guest stays silent.
    fn fake_relay(reply: Option<Duration>) -> (tempfile::TempDir, PathBuf) {
        use iii_shell_proto::{
            FsEntry, FsResult, ShellMessage, encode_frame, flags::FLAG_TERMINAL,
        };
        use std::io::{Read, Write};
        use std::os::unix::fs::PermissionsExt;

        let dir = crate::worker_admission::testing::private_tempdir().unwrap();
        let sock = dir.path().join("shell.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(&0u32.to_be_bytes()).unwrap();
            let mut len = [0u8; 4];
            s.read_exact(&mut len).unwrap();
            let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
            s.read_exact(&mut body).unwrap();
            let Some(delay) = reply else {
                std::thread::sleep(Duration::from_secs(2));
                return;
            };
            std::thread::sleep(delay);
            let root = FsResult::Stat(FsEntry {
                name: "/".into(),
                is_dir: true,
                size: 0,
                mode: "0755".into(),
                mtime: 0,
                is_symlink: false,
            });
            let frame = encode_frame(1, FLAG_TERMINAL, &ShellMessage::FsResponse(root)).unwrap();
            s.write_all(&frame).unwrap();
        });
        (dir, sock)
    }

    /// The relay accepts before the guest is up; readiness waits for the guest's answer.
    #[tokio::test]
    async fn a_guest_is_ready_once_it_answers() {
        let (_dir, sock) = fake_relay(Some(Duration::from_millis(100)));
        guest_answers(&sock, Duration::from_secs(5)).await.unwrap();
    }

    #[tokio::test]
    async fn a_relay_whose_guest_never_answers_is_not_ready() {
        let (_dir, sock) = fake_relay(None);
        let err = guest_answers(&sock, Duration::from_millis(200))
            .await
            .unwrap_err();
        assert!(err.contains("no answer"), "{err}");
    }

    /// The long readiness bound never holds a create for a VM that died: its
    /// relay closes, and the probe fails at once.
    #[tokio::test]
    async fn a_closed_relay_fails_readiness_at_once() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::worker_admission::testing::private_tempdir().unwrap();
        let sock = dir.path().join("shell.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::thread::spawn(move || drop(listener.accept()));
        let started = Instant::now();
        let err = guest_answers(&sock, GUEST_READY_TIMEOUT).await.unwrap_err();
        assert!(!err.contains("no answer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5), "{err}");
    }

    #[test]
    fn decode_stdin_invalid_base64_is_invalid_request() {
        let err = decode_stdin(Some("!!!not-base64!!!")).unwrap_err();
        assert_eq!(err.code().as_str(), "S001");
    }

    // The dispatcher surfaces ENOENT (execve of a missing binary) as
    // `format!("spawn: {e}")` where `e` is a `std::io::Error`. Display
    // for that is `"No such file or directory (os error 2)"`. POSIX
    // shells exit 127 for "command not found"; sandbox::exec must
    // mirror that instead of returning S300 BootFailed (which is
    // reserved for genuine VM-boot failures).
    #[test]
    fn classify_spawn_enoent_returns_exit_127() {
        let resp = classify_dispatcher_spawn_error(
            "spawn: No such file or directory (os error 2)",
            "/no/such/binary",
            42,
        )
        .expect("ENOENT must classify as a synthetic ExecResponse, not None");
        assert_eq!(resp.exit_code, Some(127));
        assert!(!resp.success);
        assert!(!resp.timed_out);
        assert!(resp.stdout.is_empty());
        assert!(
            resp.stderr.contains("/no/such/binary") && resp.stderr.contains("not found"),
            "stderr should look like POSIX shell 'not found' message, got {:?}",
            resp.stderr
        );
        assert_eq!(resp.duration_ms, 42);
    }

    // Adversarial probe B15 from test.mjs uses a non-ASCII path; the
    // classifier must not be sensitive to the cmd's contents — only to
    // the dispatcher's message.
    #[test]
    fn classify_spawn_enoent_handles_non_ascii_cmd() {
        let resp = classify_dispatcher_spawn_error(
            "spawn: No such file or directory (os error 2)",
            "/💥/no/such",
            0,
        )
        .expect("ENOENT must classify regardless of cmd encoding");
        assert_eq!(resp.exit_code, Some(127));
        assert!(resp.stderr.contains("/💥/no/such"));
    }

    // ENOTDIR (an intermediate path component is a regular file) is
    // also a "command not found" failure mode; same exit code.
    #[test]
    fn classify_spawn_enotdir_returns_exit_127() {
        let resp = classify_dispatcher_spawn_error(
            "spawn: Not a directory (os error 20)",
            "/etc/hosts/x",
            0,
        )
        .expect("ENOTDIR must classify as 127 like POSIX shells");
        assert_eq!(resp.exit_code, Some(127));
        assert!(resp.stderr.contains("not found"));
    }

    // EACCES (binary exists but isn't executable) maps to 126, the
    // POSIX shell exit code for "found but not executable".
    #[test]
    fn classify_spawn_eacces_returns_exit_126() {
        let resp = classify_dispatcher_spawn_error(
            "spawn: Permission denied (os error 13)",
            "/tmp/not-exec",
            0,
        )
        .expect("EACCES must classify as 126");
        assert_eq!(resp.exit_code, Some(126));
        assert!(
            resp.stderr.contains("permission denied"),
            "stderr should mention permission denied, got {:?}",
            resp.stderr
        );
    }

    // Non-spawn dispatcher errors (e.g. PTY allocation failure) must
    // fall through so the caller still sees S300. Reserving S300 for
    // genuine VM-level failures depends on this — over-classifying
    // here would silently swallow real boot/dispatcher bugs as exit
    // 127.
    #[test]
    fn classify_unknown_dispatcher_error_returns_none() {
        // Realistic non-spawn dispatcher message: PTY/openpty failure.
        let resp = classify_dispatcher_spawn_error(
            "openpty: Resource temporarily unavailable (os error 11)",
            "/bin/sh",
            0,
        );
        assert!(
            resp.is_none(),
            "non-ENOENT/ENOTDIR/EACCES must fall through to BootFailed, got {resp:?}"
        );
    }

    #[test]
    fn classify_empty_message_returns_none() {
        assert!(classify_dispatcher_spawn_error("", "/bin/true", 0).is_none());
    }
}

#[cfg(test)]
mod sandbox_boot_shape_tests {
    use super::*;

    const TRAMPOLINE: &str = "/tmp/iii-sandbox/sb/root";
    const CACHE: &str = "/home/u/.iii/cache/img";
    const LOWER: &str = "/home/u/.iii/cache/img.erofs";

    fn params(network: bool) -> BootParams {
        BootParams {
            rootfs: PathBuf::from(TRAMPOLINE),
            base_rootfs: PathBuf::from(CACHE),
            prepared: None,
            workdir: PathBuf::from("/tmp/iii-sandbox/sb/merged"),
            shell_sock: PathBuf::from("/tmp/iii-sandbox/sb/shell.sock"),
            cpus: 1,
            memory_mb: 512,
            env: vec!["A=1".into()],
            network,
        }
    }

    fn args(network: bool) -> Vec<String> {
        vm_boot_args(&params(network), Path::new(LOWER))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn value_of(args: &[String], flag: &str) -> Option<String> {
        let i = args.iter().position(|a| a == flag)?;
        args.get(i + 1).cloned()
    }

    #[test]
    fn sandbox_boots_on_read_only_trampoline_with_erofs_lower() {
        let a = args(false);
        assert_eq!(value_of(&a, "--rootfs").as_deref(), Some(TRAMPOLINE));
        assert!(a.iter().any(|x| x == "--rootfs-readonly"));
        assert_eq!(value_of(&a, "--rootfs-mode").as_deref(), Some("overlay"));
        assert_eq!(value_of(&a, "--rootfs-lower").as_deref(), Some(LOWER));
        assert!(
            !a.iter().any(|x| x == "--rootfs-upper"),
            "the upper must be the in-guest tmpfs"
        );
        assert!(
            !a.iter().any(|x| x == CACHE),
            "the shared cache dir must never be passed to the guest"
        );
        assert!(a.iter().any(|x| x == "--env-stdin"));
        assert!(
            !a.iter().any(|x| x == "--env" || x.contains("A=1")),
            "env values must not reach argv: {a:?}"
        );
        assert!(!a.iter().any(|x| x == "--network"));
    }

    #[test]
    fn sandbox_env_travels_in_the_stdin_frame() {
        let frame = env_stdin_frame(&params(false)).unwrap();
        let pairs = iii_supervisor::env_channel::read_frame(&mut frame.as_slice()).unwrap();
        assert_eq!(pairs, vec![("A".to_string(), "1".to_string())]);
    }

    #[test]
    fn network_flag_adds_activity_beacon() {
        let a = args(true);
        assert!(a.iter().any(|x| x == "--network"));
        assert_eq!(
            value_of(&a, "--net-activity-file").as_deref(),
            Some("/tmp/iii-sandbox/sb/net-activity")
        );
    }

    #[test]
    fn sandbox_args_parse_as_vm_boot_args() {
        use clap::Parser;

        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            args: crate::cli::vm_boot::VmBootArgs,
        }

        let mut argv = vec![OsString::from("test")];
        argv.extend(vm_boot_args(&params(true), Path::new(LOWER)));
        let parsed = Wrapper::try_parse_from(argv)
            .expect("__vm-boot must accept the sandbox args")
            .args;
        assert!(parsed.rootfs_readonly);
        assert_eq!(parsed.rootfs, TRAMPOLINE);
        assert_eq!(parsed.rootfs_mode, "overlay");
        assert_eq!(parsed.rootfs_lower.as_deref(), Some(LOWER));
        assert!(parsed.rootfs_upper.is_none());
    }

    #[test]
    fn refuses_to_boot_without_embedded_init() {
        assert!(matches!(
            require_overlay_capable(false),
            Err(SandboxError::BootFailed(m))
                if m.contains("embedded iii-init") && m.contains("BUILD_TARGET=")
        ));
        assert!(require_overlay_capable(true).is_ok());
    }

    fn fake_cache(dir: &Path) -> PathBuf {
        let cache = dir.join("img");
        std::fs::create_dir_all(cache.join("bin")).unwrap();
        std::fs::write(cache.join("bin/sh"), b"#!fake").unwrap();
        cache
    }

    #[cfg(not(feature = "embed-init"))]
    #[tokio::test]
    async fn non_embedded_build_refuses_before_touching_the_cache() {
        let td = crate::worker_admission::testing::private_tempdir().unwrap();
        let cache = fake_cache(td.path());
        assert!(prepare_overlay_lower(&cache).await.is_err());
        assert!(!cache.with_file_name("img.erofs").exists());
    }

    #[cfg(feature = "embed-init")]
    #[tokio::test]
    async fn embedded_build_prepares_erofs_lower_next_to_cache() {
        assert!(
            iii_filesystem::init::has_init(),
            "embed-init build embedded only the 1-byte placeholder: build iii-init \
             for x86_64-unknown-linux-musl --release first"
        );
        let td = crate::worker_admission::testing::private_tempdir().unwrap();
        let cache = fake_cache(td.path());
        let lower = prepare_overlay_lower(&cache).await.unwrap();
        assert_eq!(lower, cache.with_file_name("img.erofs"));
        assert!(std::fs::metadata(&lower).unwrap().len() > 0);
        // Nothing was written inside the shared cache dir.
        let entries: Vec<OsString> = std::fs::read_dir(&cache)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![OsString::from("bin")]);
    }

    #[cfg(not(feature = "embed-init"))]
    #[test]
    fn launcher_preflight_refuses_without_embedded_init() {
        assert!(matches!(
            VmBootLauncher.preflight(),
            Err(SandboxError::BootFailed(_))
        ));
    }

    #[cfg(feature = "embed-init")]
    #[test]
    fn launcher_preflight_accepts_embedded_init() {
        assert!(VmBootLauncher.preflight().is_ok());
    }

    /// The joint the shape tests above cannot see: the sandbox args, through
    /// the real `__vm-boot` parser and `root_share`, must yield a share that
    /// refuses guest writes and leaves the host trampoline untouched.
    #[test]
    fn parsed_sandbox_args_serve_a_read_only_root() {
        use clap::Parser;
        use iii_filesystem::{Context, DynFileSystem, Extensions, FsOptions};

        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            args: crate::cli::vm_boot::VmBootArgs,
        }

        let td = crate::worker_admission::testing::private_tempdir().unwrap();
        let mut p = params(false);
        p.rootfs = td.path().to_path_buf();
        let mut argv = vec![OsString::from("test")];
        argv.extend(vm_boot_args(&p, Path::new(LOWER)));
        let parsed = Wrapper::try_parse_from(argv).unwrap().args;

        let fs = crate::cli::vm_boot::root_share(&parsed).unwrap();
        fs.init(FsOptions::empty()).unwrap();
        let ctx = Context {
            uid: 0,
            gid: 0,
            pid: 1,
        };
        let name = std::ffi::CString::new("planted").unwrap();
        let err = fs
            .mkdir(ctx, 1, &name, 0o755, 0, Extensions::default())
            .err()
            .expect("mkdir on the sandbox root must fail");
        assert_eq!(err.raw_os_error(), Some(30), "expected EROFS, got {err:?}");
        assert!(std::fs::read_dir(td.path()).unwrap().next().is_none());
    }
}
