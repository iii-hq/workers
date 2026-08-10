//! The layer between the wire and the sandbox: validate a request against
//! the caps, take a concurrency permit, hand a `RunSpec` to the `Runner`, and
//! turn the raw `RunOutcome` back into a `RunResponse` or a structured
//! `PythonEngineError`.
//!
//! `classify` is the point of this module. Its ordering is load-bearing: host
//! signals (`timed_out`, `disk_exceeded`, `memory_denied`) are checked before
//! anything the guest wrote (the envelope, the exit status), because a tenant
//! controls every byte under `/out` and could otherwise forge its way past a kill —
//! see `timeout_wins_over_a_forged_success_envelope` in `tests/manager.rs`.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Deserialize;

use crate::config::{
    PythonEngineConfig, MAX_CODE_BYTES, MAX_LOG_BYTES, MAX_LOG_LINES, MAX_PAYLOAD_BYTES,
};
use crate::error::{ErrorKind, PythonEngineError};
use crate::runner::{
    ExitKind, PersistentRuntime, PersistentSpec, RunOutcome, RunSpec, Runner, MAX_OUT_DIR_BYTES,
    MAX_OUT_DIR_ENTRIES,
};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RunRequest {
    /// Python source. Runs top-level; assign to `result` to return a value.
    pub code: String,
    /// Visible to the code as the global `payload` (None when omitted).
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
    /// Wall-clock budget, default 5000, clamped to the config ceiling.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Linear-memory cap in MiB, default 128, clamped to the config ceiling.
    #[serde(default)]
    pub memory_mb: Option<u64>,
}

// `code` is tenant-authored source — never print it, same rule
// `error.rs`'s module doc states for why `PythonEngineError` may derive
// `Debug` freely (it carries none). `payload` isn't source, but it is
// caller-supplied data of unknown shape that can carry secrets or PII just
// as easily as `code` can carry a hazardous string, so it gets the same
// redaction; presence (`Some`/`None`) still shows, only the contents don't.
// `timeout_ms`/`memory_mb` are plain knobs and stay visible — exactly what a
// diagnostic would want.
impl std::fmt::Debug for RunRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunRequest")
            .field("code", &"<redacted>")
            .field("payload", &self.payload.as_ref().map(|_| "<redacted>"))
            .field("timeout_ms", &self.timeout_ms)
            .field("memory_mb", &self.memory_mb)
            .finish()
    }
}

/// What a run produced, in the shape the sandbox actually captured it.
///
/// Two streams, not one pre-flattened log list. A host that reports a run as
/// a process — `stdout`/`stderr`/`exit_code` — needs them separate, and a
/// host that wants a level-tagged list can split them itself. Deciding that
/// here would force one of the two to unpick the other's choice.
///
/// Each stream is independently capped at `MAX_LOG_LINES` lines and
/// `MAX_LOG_BYTES` bytes; `truncated` says whether anything was dropped,
/// either at the pipe (the guest outran the buffer) or here.
#[derive(Debug)]
pub struct RunOutput {
    pub result: serde_json::Value,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

#[derive(Deserialize)]
struct Envelope {
    ok: bool,
    #[serde(default)]
    result: serde_json::Value,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    traceback: Option<String>,
}

/// One live runtime: a durable working directory, and when it was last used.
///
/// The interpreter is NOT here. `python.wasm` is a WASI command module —
/// exports `memory` and `_start`, nothing else — so nothing can be called back
/// into after `_start` returns. What persists is the directory, which is also
/// exactly what sandbox-code-runner's `keep` promises.
struct Runtime {
    work_dir: tempfile::TempDir,
    last_activity: Mutex<Instant>,
    /// Fixed when the runtime is created. Linear memory only ever grows, so a
    /// per-call ceiling on a shared interpreter would be a promise the sandbox
    /// cannot keep — the second caller inherits whatever the first grew to.
    memory_mb: u64,
    /// The warm interpreter, booted on first use and `None` again once it has
    /// been poisoned. An async mutex because one interpreter serves one turn
    /// at a time and the wait is a real await, not a spin.
    interp: tokio::sync::Mutex<Option<PersistentRuntime>>,
    /// Exempt from the idle sweep. Set for a runtime that backs a REGISTERED
    /// function: nothing bumps its activity between invocations, so a
    /// registration that is merely unpopular would otherwise be reaped and its
    /// next caller would get `RuntimeNotFound` for a function the catalog
    /// still advertises. `teardown` is the only way to remove one.
    pinned: AtomicBool,
}

pub struct Manager {
    cfg: Arc<PythonEngineConfig>,
    runner: Arc<Runner>,
    /// What the guest's `iii` global can reach. `None` on a bare `Manager`;
    /// the hosting worker installs one with [`Manager::with_bridge`].
    bridge: Option<Arc<dyn crate::runner::GuestBridge>>,
    permits: Arc<tokio::sync::Semaphore>,
    runtimes: Mutex<HashMap<String, Arc<Runtime>>>,
}

impl Manager {
    pub fn new(cfg: Arc<PythonEngineConfig>, runner: Arc<Runner>) -> Arc<Self> {
        let permits = Arc::new(tokio::sync::Semaphore::new(cfg.max_concurrent_runs));
        Arc::new(Self {
            cfg,
            runner,
            permits,
            bridge: None,
            runtimes: Mutex::new(HashMap::new()),
        })
    }

    /// Give guest code an `iii` global backed by `bridge`.
    ///
    /// Separate from `new` so the bare constructor stays usable in tests and
    /// in a host that deliberately offers no bus access — an absent global is
    /// a clearer contract than one that always refuses.
    pub fn with_bridge(
        cfg: Arc<PythonEngineConfig>,
        runner: Arc<Runner>,
        bridge: Arc<dyn crate::runner::GuestBridge>,
    ) -> Arc<Self> {
        let permits = Arc::new(tokio::sync::Semaphore::new(cfg.max_concurrent_runs));
        Arc::new(Self {
            cfg,
            runner,
            permits,
            bridge: Some(bridge),
            runtimes: Mutex::new(HashMap::new()),
        })
    }

    /// A one-shot run: nothing persists, and nothing is addressable
    /// afterwards.
    pub async fn run(&self, req: RunRequest) -> Result<RunOutput, PythonEngineError> {
        self.run_with(req, None).await
    }

    /// Run in an existing runtime: same `/work`, and the same interpreter.
    ///
    /// Globals bound by an earlier call are still bound, and modules imported
    /// by an earlier call are still imported. That is a superset of what
    /// sandbox-code-runner promises for its own `keep`, which persists files
    /// and boots a fresh interpreter per call.
    ///
    /// A call that overruns its budget takes the interpreter with it — the
    /// only kill that reaches a guest parked in a host call unwinds `_start`,
    /// leaving nothing resumable. The directory survives, so the next call on
    /// this id boots a fresh interpreter on the same files.
    pub async fn run_in(
        &self,
        runtime_id: &str,
        req: RunRequest,
    ) -> Result<RunOutput, PythonEngineError> {
        if req.memory_mb.is_some() {
            return Err(PythonEngineError::new(
                ErrorKind::InvalidInput,
                "memory_mb is fixed when a runtime is created and cannot be set per call — \
                 wasm linear memory only grows, so a later caller would inherit whatever an \
                 earlier one already grew to",
            ));
        }
        let rt = self.lookup(runtime_id)?;
        let (code, payload_json, timeout_ms) = self.validate(&req)?;

        *rt.last_activity.lock().unwrap() = Instant::now();
        let out = self.turn_in(&rt, code, payload_json, timeout_ms).await;
        // Bumped again on the way out: a long call must not look idle to a
        // sweep that fires while it is still running.
        *rt.last_activity.lock().unwrap() = Instant::now();
        out
    }

    /// One turn on `rt`'s interpreter, booting or rebooting it as needed.
    async fn turn_in(
        &self,
        rt: &Runtime,
        code: String,
        payload_json: Option<String>,
        timeout_ms: u64,
    ) -> Result<RunOutput, PythonEngineError> {
        // Held across the whole turn: one interpreter runs one turn at a time,
        // and a second caller must queue rather than interleave into a shared
        // address space.
        let mut slot = rt.interp.lock().await;

        // Taken after validation and after the runtime lock, so a request that
        // fails its caps never queues behind runs that will actually execute.
        let _permit =
            self.permits.clone().acquire_owned().await.map_err(|_| {
                PythonEngineError::new(ErrorKind::Internal, "worker is shutting down")
            })?;

        // `None` here is either "never used" or "the last call poisoned it".
        // Both boot the same way, on the same directory.
        if !slot.as_ref().is_some_and(|i| i.is_live()) {
            let fresh = self
                .runner
                .spawn_persistent(PersistentSpec {
                    work_dir: rt.work_dir.path().to_path_buf(),
                    memory_mb: rt.memory_mb,
                    bridge: self.bridge.clone(),
                    namespace: None,
                })
                .await
                .map_err(|e| {
                    PythonEngineError::new(
                        if e.memory_denied {
                            ErrorKind::OutOfMemory
                        } else {
                            ErrorKind::Internal
                        },
                        format!("starting the interpreter: {:#}", e.error),
                    )
                })?;
            *slot = Some(fresh);
        }

        let outcome = slot
            .as_ref()
            .expect("just booted")
            .run(code, payload_json, timeout_ms)
            .await;
        // Drop a poisoned interpreter now rather than on the next call, so a
        // sweep or a teardown is not holding a corpse.
        if !slot.as_ref().is_some_and(|i| i.is_live()) {
            *slot = None;
        }
        let outcome =
            outcome.map_err(|e| PythonEngineError::new(ErrorKind::Internal, format!("{e:#}")))?;
        classify(outcome, timeout_ms)
    }

    /// Create a runtime and return the id that addresses it.
    ///
    /// The directory name is `tempfile`'s own random suffix, never derived
    /// from the id: a `runtime_id` is a capability, and a path is enumerable
    /// by anything on the box.
    pub fn create_runtime(&self, memory_mb: Option<u64>) -> Result<String, PythonEngineError> {
        let mut runtimes = self.runtimes.lock().unwrap();
        if runtimes.len() >= self.cfg.max_runtimes {
            return Err(PythonEngineError::new(
                ErrorKind::Capacity,
                format!("all {} runtime slots are in use", self.cfg.max_runtimes),
            ));
        }
        let work_dir = tempfile::Builder::new()
            .prefix("iii-python-work-")
            .tempdir()
            .map_err(|e| {
                PythonEngineError::new(
                    ErrorKind::Internal,
                    format!("creating the runtime's working directory: {e}"),
                )
            })?;
        let id = format!("rt-{}", uuid::Uuid::new_v4());
        runtimes.insert(
            id.clone(),
            Arc::new(Runtime {
                work_dir,
                last_activity: Mutex::new(Instant::now()),
                memory_mb: self.cfg.clamp_memory(memory_mb),
                interp: tokio::sync::Mutex::new(None),
                pinned: AtomicBool::new(false),
            }),
        );
        Ok(id)
    }

    /// Destroy a runtime and its directory. Dropping the `TempDir` is what
    /// removes the tree, so every exit path gets cleanup for free.
    pub fn destroy_runtime(&self, runtime_id: &str) -> Result<(), PythonEngineError> {
        self.runtimes
            .lock()
            .unwrap()
            .remove(runtime_id)
            .map(|_| ())
            .ok_or_else(|| {
                PythonEngineError::new(
                    ErrorKind::RuntimeNotFound,
                    format!("unknown runtime_id {runtime_id}"),
                )
            })
    }

    /// Reap runtimes idle past `idle_ttl_secs`. The only backstop for a
    /// caller that never tears one down.
    pub fn sweep_idle(&self) -> Vec<String> {
        let ttl = std::time::Duration::from_secs(self.cfg.idle_ttl_secs);
        let mut runtimes = self.runtimes.lock().unwrap();
        let stale: Vec<String> = runtimes
            .iter()
            .filter(|(_, rt)| !rt.pinned.load(Ordering::Relaxed))
            .filter(|(_, rt)| rt.last_activity.lock().unwrap().elapsed() >= ttl)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &stale {
            runtimes.remove(id);
        }
        stale
    }

    /// Exempt `runtime_id` from the idle sweep.
    ///
    /// For a runtime backing a registered function, whose lifetime is the
    /// registration's and not the traffic's. Idempotent, and there is
    /// deliberately no unpin: the only way out is `destroy_runtime`, so a
    /// pinned runtime cannot be quietly returned to the sweep by anything but
    /// the teardown that also unregisters its functions.
    pub fn pin_runtime(&self, runtime_id: &str) -> Result<(), PythonEngineError> {
        self.lookup(runtime_id)?
            .pinned
            .store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn live_runtime_count(&self) -> usize {
        self.runtimes.lock().unwrap().len()
    }

    fn lookup(&self, runtime_id: &str) -> Result<Arc<Runtime>, PythonEngineError> {
        self.runtimes
            .lock()
            .unwrap()
            .get(runtime_id)
            .cloned()
            .ok_or_else(|| {
                PythonEngineError::new(
                    ErrorKind::RuntimeNotFound,
                    format!("unknown runtime_id {runtime_id}"),
                )
            })
    }

    /// The caps every request faces, whichever path runs it. One
    /// implementation so a persistent turn cannot accept code or a payload a
    /// one-shot run would have refused.
    fn validate(
        &self,
        req: &RunRequest,
    ) -> Result<(String, Option<String>, u64), PythonEngineError> {
        if req.code.len() > MAX_CODE_BYTES {
            return Err(PythonEngineError::new(
                ErrorKind::InvalidInput,
                format!("code is {} bytes; max {MAX_CODE_BYTES}", req.code.len()),
            ));
        }
        let payload_json = match &req.payload {
            None => None,
            Some(v) => {
                let s = serde_json::to_string(v).map_err(|e| {
                    PythonEngineError::new(ErrorKind::InvalidInput, format!("payload: {e}"))
                })?;
                if s.len() > MAX_PAYLOAD_BYTES {
                    return Err(PythonEngineError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "payload serializes to {} bytes; max {MAX_PAYLOAD_BYTES}",
                            s.len()
                        ),
                    ));
                }
                Some(s)
            }
        };
        Ok((
            req.code.clone(),
            payload_json,
            self.cfg.clamp_timeout(req.timeout_ms),
        ))
    }

    async fn run_with(
        &self,
        req: RunRequest,
        work_dir: Option<std::path::PathBuf>,
    ) -> Result<RunOutput, PythonEngineError> {
        let (code, payload_json, timeout_ms) = self.validate(&req)?;
        let memory_mb = self.cfg.clamp_memory(req.memory_mb);

        // Permits are taken only after validation: a request that fails its
        // caps must never queue behind runs that are actually going to
        // execute.
        let _permit =
            self.permits.clone().acquire_owned().await.map_err(|_| {
                PythonEngineError::new(ErrorKind::Internal, "worker is shutting down")
            })?;

        let spec = RunSpec {
            code: code.into_bytes(),
            payload_json,
            timeout_ms,
            memory_mb,
            work_dir,
            bridge: self.bridge.clone(),
            namespace: None,
        };
        // Awaited directly — NOT `spawn_blocking`. `Runner::run` drives the
        // guest on a fiber under `call_async`; its internal awaits (the
        // epoch-slice yield, the wall-clock backstop timeout) need a real
        // reactor. Moving it to a blocking thread would starve that reactor
        // and disarm the very backstop that kills a guest parked in a host
        // call (`time.sleep(86400)`), which would then hold the thread and
        // this permit forever.
        let outcome = self
            .runner
            .run(&spec)
            .await
            .map_err(|e| PythonEngineError::new(ErrorKind::Internal, format!("{e:#}")))?;

        classify(outcome, timeout_ms)
    }
}

/// Host-side signals first; guest-writable bytes only after.
fn classify(out: RunOutcome, timeout_ms: u64) -> Result<RunOutput, PythonEngineError> {
    let (stdout, stderr, truncated) = capture_streams(&out);

    if out.timed_out {
        return Err(PythonEngineError::new(
            ErrorKind::Timeout,
            format!("run exceeded {timeout_ms} ms and was killed"),
        ));
    }

    // Also host-derived, and also unconditional: the guest is killed the
    // moment `/out` goes over, so there is no envelope it could have written
    // that deserves to win here.
    if let Some(which) = out.disk_exceeded {
        let (bytes, entries) = match which {
            "/work" => (
                crate::runner::MAX_WORK_DIR_BYTES,
                crate::runner::MAX_WORK_DIR_ENTRIES,
            ),
            _ => (MAX_OUT_DIR_BYTES, MAX_OUT_DIR_ENTRIES),
        };
        return Err(PythonEngineError::new(
            ErrorKind::DiskQuotaExceeded,
            format!(
                "run wrote more than {bytes} bytes (or more than {entries} files) under \
                 {which} and was killed"
            ),
        ));
    }

    let envelope: Option<Envelope> = out
        .envelope
        .as_deref()
        .and_then(|b| serde_json::from_slice(b).ok());
    let clean_success = matches!(&envelope, Some(e) if e.ok);

    if out.memory_denied && !clean_success {
        return Err(PythonEngineError::new(
            ErrorKind::OutOfMemory,
            "run exceeded its memory cap and was killed",
        ));
    }

    match envelope {
        Some(e) if e.ok => Ok(RunOutput {
            result: e.result,
            stdout,
            stderr,
            truncated,
        }),
        Some(e) => {
            let kind = match e.kind.as_deref() {
                Some("syntax_error") => ErrorKind::SyntaxError,
                Some("python_exception") => ErrorKind::PythonException,
                Some("result_too_large") => ErrorKind::ResultTooLarge,
                // An envelope with an unrecognised kind is guest-shaped
                // noise, not infrastructure: attribute it to the tenant.
                _ => ErrorKind::PythonException,
            };
            let mut err = PythonEngineError::new(
                kind,
                e.message
                    .unwrap_or_else(|| "tenant code failed".to_string()),
            );
            err.traceback = e.traceback;
            Err(err)
        }
        None => match out.exit {
            ExitKind::Clean(status) => Err(PythonEngineError::new(
                ErrorKind::PythonException,
                format!("code exited with status {status} before completing"),
            )),
            // A trap with no envelope is guest-shaped, not infrastructure:
            // only tenant code reaches this arm. A genuine host-side failure
            // (instantiation, WASI setup) never becomes a `RunOutcome` at
            // all — `Runner::run` returns it as an `Err`, handled above as
            // `Internal`. Two ways tenant code lands here: `os._exit(200)`
            // (wasmtime-wasi's `proc_exit` only maps status 0..126 to a
            // clean `I32Exit`; anything >= 126 comes back as a plain trap)
            // and real wasm stack exhaustion from runaway recursion. Both
            // are the tenant's own doing, so this is `python_exception`, not
            // `internal` — an unattributable exit here would let hostile
            // input page an operator. The raw trap text can be an unbounded
            // wasm backtrace, so it is logged for an operator to see and
            // never handed to the caller.
            ExitKind::Trap(reason) => {
                tracing::warn!(
                    trap = %reason,
                    "guest execution trapped without producing a result; classifying as a tenant exception"
                );
                Err(PythonEngineError::new(
                    ErrorKind::PythonException,
                    "execution ended abnormally without producing a result",
                ))
            }
        },
    }
}

/// Cap each captured stream independently and report whether anything was
/// lost, either at the pipe (the guest outran the buffer, `CapturedStream::
/// dropped`) or here.
///
/// Per stream rather than across both: they are two fields on the way out, so
/// a noisy stderr must not be able to consume the budget a caller needed for
/// stdout.
fn capture_streams(out: &RunOutcome) -> (String, String, bool) {
    let mut truncated = out.stdout.dropped > 0 || out.stderr.dropped > 0;
    let mut cap = |stream: &crate::runner::CapturedStream| {
        let text = String::from_utf8_lossy(&stream.bytes);
        let mut kept = String::new();
        for (n, line) in text.lines().enumerate() {
            if n >= MAX_LOG_LINES || kept.len() + line.len() > MAX_LOG_BYTES {
                truncated = true;
                break;
            }
            kept.push_str(line);
            kept.push('\n');
        }
        kept
    };
    let stdout = cap(&out.stdout);
    let stderr = cap(&out.stderr);
    (stdout, stderr, truncated)
}
