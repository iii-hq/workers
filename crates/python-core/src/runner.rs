//! The sandbox proper: one wasmtime `Engine`/`Module`/`InstancePre` for the
//! process, one fresh `Store` per call. Everything this module returns is a
//! *raw* signal — `manager.rs` does the classifying. In particular the three
//! containment flags (`timed_out`, `memory_denied`, `disk_exceeded`) are
//! derived only from host-side observations, never from anything the guest
//! wrote, so a tenant forging `/out/result.json` cannot talk its way out of a
//! kill.
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bytes::Bytes;
use tokio::io::AsyncWrite;
use wasmtime::{Engine, InstancePre, Linker, ResourceLimiter, Store, Trap};
use wasmtime_wasi::cli::{IsTerminal, StdinStream, StdoutStream};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::p2::{InputStream, OutputStream, Pollable, StreamResult};
use wasmtime_wasi::{DirPerms, FilePerms, I32Exit, WasiCtxBuilder};

use crate::artifact::{self, EPOCH_TICK_MS};
use crate::config::{MAX_LOG_BYTES, MAX_PAYLOAD_BYTES, MAX_RESULT_BYTES};

/// The interpreter's memory section declares a 640-page (40 MiB) minimum, so
/// wasmtime asks the limiter for that much before a single instruction runs.
/// A `memory_mb` below this can never host a run.
pub const MIN_MEMORY_MB: u64 = 40;

/// Total bytes the guest may hold under `/out`.
///
/// `/out` is the only resource wasmtime does not bound for us, and it is not
/// free disk: `tempfile::tempdir()` resolves under `$TMPDIR`, which on most
/// Linux hosts is tmpfs — i.e. host RAM, spent entirely outside `memory_mb`.
/// Four MiB costs a legitimate tenant nothing, since the only file that
/// matters is refused past `MAX_RESULT_BYTES` (1 MiB) on read.
pub const MAX_OUT_DIR_BYTES: u64 = 4 * 1024 * 1024;

/// Directory entries the scan will visit before calling it a violation.
///
/// The scan has to be bounded as well as the total: a guest that creates a
/// million small files would otherwise make the *walk* the attack. A
/// legitimate run leaves exactly one entry, so this is 64x headroom.
/// Byte budget for a runtime's `/work`, which unlike `/out` accumulates
/// across every call on that runtime.
///
/// Far larger than `/out`'s, and affordable for the same reason the entry cap
/// is: the walk only runs while a call is in flight, and a parked runtime
/// writes nothing. The host ceiling is `max_runtimes * MAX_WORK_DIR_BYTES`,
/// which belongs in the operator docs — on a tmpfs `$TMPDIR` that is host RAM.
pub const MAX_WORK_DIR_BYTES: u64 = 256 * 1024 * 1024;
/// Entry budget for `/work`. Also the bound on its walk.
pub const MAX_WORK_DIR_ENTRIES: usize = 20_000;

pub const MAX_OUT_DIR_ENTRIES: usize = 64;

/// How often `/out` is measured while the guest runs.
///
/// The bound is therefore soft by one interval's worth of writes (tens of MB
/// at WASI write speeds, not the tens of GB an unbounded run reaches). Buying
/// a tighter bound means paying a `read_dir` more often for every run,
/// including the honest ones.
const OUT_DIR_POLL: Duration = Duration::from_millis(250);

/// How long the guest may hold the worker thread before it must hand it back.
///
/// One tick, i.e. ~`EPOCH_TICK_MS`. The epoch deadline is re-armed this far
/// ahead over and over; each expiry yields to the executor and extends. This
/// is what stops a spinning guest from owning a tokio worker for its whole
/// budget — which would also stall the time driver, and with it the very
/// backstop a *parked* guest depends on.
const EPOCH_SLICE_TICKS: u64 = 1;

// Both kills are armed at the same budget and still do not race, even though
// slicing now makes the fiber yield regularly. `tokio::time::timeout` polls
// the inner future first and only consults its delay on `Pending`, and the
// only `Pending` a guest can produce after the budget is spent comes from a
// host call — once `elapsed >= budget` the slice callback returns `Interrupt`,
// which is `Ready`, not a yield. So executing wasm always exits by the epoch
// (precise, and it names itself `Trap::Interrupt`) and the backstop only ever
// reaches a guest that has parked. I added a grace constant here to separate
// them and then deleted it: a mutation run at zero grace kept taking the epoch
// path 3/3, which is the same answer as the previous round for a related
// reason, so the constant was inert.

pub struct RunSpec {
    pub code: Vec<u8>,
    pub payload_json: Option<String>,
    pub timeout_ms: u64,
    pub memory_mb: u64,
    /// A durable directory to preopen at `/work`, shared by every call on one
    /// runtime. `None` for a one-shot run, which gets nothing.
    ///
    /// A one-shot run that is handed a `work_dir` sees the files a persistent
    /// runtime left there, but brings none of its interpreter state — see
    /// `PersistentRuntime` for the half that does survive.
    pub work_dir: Option<PathBuf>,
    /// What the guest's `iii` global can do. `None` removes the global
    /// entirely rather than leaving one that refuses — a guest surface that
    /// exists and always fails is worse than an absent one.
    pub bridge: Option<Arc<dyn GuestBridge>>,
    /// Published to the guest as `iii.namespace`.
    pub namespace: Option<String>,
}

pub struct CapturedStream {
    pub bytes: Vec<u8>,
    pub dropped: u64,
}

#[derive(Debug)]
pub enum ExitKind {
    Clean(i32),
    Trap(String),
}

pub struct RunOutcome {
    pub exit: ExitKind,
    pub envelope: Option<Vec<u8>>,
    pub stdout: CapturedStream,
    pub stderr: CapturedStream,
    /// The run exceeded its wall-clock budget. Host-derived, and true whichever
    /// of the two kills fired — the epoch trap for executing wasm, or the
    /// wall-clock backstop for a guest parked in a host call.
    pub timed_out: bool,
    pub memory_denied: bool,
    /// Which guest-writable directory blew its budget, if either did —
    /// `"/out"` or `"/work"`. Host-derived like the other two: measured by the
    /// host walking its own directories, never reported by the guest.
    ///
    /// Names the directory rather than being a bare bool so the caller can be
    /// told which budget to shrink; the two have very different sizes and very
    /// different fixes.
    pub disk_exceeded: Option<&'static str>,
}

/// ResourceLimiter that records denial instead of erroring: growth beyond
/// the cap returns Ok(false), which CPython sees as allocation failure
/// (MemoryError) — the worker never trusts the guest to report it.
struct TrackedLimiter {
    max_memory_bytes: usize,
    denied: Arc<AtomicBool>,
}

impl ResourceLimiter for TrackedLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _max: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.max_memory_bytes {
            self.denied.store(true, Ordering::Relaxed);
            Ok(false)
        } else {
            Ok(true)
        }
    }
    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _max: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

/// What `check_write` advertises. Matching wasmtime-wasi's own stdio impl:
/// large enough that the guest never sees back-pressure, since this sink
/// never blocks and never refuses.
const WRITE_BUDGET: usize = 1024 * 1024;

/// Capped in-memory sink for guest stdout/stderr. Writes past the cap are
/// COUNTED AND DISCARDED, never refused: a tenant printing gigabytes must
/// neither buffer gigabytes on the host heap nor see print() start raising.
///
/// The clones share one buffer and one counter, which is what lets the caller
/// keep a handle after handing one to `WasiCtxBuilder`.
#[derive(Clone)]
struct CappedPipe {
    state: Arc<Mutex<Vec<u8>>>,
    dropped: Arc<AtomicU64>,
    cap: usize,
}

impl CappedPipe {
    fn new(cap: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(Vec::new())),
            dropped: Arc::new(AtomicU64::new(0)),
            cap,
        }
    }
    fn write_bytes(&self, buf: &[u8]) {
        let mut b = self.state.lock().unwrap();
        let room = self.cap.saturating_sub(b.len());
        let take = room.min(buf.len());
        b.extend_from_slice(&buf[..take]);
        let overflow = (buf.len() - take) as u64;
        if overflow > 0 {
            self.dropped.fetch_add(overflow, Ordering::Relaxed);
        }
    }
    /// Drain what has been captured so far and reset the cap.
    ///
    /// A persistent runtime calls this at every turn boundary, which is what
    /// makes `MAX_LOG_BYTES` a per-turn ceiling there rather than a per-
    /// interpreter one — otherwise the tenth caller on a chatty runtime would
    /// get no logs at all because the first nine filled the buffer.
    fn take_captured(&self) -> CapturedStream {
        CapturedStream {
            bytes: std::mem::take(&mut *self.state.lock().unwrap()),
            dropped: self.dropped.swap(0, Ordering::Relaxed),
        }
    }
    fn into_captured(self) -> CapturedStream {
        self.take_captured()
    }
}

// wasmtime-wasi 47.0.3 asks for two layers here. `StdoutStream` is what
// `WasiCtxBuilder::stdout`/`stderr` accept; it is a *factory* that hands out a
// fresh stream per `get-stdout` call, so the capping state has to live behind
// the Arc rather than in the stream itself. `p2_stream` has a default impl
// that wraps `async_stream` in an `AsyncWriteStream` — which spawns a tokio
// task and applies back-pressure — so we override it, exactly as every
// in-tree impl (`StdioOutputStream`, `MemoryOutputPipe`) does. The preview1
// path we use reaches stdio only through `p2_stream`.
impl IsTerminal for CappedPipe {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for CappedPipe {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

impl OutputStream for CappedPipe {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.write_bytes(&bytes);
        Ok(())
    }
    fn flush(&mut self) -> StreamResult<()> {
        Ok(()) // nothing is buffered anywhere else
    }
    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(WRITE_BUDGET)
    }
}

#[wasmtime_wasi::async_trait]
impl Pollable for CappedPipe {
    async fn ready(&mut self) {} // always writable
}

impl AsyncWrite for CappedPipe {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.write_bytes(buf);
        // Report a full write: the overflow was accounted for in `dropped`,
        // and a short write here would surface to the guest as a failed
        // `print()`.
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Read `/out/result.json`, refusing anything over `MAX_RESULT_BYTES`.
///
/// `wrapper.py` enforces that cap, but only cooperating code goes through
/// `wrapper.py`: tenant code shares the interpreter and can write the file
/// itself, so the host must never size an allocation from a guest-chosen
/// number. `take()` bounds the read at cap+1 bytes — enough to tell "at the
/// cap" from "over it" without reading the rest. An over-cap file is reported
/// as *absent* rather than truncated: a truncated prefix could still parse
/// into a plausible-looking envelope.
///
/// `/out` has `DirPerms::MUTATE`, which is all `path_symlink` is gated on
/// (`wasmtime-wasi/src/filesystem.rs`), and cap-primitives rejects only
/// *rooted* symlink targets — so `os.symlink("../../../../etc/passwd",
/// "/out/result.json")` is a link the guest can create. `File::open` is the
/// host's unsandboxed opener and would follow it, making the host read a
/// guest-chosen path with the worker's full authority (and block outright on
/// a FIFO). Refuse anything that is not a regular file. No TOCTOU: the guest
/// is dead by the time this runs.
fn read_envelope(path: &Path) -> Option<Vec<u8>> {
    if !std::fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_RESULT_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .ok()?;
    (buf.len() <= MAX_RESULT_BYTES).then_some(buf)
}

/// A wasm backtrace has no bound the guest doesn't choose:
/// `sys.setrecursionlimit(10**9)` plus recursion renders thousands of frames,
/// hundreds of KB, repeatably. It is already kept out of the caller's error —
/// but it is *stored* here and warn-logged by `manager.rs`, and an operator's
/// log is a resource too. Counted in chars so the cut always lands on a
/// boundary.
const MAX_TRAP_CHARS: usize = 4096;

fn trap_text(e: impl std::fmt::Display) -> String {
    let s = format!("{e:#}");
    match s.char_indices().nth(MAX_TRAP_CHARS) {
        Some((i, _)) => format!("{}... (trap text truncated)", &s[..i]),
        None => s,
    }
}

/// True once `/out` is over either bound. Walks the tree — the guest has
/// `MUTATE`, so it can `mkdir` and hide bytes a level down — but stops the
/// instant either budget is blown, so the walk can never cost more than
/// `MAX_OUT_DIR_ENTRIES` stats.
///
/// `DirEntry::metadata` does not traverse symlinks, which is what we want: a
/// link's target is somebody else's bytes, and following one would both
/// mis-account the total and hand the guest a way to make the host stat an
/// arbitrary path.
fn dir_over_budget(dir: &Path, max_bytes: u64, max_entries: usize) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    let mut bytes = 0u64;
    let mut entries = 0usize;
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in read.flatten() {
            entries += 1;
            if entries > max_entries {
                return true;
            }
            let Ok(md) = entry.metadata() else { continue };
            if md.is_dir() {
                stack.push(entry.path());
            } else {
                bytes += md.len();
                if bytes > max_bytes {
                    return true;
                }
            }
        }
    }
    false
}

/// Resolves the first time either guest-writable directory is over budget,
/// naming which — and never otherwise, so it is only ever useful as the losing
/// side of a `select!`.
///
/// One watchdog rather than two so the two budgets cannot race to report
/// different answers for the same kill.
async fn disk_watchdog(out_dir: PathBuf, work_dir: Option<PathBuf>) -> &'static str {
    loop {
        tokio::time::sleep(OUT_DIR_POLL).await;
        if dir_over_budget(&out_dir, MAX_OUT_DIR_BYTES, MAX_OUT_DIR_ENTRIES) {
            return "/out";
        }
        if let Some(w) = &work_dir {
            if dir_over_budget(w, MAX_WORK_DIR_BYTES, MAX_WORK_DIR_ENTRIES) {
                return "/work";
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The guest `iii` bridge
// ---------------------------------------------------------------------------

/// Cap on one guest-written `iii` request frame.
///
/// Enforced DURING accumulation, not after: the host owns the sink, so unlike
/// the V8 sibling — where `#[string]` op args are byte-copied onto the Rust
/// heap before any check in the op body runs — there is no window in which an
/// oversized payload is fully resident. Past the cap the buffer stops growing
/// and the frame is answered with an error rather than truncated-and-parsed,
/// because a truncated prefix can still parse into something plausible (the
/// same argument `read_envelope` makes about the result envelope).
pub const MAX_III_REQUEST_BYTES: usize = MAX_PAYLOAD_BYTES;

/// Cap on a guest-supplied `function_id`, matching the V8 sibling's.
pub const MAX_FUNCTION_ID_BYTES: usize = 512;

/// What the host can do on the guest's behalf.
///
/// One method, deliberately. This crate is bus-free and must stay so; the
/// hosting worker implements this against its own connection, exactly as
/// node-core's `Engine` trait works. Registration, triggers and shutdown are
/// NOT here — see the README for why.
pub trait GuestBridge: Send + Sync + 'static {
    fn call(
        &self,
        fn_id: String,
        payload: serde_json::Value,
        timeout_ms: u64,
    ) -> futures::future::BoxFuture<'static, Result<serde_json::Value, String>>;
}

/// Bytes queued for the guest's stdin, with a park that costs nothing.
///
/// The contract `wasmtime-wasi` enforces is sharp: `blocking_read` loops
/// `ready()`-then-`read()` and TRAPS with "max blocking attempts exceeded"
/// after ten empty reads. So `ready()` must resolve only when there is
/// something to read, and `read()` after that must never come back empty.
#[derive(Clone)]
struct FrameQueue {
    bytes: Arc<Mutex<std::collections::VecDeque<u8>>>,
    /// `notify_one` rather than `notify_waiters`: it stores a permit when
    /// nobody is waiting yet, so a push that lands before the guest parks is
    /// not lost. `notify_waiters` would drop it and the guest would hang.
    notify: Arc<tokio::sync::Notify>,
}

impl FrameQueue {
    fn new() -> Self {
        Self {
            bytes: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Queue a length-prefixed frame: `<decimal length>\n<body>`.
    ///
    /// Length-prefixed rather than delimited so the body needs no escaping and
    /// the guest's read is exact.
    fn push_frame(&self, body: &[u8]) {
        let mut q = self.bytes.lock().unwrap();
        q.extend(format!("{}\n", body.len()).as_bytes());
        q.extend(body);
        drop(q);
        self.notify.notify_one();
    }
}

impl IsTerminal for FrameQueue {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdinStream for FrameQueue {
    fn p2_stream(&self) -> Box<dyn InputStream> {
        Box::new(self.clone())
    }
    // Never taken: preview1 reaches stdio only through `p2_stream`, which is
    // overridden above. The default `p2_stream` wraps this in an
    // `AsyncReadStream`, which is exactly the adapter we are avoiding.
    fn async_stream(&self) -> Box<dyn tokio::io::AsyncRead + Send + Sync> {
        Box::new(tokio::io::empty())
    }
}

#[wasmtime_wasi::async_trait]
impl Pollable for FrameQueue {
    async fn ready(&mut self) {
        loop {
            if !self.bytes.lock().unwrap().is_empty() {
                return;
            }
            self.notify.notified().await;
        }
    }
}

impl InputStream for FrameQueue {
    fn read(&mut self, size: usize) -> StreamResult<Bytes> {
        let mut q = self.bytes.lock().unwrap();
        // `ready()` only resolves when there is something here, so this is the
        // spurious-wakeup path. Empty is correct and safe: `blocking_read`
        // tolerates it, and its ten-attempt trap cannot be reached because
        // `ready()` will not resolve again until a push.
        if q.is_empty() {
            return Ok(Bytes::new());
        }
        let take = size.min(q.len());
        let out: Vec<u8> = q.drain(..take).collect();
        Ok(Bytes::from(out))
    }
}

/// Where the scanner is between guest writes.
#[derive(PartialEq, Eq)]
enum SinkMode {
    /// Looking for the frame marker; everything else is tenant output.
    Scanning,
    /// Inside a frame, accumulating until the terminating newline.
    InFrame,
    /// Inside a frame that already blew its cap: still looking for the
    /// terminating newline, but no longer keeping any of it.
    Overflowed,
}

/// Splits a guest's stdout into tenant output and `iii` request frames.
///
/// The guest marks a request with `\n<sentinel>\n<json>\n`. Everything else is
/// the tenant's own output and goes to the capped log pipe.
///
/// The sentinel is NOT a security boundary and nothing here pretends
/// otherwise: tenant code shares the interpreter, so it can reassign
/// `sys.stdout.write` or walk the frames to recover the sentinel and forge a
/// request. That costs it nothing, because every check is host-side — the
/// function id, the payload size and the timeout are all validated here, and a
/// forged frame gets exactly the same treatment as a real one.
struct FramingSink {
    logs: CappedPipe,
    marker: Vec<u8>,
    pending: Vec<u8>,
    mode: SinkMode,
    tx: tokio::sync::mpsc::UnboundedSender<Result<Vec<u8>, ()>>,
}

impl FramingSink {
    fn new(
        logs: CappedPipe,
        sentinel: &str,
        tx: tokio::sync::mpsc::UnboundedSender<Result<Vec<u8>, ()>>,
    ) -> Self {
        Self {
            logs,
            marker: format!("\n{sentinel}\n").into_bytes(),
            pending: Vec::new(),
            mode: SinkMode::Scanning,
            tx,
        }
    }

    /// Flush whatever the rolling tail is still holding.
    ///
    /// The tail exists so a marker split across two writes is still found, but
    /// it also means the LAST bytes a guest writes are held back waiting for a
    /// write that never comes. Without this, a `print` after the guest's final
    /// `iii.trigger` vanishes — which is exactly what
    /// `tenant_output_and_request_frames_do_not_contaminate_each_other`
    /// caught.
    ///
    /// A partial frame is discarded rather than logged: it was never a
    /// complete request, and emitting half of one as tenant output would put
    /// the sentinel into the caller's stdout.
    fn finish(&mut self) {
        if self.mode == SinkMode::Scanning {
            let rest: Vec<u8> = std::mem::take(&mut self.pending);
            self.logs.write_bytes(&rest);
        } else {
            self.pending.clear();
        }
    }

    fn feed(&mut self, buf: &[u8]) {
        self.pending.extend_from_slice(buf);
        loop {
            match self.mode {
                SinkMode::Scanning => {
                    match find(&self.pending, &self.marker) {
                        Some(i) => {
                            let head: Vec<u8> = self.pending.drain(..i).collect();
                            self.logs.write_bytes(&head);
                            self.pending.drain(..self.marker.len());
                            self.mode = SinkMode::InFrame;
                        }
                        None => {
                            // Hold back enough that a marker split across two
                            // writes is still found. Without this the frame is
                            // silently logged as tenant output and the guest
                            // waits forever for an answer.
                            let keep = (self.marker.len() - 1).min(self.pending.len());
                            let flush = self.pending.len() - keep;
                            let head: Vec<u8> = self.pending.drain(..flush).collect();
                            self.logs.write_bytes(&head);
                            return;
                        }
                    }
                }
                SinkMode::InFrame | SinkMode::Overflowed => {
                    match self.pending.iter().position(|b| *b == b'\n') {
                        Some(i) => {
                            let body: Vec<u8> = self.pending.drain(..i).collect();
                            self.pending.drain(..1);
                            let overflowed = self.mode == SinkMode::Overflowed;
                            self.mode = SinkMode::Scanning;
                            let _ = self.tx.send(if overflowed { Err(()) } else { Ok(body) });
                        }
                        None => {
                            if self.pending.len() > MAX_III_REQUEST_BYTES {
                                // Stop growing. The bytes already held are
                                // dropped rather than kept-and-truncated.
                                self.pending.clear();
                                self.mode = SinkMode::Overflowed;
                            }
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// The `StdoutStream` face of [`FramingSink`].
///
/// `StdoutStream` is a factory — it hands out a fresh stream per `get-stdout`
/// — so the scanner state has to live behind the `Arc`, exactly as
/// `CappedPipe`'s capping state does.
#[derive(Clone)]
struct FramingStdout {
    inner: Arc<Mutex<FramingSink>>,
}

impl IsTerminal for FramingStdout {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for FramingStdout {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

impl OutputStream for FramingStdout {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.inner.lock().unwrap().feed(&bytes);
        Ok(())
    }
    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }
    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(WRITE_BUDGET)
    }
}

#[wasmtime_wasi::async_trait]
impl Pollable for FramingStdout {
    async fn ready(&mut self) {}
}

impl AsyncWrite for FramingStdout {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.inner.lock().unwrap().feed(buf);
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// One guest request, as the guest writes it.
#[derive(serde::Deserialize)]
struct GuestCall {
    function_id: String,
    #[serde(default)]
    payload: serde_json::Value,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// Answer one guest request. Every check is host-side, which is what makes a
/// forged frame harmless.
async fn answer_guest_call(
    frame: Result<Vec<u8>, ()>,
    bridge: &Arc<dyn GuestBridge>,
    remaining: Duration,
) -> Vec<u8> {
    let reply = |ok: bool, v: serde_json::Value| {
        serde_json::to_vec(&serde_json::json!({ "ok": ok, "value": v }))
            .unwrap_or_else(|_| b"{\"ok\":false,\"value\":\"reply encoding failed\"}".to_vec())
    };
    let Ok(body) = frame else {
        return reply(
            false,
            serde_json::json!(format!(
                "iii request exceeded {MAX_III_REQUEST_BYTES} bytes and was refused"
            )),
        );
    };
    let call: GuestCall = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => {
            return reply(
                false,
                serde_json::json!(format!("malformed iii request: {e}")),
            )
        }
    };
    if call.function_id.len() > MAX_FUNCTION_ID_BYTES {
        return reply(
            false,
            serde_json::json!(format!(
                "function_id is {} bytes; max {MAX_FUNCTION_ID_BYTES}",
                call.function_id.len()
            )),
        );
    }
    // Clamped to what is LEFT of this run's budget, not just to a ceiling.
    // Unclamped, an `iii.trigger(timeout=30000)` inside a 5s run would blow
    // the run's own deadline every time and the tenant would never see the
    // bus error it could have handled. The margin is what leaves room for
    // that error to surface as a Python exception.
    let headroom = remaining.saturating_sub(Duration::from_millis(200));
    let asked = call.timeout_ms.unwrap_or(u64::MAX);
    let timeout_ms = asked.min(headroom.as_millis() as u64).max(1);

    match bridge
        .call(call.function_id, call.payload, timeout_ms)
        .await
    {
        Ok(v) => reply(true, v),
        Err(e) => reply(false, serde_json::json!(e)),
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

struct Host {
    wasi: WasiP1Ctx,
    limiter: TrackedLimiter,
}

pub struct Runner {
    engine: Engine,
    instance_pre: InstancePre<Host>,
    stdlib_root: PathBuf,
}

/// Everything `build_store` needs that is not the `Runner` itself. A struct
/// rather than ten positional arguments because both callers construct it and
/// a silently swapped pair of paths would be a sandbox escape.
struct StoreParts {
    run_dir: PathBuf,
    out_dir: PathBuf,
    work_dir: Option<PathBuf>,
    memory_mb: u64,
    stdout: CappedPipe,
    stderr: CappedPipe,
    denied: Arc<AtomicBool>,
    framing: Option<FramingStdout>,
    stdin_q: FrameQueue,
    /// When the running turn must die. Shared with the epoch callback.
    deadline: Arc<Mutex<Option<Instant>>>,
}

impl Runner {
    /// Build the sandbox. One implementation so a one-shot run and a
    /// persistent runtime cannot drift apart on a preopen or a permission.
    fn build_store(&self, p: StoreParts) -> Result<Store<Host>> {
        // No environment is passed: with the artifact root preopened at `/`,
        // CPython finds its own stdlib, and inheriting host env into an
        // untrusted guest would be a leak. The longer `/run` and `/out`
        // preopens shadow the paths they'd otherwise inherit from `/`.
        let mut builder = WasiCtxBuilder::new();
        builder
            .args(&["python", "-I", "-B", "/run/main.py", "/run", "/out"])
            .preopened_dir(&self.stdlib_root, "/", DirPerms::READ, FilePerms::READ)?
            .preopened_dir(&p.run_dir, "/run", DirPerms::READ, FilePerms::READ)?
            .preopened_dir(&p.out_dir, "/out", DirPerms::all(), FilePerms::all())?
            .stderr(p.stderr.clone());
        match &p.framing {
            // With a bridge, stdout is scanned for request frames and the
            // tenant's own output is forwarded to the same capped pipe it
            // would otherwise have written to directly.
            Some(f) => {
                builder.stdout(f.clone()).stdin(p.stdin_q.clone());
            }
            None => {
                builder.stdout(p.stdout.clone());
            }
        }
        // `/work` is deliberately NOT passed to the wrapper as an argv entry:
        // it is the guest's own directory, addressed by absolute path, and the
        // wrapper has no business in it. That keeps `wrapper.py` unchanged.
        if let Some(w) = &p.work_dir {
            builder.preopened_dir(w, "/work", DirPerms::all(), FilePerms::all())?;
        }
        let wasi = builder.build_p1();

        let host = Host {
            wasi,
            limiter: TrackedLimiter {
                max_memory_bytes: (p.memory_mb as usize) * 1024 * 1024,
                denied: p.denied.clone(),
            },
        };
        let mut store = Store::new(&self.engine, host);
        store.limiter(|h| &mut h.limiter);
        // Slice the budget rather than spending it in one epoch deadline. The
        // callback runs on every slice expiry and is the *only* thing that can
        // end the run by epoch, so `Trap::Interrupt` keeps meaning exactly
        // "the wall-clock budget is spent" — a slice expiry yields instead.
        // (`UpdateDeadline::Interrupt` from a callback raises the identical
        // `Trap::Interrupt` as the no-callback default, so nothing downstream
        // has to learn a second shape.)
        store.set_epoch_deadline(EPOCH_SLICE_TICKS);
        let deadline = p.deadline.clone();
        store.epoch_deadline_callback(move |_| {
            // The cell is the whole of the per-turn arming contract: `Some(t)`
            // means a turn is running and must die at `t`; `None` means no
            // turn is running, so nothing can be over budget. A persistent
            // runtime CLEARS it at turn end — leaving the previous turn's
            // instant armed would trap the next turn's first instruction.
            Ok(
                if deadline
                    .lock()
                    .unwrap()
                    .is_some_and(|d| Instant::now() >= d)
                {
                    wasmtime::UpdateDeadline::Interrupt
                } else {
                    // `YieldCustom`, not `Yield`. Plain `Yield` uses wasmtime's
                    // runtime-agnostic yield — `wake_by_ref()` then `Pending` —
                    // an *immediate* self-wake, so the run queue is never empty.
                    // The current-thread scheduler this was measured on only
                    // reaches its driver poll once every `event_interval` (61)
                    // task polls, and each of those polls is a full ~10 ms epoch
                    // slice, so timers wait tens of slices for their turn.
                    // (On the multi-thread flavor production actually runs, the
                    // same self-wake also lands the task in the worker's LIFO
                    // slot and gets it polled straight back. Both reasons point
                    // the same way.) `tokio::task::yield_now()` instead defers the
                    // waker, so the queue drains and the driver runs every round —
                    // which is why `YieldCustom` exists and what its docs
                    // recommend. Measured with three spinning guests on one
                    // worker: plain `Yield` fired its slices (410 of them) and
                    // starved the runtime anyway, 1 s of unrelated timer work
                    // taking 3.01 s; `YieldCustom`, 1.07 s.
                    wasmtime::UpdateDeadline::YieldCustom(
                        EPOCH_SLICE_TICKS,
                        Box::pin(tokio::task::yield_now()),
                    )
                },
            )
        });

        Ok(store)
    }
}

impl Runner {
    /// Boot once per process: extract, compile/load, pre-instantiate, start ticker.
    ///
    /// Call this exactly once. Every call starts another epoch ticker on the
    /// same-but-separate `Engine`; two runners in one process would be two
    /// ~19 MB module images and two ticker threads, not a shared one.
    pub fn boot() -> Result<Arc<Runner>> {
        let stdlib_root = artifact::ensure_extracted()?;
        let engine = artifact::sandbox_engine()?;
        let module = artifact::load_module(&engine, &stdlib_root)?;
        let mut linker: Linker<Host> = Linker::new(&engine);
        p1::add_to_linker_async(&mut linker, |h: &mut Host| &mut h.wasi)?;
        let instance_pre = linker.instantiate_pre(&module)?;

        // One ticker for the process lifetime; every run's deadline is
        // denominated in these ticks. Detached deliberately — it dies with
        // the process.
        let ticker_engine = engine.clone();
        std::thread::Builder::new()
            .name("python-engine-epoch".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(EPOCH_TICK_MS));
                ticker_engine.increment_epoch();
            })
            .context("spawning epoch ticker")?;

        Ok(Arc::new(Runner {
            engine,
            instance_pre,
            stdlib_root,
        }))
    }

    /// Async because it is the only way to bound the run's wall clock.
    ///
    /// Epoch interruption is observed only at wasm loop back-edges and
    /// function entries, so a guest parked in a host call — `time.sleep`, any
    /// long `poll_oneoff` — is invisible to it and holds the thread for as
    /// long as it likes. Under `call_async` the guest runs on a fiber, and
    /// dropping the timed-out future makes wasmtime resume that fiber with an
    /// error so it unwinds; the thread and the store come back.
    pub async fn run(&self, spec: &RunSpec) -> Result<RunOutcome> {
        // Per-run host dirs; RAII cleanup on every path.
        let scratch = tempfile::tempdir().context("creating run scratch dir")?;
        let run_dir = scratch.path().join("run");
        let out_dir = scratch.path().join("out");
        std::fs::create_dir_all(&run_dir)?;
        std::fs::create_dir_all(&out_dir)?;
        std::fs::write(run_dir.join("main.py"), include_str!("wrapper.py"))?;
        std::fs::write(run_dir.join("code.py"), &spec.code)?;
        if let Some(p) = &spec.payload_json {
            std::fs::write(run_dir.join("payload.json"), p)?;
        }

        let stdout = CappedPipe::new(MAX_LOG_BYTES);
        let stderr = CappedPipe::new(MAX_LOG_BYTES);
        let denied = Arc::new(AtomicBool::new(false));

        // The bridge, when there is one. The sentinel is per-run and random:
        // it is not a security boundary (tenant code shares the interpreter
        // and can read it), but a fixed one would let tenant output collide
        // with a frame by accident.
        let sentinel = uuid::Uuid::new_v4().to_string();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let stdin_q = FrameQueue::new();
        let framing = spec.bridge.as_ref().map(|_| FramingStdout {
            inner: Arc::new(Mutex::new(FramingSink::new(
                stdout.clone(),
                &sentinel,
                frame_tx,
            ))),
        });
        if spec.bridge.is_some() {
            // The guest learns the sentinel and its namespace from a file in
            // its read-only `/run`, so the wrapper's two-argument contract is
            // unchanged and the whole bridge stays optional: no file, no
            // `iii` global.
            std::fs::write(
                run_dir.join("iii.json"),
                serde_json::to_vec(&serde_json::json!({
                    "sentinel": sentinel,
                    "namespace": spec.namespace.clone().unwrap_or_default(),
                }))?,
            )?;
        }

        let budget = Duration::from_millis(spec.timeout_ms);
        let started = Instant::now();
        let deadline = Arc::new(Mutex::new(Some(started + budget)));
        let mut store = self.build_store(StoreParts {
            run_dir: run_dir.clone(),
            out_dir: out_dir.clone(),
            work_dir: spec.work_dir.clone(),
            memory_mb: spec.memory_mb,
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            denied: denied.clone(),
            framing: framing.clone(),
            stdin_q: stdin_q.clone(),
            deadline,
        })?;

        // Instantiation is outside the wall-clock backstop on purpose: it runs
        // no guest code and makes no host calls, so it cannot park. Its only
        // failure of interest is the limiter refusing the module's minimum
        // memory, below.
        let instance = match self.instance_pre.instantiate_async(&mut store).await {
            Ok(i) => i,
            // The interpreter's 40 MiB minimum memory (MIN_MEMORY_MB) is
            // requested from the limiter before any code runs, so a
            // `memory_mb` under that fails here rather than at a `memory.grow`
            // — and `config.rs` clamps `memory_mb` to 1..=max, so ordinary
            // tenant input reaches this. That is a resource condition, not an
            // internal fault: report it as a denied run so the manager can
            // classify it as such instead of as `internal`.
            Err(e) if denied.load(Ordering::Relaxed) => {
                return Ok(RunOutcome {
                    exit: ExitKind::Trap(trap_text(e)),
                    envelope: None,
                    stdout: stdout.into_captured(),
                    stderr: stderr.into_captured(),
                    timed_out: false,
                    memory_denied: true,
                    disk_exceeded: None,
                });
            }
            Err(e) => {
                return Err(anyhow::Error::from(e).context("instantiating the interpreter"));
            }
        };
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(anyhow::Error::from)
            .context("locating _start")?;
        // A normal CPython run does NOT return Ok(()): it calls proc_exit(0),
        // which arrives here as an Err carrying I32Exit. Only a genuine trap
        // (epoch kill, unreachable, a host-side error) takes the other branch.
        // A denied memory growth is *not* a trap — it makes memory.grow return
        // -1, which CPython reports to the tenant as MemoryError.
        let call = start.call_async(&mut store, ());
        // The third kill. `/out` is guest-writable by necessity — the envelope
        // arrives through it — and nothing in wasmtime bounds it, so the host
        // measures it instead. Losing this `select!` drops `call`, which is the
        // same unwind-the-fiber-and-hand-back-the-store path the wall-clock
        // backstop already proves works; `store` is not touched afterwards on
        // any arm.
        // Answers guest requests for as long as the run lasts. Never
        // resolves, so it only ever runs concurrently with the arms that do.
        let bridge_pump = async {
            if let Some(bridge) = spec.bridge.clone() {
                while let Some(frame) = frame_rx.recv().await {
                    let remaining = budget.saturating_sub(started.elapsed());
                    let reply = answer_guest_call(frame, &bridge, remaining).await;
                    stdin_q.push_frame(&reply);
                }
            }
            std::future::pending::<()>().await
        };

        let (exit, killed, disk_exceeded) = tokio::select! {
            timed = tokio::time::timeout(budget, call) => match timed {
                // The backstop fired: the future has been dropped, wasmtime has
                // unwound the fiber, and this thread is ours again. The guest was
                // parked in a host call, so there is no wasm trap to report.
                Err(_elapsed) => (
                    ExitKind::Trap(format!(
                        "run exceeded its {}ms budget while parked in a host call",
                        spec.timeout_ms
                    )),
                    true,
                    None,
                ),
                Ok(Ok(())) => (ExitKind::Clean(0), false, None),
                Ok(Err(e)) => match e.downcast_ref::<I32Exit>() {
                    Some(code) => (ExitKind::Clean(code.0), false, None),
                    None => {
                        // `Trap::Interrupt` has exactly one producer: the epoch
                        // deadline. Exact, and not forgeable by the guest.
                        let by_epoch = matches!(e.downcast_ref::<Trap>(), Some(Trap::Interrupt));
                        (ExitKind::Trap(trap_text(e)), by_epoch, None)
                    }
                },
            },
            () = bridge_pump => unreachable!("the bridge pump never resolves"),
            which = disk_watchdog(out_dir.clone(), spec.work_dir.clone()) => (
                ExitKind::Trap(format!("run exceeded its {which} disk budget")),
                false,
                Some(which),
            ),
        };

        // Every term is host-side. `killed` is whichever kill fired; the
        // elapsed check is belt-and-braces for a trap that lands on the
        // deadline boundary before either mechanism names itself. A disk kill
        // is excluded: it can land at any point in the budget, and calling it
        // a timeout would tell the caller the wrong thing to fix.
        let timed_out = disk_exceeded.is_none()
            && (killed || (matches!(exit, ExitKind::Trap(_)) && started.elapsed() >= budget));
        // Drain the scanner's rolling tail before the streams are read.
        if let Some(f) = &framing {
            f.inner.lock().unwrap().finish();
        }
        let envelope = read_envelope(&out_dir.join("result.json"));

        Ok(RunOutcome {
            exit,
            envelope,
            stdout: stdout.into_captured(),
            stderr: stderr.into_captured(),
            timed_out,
            memory_denied: denied.load(Ordering::Relaxed),
            disk_exceeded,
        })
    }
}

// ------------------------------------------------------------- persistence

/// What `spawn_persistent` needs to boot an interpreter that outlives a call.
///
/// `memory_mb` is here and NOT on the turn on purpose: wasm linear memory only
/// ever grows, so a per-call ceiling on a shared interpreter would be a promise
/// the sandbox cannot keep — the second caller inherits whatever the first grew
/// to. Creation-time only, and callers must refuse a per-call `memory_mb`
/// against an existing runtime rather than silently ignoring it.
pub struct PersistentSpec {
    pub work_dir: PathBuf,
    pub memory_mb: u64,
    pub bridge: Option<Arc<dyn GuestBridge>>,
    pub namespace: Option<String>,
}

/// Spawning failed. `memory_denied` separates "this host is out of memory for
/// the interpreter's 40 MiB floor" from an internal fault, because the two
/// reach the caller as very different errors.
pub struct SpawnFailed {
    pub memory_denied: bool,
    pub error: anyhow::Error,
}

struct Turn {
    code: String,
    payload_json: Option<String>,
    timeout_ms: u64,
    reply: tokio::sync::oneshot::Sender<RunOutcome>,
}

/// A CPython interpreter parked on stdin between calls.
///
/// Everything a turn leaves behind — imported modules, globals bound in a
/// previous `exec`, open handles — is still there for the next one. That is
/// the point, and it is also why a `runtime_id` is a capability: two callers
/// sharing one of these share an address space.
pub struct PersistentRuntime {
    turns: tokio::sync::mpsc::Sender<Turn>,
}

impl PersistentRuntime {
    /// Run one turn on this interpreter.
    ///
    /// `Err` means the interpreter is gone — poisoned by a previous timeout,
    /// or dead of a trap. The working directory is untouched, so the caller's
    /// recovery is to boot a fresh interpreter on it.
    pub async fn run(
        &self,
        code: String,
        payload_json: Option<String>,
        timeout_ms: u64,
    ) -> Result<RunOutcome> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.turns
            .send(Turn {
                code,
                payload_json,
                timeout_ms,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("the interpreter is no longer running"))?;
        answer
            .await
            .map_err(|_| anyhow::anyhow!("the interpreter died mid-turn"))
    }

    /// False once the interpreter's task has exited. Lets a manager evict a
    /// poisoned runtime on sweep instead of only on the next call.
    pub fn is_live(&self) -> bool {
        !self.turns.is_closed()
    }
}

/// `_start` returning, in the two shapes that matter. Shared with the one-shot
/// path's reading of the same result: a clean CPython exit arrives as an
/// `I32Exit` error, not `Ok(())`.
fn exit_of(r: std::result::Result<(), wasmtime::Error>) -> (ExitKind, bool) {
    match r {
        Ok(()) => (ExitKind::Clean(0), false),
        Err(e) => match e.downcast_ref::<I32Exit>() {
            Some(code) => (ExitKind::Clean(code.0), false),
            None => {
                // `Trap::Interrupt` has exactly one producer: the epoch
                // deadline. Exact, and not forgeable by the guest.
                let by_epoch = matches!(e.downcast_ref::<Trap>(), Some(Trap::Interrupt));
                (ExitKind::Trap(trap_text(e)), by_epoch)
            }
        },
    }
}

/// Empty a directory without removing it — the guest holds a preopen on it.
///
/// `file_type()` does not follow symlinks, so a guest-planted symlink to a
/// directory takes the `remove_file` arm and unlinks the link rather than
/// recursing through it into the host.
fn wipe_dir(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let _ = if is_dir {
            std::fs::remove_dir_all(e.path())
        } else {
            std::fs::remove_file(e.path())
        };
    }
}

/// Sleep until `at`, or forever when there is nothing to wait for.
async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(tokio::time::Instant::from_std(t)).await,
        None => std::future::pending().await,
    }
}

/// Is this frame the wrapper saying "the turn is over"?
///
/// Not a trust boundary — the guest writes it, and everything under `/out` was
/// already guest-written. A tenant that forges one early only makes the host
/// read an envelope the wrapper has not produced yet, which comes back as its
/// own failed run. The host-derived kills are still checked first, upstream.
fn is_turn_done(frame: &std::result::Result<Vec<u8>, ()>) -> bool {
    let Ok(body) = frame else { return false };
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("kind")?.as_str().map(|s| s == "turn_done"))
        .unwrap_or(false)
}

impl Runner {
    /// Boot an interpreter and park it on stdin, ready for turns.
    ///
    /// The store cannot be held across calls in a map: `call_async` borrows it
    /// for the future's life, so a suspended `_start` future beside its own
    /// store would be self-referential. Instead a task owns both and turns
    /// arrive over a channel — the same shape node-core's runtimes use.
    pub async fn spawn_persistent(
        &self,
        spec: PersistentSpec,
    ) -> std::result::Result<PersistentRuntime, SpawnFailed> {
        let plain = |e: anyhow::Error| SpawnFailed {
            memory_denied: false,
            error: e,
        };
        let scratch = tempfile::tempdir()
            .context("creating runtime scratch dir")
            .map_err(plain)?;
        let run_dir = scratch.path().join("run");
        let out_dir = scratch.path().join("out");
        let setup = || -> Result<String> {
            std::fs::create_dir_all(&run_dir)?;
            std::fs::create_dir_all(&out_dir)?;
            std::fs::write(run_dir.join("main.py"), include_str!("wrapper.py"))?;
            // No `code.py` and no `payload.json`: a persistent interpreter is
            // told what to run per turn, over stdin.
            let sentinel = uuid::Uuid::new_v4().to_string();
            std::fs::write(
                run_dir.join("iii.json"),
                serde_json::to_vec(&serde_json::json!({
                    "sentinel": sentinel,
                    "namespace": spec.namespace.clone().unwrap_or_default(),
                    "persistent": true,
                }))?,
            )?;
            Ok(sentinel)
        };
        let sentinel = setup()
            .context("preparing the runtime dir")
            .map_err(plain)?;

        let stdout = CappedPipe::new(MAX_LOG_BYTES);
        let stderr = CappedPipe::new(MAX_LOG_BYTES);
        let denied = Arc::new(AtomicBool::new(false));
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
        let stdin_q = FrameQueue::new();
        // Always framed, bridge or not: the turn-done frame rides the same
        // channel, so a persistent runtime is never unframed.
        let framing = FramingStdout {
            inner: Arc::new(Mutex::new(FramingSink::new(
                stdout.clone(),
                &sentinel,
                frame_tx,
            ))),
        };
        // `None` = no turn running, so nothing can be over budget. The task
        // arms it at turn start and clears it at turn end.
        let deadline = Arc::new(Mutex::new(None));

        let mut store = self
            .build_store(StoreParts {
                run_dir,
                out_dir: out_dir.clone(),
                work_dir: Some(spec.work_dir.clone()),
                memory_mb: spec.memory_mb,
                stdout: stdout.clone(),
                stderr: stderr.clone(),
                denied: denied.clone(),
                framing: Some(framing.clone()),
                stdin_q: stdin_q.clone(),
                deadline: deadline.clone(),
            })
            .map_err(plain)?;
        let instance = self
            .instance_pre
            .instantiate_async(&mut store)
            .await
            .map_err(|e| SpawnFailed {
                memory_denied: denied.load(Ordering::Relaxed),
                error: anyhow::Error::from(e).context("instantiating the interpreter"),
            })?;
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(|e| plain(anyhow::Error::from(e).context("locating _start")))?;

        // Depth 1: a caller whose turn is already queued waits in `send`, so
        // back-pressure lands on the caller rather than in an unbounded queue.
        let (turn_tx, mut turn_rx) = tokio::sync::mpsc::channel::<Turn>(1);
        let bridge = spec.bridge.clone();
        let work_dir = spec.work_dir.clone();
        tokio::spawn(async move {
            // Owned by the task so the sandbox outlives every turn and is
            // removed when the interpreter dies.
            let _scratch = scratch;
            let call = start.call_async(&mut store, ());
            tokio::pin!(call);
            // Created once, not per loop iteration: a chatty guest would
            // otherwise restart the watchdog's poll interval on every frame
            // and push detection out indefinitely.
            let watchdog = disk_watchdog(out_dir.clone(), Some(work_dir));
            tokio::pin!(watchdog);

            let envelope_path = out_dir.join("result.json");
            let mut current: Option<(Turn, Instant)> = None;

            // Every exit from this loop is terminal for the interpreter. The
            // working directory is not ours to clean up — it outlives us, and
            // keeping it is the whole recovery story for a poisoned runtime.
            loop {
                let at = current.as_ref().map(|(_, d)| *d);
                tokio::select! {
                    // A running turn's deadline. Nothing else can preempt a
                    // guest parked in a host call, and dropping `call` is what
                    // unwinds the fiber — so this arm ends the interpreter.
                    () = sleep_until(at), if at.is_some() => {
                        let (turn, _) = current.take().expect("armed only with a turn");
                        framing.inner.lock().unwrap().finish();
                        let _ = turn.reply.send(RunOutcome {
                            exit: ExitKind::Trap(format!(
                                "run exceeded its {}ms budget", turn.timeout_ms
                            )),
                            envelope: None,
                            stdout: stdout.take_captured(),
                            stderr: stderr.take_captured(),
                            timed_out: true,
                            memory_denied: denied.load(Ordering::Relaxed),
                            disk_exceeded: None,
                        });
                        break;
                    }
                    // The interpreter exited or trapped. Terminal either way:
                    // `_start` does not come back.
                    r = &mut call => {
                        let (exit, by_epoch) = exit_of(r);
                        if let Some((turn, _)) = current.take() {
                            framing.inner.lock().unwrap().finish();
                            let _ = turn.reply.send(RunOutcome {
                                exit,
                                envelope: read_envelope(&envelope_path),
                                stdout: stdout.take_captured(),
                                stderr: stderr.take_captured(),
                                timed_out: by_epoch,
                                memory_denied: denied.load(Ordering::Relaxed),
                                disk_exceeded: None,
                            });
                        }
                        break;
                    }
                    // A guest-writable directory blew its budget.
                    which = &mut watchdog => {
                        if let Some((turn, _)) = current.take() {
                            framing.inner.lock().unwrap().finish();
                            let _ = turn.reply.send(RunOutcome {
                                exit: ExitKind::Trap(format!("run exceeded its {which} disk budget")),
                                envelope: None,
                                stdout: stdout.take_captured(),
                                stderr: stderr.take_captured(),
                                timed_out: false,
                                memory_denied: denied.load(Ordering::Relaxed),
                                disk_exceeded: Some(which),
                            });
                        }
                        break;
                    }
                    Some(frame) = frame_rx.recv() => {
                        if is_turn_done(&frame) {
                            // Belt-and-braces. Turn start arms the cell BEFORE
                            // pushing the frame that resumes the guest, so a
                            // stale instant is already unreachable: between
                            // turns the guest is parked in `fd_read` and no
                            // wasm executes, so the epoch callback cannot run.
                            // The clear is what keeps that true if the two
                            // lines at turn start are ever reordered — which
                            // is why no test here claims to catch its removal.
                            *deadline.lock().unwrap() = None;
                            if let Some((turn, _)) = current.take() {
                                framing.inner.lock().unwrap().finish();
                                let _ = turn.reply.send(RunOutcome {
                                    exit: ExitKind::Clean(0),
                                    envelope: read_envelope(&envelope_path),
                                    stdout: stdout.take_captured(),
                                    stderr: stderr.take_captured(),
                                    timed_out: false,
                                    memory_denied: denied.load(Ordering::Relaxed),
                                    disk_exceeded: None,
                                });
                            }
                        } else {
                            // An `iii.trigger`. Awaiting it inline stops
                            // polling `call`, which is exactly right: the
                            // guest is blocked on `fd_read` for this answer
                            // and has nothing to make progress on.
                            let remaining = at
                                .map(|d| d.saturating_duration_since(Instant::now()))
                                .unwrap_or(Duration::ZERO);
                            let reply = match &bridge {
                                Some(b) => answer_guest_call(frame, b, remaining).await,
                                None => br#"{"ok":false,"value":"this runtime has no host bridge"}"#
                                    .to_vec(),
                            };
                            stdin_q.push_frame(&reply);
                        }
                    }
                    // `recv` rather than `Some(turn) = recv`: a failed
                    // pattern only disables the branch for one `select!` call,
                    // so a torn-down runtime would leave this task parked on
                    // `call` forever, holding its store and its scratch dir. It
                    // is disabled while a turn is running, so a teardown
                    // mid-turn is noticed as soon as that turn finishes.
                    queued = turn_rx.recv(), if current.is_none() => {
                        let Some(turn) = queued else { break };
                        // `/out` is per-call: its budget would otherwise be
                        // spent by whichever turn wrote first, while `/work`
                        // deliberately accumulates.
                        wipe_dir(&out_dir);
                        // Sticky otherwise — one recovered MemoryError would
                        // make every later turn classify as out-of-memory.
                        denied.store(false, Ordering::Relaxed);
                        let at = Instant::now() + Duration::from_millis(turn.timeout_ms);
                        *deadline.lock().unwrap() = Some(at);
                        let payload = turn
                            .payload_json
                            .as_deref()
                            .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
                            .unwrap_or(serde_json::Value::Null);
                        let body = serde_json::to_vec(&serde_json::json!({
                            "code": turn.code,
                            "payload": payload,
                        }))
                        .expect("a string and a parsed value serialize");
                        stdin_q.push_frame(&body);
                        current = Some((turn, at));
                    }
                }
            }
        });

        Ok(PersistentRuntime { turns: turn_tx })
    }
}

#[cfg(test)]
mod sink_tests {
    use super::*;

    type Frames = tokio::sync::mpsc::UnboundedReceiver<Result<Vec<u8>, ()>>;

    fn sink(sentinel: &str) -> (FramingSink, CappedPipe, Frames) {
        let logs = CappedPipe::new(MAX_LOG_BYTES);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (FramingSink::new(logs.clone(), sentinel, tx), logs, rx)
    }

    fn logged(p: CappedPipe) -> String {
        String::from_utf8_lossy(&p.into_captured().bytes).into_owned()
    }

    /// The straddle case, driven directly.
    ///
    /// A guest test cannot reach this: CPython buffers, so it emits the whole
    /// frame in one `write` and the marker never splits — which is exactly why
    /// removing the rolling tail left the end-to-end test green when it was
    /// mutated. This is the test that actually pins it.
    ///
    /// Mutation: set `keep` to 0 in `feed`'s `None` arm.
    #[test]
    fn a_marker_split_across_two_writes_is_still_a_frame() {
        let (mut s, logs, mut rx) = sink("SENTINEL");
        s.feed(b"before\n\nSENT");
        s.feed(b"INEL\n{\"function_id\":\"x::y\"}\nafter\n");
        s.finish();

        let frame = rx.try_recv().expect("a frame must have been emitted");
        assert_eq!(frame.unwrap(), br#"{"function_id":"x::y"}"#.to_vec());
        assert_eq!(logged(logs), "before\nafter\n");
    }

    /// One byte at a time — the worst case for any scanner that assumes a
    /// write boundary means something.
    #[test]
    fn a_frame_delivered_one_byte_at_a_time_is_still_a_frame() {
        let (mut s, logs, mut rx) = sink("S");
        for b in b"hi\n\nS\n{\"function_id\":\"a::b\"}\nbye\n" {
            s.feed(&[*b]);
        }
        s.finish();
        assert_eq!(
            rx.try_recv().expect("a frame").unwrap(),
            br#"{"function_id":"a::b"}"#.to_vec()
        );
        assert_eq!(logged(logs), "hi\nbye\n");
    }

    /// Past the cap the buffer STOPS GROWING rather than accumulating and
    /// being checked afterwards — the whole point of the host owning the sink.
    /// The frame is refused, not truncated-and-parsed: a truncated prefix can
    /// still parse into something plausible.
    ///
    /// The peak assertion is what discriminates. Asserting only that the frame
    /// was refused passes against a mutant with no guard at all, because the
    /// terminating newline still ends the frame — it just does so after the
    /// guest chose how much host memory to spend.
    #[test]
    fn an_oversized_frame_is_refused_without_being_buffered() {
        let (mut s, _logs, mut rx) = sink("S");
        s.feed(b"\nS\n");
        let chunk = vec![b'x'; 64 * 1024];
        let mut peak = 0usize;
        for _ in 0..(MAX_III_REQUEST_BYTES / chunk.len() + 4) {
            s.feed(&chunk);
            peak = peak.max(s.pending.len());
        }
        s.feed(b"\n");
        assert!(
            rx.try_recv().expect("a verdict").is_err(),
            "an over-cap frame must be refused"
        );
        assert!(
            peak <= MAX_III_REQUEST_BYTES + chunk.len(),
            "peak buffer was {peak}; the cap must bound it to one write's overshoot"
        );
    }

    /// Text that merely contains the sentinel mid-line is tenant output, not
    /// a frame: the marker is newline-delimited on both sides.
    #[test]
    fn a_sentinel_inside_a_line_is_not_a_frame() {
        let (mut s, logs, mut rx) = sink("S");
        s.feed(b"look at this S thing\n");
        s.finish();
        assert!(rx.try_recv().is_err(), "no frame should have been emitted");
        assert_eq!(logged(logs), "look at this S thing\n");
    }
}
