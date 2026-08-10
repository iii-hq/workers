//! One V8 isolate on one OS thread, driven by a multiplexed loop.
//!
//! Isolates are not `Send`, so each runtime owns a thread and is reached only
//! through [`Command`]s on an mpsc channel. The loop interleaves starting new
//! commands, polling in-flight promises, and pumping the deno_core event loop,
//! which is what lets one JS handler await another handler in the same isolate
//! instead of deadlocking behind it.

use std::cell::RefCell;
use std::future::poll_fn;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use deno_core::error::JsError;
use deno_core::{v8, JsRuntime, OpState, PollEventLoopOptions, RuntimeOptions};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::engine::Engine;
use crate::error::NodeEngineError;
use crate::ops::{node_engine_ext, with_ops_state, OpsState};
use crate::protocol::{wrap_eval, wrap_invoke, Envelope};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct LogLine {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvalOutcome {
    pub result: Value,
    pub logs: Vec<LogLine>,
    pub registered: Vec<String>,
}

pub enum Command {
    Eval {
        code: String,
        timeout: Duration,
        reply: oneshot::Sender<Result<EvalOutcome, NodeEngineError>>,
    },
    Invoke {
        fn_id: String,
        payload: Value,
        timeout: Duration,
        reply: oneshot::Sender<Result<Value, NodeEngineError>>,
        /// `None` for a plain function invocation; `Some("registerTrigger" |
        /// "unregisterTrigger")` when this targets a trigger TYPE's callback
        /// instead — see `wrap_invoke`.
        method: Option<String>,
    },
}

#[derive(Clone)]
pub struct RuntimeOpts {
    pub heap_mb: usize,
    /// Ceiling on off-heap (ArrayBuffer) memory — see `crate::allocator`.
    pub external_mb: usize,
    /// Required prefix for ids the evaluated code may register.
    pub namespace: String,
    /// Budget for `iii.trigger`, for handler invocations arriving from
    /// the bus, and for any single event-loop pump.
    pub call_timeout_ms: u64,
    /// Ceiling a guest-supplied `iii.trigger({..., timeout})` is clamped to.
    /// Same value (`NodeEngineConfig::max_timeout_ms`) the RPC-level eval
    /// timeout is clamped to via `clamp_timeout` in `manager.rs` — that
    /// clamp only ever covered the eval's own deadline, not this op's
    /// argument, which reached `Engine::call` unclamped before this field
    /// existed.
    pub max_timeout_ms: u64,
    /// Shared with every other runtime; guards the duplicate-id abort.
    pub ids: crate::ids::IdRegistry,
    /// This runtime's own id, used as the ownership key in `ids`.
    pub runtime_id: String,
    /// The SAME `Arc` as `manager::Runtime::last_activity`. `run`/`register`
    /// bump it from the manager's side; `op_iii_register`'s invoke proxy (see
    /// `OpsState::last_activity`) bumps it from this thread's side, since an
    /// INVOKE dispatched straight from the bus never reaches the manager at
    /// all.
    pub last_activity: Arc<std::sync::Mutex<std::time::Instant>>,
    /// Per-runtime scratch quota, bytes. 0 removes `iii.files` entirely.
    pub scratch_mb: usize,
    /// Per-runtime scratch quota, entry count.
    pub scratch_files: usize,
    /// Where the scratch directory is created; `None` uses the system temp
    /// directory.
    pub scratch_root: Option<String>,
}

// `runtime_id` is a capability — same rule as `RunRequest` and `IdRegistry`.
// `ids` already redacts itself (see `IdRegistry`'s own hand-rolled `Debug`,
// just above it in `crate::ids`), so only `runtime_id` needs guarding here.
impl std::fmt::Debug for RuntimeOpts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeOpts")
            .field("heap_mb", &self.heap_mb)
            .field("external_mb", &self.external_mb)
            .field("namespace", &self.namespace)
            .field("call_timeout_ms", &self.call_timeout_ms)
            .field("max_timeout_ms", &self.max_timeout_ms)
            .field("ids", &self.ids)
            .field("runtime_id", &"<redacted>")
            .field("last_activity", &self.last_activity)
            .field("scratch_mb", &self.scratch_mb)
            .field("scratch_files", &self.scratch_files)
            .field("scratch_root", &self.scratch_root)
            .finish()
    }
}

/// Create this runtime's private scratch directory, or `None` when the
/// feature is switched off.
///
/// The directory name is NOT derived from `runtime_id`. `tempfile` mints its
/// own independent random suffix, so `ls $TMPDIR` does not enumerate live
/// runtime capabilities and the mapping is not derivable in either direction.
/// "Name the dir after the runtime, for debuggability" is the obvious future
/// improvement and it is exactly wrong: `runtime_id` is the capability to
/// eval into and tear down a runtime, and that leak has already been found
/// and fixed four separate times in this crate. An operator who genuinely
/// needs the correlation can add a `debug` tracing line — an explicit,
/// reviewable choice rather than a default.
///
/// `tempfile::Builder` also gives O_EXCL creation (so no pre-planted
/// directory or symlink at the root) and mode 0700 on unix, in one call.
fn make_scratch(opts: &RuntimeOpts) -> std::io::Result<Option<tempfile::TempDir>> {
    if opts.scratch_mb == 0 {
        return Ok(None);
    }
    let mut builder = tempfile::Builder::new();
    builder.prefix("node-engine-");
    let dir = match &opts.scratch_root {
        Some(root) => {
            std::fs::create_dir_all(root)?;
            builder.tempdir_in(root)?
        }
        None => builder.tempdir()?,
    };
    Ok(Some(dir))
}

/// Initialise the V8 platform exactly once per process, before any isolate is
/// created. Safe to call from anywhere; later calls are no-ops.
pub fn init_v8_platform() {
    static ONCE: once_cell::sync::OnceCell<()> = once_cell::sync::OnceCell::new();
    ONCE.get_or_init(|| JsRuntime::init_platform(None));
}

pub type Unregisters = Arc<
    std::sync::Mutex<
        Vec<(
            crate::ops::RegistrationKind,
            String,
            crate::engine::UnregisterFn,
        )>,
    >,
>;

pub struct RuntimeThread {
    tx: mpsc::UnboundedSender<Command>,
    unregisters: Unregisters,
    join: Option<std::thread::JoinHandle<()>>,
}

impl RuntimeThread {
    pub fn spawn(opts: RuntimeOpts, engine: Arc<dyn Engine>) -> RuntimeThread {
        let (tx, rx) = mpsc::unbounded_channel();
        let weak_tx = tx.downgrade();
        let unregisters: Unregisters = Arc::new(std::sync::Mutex::new(Vec::new()));
        let thread_unregisters = unregisters.clone();
        let join = std::thread::Builder::new()
            .name("node-engine-isolate".to_string())
            .spawn(move || {
                let tokio_rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("current-thread runtime builds");
                tokio_rt.block_on(run_loop(rx, opts, engine, thread_unregisters, weak_tx));
            })
            .expect("isolate thread spawns");
        RuntimeThread {
            tx,
            unregisters,
            join: Some(join),
        }
    }

    /// Every live registration this runtime made, so the manager can undo them
    /// from its own thread at teardown.
    pub fn unregisters(&self) -> Unregisters {
        self.unregisters.clone()
    }

    /// Queue a command. `Err` returns the command when the loop has exited.
    ///
    /// Boxed, not bare `Command`: `Invoke` grew a `method: Option<String>`
    /// field alongside `fn_id`/`payload`/`timeout`/`reply`, and clippy's
    /// `result_large_err` flags a bare `Command` here as bloating every
    /// caller's stack frame for the `Ok(())` case too. Every current caller
    /// only checks `.is_err()`, so this costs nothing today; a future one
    /// that wants the command back still can.
    ///
    /// There is deliberately no accessor handing out a strong sender clone:
    /// the loop only sees its channel close when EVERY sender is gone, so a
    /// clone parked in a registry or a long-lived task would make `Drop`'s
    /// join below block forever. The registration proxy holds a
    /// `WeakUnboundedSender` for the same reason.
    pub fn send(&self, cmd: Command) -> Result<(), Box<Command>> {
        self.tx.send(cmd).map_err(|e| Box::new(e.0))
    }

    /// Close the channel and join the isolate thread.
    pub fn shutdown(self) {
        drop(self);
    }
}

impl Drop for RuntimeThread {
    fn drop(&mut self) {
        // The sender MUST be dropped before the join: the loop only exits on
        // `Poll::Ready(None)`, so joining first would hang forever. `Drop::drop`
        // runs before the struct's own fields are dropped, hence the swap.
        //
        // ponytail: blocking join on the caller's thread. Teardown is rare and
        // the loop exits as soon as the channel closes; move to a detached
        // reaper task if teardown ever shows up on a hot path.
        let (dead_tx, _) = mpsc::unbounded_channel();
        drop(std::mem::replace(&mut self.tx, dead_tx));
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Data the heap-limit callback needs. V8 gives the callback a raw pointer, so
/// this lives in a `Box` whose ownership is described at the call site.
struct HeapGuard {
    oom: Arc<AtomicBool>,
    isolate: v8::IsolateHandle,
}

/// # Safety
/// `data` must point at a live `HeapGuard`. The only caller is `run_loop`,
/// which owns it in a `Box` bound BEFORE its `JsRuntime` — locals drop in
/// reverse declaration order, so the isolate is always disposed (and can no
/// longer invoke this callback) before the `Box` is freed, on the unwind path
/// as well as the normal one. V8 calls this on the isolate's own thread while
/// JS is running.
unsafe extern "C" fn near_heap_limit(
    data: *mut std::ffi::c_void,
    current_limit: usize,
    _initial_limit: usize,
) -> usize {
    let guard = &*(data as *const HeapGuard);
    guard.oom.store(true, Ordering::SeqCst);
    guard.isolate.terminate_execution();
    // Hand V8 headroom so it can unwind instead of aborting the process.
    current_limit * 2
}

enum Reply {
    Eval(oneshot::Sender<Result<EvalOutcome, NodeEngineError>>),
    Invoke(oneshot::Sender<Result<Value, NodeEngineError>>),
}

impl Reply {
    fn fail(self, err: NodeEngineError) {
        match self {
            Reply::Eval(tx) => {
                let _ = tx.send(Err(err));
            }
            Reply::Invoke(tx) => {
                let _ = tx.send(Err(err));
            }
        }
    }
}

type SettleFuture =
    Pin<Box<dyn std::future::Future<Output = Result<v8::Global<v8::Value>, Box<JsError>>>>>;

struct Pending {
    fut: SettleFuture,
    reply: Reply,
    deadline: Instant,
}

/// Why the loop is exiting, and who still needs answering.
///
/// `reply` carries the caller whose own command killed the isolate. `start`
/// deliberately hands it back instead of sending it, because that caller may
/// re-send the instant it sees the error — and it must find a closed channel,
/// not queue work onto a dead runtime. Teardown closes `rx` first, then
/// answers.
struct FatalExit {
    err: NodeEngineError,
    reply: Option<Reply>,
}

/// Deadline the watchdog is currently enforcing. `None` = nothing in flight.
pub type DeadlineSlot = Arc<std::sync::Mutex<Option<std::time::Instant>>>;

/// The only thing that can stop runaway synchronous JavaScript.
///
/// While V8 executes a synchronous script the isolate thread is inside V8 and
/// never returns to tokio, so no timer *on that thread* can fire — a bare
/// `for(;;){}` would hang it forever. `v8::IsolateHandle` is `Send + Sync`
/// precisely so another thread can interrupt it, which is what this does.
fn spawn_watchdog(
    isolate: v8::IsolateHandle,
    deadline: DeadlineSlot,
    terminated: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("node-engine-watchdog".to_string())
        .spawn(move || {
            // ponytail: 25ms poll rather than a condvar. Deadlines are in the
            // seconds range, so finer wakeups buy nothing; switch to a condvar
            // only if sub-tick precision ever matters.
            const TICK: Duration = Duration::from_millis(25);
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(TICK);
                // Check and terminate under ONE lock acquisition. Releasing
                // between them lets the isolate thread settle the command,
                // answer its caller Ok, and republish — and then get killed
                // anyway, handing the next unrelated caller a bogus timeout.
                // `terminate_execution` is non-blocking and the isolate thread
                // never holds this lock across anything slow, so holding it
                // here cannot deadlock.
                let mut slot = deadline.lock().unwrap();
                if slot.is_some_and(|d| std::time::Instant::now() >= d) {
                    terminated.store(true, Ordering::SeqCst);
                    isolate.terminate_execution();
                    *slot = None;
                }
            }
        })
        .expect("watchdog thread spawns")
}

/// Why a V8 failure happened. A terminated isolate reports an ordinary script
/// error, so the flags — not the message — say whether this was a deadline, a
/// heap cap, or genuine bad JavaScript.
fn classify(
    oom: &AtomicBool,
    terminated: &AtomicBool,
    e: &dyn std::fmt::Display,
) -> NodeEngineError {
    if oom.load(Ordering::SeqCst) {
        NodeEngineError::Oom
    } else if terminated.load(Ordering::SeqCst) {
        NodeEngineError::Timeout
    } else {
        NodeEngineError::eval_failed(e.to_string())
    }
}

async fn run_loop(
    mut rx: mpsc::UnboundedReceiver<Command>,
    opts: RuntimeOpts,
    engine: Arc<dyn Engine>,
    unregisters: Unregisters,
    command_tx: mpsc::WeakUnboundedSender<Command>,
) {
    let heap_bytes = opts.heap_mb * 1024 * 1024;
    // `heap_limits` bounds the object heap only; this bounds everything V8
    // allocates beside it. The handle is kept alive by the isolate.
    let (_external, external_allocator) = crate::allocator::capped(opts.external_mb * 1024 * 1024);

    // Bound BEFORE `js`, initialised after it. Locals drop in reverse
    // declaration order, so `js` disposes the isolate first and this `Box`
    // is freed second — the ordering V8 requires, and unlike a trailing
    // `drop(js); drop(heap_guard);` pair it still holds if the loop unwinds.
    //
    // `needless_late_init` is exactly wrong here, and its suggested fix is a
    // silent memory-safety regression: collapsing this into a single
    // `let heap_guard = Box::new(..)` below moves the binding AFTER `js`'s,
    // which flips the drop order and leaves V8 holding a freed pointer
    // through isolate disposal. Do not "clean this up".
    #[allow(clippy::needless_late_init)]
    let heap_guard: Box<HeapGuard>;

    // Before the isolate, because it has to go into the `OpsState` handed to
    // `node_engine_ext::init`. Never `unwrap()`, and never degrade silently to
    // "no filesystem": a runtime whose config promises a directory and quietly
    // has none is worse than one that refuses to start. Returning here drops
    // `rx`, so the manager's first send fails and it reaps — the same
    // observable outcome as the prelude-failure path below, with less code.
    let scratch = match make_scratch(&opts) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "scratch directory could not be created; runtime is unusable");
            return;
        }
    };

    let mut js = JsRuntime::new(RuntimeOptions {
        create_params: Some(
            v8::CreateParams::default()
                .heap_limits(0, heap_bytes)
                .array_buffer_allocator(external_allocator.make_shared()),
        ),
        extensions: vec![node_engine_ext::init(OpsState {
            engine: engine.clone(),
            namespace: opts.namespace.clone(),
            logs: Vec::new(),
            log_bytes: 0,
            log_truncated: false,
            detached_log_bytes: 0,
            capturing: false,
            registered: Vec::new(),
            unregisters: unregisters.clone(),
            pending_registrations: 0,
            call_timeout_ms: opts.call_timeout_ms,
            max_timeout_ms: opts.max_timeout_ms,
            inflight_calls: 0,
            command_tx,
            invoke_timeout_ms: opts.call_timeout_ms,
            ids: opts.ids.clone(),
            runtime_id: opts.runtime_id.clone(),
            last_activity: opts.last_activity.clone(),
            scratch,
            scratch_max_bytes: opts.scratch_mb as u64 * 1024 * 1024,
            scratch_max_files: opts.scratch_files,
        })],
        ..Default::default()
    });
    let op_state = js.op_state();

    let oom = Arc::new(AtomicBool::new(false));
    let terminated = Arc::new(AtomicBool::new(false));
    let isolate_handle = js.v8_isolate().thread_safe_handle();
    heap_guard = Box::new(HeapGuard {
        oom: oom.clone(),
        isolate: isolate_handle.clone(),
    });
    let heap_guard_ptr: *const HeapGuard = &*heap_guard;
    js.v8_isolate()
        .add_near_heap_limit_callback(near_heap_limit, heap_guard_ptr as *mut std::ffi::c_void);

    let deadline_slot: DeadlineSlot = Arc::new(std::sync::Mutex::new(None));
    let stop_watchdog = Arc::new(AtomicBool::new(false));
    let watchdog = spawn_watchdog(
        isolate_handle,
        deadline_slot.clone(),
        terminated.clone(),
        stop_watchdog.clone(),
    );

    // Ceiling on any single event-loop pump, including detached tenant work
    // running with nothing in `pending`. Reuses the configured per-call budget
    // — work outside a command that outlives a whole call timeout, with no
    // caller waiting on it, is the pathological case, not a legitimate one.
    let pump_budget = Duration::from_millis(opts.call_timeout_ms);

    let mut pending: Vec<Pending> = Vec::new();
    let mut closed = false;
    // Set once the isolate is unusable. V8 termination is sticky, so there is
    // no recovering a runtime past this point — it dies and callers are told.
    let mut fatal: Option<FatalExit> = None;

    if let Err(e) = js.execute_script("[node-engine:prelude]", include_str!("prelude.js")) {
        tracing::error!(error = %e, "prelude failed to evaluate; runtime is unusable");
        closed = true;
    }

    // The prefix `op_iii_register` will hold this runtime to, published to the
    // code that has to satisfy it. Without this, evaluated code that wants to
    // register has to be told its own namespace out of band or guess, and a
    // guess is a `namespace_denied` after the work is already done. Not a
    // security boundary — the op re-checks every id against the same value —
    // so it is read-only only to keep the isolate honest with itself.
    //
    // `include_str!("prelude.js")` above is a compile-time constant, so it
    // cannot carry a per-runtime value — this per-runtime script is why
    // `namespace` is published here rather than baked into the prelude's own
    // object literal. `Object.freeze` rides the same script, right after: it
    // has to wait for `namespace` to exist (freezing first would make this
    // `defineProperty` throw, adding a property to a non-extensible object),
    // and nothing runs between the prelude finishing and this script — both
    // are plain synchronous `execute_script` calls, with no tenant code
    // scheduled in between — so the object is never observably unfrozen from
    // the outside.
    //
    // `scratch_mb: 0` also drops `iii.files` here rather than in the prelude:
    // the prelude is a compile-time constant and cannot carry a per-runtime
    // value, and this script already runs in the one window where `iii` exists
    // but is not yet frozen.
    let drop_files = if opts.scratch_mb == 0 {
        "delete globalThis.iii.files;\n"
    } else {
        ""
    };
    let publish_ns = format!(
        r#"{}Object.defineProperty(globalThis.iii, "namespace", {{ value: {}, enumerable: true }});
Object.freeze(globalThis.iii);"#,
        drop_files,
        serde_json::to_string(&opts.namespace).expect("namespace serializes")
    );
    if let Err(e) = js.execute_script("[node-engine:namespace]", publish_ns) {
        tracing::error!(error = %e, "namespace publish failed; runtime is unusable");
        closed = true;
    }

    while fatal.is_none() && !(closed && pending.is_empty()) {
        // Drain what is already queued BEFORE the tick, so every in-flight
        // command's deadline is known to the `timeout_at` that wraps it.
        loop {
            match rx.try_recv() {
                Ok(cmd) => {
                    fatal = start(
                        &mut js,
                        &mut pending,
                        cmd,
                        &oom,
                        &terminated,
                        &deadline_slot,
                        &op_state,
                    );
                    if fatal.is_some() {
                        break;
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        if fatal.is_some() || (closed && pending.is_empty()) {
            break;
        }

        let next_deadline = pending.iter().map(|p| p.deadline).min();
        *deadline_slot.lock().unwrap() = next_deadline.map(|d| d.into_std());

        let mut started_this_tick = false;

        // One tick: accept commands, poll settled promises, pump the event
        // loop. Returns the indices that settled, newest-last.
        let tick = poll_fn(|cx| {
            while !closed {
                match rx.poll_recv(cx) {
                    Poll::Ready(Some(cmd)) => {
                        started_this_tick = true;
                        if let Some(err) = start(
                            &mut js,
                            &mut pending,
                            cmd,
                            &oom,
                            &terminated,
                            &deadline_slot,
                            &op_state,
                        ) {
                            return Poll::Ready(Err(err));
                        }
                    }
                    Poll::Ready(None) => closed = true,
                    Poll::Pending => break,
                }
            }

            let mut settled = Vec::new();
            for (i, p) in pending.iter_mut().enumerate() {
                if let Poll::Ready(res) = p.fut.as_mut().poll(cx) {
                    settled.push((i, res));
                }
            }

            // Drives ops and microtasks — which means TENANT JS RUNS INSIDE
            // THIS CALL, including detached work with nothing in `pending`:
            // `iii.trigger(...).catch(() => { for(;;){} })` settles its
            // eval immediately and wedges the thread later, when the rejection
            // handler runs. With `pending` empty the slot is `None`, so the
            // watchdog never fires, and `next_deadline` is `None`, so no
            // `timeout_at` wraps this tick — both mechanisms disarmed at once.
            // Arm the watchdog for the pump itself so no `poll_event_loop`
            // call can ever run unbounded.
            {
                let mut slot = deadline_slot.lock().unwrap();
                *slot = Some(match next_deadline {
                    // A pending command already bounds this pump with its own
                    // deadline. Do NOT also apply `pump_budget` here: it is
                    // `default_timeout_ms`, while an eval may legitimately
                    // request up to `max_timeout_ms`, so tightening it would
                    // kill a long-but-valid eval early — and only after its
                    // first `await`, since work before that runs under
                    // `execute_script`, which is asymmetric and surprising.
                    Some(d) => d.into_std(),
                    // Nothing pending: this is the detached-work window, and
                    // `pump_budget` is the only thing standing between a
                    // spinning microtask and a permanently wedged thread.
                    None => std::time::Instant::now() + pump_budget,
                });
            }
            let _ = js.poll_event_loop(cx, PollEventLoopOptions::default());
            *deadline_slot.lock().unwrap() = next_deadline.map(|d| d.into_std());

            // Settled results first: a promise that genuinely resolved in this
            // same poll must reach its caller even if the watchdog fired
            // during the pump. Checking `terminated` above this would discard
            // it and answer `Timeout` instead of the value it already had.
            if !settled.is_empty() {
                return Poll::Ready(Ok(settled));
            }

            // A terminated isolate cannot make progress. Surface it so the
            // outer loop's dead-flag check tears the runtime down; returning
            // `Pending` here would park forever when `next_deadline` is `None`.
            if terminated.load(Ordering::SeqCst) || oom.load(Ordering::SeqCst) {
                return Poll::Ready(Ok(Vec::new()));
            }
            // A command that arrived mid-tick is NOT covered by the
            // `timeout_at` wrapping this tick — that wrapper was built from the
            // deadlines known beforehand. Yield so the outer loop recomputes,
            // or a never-settling first command would wait forever.
            if started_this_tick || (closed && pending.is_empty()) {
                return Poll::Ready(Ok(Vec::new()));
            }
            Poll::Pending
        });

        // Two complementary deadline mechanisms, both required:
        //   - the watchdog thread interrupts V8 spinning on synchronous JS,
        //     when this thread cannot run timers at all;
        //   - this `timeout_at` catches a runtime parked on a promise that
        //     never settles, where this thread is free but V8 is idle and has
        //     nothing to terminate.
        let ticked = match next_deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, tick).await {
                Ok(t) => t,
                Err(_) => Err(FatalExit {
                    err: if oom.load(Ordering::SeqCst) {
                        NodeEngineError::Oom
                    } else {
                        NodeEngineError::Timeout
                    },
                    // Nobody to hand back — every affected caller is in
                    // `pending` and is answered during teardown.
                    reply: None,
                }),
            },
            None => tick.await,
        };

        match ticked {
            Err(err) => fatal = Some(err),
            // Settle back-to-front so indices stay valid as entries are removed.
            Ok(settled) => {
                // Blank the slot BEFORE answering anyone. A command that
                // finished at or just after its own deadline would otherwise
                // leave that expired deadline visible while we reply, and the
                // watchdog would kill the runtime out from under the next
                // command. The watchdog's single-lock check cannot help here
                // — the slot's contents are genuinely stale, not raced.
                *deadline_slot.lock().unwrap() = None;
                for (i, res) in settled.into_iter().rev() {
                    let p = pending.remove(i);
                    complete(&mut js, &oom, &terminated, p.reply, res, &op_state);
                }
                *deadline_slot.lock().unwrap() = pending
                    .iter()
                    .map(|p| p.deadline)
                    .min()
                    .map(|d| d.into_std());
            }
        }

        // Either terminator may have fired without this loop being the one to
        // observe it — a heap trip inside a microtask, or a watchdog kill that
        // `complete` classified and replied to. V8 termination is sticky, so
        // the isolate is unusable either way: tear down now rather than leave
        // a zombie holding a capacity slot and handing later callers spurious
        // timeouts. Both flags are set-once and only ever mean "isolate dead".
        if fatal.is_none() && (oom.load(Ordering::SeqCst) || terminated.load(Ordering::SeqCst)) {
            fatal = Some(FatalExit {
                err: if oom.load(Ordering::SeqCst) {
                    NodeEngineError::Oom
                } else {
                    NodeEngineError::Timeout
                },
                reply: None,
            });
        }
    }

    // Close BEFORE anyone is answered — including the caller whose own command
    // killed the isolate, which is why `start` handed its reply back rather
    // than sending it. Any caller that re-sends the instant it sees its error
    // must find a closed channel. Dropping each queued command resolves its
    // awaiting proxy as `runtime_gone`.
    if fatal.is_some() {
        rx.close();
        while rx.try_recv().is_ok() {}
    }

    stop_watchdog.store(true, Ordering::SeqCst);
    *deadline_slot.lock().unwrap() = None;
    let _ = watchdog.join();

    if let Some(FatalExit { err, reply }) = fatal {
        if let Some(reply) = reply {
            reply.fail(err.clone());
        }
        for p in pending.drain(..) {
            p.reply.fail(err.clone());
        }
    }

    // No explicit drops: `heap_guard` is bound before `js`, so scope exit
    // disposes the isolate first and frees the callback data second — on the
    // unwind path too. `heap_guard` is referenced here only to keep the
    // binding alive to this point.
    let _ = &heap_guard;
}

/// Start one command. Returns `Some(FatalExit)` when the isolate died running
/// it: the runtime must tear down, and the caller is answered by teardown
/// AFTER the channel is closed — never here.
fn start(
    js: &mut JsRuntime,
    pending: &mut Vec<Pending>,
    cmd: Command,
    oom: &AtomicBool,
    terminated: &AtomicBool,
    deadline_slot: &DeadlineSlot,
    op_state: &Rc<RefCell<OpState>>,
) -> Option<FatalExit> {
    let now = Instant::now();
    let is_eval = matches!(cmd, Command::Eval { .. });
    let (name, source, deadline, reply) = match cmd {
        Command::Eval {
            code,
            timeout,
            reply,
        } => {
            // Evals are serialised per runtime by the manager, so the buffer
            // and the registration delta belong to exactly one caller.
            with_ops_state(op_state, |s| {
                s.capturing = true;
                s.logs.clear();
                s.log_bytes = 0;
                s.log_truncated = false;
                s.detached_log_bytes = 0;
                s.registered.clear();
            });
            (
                "[node-engine:eval]",
                wrap_eval(&code),
                now + timeout,
                Reply::Eval(reply),
            )
        }
        Command::Invoke {
            fn_id,
            payload,
            timeout,
            reply,
            method,
        } => (
            "[node-engine:invoke]",
            wrap_invoke(&fn_id, &payload, method.as_deref()),
            now + timeout,
            Reply::Invoke(reply),
        ),
    };

    // Publish BEFORE running. A synchronous script never returns control here,
    // so the watchdog has to already know when to interrupt it.
    {
        let earliest = pending
            .iter()
            .map(|p| p.deadline)
            .chain(std::iter::once(deadline))
            .min();
        *deadline_slot.lock().unwrap() = earliest.map(|d| d.into_std());
    }

    match js.execute_script(name, source) {
        Ok(global) => {
            pending.push(Pending {
                fut: Box::pin(js.resolve(global)),
                reply,
                deadline,
            });
            None
        }
        Err(e) => {
            let logs = if is_eval {
                with_ops_state(op_state, |s| {
                    s.capturing = false;
                    std::mem::take(&mut s.logs)
                })
            } else {
                Vec::new()
            };
            match classify(oom, terminated, &e) {
                err @ NodeEngineError::EvalFailed { .. } => {
                    reply.fail(err.with_logs(logs));
                    None
                }
                err => Some(FatalExit {
                    err,
                    reply: Some(reply),
                }),
            }
        }
    }
}

fn complete(
    js: &mut JsRuntime,
    oom: &AtomicBool,
    terminated: &AtomicBool,
    reply: Reply,
    res: Result<v8::Global<v8::Value>, Box<JsError>>,
    op_state: &Rc<RefCell<OpState>>,
) {
    let is_eval = matches!(reply, Reply::Eval(_));

    // Every eval exit path closes capture, so a later handler invocation with
    // no eval in flight logs to tracing instead of a stale buffer.
    let finish_eval = |op_state: &Rc<RefCell<OpState>>| {
        with_ops_state(op_state, |s| {
            s.capturing = false;
            (
                std::mem::take(&mut s.logs),
                // `OpsState::registered` is kind-tagged so a cross-kind
                // unregister cannot drop the wrong entry (see ops.rs); the
                // kind is internal bookkeeping and never leaves the worker.
                std::mem::take(&mut s.registered)
                    .into_iter()
                    .map(|(_, id)| id)
                    .collect(),
            )
        })
    };

    let envelope = match res {
        Err(e) => {
            let logs = if is_eval {
                finish_eval(op_state).0
            } else {
                Vec::new()
            };
            reply.fail(classify(oom, terminated, &e).with_logs(logs));
            return;
        }
        Ok(global) => {
            let text = {
                deno_core::scope!(scope, js);
                let local = v8::Local::new(scope, global);
                local.to_rust_string_lossy(scope)
            };
            Envelope::parse(&text)
        }
    };

    match envelope {
        Err(e) => {
            let logs = if is_eval {
                finish_eval(op_state).0
            } else {
                Vec::new()
            };
            reply.fail(e.with_logs(logs));
        }
        Ok(Envelope::Err(message)) => {
            // The logs are the point: a tenant script that throws still
            // produced whatever it printed first, and a host reporting this
            // as a process-shaped response owes the caller that stdout.
            let logs = if is_eval {
                finish_eval(op_state).0
            } else {
                Vec::new()
            };
            reply.fail(NodeEngineError::EvalFailed { message, logs });
        }
        Ok(Envelope::Ok(value)) => match reply {
            Reply::Eval(tx) => {
                let (logs, registered) = finish_eval(op_state);
                let _ = tx.send(Ok(EvalOutcome {
                    result: value,
                    logs,
                    registered,
                }));
            }
            // Deliberately does not clear `s.registered`: an id registered by
            // a handler invoked outside an eval lingers there until the next
            // eval clears it. Harmless — nothing reads `s.registered` between
            // an INVOKE's own completion (this branch, which does not take
            // it) and the next EVAL's start (`start`, which clears it before
            // capturing begins), so a stale entry is invisible to any reader
            // before it is wiped and can never contaminate a later eval's
            // reported list. (`op_iii_register`'s dedup check used to read
            // this list too, which is what the "more conservative, not
            // wrong" framing here originally described — that check was
            // removed once the kind-tagged `unregisters` lookup made it both
            // redundant AND, for a cross-kind id collision, actively wrong.)
            Reply::Invoke(tx) => {
                let _ = tx.send(Ok(value));
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FakeEngine;
    use crate::engine::TriggerCallback;
    use serde_json::json;

    /// Distinct from `call_timeout_ms` (2_000) so a test can tell "clamped to
    /// the ceiling" apart from "fell back to the default".
    const TEST_MAX_TIMEOUT_MS: u64 = 10_000;

    /// Small enough that a quota test can fill it without writing megabytes,
    /// large enough that no ordinary test trips it by accident.
    const TEST_SCRATCH_MB: usize = 1;
    const TEST_SCRATCH_FILES: usize = 4;

    async fn eval(
        rt: &RuntimeThread,
        code: &str,
        timeout_ms: u64,
    ) -> Result<EvalOutcome, NodeEngineError> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        rt.send(Command::Eval {
            code: code.to_string(),
            timeout: Duration::from_millis(timeout_ms),
            reply,
        })
        .map_err(|_| "runtime thread alive")
        .expect("runtime thread alive");
        rx.await.expect("runtime thread replied")
    }

    fn spawn_rt() -> RuntimeThread {
        init_v8_platform();
        RuntimeThread::spawn(
            RuntimeOpts {
                heap_mb: 32,
                external_mb: 8,
                namespace: "test::".into(),
                call_timeout_ms: 2_000,
                max_timeout_ms: TEST_MAX_TIMEOUT_MS,
                ids: crate::ids::IdRegistry::default(),
                runtime_id: "rt-test".into(),
                last_activity: Arc::new(std::sync::Mutex::new(std::time::Instant::now())),
                scratch_mb: TEST_SCRATCH_MB,
                scratch_files: TEST_SCRATCH_FILES,
                scratch_root: None,
            },
            FakeEngine::new(),
        )
    }

    fn spawn_rt_with(engine: Arc<FakeEngine>) -> RuntimeThread {
        spawn_rt_with_namespace(engine, "test::")
    }

    /// Two runtimes on different namespaces — same shape as `spawn_rt_with`,
    /// parameterised, for tests that need to prove one runtime cannot touch
    /// another's ids.
    fn spawn_rt_with_namespace(engine: Arc<FakeEngine>, namespace: &str) -> RuntimeThread {
        spawn_rt_with_namespace_and_timeout(engine, namespace, 2_000)
    }

    fn spawn_rt_with_namespace_and_timeout(
        engine: Arc<FakeEngine>,
        namespace: &str,
        call_timeout_ms: u64,
    ) -> RuntimeThread {
        init_v8_platform();
        RuntimeThread::spawn(
            RuntimeOpts {
                heap_mb: 32,
                external_mb: 8,
                namespace: namespace.into(),
                call_timeout_ms,
                max_timeout_ms: TEST_MAX_TIMEOUT_MS,
                ids: crate::ids::IdRegistry::default(),
                runtime_id: format!("rt-test-{}", namespace.trim_end_matches(':')),
                last_activity: Arc::new(std::sync::Mutex::new(std::time::Instant::now())),
                scratch_mb: TEST_SCRATCH_MB,
                scratch_files: TEST_SCRATCH_FILES,
                scratch_root: None,
            },
            engine,
        )
    }

    /// The finding this guards: a derived `Debug` on `RuntimeOpts` would print
    /// `runtime_id` — the capability to eval into or tear down this runtime —
    /// the moment anything formats it with `{:?}`, e.g. a log line built from
    /// the options on an error path.
    #[test]
    fn runtime_opts_debug_does_not_leak_the_runtime_id() {
        let opts = RuntimeOpts {
            heap_mb: 32,
            external_mb: 8,
            namespace: "test::".into(),
            call_timeout_ms: 2_000,
            max_timeout_ms: TEST_MAX_TIMEOUT_MS,
            ids: crate::ids::IdRegistry::default(),
            runtime_id: "rt-secret-capability".into(),
            last_activity: Arc::new(std::sync::Mutex::new(std::time::Instant::now())),
            scratch_mb: TEST_SCRATCH_MB,
            scratch_files: TEST_SCRATCH_FILES,
            scratch_root: None,
        };
        let rendered = format!("{opts:?}");
        assert!(
            !rendered.contains("rt-secret-capability"),
            "leaked the runtime_id: {rendered}"
        );
        assert!(
            rendered.contains("test::"),
            "non-secret fields should still show: {rendered}"
        );
    }

    #[tokio::test]
    async fn returns_the_completion_value() {
        let rt = spawn_rt();
        let out = eval(&rt, "return 1 + 1", 2_000).await.unwrap();
        assert_eq!(out.result, json!(2));
        rt.shutdown();
    }

    #[tokio::test]
    async fn supports_top_level_await() {
        let rt = spawn_rt();
        let out = eval(&rt, "return await Promise.resolve({ a: 1 })", 2_000)
            .await
            .unwrap();
        assert_eq!(out.result, json!({ "a": 1 }));
        rt.shutdown();
    }

    #[tokio::test]
    async fn undefined_completion_is_null() {
        let rt = spawn_rt();
        let out = eval(&rt, "const x = 1;", 2_000).await.unwrap();
        assert_eq!(out.result, json!(null));
        rt.shutdown();
    }

    #[tokio::test]
    async fn globals_persist_across_evals_in_one_runtime() {
        let rt = spawn_rt();
        eval(&rt, "globalThis.counter = 41", 2_000).await.unwrap();
        let out = eval(&rt, "return ++globalThis.counter", 2_000)
            .await
            .unwrap();
        assert_eq!(out.result, json!(42));
        rt.shutdown();
    }

    #[tokio::test]
    async fn a_thrown_error_becomes_eval_failed_with_a_stack() {
        let rt = spawn_rt();
        let err = eval(&rt, "throw new Error('boom')", 2_000)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::eval_failed");
        assert!(err.to_string().contains("boom"));
        rt.shutdown();
    }

    #[tokio::test]
    async fn a_syntax_error_becomes_eval_failed() {
        let rt = spawn_rt();
        let err = eval(&rt, "this is not javascript", 2_000)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::eval_failed");
        rt.shutdown();
    }

    #[tokio::test]
    async fn a_busy_loop_is_killed_at_the_deadline() {
        let rt = spawn_rt();
        let err = eval(&rt, "for(;;){}", 300).await.unwrap_err();
        assert_eq!(err.code(), "node-engine::timeout");
        // The isolate is dead: the channel is closed, so a follow-up send
        // fails. This is the invariant that forces teardown to close `rx`
        // before answering the caller whose command did the killing.
        let (reply, _rx) = tokio::sync::oneshot::channel();
        let send = rt.send(Command::Eval {
            code: "return 1".into(),
            timeout: Duration::from_millis(500),
            reply,
        });
        assert!(
            send.is_err(),
            "sender must be closed after the runtime dies"
        );
    }

    /// Detached tenant work — a rejection handler on a promise the eval never
    /// awaited — runs inside `poll_event_loop` AFTER the eval has replied,
    /// when `pending` is empty and both deadline mechanisms would otherwise be
    /// disarmed. Without the pump window this wedges the isolate thread
    /// permanently, which in turn makes `teardown` and the idle sweeper block
    /// forever on `RuntimeThread::Drop`'s join.
    #[tokio::test]
    async fn detached_work_cannot_wedge_the_isolate_after_the_eval_replies() {
        let fake = FakeEngine::new();
        // The rejection MUST land in a later pump than the eval's own settle.
        // A synchronous fake resolves the op and runs its `.catch` inside the
        // same `poll_event_loop` call, while the eval is still pending and its
        // deadline still covers everything — the detached window would be
        // unreachable and this test would pass without proving anything.
        fake.delay_calls(Duration::from_millis(300));
        let rt = spawn_rt_with(fake);

        // Nothing is awaited, so the eval settles at once.
        let out = eval(
            &rt,
            "iii.trigger({ function_id: 'does::not::exist', payload: {} }).catch(() => { for(;;){} }); \
             return 'done'",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("done"));

        // `pending` is now empty and the slot is `None`. Wait for the delayed
        // rejection to arrive and start spinning inside the pump — with no
        // eval deadline and no `timeout_at`, only the pump window can stop it.
        tokio::time::sleep(Duration::from_millis(600)).await;

        // Without the pump window this join never returns. `shutdown` blocks,
        // so it has to run off the async runtime to be timeoutable at all.
        let joined = tokio::task::spawn_blocking(move || rt.shutdown());
        tokio::time::timeout(Duration::from_secs(20), joined)
            .await
            .expect("isolate thread never exited — detached JS wedged the pump")
            .expect("shutdown task joined");
    }

    #[tokio::test]
    async fn a_never_settling_promise_is_killed_at_the_deadline() {
        let rt = spawn_rt();
        let err = eval(&rt, "await new Promise(() => {})", 300)
            .await
            .unwrap_err();
        assert_eq!(err.code(), "node-engine::timeout");
    }

    /// The containment boundary. `Deno.core.ops.op_panic` is a literal
    /// `panic!` that would kill this worker thread and with it every other
    /// tenant's runtime, and `op_print` writes to the worker's stdout.
    /// Neither may be reachable from tenant code.
    #[tokio::test]
    async fn deno_core_globals_are_removed_from_the_isolate() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "return [typeof Deno, typeof __bootstrap, typeof globalThis.Deno]",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!(["undefined", "undefined", "undefined"]));
        rt.shutdown();
    }

    #[tokio::test]
    async fn op_panic_is_unreachable_from_tenant_code() {
        let rt = spawn_rt();
        // Reaching it would panic the isolate thread, not fail this eval.
        let out = eval(
            &rt,
            "try { Deno.core.ops.op_panic('pwned'); return 'REACHED' } \
             catch (e) { return 'blocked' }",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("blocked"));
        // The runtime is still alive and serving.
        assert_eq!(eval(&rt, "return 1", 2_000).await.unwrap().result, json!(1));
        rt.shutdown();
    }

    /// The tamper must be planted in one eval and observed in the NEXT.
    ///
    /// Within a single eval it is unobservable: `wrap_eval` emits
    /// `globalThis.__iii.settle(<async IIFE>)`, and JS resolves the callee
    /// before evaluating the argument, so code inside the IIFE cannot affect
    /// the call carrying it. Globals persist across evals, so the real
    /// exploit is cross-call. Two independent forms are planted here:
    /// overwriting the `settle` property, and rebinding `__iii` itself to a
    /// forged object — `Object.freeze` alone would only stop the former;
    /// `defineProperty`'s non-writable, non-configurable binding stops both.
    #[tokio::test]
    async fn a_forged_settle_cannot_survive_into_the_next_eval() {
        let rt = spawn_rt();
        eval(
            &rt,
            "try { globalThis.__iii.settle = () => '{\"ok\":\"forged\"}' } catch (e) {} \
             try { globalThis.__iii = { settle: () => '{\"ok\":\"forged\"}', invoke: () => {} } } \
             catch (e) {} \
             return 'planted'",
            2_000,
        )
        .await
        .unwrap();

        // On a writable __iii, or one whose binding can be replaced wholesale,
        // this eval's wrapper would resolve the forged `settle` and return
        // "forged", so this assertion genuinely fails against unfixed code.
        let after = eval(&rt, "return 'real'", 2_000).await.unwrap();
        assert_eq!(after.result, json!("real"));
        rt.shutdown();
    }

    #[tokio::test]
    async fn unbounded_allocation_is_killed_by_the_heap_cap() {
        let rt = spawn_rt();
        let err = eval(
            &rt,
            "const a = []; for(;;) { a.push(new Array(100000).fill('x')); }",
            30_000,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "node-engine::oom");
    }

    #[tokio::test]
    async fn console_lines_are_captured_in_order_with_levels() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "console.log('one'); console.warn('two'); console.error('three'); return 'done'",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("done"));
        assert_eq!(
            out.logs,
            vec![
                LogLine {
                    level: "log".into(),
                    message: "one".into()
                },
                LogLine {
                    level: "warn".into(),
                    message: "two".into()
                },
                LogLine {
                    level: "error".into(),
                    message: "three".into()
                },
            ]
        );
        rt.shutdown();
    }

    #[tokio::test]
    async fn console_formats_multiple_and_non_string_arguments() {
        let rt = spawn_rt();
        let out = eval(&rt, "console.log('n =', 42, { a: 1 }); return null", 2_000)
            .await
            .unwrap();
        assert_eq!(out.logs.len(), 1);
        assert_eq!(out.logs[0].message, r#"n = 42 {"a":1}"#);
        rt.shutdown();
    }

    /// The log buffer is Rust-side, outside V8's heap cap, so a flood through
    /// this sanctioned op must not become an unbounded process allocation.
    #[tokio::test]
    async fn a_console_flood_is_capped() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "for (let i = 0; i < 200000; i++) console.log('x'.repeat(64)); return 'done'",
            20_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("done"));
        // 1000 lines plus at most one truncation marker.
        assert!(out.logs.len() <= 1_001, "buffer grew to {}", out.logs.len());
        let last = out.logs.last().expect("some logs captured");
        assert!(
            last.message.contains("truncated"),
            "expected a truncation marker, got {:?}",
            last
        );
        rt.shutdown();
    }

    /// `a_console_flood_is_capped` uses 64-byte lines, so it trips
    /// `MAX_LOG_LINES` (1000 * 64B = 64,000B, well under the 256KiB byte cap)
    /// and never exercises `MAX_LOG_BYTES` at all. Fewer, larger messages —
    /// 256KiB / 2048B ~= 128 lines — trip the byte cap on its own, proving
    /// the byte accounting is real and not dead code shadowed by the line
    /// check.
    #[tokio::test]
    async fn a_console_flood_of_large_messages_is_capped_by_bytes() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "for (let i = 0; i < 500; i++) console.log('x'.repeat(2048)); return 'done'",
            5_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("done"));
        // Far short of the 1000-line cap: only the byte cap could have
        // stopped this.
        assert!(
            out.logs.len() > 100 && out.logs.len() <= 130,
            "expected ~128 lines plus a truncation marker, got {}",
            out.logs.len()
        );
        let last = out.logs.last().expect("some logs captured");
        assert!(
            last.message.contains("truncated"),
            "expected a truncation marker, got {:?}",
            last
        );
        rt.shutdown();
    }

    #[tokio::test]
    async fn logs_do_not_leak_between_evals() {
        let rt = spawn_rt();
        eval(&rt, "console.log('first'); return null", 2_000)
            .await
            .unwrap();
        let out = eval(&rt, "return null", 2_000).await.unwrap();
        assert!(out.logs.is_empty(), "second eval saw {:?}", out.logs);
        rt.shutdown();
    }

    #[tokio::test]
    async fn trigger_takes_the_sdk_request_object() {
        let fake = FakeEngine::new();
        fake.with_response("test::double", Ok(json!(42)));
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "return await iii.trigger({ function_id: 'test::double', payload: { n: 21 } })",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!(42));
        assert_eq!(
            fake.calls(),
            vec![("test::double".to_string(), json!({ "n": 21 }))]
        );
        rt.shutdown();
    }

    #[tokio::test]
    async fn call_function_no_longer_exists() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let out = eval(&rt, "return typeof iii.callFunction", 2_000)
            .await
            .unwrap();
        assert_eq!(out.result, json!("undefined"));
        rt.shutdown();
    }

    #[tokio::test]
    async fn trigger_rejects_a_request_without_a_function_id() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let err = eval(&rt, "return await iii.trigger({ payload: {} })", 2_000)
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("function_id must be a non-empty string"),
            "unhelpful message: {err}"
        );
        rt.shutdown();
    }

    /// `trigger`'s other three validation branches — only `function_id`'s
    /// had coverage. Each case's code is a separate eval so one failure
    /// doesn't hide the rest; each asserts the message names what was wrong.
    #[tokio::test]
    async fn trigger_rejects_a_malformed_request_on_every_field() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());

        let not_an_object = eval(&rt, "return await iii.trigger('nope')", 2_000)
            .await
            .unwrap_err();
        assert!(
            not_an_object
                .to_string()
                .contains("request must be an object"),
            "unhelpful message: {not_an_object}"
        );

        let bad_action = eval(
            &rt,
            "return await iii.trigger({ function_id: 'x::y', action: 3 })",
            2_000,
        )
        .await
        .unwrap_err();
        assert!(
            bad_action.to_string().contains("action must be a string"),
            "unhelpful message: {bad_action}"
        );

        let bad_timeout = eval(
            &rt,
            "return await iii.trigger({ function_id: 'x::y', timeout: '500' })",
            2_000,
        )
        .await
        .unwrap_err();
        assert!(
            bad_timeout.to_string().contains("timeout must be a number"),
            "unhelpful message: {bad_timeout}"
        );

        rt.shutdown();
    }

    #[tokio::test]
    async fn trigger_resolves_with_the_engine_result() {
        let fake = FakeEngine::new();
        fake.with_response("state::get", Ok(json!({ "value": 7 })));
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "const r = await iii.trigger({ function_id: 'state::get', payload: { key: 'k' } }); \
             return r.value",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!(7));
        assert_eq!(
            fake.calls(),
            vec![("state::get".to_string(), json!({ "key": "k" }))]
        );
        rt.shutdown();
    }

    #[tokio::test]
    async fn trigger_rejects_with_the_engine_error() {
        let fake = FakeEngine::new();
        fake.with_response("thing::fail", Err("upstream exploded".into()));
        let rt = spawn_rt_with(fake);
        let out = eval(
            &rt,
            "try { await iii.trigger({ function_id: 'thing::fail', payload: {} }); \
               return 'no throw' } \
             catch (e) { return e.message }",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("upstream exploded"));
        rt.shutdown();
    }

    /// `FakeEngine::call` ignores `action` (see its `calls()`, which only
    /// records `fn_id`/`payload`), so this only proves an `action` field on
    /// the request object reaches `op_iii_call` and `Engine::call` without
    /// tripping any of `trigger`'s own validation. The string -> `TriggerAction`
    /// decode `IIIEngine` does with that value (`parse_trigger_action`) is
    /// covered directly in `src/engine.rs`'s test module — it had zero
    /// coverage before this task, since nothing called `Engine::call` with
    /// `Some(_)` until `trigger` did.
    #[tokio::test]
    async fn trigger_forwards_the_action_string_to_the_engine() {
        let fake = FakeEngine::new();
        fake.with_response("app::react", Ok(json!(null)));
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "return await iii.trigger({ function_id: 'app::react', payload: {}, action: 'void' })",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(fake.calls(), vec![("app::react".to_string(), json!({}))]);
        rt.shutdown();
    }

    /// `trigger`'s payload crosses into Rust as JSON text built from the
    /// pre-captured `stringify`, never the global `JSON.stringify` — see the
    /// capture block near the top of `prelude.js`. Overwriting the global
    /// once turned a thrown error into a reported success elsewhere in this
    /// codebase; this proves `trigger` does not repeat that mistake. Unlike
    /// the `settle`-forgery test, one eval is enough here: `stringify` is a
    /// closure variable resolved once when the prelude loads, not a property
    /// re-read at call time, so even tampering in the SAME eval that calls
    /// `trigger` cannot reach it.
    #[tokio::test]
    async fn trigger_payload_survives_a_hijacked_json_stringify() {
        let fake = FakeEngine::new();
        fake.with_response("test::echo", Ok(json!("ok")));
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "JSON.stringify = () => { throw new Error('hijacked') }; \
             return await iii.trigger({ function_id: 'test::echo', payload: { n: 1 } })",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("ok"));
        assert_eq!(
            fake.calls(),
            vec![("test::echo".to_string(), json!({ "n": 1 }))]
        );
        rt.shutdown();
    }

    /// Everything `op_iii_call` holds lives on the Rust heap, outside V8's
    /// cap, so both bounds have to hold or a tenant routes around its own
    /// isolate's memory limit through a sanctioned op.
    #[tokio::test]
    async fn an_oversized_payload_is_rejected() {
        let fake = FakeEngine::new();
        fake.with_response("thing::ping", Ok(json!("pong")));
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "try { await iii.trigger({ function_id: 'thing::ping', \
                 payload: { blob: 'x'.repeat(2 * 1024 * 1024) } }); \
               return 'accepted' } \
             catch (e) { return e.message }",
            10_000,
        )
        .await
        .unwrap();
        assert!(
            out.result.as_str().unwrap().contains("limit"),
            "expected a size-limit rejection, got {:?}",
            out.result
        );
        assert!(
            fake.calls().is_empty(),
            "oversized payload reached the engine"
        );
        rt.shutdown();
    }

    /// `action`'s same hazard as `payload_json`: `#[string]` args are
    /// byte-copied to the Rust heap at argument-dispatch time, before
    /// `op_iii_call`'s body — and so before `MAX_CALL_PAYLOAD_BYTES` — ever
    /// runs. Without its own cap this string is uncapped Rust-heap pinning
    /// that `MAX_INFLIGHT_CALLS` does nothing to bound.
    #[tokio::test]
    async fn an_oversized_action_is_rejected() {
        let fake = FakeEngine::new();
        fake.with_response("thing::ping", Ok(json!("pong")));
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "try { await iii.trigger({ function_id: 'thing::ping', payload: {}, \
                 action: 'x'.repeat(2000) }); \
               return 'accepted' } \
             catch (e) { return e.message }",
            10_000,
        )
        .await
        .unwrap();
        assert!(
            out.result.as_str().unwrap().contains("limit"),
            "expected a size-limit rejection, got {:?}",
            out.result
        );
        assert!(
            fake.calls().is_empty(),
            "oversized action reached the engine"
        );
        rt.shutdown();
    }

    /// `function_id` is the third `#[string]` of the same shape, and was the
    /// one left uncapped: measured at one eval firing 32 un-awaited calls
    /// with an 8 MiB id, a single runtime went from 36 MiB to 316 MiB RSS
    /// with 0 of the 32 refused — deno_core byte-copies the argument before
    /// `op_iii_call`'s body runs, and it stays pinned across `engine.call`'s
    /// await. At `heap_mb: 128` that is roughly 4 GiB from one runtime,
    /// times `max_runtimes`.
    #[tokio::test]
    async fn an_oversized_function_id_is_rejected() {
        let fake = FakeEngine::new();
        fake.with_response("thing::ping", Ok(json!("pong")));
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "try { await iii.trigger({ function_id: 'thing::' + 'x'.repeat(1024), \
                 payload: {} }); \
               return 'accepted' } \
             catch (e) { return e.message }",
            10_000,
        )
        .await
        .unwrap();
        assert!(
            out.result.as_str().unwrap().contains("limit"),
            "expected a size-limit rejection, got {:?}",
            out.result
        );
        assert!(
            fake.calls().is_empty(),
            "oversized function id reached the engine"
        );
        rt.shutdown();
    }

    /// A guest-supplied `timeout` is a CEILING clamp, not passed through
    /// verbatim: `iii.trigger({..., timeout: Number.MAX_SAFE_INTEGER})`
    /// reaching `Engine::call` unclamped would, on the real `IIIEngine`,
    /// become a `tokio::time::timeout` that effectively never expires,
    /// holding an inflight slot and an entry in the SDK's process-wide
    /// pending-invocation map far past the operator's configured
    /// `max_timeout_ms`. A SHORT requested timeout must still pass through
    /// unchanged — this is a ceiling, not a fixed override.
    #[tokio::test]
    async fn triggers_timeout_is_clamped_to_the_configured_ceiling() {
        let fake = FakeEngine::new();
        fake.with_response("thing::ping", Ok(json!("pong")));
        let rt = spawn_rt_with(fake.clone());

        eval(
            &rt,
            "return await iii.trigger({ function_id: 'thing::ping', payload: {}, timeout: 500 })",
            2_000,
        )
        .await
        .unwrap();
        eval(
            &rt,
            "return await iii.trigger({ function_id: 'thing::ping', payload: {}, \
                 timeout: Number.MAX_SAFE_INTEGER })",
            2_000,
        )
        .await
        .unwrap();

        assert_eq!(fake.call_timeouts(), vec![500, TEST_MAX_TIMEOUT_MS]);
        rt.shutdown();
    }

    #[tokio::test]
    async fn concurrent_calls_are_capped() {
        let fake = FakeEngine::new();
        // A real engine call stays pending; the default fake resolves at once,
        // which would drain the counter before the next call started.
        fake.hang_calls();
        let rt = spawn_rt_with(fake);
        let out = eval(
            &rt,
            // The 32 calls that win a slot hang forever, so the rejections
            // cannot be awaited directly. Drain several microtask rounds
            // instead: an op rejection takes a few hops to reach `.catch`
            // (op promise -> trigger's async fn -> handler), and one
            // round lands short. Rounds are deterministic, not timing-based,
            // so this is a fixed count with margin, not a sleep.
            "let rejected = 0; \
             for (let i = 0; i < 200; i++) { \
               iii.trigger({ function_id: 'x::y', payload: {} }).catch(() => { rejected++ }); \
             } \
             for (let i = 0; i < 10; i++) await Promise.resolve(); \
             return rejected",
            5_000,
        )
        .await
        .unwrap();
        // 200 fired, at most 32 may be in flight, so the rest must be refused.
        assert!(
            out.result.as_u64().unwrap() >= 168,
            "expected the excess to be rejected, got {:?}",
            out.result
        );
        // Deliberately no shutdown(): the isolate still holds hung calls, and
        // dropping it is exactly how a real torn-down runtime sheds them.
    }

    /// `concurrent_calls_are_capped` only proves the cap trips; it does not
    /// prove `release_inflight` ever runs. If the counter only ever grew, the
    /// cap would degrade into a one-shot budget: the 33rd call ever made on a
    /// long-lived runtime would fail forever after, not just while 32 calls
    /// are genuinely outstanding. 40 sequential AWAITED calls — each one must
    /// fully settle (and release its slot) before the next starts, since nothing
    /// here is concurrent — crosses the 32-call cap entirely, so this fails if
    /// the slot is never actually freed.
    #[tokio::test]
    async fn the_inflight_slot_is_released_after_each_call_so_later_calls_still_succeed() {
        let fake = FakeEngine::new();
        fake.with_response("thing::ping", Ok(json!("pong")));
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "for (let i = 0; i < 40; i++) { \
               await iii.trigger({ function_id: 'thing::ping', payload: {} }); \
             } \
             return 'done'",
            5_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("done"));
        assert_eq!(fake.calls().len(), 40);
        rt.shutdown();
    }

    /// A payload `JSON.stringify` cannot represent (a function) becomes
    /// `undefined`, which the op receives as `""` and fails to parse. That
    /// path has its own `release_inflight` call, so a leak there would slowly
    /// exhaust the 32-slot cap and wedge the runtime.
    #[tokio::test]
    async fn an_unserializable_payload_rejects_without_leaking_a_slot() {
        let fake = FakeEngine::new();
        fake.with_response("thing::ping", Ok(json!("pong")));
        let rt = spawn_rt_with(fake.clone());
        // NOTE: no `//` comments inside this JS string. Rust's `\`-newline
        // continuation strips the newline, so the whole literal is ONE line
        // and a `//` would comment out everything after it — including the
        // `return`. Explanations go in Rust comments, out here.
        //
        // The trailing call is the actual assertion: if the rejected calls
        // leaked their slots, 100 of them would have exhausted the 32-slot
        // cap and this one would fail too.
        let out = eval(
            &rt,
            "let refused = 0; \
             for (let i = 0; i < 100; i++) { \
               try { await iii.trigger({ function_id: 'thing::ping', payload: () => {} }) } \
               catch (e) { refused++ } \
             } \
             return [refused, await iii.trigger({ function_id: 'thing::ping', payload: {} })]",
            10_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!([100, "pong"]));
        rt.shutdown();
    }

    #[tokio::test]
    async fn trigger_defaults_a_missing_payload_to_an_empty_object() {
        let fake = FakeEngine::new();
        fake.with_response("thing::ping", Ok(json!("pong")));
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "return await iii.trigger({ function_id: 'thing::ping' })",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("pong"));
        assert_eq!(fake.calls(), vec![("thing::ping".to_string(), json!({}))]);
        rt.shutdown();
    }

    #[tokio::test]
    async fn register_then_invoke_round_trips_through_the_engine() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "iii.registerFunction('test::double', async (p) => ({ n: p.n * 2 })); return 'ok'",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.registered, vec!["test::double".to_string()]);
        assert_eq!(fake.registered_ids(), vec!["test::double".to_string()]);
        assert_eq!(
            fake.invoke("test::double", json!({ "n": 21 })).await,
            Ok(json!({ "n": 42 }))
        );
        rt.shutdown();
    }

    #[tokio::test]
    async fn a_synchronous_handler_works_too() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "iii.registerFunction('test::inc', (p) => p.n + 1); return null",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(
            fake.invoke("test::inc", json!({ "n": 1 })).await,
            Ok(json!(2))
        );
        rt.shutdown();
    }

    #[tokio::test]
    async fn a_throwing_handler_surfaces_its_message() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "iii.registerFunction('test::boom', () => { throw new Error('nope') }); return null",
            2_000,
        )
        .await
        .unwrap();
        let err = fake.invoke("test::boom", json!({})).await.unwrap_err();
        assert!(err.contains("nope"), "got {err}");
        rt.shutdown();
    }

    #[tokio::test]
    async fn ids_outside_the_namespace_are_rejected() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "try { iii.registerFunction('state::get', () => 1); return 'no throw' } \
             catch (e) { return e.message }",
            2_000,
        )
        .await
        .unwrap();
        assert!(
            out.result.as_str().unwrap().contains("test::"),
            "message should name the namespace, got {:?}",
            out.result
        );
        assert!(fake.registered_ids().is_empty());
        rt.shutdown();
    }

    /// Pins the SDK shape: `(functionId, handler, options?)` returning a ref
    /// whose `unregister` is a function.
    #[tokio::test]
    async fn register_function_takes_an_options_object_and_returns_a_ref() {
        let fake = FakeEngine::new();
        let out = eval(
            &spawn_rt_with(fake.clone()),
            "const ref = iii.registerFunction('test::x', () => 1, { description: 'd' });
             return typeof ref.unregister",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("function"));
    }

    #[tokio::test]
    async fn unregister_removes_the_function_from_the_engine() {
        let fake = FakeEngine::new();
        eval(
            &spawn_rt_with(fake.clone()),
            "const ref = iii.registerFunction('test::x', () => 1);
             ref.unregister();
             return 1",
            2_000,
        )
        .await
        .unwrap();
        assert!(
            !fake.registered_ids().contains(&"test::x".to_string()),
            "unregister() left the function published"
        );
    }

    /// `unregister()` also drops the prelude's own `handlers` entry — not
    /// just the engine-side registration — or an id the bus no longer routes
    /// would still answer an INVOKE dispatched straight to this isolate.
    #[tokio::test]
    async fn unregister_removes_the_handlers_map_entry_too() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "const ref = iii.registerFunction('test::x', () => 1);
             ref.unregister();
             return 1",
            2_000,
        )
        .await
        .unwrap();
        let out = eval(&rt, "return __iii.invoke('test::x', '{}')", 2_000)
            .await
            .unwrap_err();
        assert!(
            out.to_string().contains("no handler registered"),
            "got {out}"
        );
        rt.shutdown();
    }

    /// `out.registered` is the eval's reported delta — `RunResponse.registered`
    /// downstream, often the only record a one-shot `run` caller ever gets of
    /// what its eval published. Unregistering inside the same eval that
    /// registered must retract the id from that report too, or a caller is
    /// told it published something that is not live.
    #[tokio::test]
    async fn unregister_in_the_same_eval_retracts_it_from_the_registered_report() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());

        // Control: a plain registration must still be reported — proves the
        // fix below did not simply stop reporting registrations at all.
        let kept = eval(
            &rt,
            "iii.registerFunction('test::kept', () => 1); return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(kept.registered, vec!["test::kept".to_string()]);

        let unregistered = eval(
            &rt,
            "const ref = iii.registerFunction('test::x', () => 1);
             ref.unregister();
             return 1",
            2_000,
        )
        .await
        .unwrap();
        assert!(
            unregistered.registered.is_empty(),
            "unregister() in the same eval should retract the id from the \
             response, got {:?}",
            unregistered.registered
        );
        rt.shutdown();
    }

    /// The SDK also accepts an `HttpInvocationConfig` object in the handler
    /// position — an engine-side HTTP binding with no isolate involved.
    /// node-engine cannot publish that; it must refuse, not silently accept
    /// or coerce, and the refusal must name the real alternative.
    #[tokio::test]
    async fn a_non_function_handler_is_refused_naming_the_alternative() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "try { iii.registerFunction('test::http', { url: 'https://example.com' }); \
               return 'no throw' } \
             catch (e) { return e.message }",
            2_000,
        )
        .await
        .unwrap();
        let msg = out.result.as_str().unwrap();
        assert!(
            msg.contains("HttpInvocationConfig") && msg.contains("engine::functions::register"),
            "should name the alternative: {msg}"
        );
        assert!(fake.registered_ids().is_empty());
        rt.shutdown();
    }

    /// End-to-end guest-surface proof that one runtime cannot touch another's
    /// namespace at all — this is `op_iii_register`'s existing check, not
    /// `op_iii_unregister`'s new one: the prelude's `registerFunction` only
    /// ever hands `unregister()` an id that already passed ITS OWN
    /// namespace check, so there is no JS-reachable way to hand a foreign id
    /// to `op_iii_unregister` for this test to exercise. `op_iii_unregister`'s
    /// own check is proven directly (with a hand-built `OpsState`, bypassing
    /// that gate on purpose) by `ops::tests::a_runtime_cannot_unregister_another_runtimes_function`.
    #[tokio::test]
    async fn a_runtime_cannot_register_over_another_runtimes_namespace() {
        let fake = FakeEngine::new();
        let victim = spawn_rt_with_namespace(fake.clone(), "victim::");
        eval(
            &victim,
            "iii.registerFunction('victim::secret', () => 1); return 1",
            2_000,
        )
        .await
        .unwrap();

        let attacker = spawn_rt_with_namespace(fake.clone(), "attacker::");
        let err = eval(
            &attacker,
            "iii.registerFunction('victim::secret', () => 2); return 1",
            2_000,
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("victim::secret"),
            "expected a namespace refusal naming the id: {err}"
        );
        assert!(
            fake.registered_ids()
                .contains(&"victim::secret".to_string()),
            "the victim's function was removed by another runtime"
        );
        victim.shutdown();
        attacker.shutdown();
    }

    /// Registrations pin Rust-heap memory for the runtime's life AND write to
    /// the trusted bus, so the count has to be bounded like every other
    /// tenant-reachable allocation path.
    #[tokio::test]
    async fn registrations_are_capped_per_runtime() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "let ok = 0, refused = 0; \
             for (let i = 0; i < 400; i++) { \
               try { iii.registerFunction('test::f' + i, () => i); ok++ } \
               catch (e) { refused++ } \
             } \
             return [ok, refused]",
            20_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!([256, 144]));
        assert_eq!(
            fake.registered_ids().len(),
            256,
            "the bus saw uncapped writes"
        );
        rt.shutdown();
    }

    #[tokio::test]
    async fn an_overlong_function_id_is_rejected() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "try { iii.registerFunction('test::' + 'x'.repeat(1024), () => 1); return 'accepted' } \
             catch (e) { return 'rejected' }",
            5_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!("rejected"));
        assert!(fake.registered_ids().is_empty());
        rt.shutdown();
    }

    #[tokio::test]
    async fn re_registering_an_id_replaces_the_handler() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "iii.registerFunction('test::v', () => 1); return null",
            2_000,
        )
        .await
        .unwrap();
        eval(
            &rt,
            "iii.registerFunction('test::v', () => 2); return null",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(fake.invoke("test::v", json!({})).await, Ok(json!(2)));
        rt.shutdown();
    }

    /// The reason the loop is multiplexed: a handler awaiting another handler
    /// in the same isolate must interleave, not deadlock.
    #[tokio::test]
    async fn a_handler_can_await_another_handler_in_the_same_runtime() {
        let fake = FakeEngine::new();
        // `iii.trigger` inside the isolate must come back in as a real
        // dispatch, the way the engine would route it.
        fake.route_calls_to_registrations();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "iii.registerFunction('test::inner', (p) => p.n * 10); \
             iii.registerFunction('test::outer', async (p) => \
               await iii.trigger({ function_id: 'test::inner', payload: { n: p.n } })); \
             return null",
            2_000,
        )
        .await
        .unwrap();

        let engine = fake.clone();
        let proxied =
            tokio::spawn(async move { engine.invoke("test::outer", json!({ "n": 4 })).await });

        let result = tokio::time::timeout(Duration::from_secs(5), proxied)
            .await
            .expect("outer/inner deadlocked")
            .expect("task joined");
        assert_eq!(result, Ok(json!(40)));
        rt.shutdown();
    }

    /// Console output from a handler invoked between evals belongs to no
    /// caller's response; it must not surface in the next eval's logs.
    #[tokio::test]
    async fn handler_logs_outside_an_eval_do_not_leak_into_the_next_eval() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "iii.registerFunction('test::noisy', () => { console.log('from handler'); return 1 }); \
             return null",
            2_000,
        )
        .await
        .unwrap();

        assert_eq!(fake.invoke("test::noisy", json!({})).await, Ok(json!(1)));

        let out = eval(&rt, "return null", 2_000).await.unwrap();
        assert!(out.logs.is_empty(), "handler log leaked: {:?}", out.logs);
        rt.shutdown();
    }

    /// Returns `"<typeof fn>:<form>"`, or `"ERR: …"` when `toHandler` throws.
    /// Reaching through `.fn` on purpose: the return became `{ fn, form }` so
    /// `eject` can emit the shape the caller actually got.
    async fn to_handler(rt: &RuntimeThread, src: &str) -> serde_json::Value {
        let code = format!(
            "try {{ const h = __iii.toHandler('t::x', {}); return typeof h.fn + ':' + h.form }} \
             catch (e) {{ return 'ERR: ' + e.message }}",
            serde_json::to_string(src).unwrap()
        );
        eval(rt, &code, 2_000).await.unwrap().result
    }

    #[tokio::test]
    async fn to_handler_accepts_expression_forms() {
        let rt = spawn_rt();
        for src in [
            "(p) => p.n * 2",
            "async (p) => p.n",
            "function (p) { return p.n * 2 }",
        ] {
            assert_eq!(
                to_handler(&rt, src).await,
                json!("function:expression"),
                "src: {src}"
            );
        }
        assert_eq!(
            to_handler(&rt, "return payload.n + 1").await,
            json!("function:body"),
            "a function body resolves to the body form"
        );
        rt.shutdown();
    }

    #[tokio::test]
    async fn to_handler_falls_back_to_a_body_only_on_a_syntax_error() {
        let rt = spawn_rt();
        for src in ["return p.n * 2", "const x = 1; return x"] {
            assert_eq!(
                to_handler(&rt, src).await,
                json!("function:body"),
                "src: {src}"
            );
        }
        rt.shutdown();
    }

    /// The whole point of gating the fallback on `SyntaxError`. An
    /// unconditional fallback would turn each of these into a live function
    /// that returns null forever, which is indistinguishable on the wire from
    /// a deliberate null.
    #[tokio::test]
    async fn to_handler_rejects_expressions_that_are_not_functions() {
        let rt = spawn_rt();
        for src in ["42", "{ a: 1 }", "null", "payload.n * 2"] {
            let out = to_handler(&rt, src).await;
            let msg = out.as_str().unwrap();
            assert!(msg.starts_with("ERR: "), "src {src:?} produced {out:?}");
            assert!(msg.contains("t::x"), "message should name the id: {msg}");
        }
        rt.shutdown();
    }

    /// A handler is free to `throw null`. Reading `.message` off that would
    /// raise an unrelated TypeError and lose the id, so every path formats
    /// through `formatError`.
    #[tokio::test]
    async fn to_handler_survives_a_handler_that_throws_a_non_error() {
        let rt = spawn_rt();
        for src in ["(() => { throw null })()", "(() => { throw 7 })()"] {
            let out = to_handler(&rt, src).await;
            let msg = out.as_str().unwrap();
            assert!(msg.starts_with("ERR: "), "src {src:?} produced {out:?}");
            assert!(msg.contains("t::x"), "id lost for {src:?}: {msg}");
            assert!(
                !msg.contains("Cannot read properties"),
                "leaked an unrelated TypeError for {src:?}: {msg}"
            );
        }
        rt.shutdown();
    }

    /// `toHandler`'s own not-a-function error must not be caught and re-wrapped
    /// by `toHandler` itself.
    #[tokio::test]
    async fn to_handler_does_not_double_wrap_its_own_error() {
        let rt = spawn_rt();
        let out = to_handler(&rt, "42").await;
        let msg = out.as_str().unwrap();
        assert_eq!(
            msg.matches("handler for").count(),
            1,
            "message wrapped twice: {msg}"
        );
        rt.shutdown();
    }

    #[tokio::test]
    async fn to_handler_rejects_text_that_parses_as_neither() {
        let rt = spawn_rt();
        let out = to_handler(&rt, "@@@").await;
        let msg = out.as_str().unwrap();
        assert!(msg.starts_with("ERR: "), "got {out:?}");
        // Without this the test passes before `toHandler` exists: the helper's
        // catch turns "__iii.toHandler is not a function" into an "ERR: …"
        // string, so it could never observe its own regression.
        assert!(msg.contains("t::x"), "message should name the id: {msg}");
        rt.shutdown();
    }

    /// `iii.shutdown()` exists as a real property, not a missing one — the
    /// SDK's `shutdown()` closes the client's connection, but a node-engine
    /// runtime does not own one; the worker does, shared by every tenant.
    /// Throwing here, by name, is what tells guest code the real way to
    /// dispose a runtime instead of "undefined is not a function".
    #[tokio::test]
    async fn shutdown_throws_a_message_that_names_the_alternative() {
        let fake = FakeEngine::new();
        let out = eval(
            &spawn_rt_with(fake.clone()),
            "try { iii.shutdown(); return 'no throw' } catch (e) { return e.message }",
            2_000,
        )
        .await
        .unwrap();
        let msg = out.result.as_str().unwrap();
        assert!(msg.contains("node-engine::teardown"), "unhelpful: {msg}");
        assert!(msg.contains("shared by every runtime"), "unhelpful: {msg}");
    }

    /// An agent's first move against an unknown global is printing it — an
    /// opaque `{}` here previously cost a live session (in the sibling
    /// sandbox-code-runner worker) six blind runs before anyone worked out
    /// what the global even was.
    #[tokio::test]
    async fn the_iii_global_prints_a_usage_hint_rather_than_an_opaque_object() {
        let fake = FakeEngine::new();
        let out = eval(&spawn_rt_with(fake.clone()), "return String(iii)", 2_000)
            .await
            .unwrap();
        let hint = out.result.as_str().unwrap();
        assert!(
            hint.contains("iii.trigger"),
            "hint does not name trigger: {hint}"
        );
        assert!(
            hint.contains("registerFunction"),
            "hint does not name registerFunction: {hint}"
        );
    }

    /// `settle`'s return value IS the eval's result as Rust reads it, so a
    /// writable `iii` method would be a way for tenant code to forge this
    /// runtime's results — the same reason `__iii` is frozen and
    /// non-configurable. A no-op assignment under a silently-swallowed catch
    /// (non-strict `iii.trigger = ...` on a frozen object) must leave the
    /// original `trigger` untouched.
    #[tokio::test]
    async fn the_iii_global_is_frozen() {
        let fake = FakeEngine::new();
        let out = eval(
            &spawn_rt_with(fake.clone()),
            "try { iii.trigger = () => 'forged'; } catch (_) {}
             return typeof iii.trigger === 'function' && iii.trigger.name === 'trigger'",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, serde_json::json!(true));
    }

    /// Pins the guest-visible API. The shim is hand-written against the SDK's
    /// real signatures, so drift is invisible without a golden — this test is
    /// what says "iii-sdk changed and we did not".
    ///
    /// Renders ARITY (`Function.length`) alongside the name and type, because
    /// name-and-`typeof` alone is not a signature: `registerFunction` could
    /// silently drop to one declared argument and still match a golden that
    /// only says `function`. Arity is as much of the argument shape as JS
    /// exposes reflectively — it counts parameters before the first default
    /// or rest, which is exactly the SDK-shape drift worth catching.
    ///
    /// Read at RUNTIME rather than `include_str!` so `UPDATE_GOLDENS=1
    /// cargo test` regenerates this file the same way every other golden in
    /// this crate is regenerated (see `tests/support/mod.rs`) — the golden is
    /// never meant to be hand-edited into agreement.
    #[tokio::test]
    async fn the_iii_surface_matches_its_golden() {
        let fake = FakeEngine::new();
        let out = eval(
            &spawn_rt_with(fake.clone()),
            "return Object.keys(iii).sort().flatMap(k => { \
               const v = iii[k]; \
               const sig = x => `${typeof x}` + (typeof x === 'function' ? `/${x.length}` : ''); \
               const line = `${k}:${sig(v)}`; \
               return (v && typeof v === 'object') \
                 ? [line, ...Object.keys(v).sort().map(k2 => `  ${k2}:${sig(v[k2])}`)] \
                 : [line]; \
             }).join('\\n')",
            2_000,
        )
        .await
        .unwrap();
        let rendered = out.result.as_str().unwrap();

        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/iii-surface.txt");
        if std::env::var("UPDATE_GOLDENS").as_deref() == Ok("1") {
            std::fs::write(&path, format!("{rendered}\n")).expect("write golden");
            return;
        }
        let golden = std::fs::read_to_string(&path).expect("read golden");
        assert_eq!(
            rendered,
            golden.trim_end(),
            "the iii surface changed; review, then regenerate with `UPDATE_GOLDENS=1 cargo test`"
        );
    }

    /// The property this exists for: a failing script still produced whatever
    /// it printed before it threw, and a host reporting the failure as a
    /// process-shaped response owes the caller that stdout.
    ///
    /// Mutation: restore `let _ = finish_eval(op_state);` at either failure
    /// site in `complete` — the logs come back empty and both arms fail.
    #[tokio::test]
    async fn a_throwing_eval_still_reports_what_it_printed() {
        let rt = spawn_rt();

        // Synchronous throw.
        let err = eval(
            &rt,
            "console.log('before'); console.error('bad'); throw new Error('boom')",
            2_000,
        )
        .await
        .unwrap_err();
        match err {
            NodeEngineError::EvalFailed { message, logs } => {
                assert!(message.contains("boom"), "message was {message}");
                let rendered: Vec<_> = logs
                    .iter()
                    .map(|l| format!("{}:{}", l.level, l.message))
                    .collect();
                assert_eq!(rendered, vec!["log:before", "error:bad"]);
            }
            other => panic!("expected EvalFailed, got {other:?}"),
        }

        // Rejected promise, which reaches `complete` by the other path.
        let err = eval(
            &rt,
            "console.log('printed'); await Promise.reject(new Error('nope'))",
            2_000,
        )
        .await
        .unwrap_err();
        match err {
            NodeEngineError::EvalFailed { message, logs } => {
                assert!(message.contains("nope"), "message was {message}");
                assert_eq!(logs.len(), 1);
                assert_eq!(logs[0].message, "printed");
            }
            other => panic!("expected EvalFailed, got {other:?}"),
        }
        rt.shutdown();
    }

    // --- iii.files: this runtime's private scratch directory ---

    /// The property `keep` exists for. Mutation: move the `TempDir` creation
    /// out of `run_loop` and into the op (a per-call directory); the second
    /// eval then reads nothing.
    #[tokio::test]
    async fn files_written_in_one_eval_are_visible_in_the_next() {
        let rt = spawn_rt();
        eval(&rt, "iii.files.write('a.txt', 'hello'); return 1", 2_000)
            .await
            .unwrap();
        let out = eval(&rt, "return iii.files.readText('a.txt')", 2_000)
            .await
            .unwrap();
        assert_eq!(out.result, serde_json::json!("hello"));
        rt.shutdown();
    }

    /// Mutation: hoist the `TempDir` to a process-global `OnceCell`; runtime B
    /// then sees runtime A's file.
    #[tokio::test]
    async fn two_runtimes_never_share_a_directory() {
        let a = spawn_rt();
        let b = spawn_rt();
        eval(&a, "iii.files.write('secret.txt', 'A'); return 1", 2_000)
            .await
            .unwrap();
        let out = eval(&b, "return iii.files.list()", 2_000).await.unwrap();
        assert_eq!(out.result, serde_json::json!([]));
        let missing = eval(
            &b,
            "try { iii.files.readText('secret.txt'); return 'read' } catch (e) { return 'refused' }",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(missing.result, serde_json::json!("refused"));
        a.shutdown();
        b.shutdown();
    }

    /// Asserts the total NEVER exceeded the cap, not merely that a call threw
    /// — a check-after mutant (`current > cap` instead of `projected > cap`)
    /// also throws, just one write too late. Also pins refuse-don't-kill: the
    /// runtime still answers afterwards.
    #[tokio::test]
    async fn the_byte_quota_refuses_a_breaching_write_and_the_runtime_survives() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "const big = 'x'.repeat(600 * 1024);
             iii.files.write('a.txt', big);
             let refused = false;
             try { iii.files.write('b.txt', big) } catch (e) { refused = true }
             const total = iii.files.list().reduce((n, f) => n + f.bytes, 0);
             return [refused, total]",
            5_000,
        )
        .await
        .unwrap();
        let arr = out.result.as_array().unwrap();
        assert_eq!(
            arr[0],
            serde_json::json!(true),
            "the second write must be refused"
        );
        assert!(
            arr[1].as_u64().unwrap() <= (TEST_SCRATCH_MB * 1024 * 1024) as u64,
            "the directory total must never exceed the cap, got {}",
            arr[1]
        );

        let alive = eval(&rt, "return 'alive'", 2_000).await.unwrap();
        assert_eq!(alive.result, serde_json::json!("alive"));
        rt.shutdown();
    }

    /// The `- target_bytes` term, at the JS level. Mutation: drop it from the
    /// projection and the second overwrite is refused.
    #[tokio::test]
    async fn overwriting_a_file_does_not_double_count() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "const big = 'x'.repeat(900 * 1024);
             iii.files.write('a.txt', big);
             iii.files.write('a.txt', big);
             iii.files.write('a.txt', big);
             return iii.files.list().length",
            5_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, serde_json::json!(1));
        rt.shutdown();
    }

    /// One-byte files, so the byte cap provably cannot be what stopped it.
    /// Mutation: delete the entry check, or flip `>` to `>=`.
    #[tokio::test]
    async fn the_entry_quota_refuses_the_file_past_the_cap() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "let written = 0;
             let refused = false;
             for (let i = 0; i < 20; i++) {
               try { iii.files.write(`f${i}.txt`, 'x'); written++ } catch (e) { refused = true; break }
             }
             return [written, refused]",
            5_000,
        )
        .await
        .unwrap();
        let arr = out.result.as_array().unwrap();
        assert_eq!(arr[0], serde_json::json!(TEST_SCRATCH_FILES));
        assert_eq!(arr[1], serde_json::json!(true));
        rt.shutdown();
    }

    /// Without `remove`, the first tenant to fill the quota has permanently
    /// bricked its own runtime. Mutation: make `remove` a reporting no-op.
    #[tokio::test]
    async fn removing_a_file_returns_its_bytes_to_the_quota() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "for (let i = 0; i < 4; i++) iii.files.write(`f${i}.txt`, 'x');
             let blocked = false;
             try { iii.files.write('extra.txt', 'x') } catch (e) { blocked = true }
             iii.files.remove('f0.txt');
             iii.files.write('extra.txt', 'x');
             return [blocked, iii.files.list().length]",
            5_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, serde_json::json!([true, 4]));
        rt.shutdown();
    }

    /// Mutation: move the quota check after the truncating open. The mutant
    /// leaves an EMPTY file — `truncate(true)` already fired — so asserting
    /// only "it threw" would miss it.
    #[tokio::test]
    async fn a_refused_write_leaves_the_previous_contents_intact() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "iii.files.write('a.txt', 'keep me');
             try { iii.files.write('a.txt', 'x'.repeat(2 * 1024 * 1024)) } catch (e) {}
             return iii.files.readText('a.txt')",
            5_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, serde_json::json!("keep me"));
        rt.shutdown();
    }

    /// Both forms, because they are two different freezes: the inner
    /// `Object.freeze(files)` in the prelude, and runtime.rs's shallow
    /// `Object.freeze(globalThis.iii)`. Mutation: drop the inner freeze and
    /// the first assertion fails.
    #[tokio::test]
    async fn iii_files_is_frozen() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "try { iii.files.write = () => 'forged' } catch (_) {}
             try { iii.files = {} } catch (_) {}
             return [typeof iii.files.write === 'function' && iii.files.write.length === 2,
                     typeof iii.files.list === 'function']",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, serde_json::json!([true, true]));
        rt.shutdown();
    }

    /// Uses V8's own codec on both sides. Mutation: hand-roll the encoder and
    /// a surrogate pair comes back mangled.
    #[tokio::test]
    async fn text_round_trips_through_utf8() {
        let rt = spawn_rt();
        let out = eval(
            &rt,
            "iii.files.write('u.txt', 'café 🎉');
             return [iii.files.readText('u.txt'), iii.files.read('u.txt') instanceof Uint8Array]",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, serde_json::json!(["café 🎉", true]));
        rt.shutdown();
    }

    // --- Task 12: iii.registerTrigger / registerTriggerType / unregisterTriggerType ---

    /// The engine call for `iii.registerTrigger` — `Engine::register_trigger`
    /// wired into the isolate for the first time. `t.unregister()` proves the
    /// SDK-shaped ref this returns is really usable, not just shaped like
    /// one: the brief's own test name promises "and can be removed", but its
    /// given body never actually called `unregister()`.
    #[tokio::test]
    async fn register_trigger_reaches_the_engine_and_can_be_removed() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "const t = await iii.registerTrigger({
                 type: 'state', function_id: 'app::react', config: { key: 'k' }
             });
             globalThis.t = t;
             return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(fake.registered_triggers().len(), 1);

        eval(&rt, "t.unregister(); return 1", 2_000).await.unwrap();
        assert_eq!(
            fake.registered_triggers().len(),
            0,
            "unregister() left the trigger live on the bus"
        );
        rt.shutdown();
    }

    /// The engine calls INTO the isolate here. This is the same path a
    /// registered function's handler uses; only the method name is new.
    ///
    /// Drives the callback with `fire_trigger_type_config` — the FULL
    /// `TriggerConfig` shape the live engine actually sends, not
    /// `fire_trigger_type`'s abbreviated `{method, config}` — and asserts on
    /// `c.id`/`c.function_id` (which trigger instance, which function to
    /// invoke — recoverable from nothing else) AND `c.config.key` (the
    /// tenant's own opaque config), so this is evidence about the real wire
    /// shape a `TriggerHandler` implementation receives, not about a test
    /// double's narrower one.
    #[tokio::test]
    async fn a_trigger_type_handler_receives_engine_callbacks() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with_namespace(fake.clone(), "app::");
        eval(
            &rt,
            "globalThis.seen = [];
             iii.registerTriggerType(
               { id: 'app::mytype', description: 'test type' },
               {
                 registerTrigger: (c) => { globalThis.seen.push(['reg', c.id, c.function_id, c.config.key]) },
                 unregisterTrigger: (c) => { globalThis.seen.push(['unreg', c.id, c.function_id, c.config.key]) },
               },
             );
             return 1",
            2_000,
        )
        .await
        .unwrap();

        fake.fire_trigger_type_config(
            "app::mytype",
            "registerTrigger",
            TriggerCallback {
                id: "trig-1".to_string(),
                function_id: "app::react".to_string(),
                config: json!({ "key": "v" }),
                metadata: None,
            },
        )
        .await
        .unwrap();
        let out = eval(&rt, "return globalThis.seen", 2_000).await.unwrap();
        assert_eq!(out.result, json!([["reg", "trig-1", "app::react", "v"]]));
        rt.shutdown();
    }

    /// A tenant's trigger-type handler that hangs must not wedge the
    /// engine's caller: the callback runs under `invoke_timeout_ms`, the
    /// same budget a registered function's handler already uses
    /// (`op_iii_register`'s proxy).
    ///
    /// This test discriminates by HANGING, not failing, if the timeout is
    /// broken: `fire_trigger_type`'s future would simply never resolve. A
    /// broken timeout should report a test failure, not wedge the whole
    /// suite/CI run — wrapped in its own `tokio::time::timeout` so a
    /// regression here fails fast with a clear message instead.
    #[tokio::test]
    async fn a_hanging_trigger_type_handler_times_out_instead_of_wedging() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with_namespace_and_timeout(fake.clone(), "app::", 50);
        eval(
            &rt,
            "iii.registerTriggerType(
               { id: 'app::slow' },
               { registerTrigger: () => new Promise(() => {}), unregisterTrigger: () => {} },
             );
             return 1",
            2_000,
        )
        .await
        .unwrap();
        let err = tokio::time::timeout(
            Duration::from_secs(5),
            fake.fire_trigger_type("app::slow", "registerTrigger", json!({ "id": "t1" })),
        )
        .await
        .expect("fire_trigger_type must resolve within 5s, not hang")
        .unwrap_err();
        assert!(err.contains("timeout"), "expected a timeout, got: {err}");
    }

    /// Task 9's review flagged `IIIEngine::triggers` as unbounded and
    /// guest-reachable the moment an op exists to reach it. Fills the shared
    /// budget with function registrations, then proves BOTH new registration
    /// kinds — not just functions — are refused once it is exhausted.
    #[tokio::test]
    async fn triggers_and_trigger_types_count_against_the_shared_registration_cap() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let fill = format!(
            "for (let i = 0; i < {}; i++) {{ iii.registerFunction('test::f' + i, () => i) }} \
             return 1",
            crate::ops::MAX_REGISTRATIONS_PER_RUNTIME,
        );
        eval(&rt, &fill, 20_000).await.unwrap();

        let trigger_err = eval(
            &rt,
            "await iii.registerTrigger({ type: 'state', function_id: 'test::f0', config: {} }); \
             return 1",
            2_000,
        )
        .await
        .unwrap_err();
        assert!(
            trigger_err.to_string().contains("256"),
            "must name the cap: {trigger_err}"
        );
        assert!(
            fake.registered_triggers().is_empty(),
            "the bus saw an uncapped trigger write"
        );

        let type_err = eval(
            &rt,
            "iii.registerTriggerType({ id: 'test::overtype' }, \
               { registerTrigger(){}, unregisterTrigger(){} }); \
             return 1",
            2_000,
        )
        .await
        .unwrap_err();
        assert!(
            type_err.to_string().contains("256"),
            "must name the cap: {type_err}"
        );
        assert!(
            fake.registered_trigger_types().is_empty(),
            "the bus saw an uncapped trigger-type write"
        );

        rt.shutdown();
    }

    #[tokio::test]
    async fn an_overlong_trigger_config_is_rejected() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let code = "try { await iii.registerTrigger({ type: 'state', function_id: 'test::f', \
                     config: { big: 'x'.repeat(2 * 1024 * 1024) } }); return 'accepted' } \
                     catch (e) { return 'rejected' }";
        let out = eval(&rt, code, 5_000).await.unwrap();
        assert_eq!(out.result, json!("rejected"));
        assert!(fake.registered_triggers().is_empty());
        rt.shutdown();
    }

    #[tokio::test]
    async fn an_overlong_trigger_type_id_is_rejected() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let code = "try { iii.registerTriggerType({ id: 'test::' + 'x'.repeat(1024) }, \
                     { registerTrigger(){}, unregisterTrigger(){} }); return 'accepted' } \
                     catch (e) { return 'rejected' }";
        let out = eval(&rt, code, 5_000).await.unwrap();
        assert_eq!(out.result, json!("rejected"));
        assert!(fake.registered_trigger_types().is_empty());
        rt.shutdown();
    }

    #[tokio::test]
    async fn trigger_type_ids_outside_the_namespace_are_rejected() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with_namespace(fake.clone(), "app::");
        let out = eval(
            &rt,
            "try { iii.registerTriggerType({ id: 'other::type' }, \
               { registerTrigger(){}, unregisterTrigger(){} }); return 'no throw' } \
             catch (e) { return e.message }",
            2_000,
        )
        .await
        .unwrap();
        assert!(
            out.result.as_str().unwrap().contains("app::"),
            "message should name the namespace, got {:?}",
            out.result
        );
        assert!(fake.registered_trigger_types().is_empty());
        rt.shutdown();
    }

    #[tokio::test]
    async fn unregister_trigger_type_removes_the_registration() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with_namespace(fake.clone(), "app::");
        eval(
            &rt,
            "const ref = iii.registerTriggerType({ id: 'app::mytype' }, \
               { registerTrigger(){}, unregisterTrigger(){} }); \
             ref.unregister(); return 1",
            2_000,
        )
        .await
        .unwrap();
        assert!(fake.registered_trigger_types().is_empty());
        rt.shutdown();
    }

    /// The standalone `iii.unregisterTriggerType(id)` form — distinct from a
    /// ref's own `unregister()` — accepts either a bare id string or the
    /// original type object (reading its `.id`).
    #[tokio::test]
    async fn unregister_trigger_type_accepts_a_bare_id_or_the_type_object() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with_namespace(fake.clone(), "app::");
        eval(
            &rt,
            "iii.registerTriggerType({ id: 'app::a' }, { registerTrigger(){}, unregisterTrigger(){} }); \
             iii.registerTriggerType({ id: 'app::b' }, { registerTrigger(){}, unregisterTrigger(){} }); \
             iii.unregisterTriggerType('app::a'); \
             iii.unregisterTriggerType({ id: 'app::b' }); \
             return 1",
            2_000,
        )
        .await
        .unwrap();
        assert!(fake.registered_trigger_types().is_empty());
        rt.shutdown();
    }

    /// Re-registering under an id this runtime already holds is a no-op at
    /// the engine layer (see `op_iii_register_trigger_type`'s doc comment for
    /// why) — but the bus-level proxy is never rebuilt, so the invoke-time
    /// lookup must still route to whichever handler was registered MOST
    /// RECENTLY, not the first, or a re-registration would silently keep
    /// firing stale JavaScript.
    #[tokio::test]
    async fn re_registering_a_trigger_type_routes_to_the_latest_handler() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with_namespace(fake.clone(), "app::");
        eval(
            &rt,
            "globalThis.seen = [];
             iii.registerTriggerType({ id: 'app::mytype' }, {
               registerTrigger: () => { globalThis.seen.push('first') },
               unregisterTrigger: () => {},
             });
             iii.registerTriggerType({ id: 'app::mytype' }, {
               registerTrigger: () => { globalThis.seen.push('second') },
               unregisterTrigger: () => {},
             });
             return 1",
            2_000,
        )
        .await
        .unwrap();

        fake.fire_trigger_type("app::mytype", "registerTrigger", json!({}))
            .await
            .unwrap();
        let out = eval(&rt, "return globalThis.seen", 2_000).await.unwrap();
        assert_eq!(out.result, json!(["second"]));
        rt.shutdown();
    }

    /// A re-registration under an id this runtime already holds must not
    /// spend a second slot from the shared cap. Fills to `MAX - 2` with
    /// functions, registers ONE trigger type five times over (four of those
    /// re-registrations), and proves exactly one more registration still
    /// fits — it would already be refused if any of the five had counted.
    #[tokio::test]
    async fn re_registering_a_trigger_type_does_not_spend_a_second_cap_slot() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with_namespace(fake.clone(), "app::");
        let fill = format!(
            "for (let i = 0; i < {}; i++) {{ iii.registerFunction('app::f' + i, () => i) }} \
             return 1",
            crate::ops::MAX_REGISTRATIONS_PER_RUNTIME - 2,
        );
        eval(&rt, &fill, 20_000).await.unwrap();

        eval(
            &rt,
            "for (let i = 0; i < 5; i++) { \
               iii.registerTriggerType({ id: 'app::mytype' }, \
                 { registerTrigger(){}, unregisterTrigger(){} }); \
             } return 1",
            5_000,
        )
        .await
        .unwrap();

        // `MAX - 2` functions + one logical trigger-type registration = `MAX
        // - 1` held; exactly one more registration must still fit.
        let out = eval(
            &rt,
            "iii.registerFunction('app::last', () => 1); return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!(1));
        rt.shutdown();
    }

    // --- Task 12 fix round: kind-tagged registrations, full trigger-type
    // payloads, the `.then` hijack, and the `registered` delta ---

    /// A function and a trigger type sharing the identical literal id must
    /// not collide: before `RegistrationKind` tagged `unregisters` entries,
    /// `op_iii_register_trigger_type`'s dedup check matched the FUNCTION
    /// entry (id-only lookup) and treated the trigger type as "already
    /// registered", so it silently never reached the bus while `eval`
    /// reported success.
    #[tokio::test]
    async fn a_function_id_and_a_trigger_type_id_do_not_collide_on_registration() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "iii.registerFunction('test::x', () => 1); return 1",
            2_000,
        )
        .await
        .unwrap();

        let out = eval(
            &rt,
            "iii.registerTriggerType({ id: 'test::x' }, \
               { registerTrigger(){}, unregisterTrigger(){} }); return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!(1));
        assert_eq!(
            fake.registered_trigger_types(),
            vec!["test::x".to_string()],
            "the trigger type never reached the bus despite eval() reporting success"
        );
        rt.shutdown();
    }

    /// Registering a function must not evict a same-named trigger type:
    /// before the kind tag, `op_iii_register`'s dedup lookup matched the
    /// TRIGGER TYPE'S entry, swap-removed it, and called ITS unregister —
    /// silently un-publishing a live trigger type as a side effect of an
    /// unrelated function registration.
    #[tokio::test]
    async fn registering_a_function_does_not_evict_a_same_named_trigger_type() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "iii.registerTriggerType({ id: 'test::x', description: 'd' }, \
               { registerTrigger(){}, unregisterTrigger(){} }); return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(fake.registered_trigger_types(), vec!["test::x".to_string()]);

        eval(
            &rt,
            "iii.registerFunction('test::x', () => 1, { description: 'd' }); return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(
            fake.registered_trigger_types(),
            vec!["test::x".to_string()],
            "registering a function evicted the same-named trigger type from the bus"
        );
        assert_eq!(fake.registered_ids(), vec!["test::x".to_string()]);
        rt.shutdown();
    }

    /// `iii.unregisterTriggerType(id)` must not remove a same-named
    /// FUNCTION: before the kind tag, `OpsState::unregister`'s lookup
    /// matched on id alone, so unregistering a trigger type that was never
    /// registered (but shares a string with a live function) silently
    /// un-published the function instead.
    #[tokio::test]
    async fn unregistering_a_trigger_type_id_does_not_remove_a_same_named_function() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "iii.registerFunction('test::x', () => 1); return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(fake.registered_ids(), vec!["test::x".to_string()]);

        eval(&rt, "iii.unregisterTriggerType('test::x'); return 1", 2_000)
            .await
            .unwrap();
        assert_eq!(
            fake.registered_ids(),
            vec!["test::x".to_string()],
            "unregisterTriggerType removed a same-named FUNCTION"
        );
        rt.shutdown();
    }

    /// `EvalOutcome.registered`/`RunResponse.registered` must report a
    /// trigger-type id the eval just published — the same delta functions
    /// already report, and with one-shot `run` the default, often the only
    /// record a caller ever gets of what its eval published.
    #[tokio::test]
    async fn registering_a_trigger_type_is_reported_in_the_registered_delta() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with_namespace(fake.clone(), "app::");
        let out = eval(
            &rt,
            "iii.registerTriggerType({ id: 'app::mytype' }, \
               { registerTrigger(){}, unregisterTrigger(){} }); return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.registered, vec!["app::mytype".to_string()]);
        rt.shutdown();
    }

    /// `invoke()` must not resolve the engine-calls-into-the-isolate path
    /// through the tenant-replaceable `Promise.prototype.then`: a hijacked
    /// `.then` sits between a handler and `settle`'s own (pre-captured)
    /// `thenPromise`, and can forge whatever this call resolves to — turning
    /// a handler that throws into a reported success, undetectable by the
    /// engine. Exercises the SAME path `op_iii_register`'s function proxy
    /// uses (`fake.invoke` re-enters through `Command::Invoke`/`wrap_invoke`,
    /// not through `__iii.invoke` called directly inside an eval), which is
    /// what makes this a real regression test rather than one only the
    /// `wrap_eval`-wrapped path would catch.
    #[tokio::test]
    async fn a_hijacked_promise_then_cannot_forge_an_invoke_result() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        eval(
            &rt,
            "iii.registerFunction('test::boom', () => { throw new Error('handler blew up') }); \
             Promise.prototype.then = function () { return 'FORGED' }; \
             return 1",
            2_000,
        )
        .await
        .unwrap();
        let err = fake.invoke("test::boom", json!({})).await.unwrap_err();
        assert!(err.contains("handler blew up"), "got {err}");
        rt.shutdown();
    }

    // --- Task 12 fix round 2: the same three collision directions, but
    // within ONE eval — `ops.registered` is eval-scoped (cleared at every
    // `start`), so a bug gated on it can only ever show up here; the
    // cross-eval versions above never exercised it at all. ---

    /// The primary same-eval probe: `op_iii_register`'s dedup check used to
    /// also match `ops.registered.contains(&fn_id)` — kind-blind, and
    /// populated by the trigger-type registration one line above — so the
    /// function registration was treated as "already registered", silently
    /// skipped the bus write, and the eval still reported success.
    #[tokio::test]
    async fn a_function_id_and_a_trigger_type_id_do_not_collide_on_registration_in_the_same_eval() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "iii.registerTriggerType({ id: 'test::x' }, \
               { registerTrigger(){}, unregisterTrigger(){} }); \
             iii.registerFunction('test::x', () => 1); \
             return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!(1));
        assert_eq!(
            fake.registered_ids(),
            vec!["test::x".to_string()],
            "the function never reached the bus despite eval() reporting success"
        );
        assert_eq!(fake.registered_trigger_types(), vec!["test::x".to_string()]);
        assert_eq!(
            out.registered,
            vec!["test::x".to_string(), "test::x".to_string()],
            "both registrations from this eval should be in the reported delta"
        );
        rt.shutdown();
    }

    /// The removal half of the case above: `OpsState::unregister`'s trim of
    /// `ops.registered` was kind-BLIND (`retain(|id| id != fn_id)`) while the
    /// bus removal beside it was kind-tagged, so unregistering ONE of a
    /// same-id function/trigger-type pair dropped BOTH from the reported
    /// delta — the eval answered `registered: []` with a live function still
    /// on the bus, understating exactly what the delta exists to report.
    #[tokio::test]
    async fn unregistering_one_kind_leaves_the_other_in_the_reported_delta() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "iii.registerFunction('test::x', () => 1); \
             iii.registerTriggerType({ id: 'test::x' }, \
               { registerTrigger(){}, unregisterTrigger(){} }); \
             iii.unregisterTriggerType('test::x'); \
             return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(
            out.registered,
            vec!["test::x".to_string()],
            "the surviving function must still be reported as registered"
        );
        assert_eq!(
            fake.registered_ids(),
            vec!["test::x".to_string()],
            "the function is live on the bus"
        );
        assert!(fake.registered_trigger_types().is_empty());
        rt.shutdown();
    }

    #[tokio::test]
    async fn registering_a_function_does_not_evict_a_same_named_trigger_type_in_the_same_eval() {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "iii.registerTriggerType({ id: 'test::x', description: 'd' }, \
               { registerTrigger(){}, unregisterTrigger(){} }); \
             iii.registerFunction('test::x', () => 1, { description: 'd' }); \
             return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!(1));
        assert_eq!(
            fake.registered_trigger_types(),
            vec!["test::x".to_string()],
            "registering a function evicted the same-named trigger type from the bus"
        );
        assert_eq!(fake.registered_ids(), vec!["test::x".to_string()]);
        rt.shutdown();
    }

    #[tokio::test]
    async fn unregistering_a_trigger_type_id_does_not_remove_a_same_named_function_in_the_same_eval(
    ) {
        let fake = FakeEngine::new();
        let rt = spawn_rt_with(fake.clone());
        let out = eval(
            &rt,
            "iii.registerFunction('test::x', () => 1); \
             iii.unregisterTriggerType('test::x'); \
             return 1",
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(out.result, json!(1));
        assert_eq!(
            fake.registered_ids(),
            vec!["test::x".to_string()],
            "unregisterTriggerType removed a same-named FUNCTION"
        );
        rt.shutdown();
    }
}
