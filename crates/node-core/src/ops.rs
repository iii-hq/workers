//! The entire Rust↔JS boundary. Ops are the only way out of an isolate, so
//! this file is the worker's attack surface — keep it small and obvious.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use deno_core::{extension, op2, OpState};
use deno_error::JsErrorBox;
use futures::future::BoxFuture;
use tokio::sync::mpsc;

use crate::engine::{
    hook_outcome, parse_guest_observe, CallResult, Engine, GuestFunctionOptions, HookMethod,
    ProxyHandler, UnregisterFn,
};
use crate::error::NodeEngineError;
use crate::runtime::{Command, LogLine};

/// Which kind of registration an `OpsState::unregisters` entry is.
///
/// Function ids, observer ids and the function ids observer hooks are set
/// for all cross as plain strings and share ONE list — see
/// `OpsState::unregisters` — and the kinds routinely share the identical
/// literal id: a guest sets hooks on the very function it registers, and may
/// name an observer after it. Without the kind as part of the lookup key,
/// `op_iii_register`'s dedup check, `op_iii_set_observer_hooks`'s dedup
/// check, and `OpsState::unregister`'s removal all match on id alone, so
/// registering one kind under an id another kind already holds either
/// silently no-ops (reports success, never reaches the bus) or evicts the
/// wrong registration — and removing one kind can remove a live
/// registration of a different kind entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationKind {
    Function,
    /// An observer this runtime created with `iii.observe`.
    Observer,
    /// The pair of engine-side observer hooks a function id carries once
    /// the guest set `onObserverAdded` or `onObserverRemoved` on it. One
    /// entry covers both hooks.
    ObserverHooks,
}

/// Observer id → the id of the function it observes, for every observer an
/// `onObserverAdded` hook of this runtime has seen and no `onObserverRemoved`
/// hook has seen leave. The hook proxies run on the engine's thread, not the
/// isolate's, hence the `Arc<Mutex<…>>`.
pub type HookedObservers = Arc<Mutex<HashMap<String, String>>>;

/// Per-isolate state the ops read and write. Lives in deno_core's `OpState`,
/// which is thread-local to the isolate, so plain `RefCell` access is enough
/// for everything except the unregister list shared with the manager.
pub struct OpsState {
    pub engine: Arc<dyn Engine>,
    /// Required prefix for ids the evaluated code may register.
    pub namespace: String,
    /// Buffer for the eval currently in flight. Capped — see `op_iii_log`.
    pub logs: Vec<LogLine>,
    /// Bytes of message text currently buffered, tracked incrementally so a
    /// flood costs O(1) per line rather than a re-sum of the whole buffer.
    pub log_bytes: usize,
    /// Whether the truncation marker has already been appended.
    pub log_truncated: bool,
    /// Bytes logged from console calls made with no eval in flight. Its own
    /// budget, reset when an eval begins — those lines go to worker tracing,
    /// never to a caller's response.
    pub detached_log_bytes: usize,
    /// False between evals: lines from an interleaved handler invocation with
    /// no eval running go to worker tracing instead of a caller's response.
    pub capturing: bool,
    /// Function ids registered by the eval currently in flight, reported back
    /// as `EvalOutcome::registered`. Kind-tagged for the same reason
    /// `unregisters` is: a kind-blind `retain` on removal once dropped every
    /// entry sharing a literal id, so an eval that registered `app::x` and
    /// removed something else named `app::x` reported `registered: []` while
    /// the function was still live on the bus.
    pub registered: Vec<(RegistrationKind, String)>,
    /// Every live registration, shared with the manager so teardown can undo
    /// them from the main thread. Functions, observers AND observer hooks all
    /// live in this one list — see `MAX_REGISTRATIONS_PER_RUNTIME` — so its
    /// length is the single source of truth the cap checks against. Tagged
    /// with `RegistrationKind` because the kinds' ids share one string
    /// namespace as far as this list is concerned — see that type's doc
    /// comment.
    pub unregisters: Arc<Mutex<Vec<(RegistrationKind, String, UnregisterFn)>>>,
    /// The observers `op_iii_call_observer` may call through: the ones this
    /// runtime's own hooks were handed. See [`HookedObservers`].
    pub hooked_observers: HookedObservers,
    /// Worker-global id ownership. Claimed before `Engine::register`, because
    /// a duplicate there aborts the process.
    pub ids: crate::ids::IdRegistry,
    /// Ownership key for `ids`.
    pub runtime_id: String,
    pub call_timeout_ms: u64,
    /// Ceiling a guest-supplied `iii.call({..., timeout_ms})` is clamped to —
    /// see `RuntimeOpts::max_timeout_ms`.
    pub max_timeout_ms: u64,
    /// Calls to `iii.call` currently awaiting their engine response.
    /// Bounds `op_iii_call`'s Rust-heap footprint — see `MAX_INFLIGHT_CALLS`.
    pub inflight_calls: usize,
    /// Weak on purpose: a strong sender here would keep the runtime's channel
    /// open forever and deadlock every shutdown.
    pub command_tx: mpsc::WeakUnboundedSender<Command>,
    pub invoke_timeout_ms: u64,
    /// Shared with `manager::Runtime::last_activity`. An INVOKE dispatched
    /// straight from the bus to this isolate's command channel never passes
    /// through `RuntimeManager::run`/`register`, so without this a runtime
    /// doing exactly what `register_function` is for —
    /// sitting on the bus and answering calls — would still be reaped as
    /// idle. Bumped by `op_iii_register`'s proxy closure, not here: this
    /// field only carries the handle to the isolate thread that built it.
    pub last_activity: Arc<Mutex<Instant>>,
    /// This runtime's private scratch directory, or `None` when `scratch_mb`
    /// is 0.
    ///
    /// Owned HERE, not by the manager. `OpsState` lives in the isolate's
    /// `OpState`, which dies when the `JsRuntime` is dropped at `run_loop`
    /// scope exit — so `TempDir::drop` removes the directory on teardown, on
    /// reap, on the idle sweep, on a timeout/OOM kill AND on an unwind,
    /// without any of those paths needing to know it exists.
    pub scratch: Option<tempfile::TempDir>,
    pub scratch_max_bytes: u64,
    pub scratch_max_files: usize,
}

impl OpsState {
    /// Undo one of THIS runtime's own registrations: the mirror of the
    /// critical section in `op_iii_register`. `unregisters` is created fresh
    /// per runtime (see `RuntimeThread::spawn`) and never shared, so a
    /// lookup here can only ever find an entry this same runtime pushed —
    /// there is nothing to check ownership of that isn't already implied by
    /// which `Vec` we're looking in. A no-op if `fn_id` was never registered
    /// (or was already unregistered): same idempotent shape as
    /// `op_iii_register`'s re-registration path, and it means a runtime
    /// unregistering something inside its own namespace that doesn't exist
    /// gets the same quiet success a register-side re-registration gets,
    /// rather than a special error that would leak existence information.
    fn unregister(&mut self, kind: RegistrationKind, fn_id: &str) {
        if let Some(unregister) = self.take(kind, fn_id) {
            unregister();
        }
    }

    /// Remove one of this runtime's registrations from the books WITHOUT
    /// running its undo, and hand the undo back. `unregister` runs it at
    /// once; `op_iii_dispose_observer` awaits the engine's own removal
    /// instead of the teardown path's fire-and-forget one.
    ///
    /// Also drops `fn_id` from `registered` — the current eval's reported
    /// delta, echoed back as `RunResponse.registered`. Without this, a
    /// register-then-unregister in one eval would tell the caller it
    /// published something that is not live, and with one-shot `run` that
    /// response is often the only record the caller ever sees.
    ///
    /// Both the claim release and the `registered` trim are gated on the
    /// removal ACTUALLY having found a `(kind, fn_id)` entry — a review
    /// caught this scoped only by `fn_id` in an earlier round: two kinds can
    /// share the identical literal id, and removing one kind at an id where
    /// only another kind was live found nothing to remove (correct) but
    /// still released the id's worker-global claim (wrong) — freeing another
    /// runtime to claim it and reach `Engine::register` on a duplicate, which
    /// panics inside an `extern "C"` op and aborts the process (see `ids.rs`'s
    /// module doc).
    ///
    /// The claim release ALSO checks whether another kind still holds this
    /// exact id after this removal: `ids` tracks id ownership flatly, not
    /// per kind, so a function whose hooks were just cleared must not have
    /// its claim freed out from under it.
    fn take(&mut self, kind: RegistrationKind, fn_id: &str) -> Option<UnregisterFn> {
        let mut registrations = self.unregisters.lock().unwrap();
        let index = registrations
            .iter()
            .position(|(k, id, _)| *k == kind && id == fn_id)?;
        let (_, _, unregister) = registrations.swap_remove(index);
        let still_claimed_by_another_kind = registrations.iter().any(|(_, id, _)| id == fn_id);
        drop(registrations);
        if !still_claimed_by_another_kind {
            self.ids.release_ids(&[fn_id.to_string()], &self.runtime_id);
        }
        self.registered.retain(|(k, id)| *k != kind || id != fn_id);
        Some(unregister)
    }
}

/// Caps on the per-eval log buffer.
///
/// This buffer lives on the Rust heap, outside V8's `heap_limits` accounting,
/// so without a cap `for(;;) console.log(i)` would let one tenant allocate
/// process memory for its whole timeout window — routing around the very
/// heap limit the isolate is configured with, through a sanctioned op. The
/// byte cap is the real bound; the line cap keeps a flood of empty strings
/// from growing the `Vec` itself.
const MAX_LOG_LINES: usize = 1_000;
const MAX_LOG_BYTES: usize = 256 * 1024;

#[op2(fast)]
fn op_iii_log(state: &mut OpState, #[string] level: String, #[string] message: String) {
    let ops = state.borrow_mut::<OpsState>();

    if !ops.capturing {
        // Tenant-controlled text going straight into the worker's log stream
        // at an unbounded rate — the one instance of this hazard class that
        // was left uncapped. Its own counter, not the eval buffer: these lines
        // belong to no eval and must not be reported as one. `escape_debug`
        // because the formatter does not escape newlines, so raw text could
        // forge records that look like the worker's own.
        if ops.detached_log_bytes >= MAX_LOG_BYTES {
            return;
        }
        ops.detached_log_bytes += message.len();
        tracing::info!(
            target: "code-runner::js",
            namespace = %ops.namespace,
            %level,
            message = %message.escape_debug(),
            "console output outside an eval"
        );
        return;
    }

    if ops.logs.len() >= MAX_LOG_LINES || ops.log_bytes >= MAX_LOG_BYTES {
        if !ops.log_truncated {
            ops.log_truncated = true;
            ops.logs.push(LogLine {
                level: "warn".to_string(),
                message: format!(
                    "[code-runner] log output truncated at {MAX_LOG_LINES} lines / \
                     {MAX_LOG_BYTES} bytes; further console output is dropped"
                ),
            });
        }
        return;
    }

    ops.log_bytes += message.len();
    ops.logs.push(LogLine { level, message });
}

/// Caps on the TEXT `op_iii_call` copies to the Rust heap at argument
/// dispatch. All three of its `#[string]` arguments are capped, so that text
/// is bounded at `MAX_INFLIGHT_CALLS * (MAX_FUNCTION_ID_BYTES +
/// MAX_CALL_PAYLOAD_BYTES + MAX_ACTION_BYTES)` per runtime — a little over
/// 32 MiB at these values — independently of V8's own `heap_mb`.
///
/// That is the whole claim. It is NOT a bound on what stays resident across
/// `engine.call`'s await: `payload_json` is dropped the moment it parses, and
/// the parsed `serde_json::Value` that replaces it is what is actually pinned
/// there. A parsed value is routinely much larger than its own text — 32
/// in-flight copies of a ~976 KiB amplifying payload (`[1,1,1,…]`) measured
/// at 1098 MiB RSS, roughly 34x the figure above. Capping the parsed value is
/// a design change, deliberately not attempted here.
///
/// `op_iii_call_observer` shares the same counter and the same three caps:
/// its first argument is an observer id instead of a function id.
const MAX_INFLIGHT_CALLS: usize = 32;
const MAX_CALL_PAYLOAD_BYTES: usize = 1024 * 1024;
/// `#[string] action` is byte-copied to the Rust heap at argument-dispatch
/// time — before this function's body even runs, and independently of
/// `MAX_CALL_PAYLOAD_BYTES` two lines up — so an uncapped `action` is the
/// exact same heap-pinning hazard `payload_json` is capped against, just
/// easy to miss because every valid action is tiny (`{"type":"void"}`, or
/// `{"type":"enqueue","queue":"..."}`). Generous relative to any real
/// action; nowhere near `MAX_CALL_PAYLOAD_BYTES`.
const MAX_ACTION_BYTES: usize = 512;

/// What a call names: a function, or an observer to call through.
enum CallTarget {
    Function(String),
    Observer(String),
}

/// Invoke any engine function. Payload and result cross as JSON text so the
/// isolate boundary has exactly one representation to reason about.
///
/// `action` and `timeout_ms_arg` carry `iii.call`'s optional `action` and
/// `timeout_ms` fields across the same one-representation boundary: an empty
/// string means "no action" (plain request/response) and `0` means "use the
/// configured default timeout" — JS `undefined` is not a value `#[string]`/
/// `#[number]` args can carry, so the prelude substitutes these sentinels
/// before the op ever sees the call. `action` is the JSON text of the
/// guest's action object, capped at `MAX_ACTION_BYTES`; a non-zero
/// `timeout_ms_arg` is clamped to `max_timeout_ms`, the same ceiling
/// `NodeEngineConfig::clamp_timeout` applies to the RPC-level eval budget.
// Plain `#[op2]` on an `async fn` IS the eager-async form in deno_ops 0.285 —
// the same shape deno_core's own `op_add_async` uses. Do not write
// `#[op2(async)]`: the bare flag requires parentheses in this version
// (`async(deferred)`, `async(lazy)`), and the bare form fails to compile with
// "expected attribute arguments in parentheses: `async(...)`".
#[op2]
#[string]
async fn op_iii_call(
    state: Rc<RefCell<OpState>>,
    #[string] fn_id: String,
    #[string] payload_json: String,
    #[string] action: String,
    #[number] timeout_ms_arg: u64,
) -> Result<String, JsErrorBox> {
    // Same hazard the log buffer had: everything below lives on the RUST heap,
    // outside V8's `heap_limits`. `call` is async, so tenant code can
    // fire it in a loop without awaiting, and each argument copy is made
    // eagerly — deno_core byte-copies every `#[string]` into an owned Rust
    // `String` at dispatch, before this body runs at all — and pinned for up
    // to `call_timeout_ms`. Without these caps a tenant routes around its own
    // isolate's memory limit through a sanctioned op: one eval passing an
    // 8 MiB `function_id` 32 times took a runtime from 36 MiB to 316 MiB RSS
    // with nothing refused, because only `payload_json` and `action` were
    // checked. All three checks run BEFORE `inflight_calls` is touched in
    // `dispatch_call`, so none of them needs its own `release_inflight` call
    // — there is nothing yet to release.
    if fn_id.len() > MAX_FUNCTION_ID_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "function id is {} bytes; the limit is {MAX_FUNCTION_ID_BYTES}",
            fn_id.len()
        )));
    }
    dispatch_call(
        state,
        CallTarget::Function(fn_id),
        payload_json,
        action,
        timeout_ms_arg,
    )
    .await
}

/// Call through an observer: the engine runs the observer's own target with
/// the payload. `iii.call({ observer, payload })`.
///
/// Only observers THIS runtime's hooks were handed may be called through
/// (`OpsState::hooked_observers`). Every guest shares the worker's one
/// connection, and the engine checks only that the CONNECTION hosts the
/// observer's function — so without this, one tenant holding another's
/// observer id (a forged `{ id }` is all `iii.call` reads) could run that
/// tenant's targets with a payload of its choosing. "Not yours" and "not
/// there" are one refusal, for the same reason `op_iii_unregister`'s are.
#[op2]
#[string]
async fn op_iii_call_observer(
    state: Rc<RefCell<OpState>>,
    #[string] observer_id: String,
    #[string] payload_json: String,
    #[string] action: String,
    #[number] timeout_ms_arg: u64,
) -> Result<String, JsErrorBox> {
    if observer_id.len() > MAX_FUNCTION_ID_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "observer id is {} bytes; the limit is {MAX_FUNCTION_ID_BYTES}",
            observer_id.len()
        )));
    }
    let hooked = {
        let mut borrowed = state.borrow_mut();
        let ops = borrowed.borrow_mut::<OpsState>();
        let hooked_observers = ops.hooked_observers.clone();
        let known = hooked_observers.lock().unwrap().contains_key(&observer_id);
        known
    };
    if !hooked {
        return Err(JsErrorBox::type_error(format!(
            "observer {observer_id:?} is not one this runtime's onObserverAdded hooks were \
             given; a runtime can only call through observers of its own functions"
        )));
    }
    dispatch_call(
        state,
        CallTarget::Observer(observer_id),
        payload_json,
        action,
        timeout_ms_arg,
    )
    .await
}

/// The shared body of `op_iii_call` and `op_iii_call_observer`, once each
/// has capped and checked its own first argument.
async fn dispatch_call(
    state: Rc<RefCell<OpState>>,
    target: CallTarget,
    payload_json: String,
    action: String,
    timeout_ms_arg: u64,
) -> Result<String, JsErrorBox> {
    if payload_json.len() > MAX_CALL_PAYLOAD_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "payload is {} bytes; the limit is {MAX_CALL_PAYLOAD_BYTES}",
            payload_json.len()
        )));
    }
    if action.len() > MAX_ACTION_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "action is {} bytes; the limit is {MAX_ACTION_BYTES}",
            action.len()
        )));
    }

    let action = (!action.is_empty()).then_some(action);

    let (engine, timeout_ms) = {
        let mut borrowed = state.borrow_mut();
        let ops = borrowed.borrow_mut::<OpsState>();
        if ops.inflight_calls >= MAX_INFLIGHT_CALLS {
            return Err(JsErrorBox::generic(format!(
                "too many concurrent iii.call calls (limit \
                 {MAX_INFLIGHT_CALLS}); await earlier calls before making more"
            )));
        }
        ops.inflight_calls += 1;
        // A requested timeout is a CEILING clamp, not a fixed override: below
        // `max_timeout_ms` it passes through unchanged, so a short request is
        // still short. `0` (unset) falls back to the configured default
        // rather than being clamped, matching `NodeEngineConfig::clamp_timeout`'s
        // "absent, not zero-duration" treatment.
        let timeout_ms = if timeout_ms_arg == 0 {
            ops.call_timeout_ms
        } else {
            timeout_ms_arg.min(ops.max_timeout_ms)
        };
        (ops.engine.clone(), timeout_ms)
    };

    let payload: serde_json::Value = match serde_json::from_str(&payload_json) {
        Ok(v) => v,
        Err(e) => {
            release_inflight(&state);
            return Err(JsErrorBox::type_error(format!("payload is not JSON: {e}")));
        }
    };
    // The text copy is dead once parsed; free it before parking on the await.
    drop(payload_json);

    let result = match target {
        CallTarget::Function(fn_id) => engine.call(fn_id, payload, timeout_ms, action).await,
        CallTarget::Observer(observer_id) => {
            engine
                .call_observer(observer_id, payload, timeout_ms, action)
                .await
        }
    };
    release_inflight(&state);

    match result {
        Ok(value) => serde_json::to_string(&value)
            .map_err(|e| JsErrorBox::generic(format!("result is not JSON: {e}"))),
        Err(message) => Err(JsErrorBox::generic(message)),
    }
}

/// Decrement the in-flight counter. Done explicitly at each exit rather than
/// via a `Drop` guard: a guard would have to borrow the `RefCell` while the
/// future is being dropped, which during isolate teardown can collide with an
/// outstanding borrow and panic. A counter leaked by a cancelled future dies
/// with the `OpsState` it lives in.
fn release_inflight(state: &Rc<RefCell<OpState>>) {
    let mut borrowed = state.borrow_mut();
    let ops = borrowed.borrow_mut::<OpsState>();
    ops.inflight_calls = ops.inflight_calls.saturating_sub(1);
}

/// Caps on registration — the third instance of the same hazard as
/// `MAX_LOG_*` and `MAX_INFLIGHT_CALLS`. Each registration pins several owned
/// `String`s and an `Arc<dyn Fn>` on the Rust heap for the life of the
/// runtime, outside V8's `heap_limits`, AND fires a real `Engine::register`
/// into the trusted bus. Uncapped, one `for` loop in tenant JS is both an
/// unbounded allocation and an unbounded write to the bus. The count cap also
/// bounds the linear de-dup scan below, keeping it trivially cheap.
pub const MAX_REGISTRATIONS_PER_RUNTIME: usize = 256;
pub const MAX_FUNCTION_ID_BYTES: usize = 512;
pub const MAX_DESCRIPTION_BYTES: usize = 4 * 1024;

/// Decode one guest-supplied format argument: "" is "not supplied",
/// anything else must parse as JSON and pass the shared wire rule
/// (`wire::register::validate_format`). Raw length is checked BEFORE parsing
/// so an oversized blob is refused for its size, not after the host paid to
/// parse it — the raw string IS the serialized form (the prelude sends
/// `JSON.stringify` output), so the two length checks are the same rule.
fn parse_format(field: &'static str, raw: &str) -> Result<Option<serde_json::Value>, JsErrorBox> {
    if raw.is_empty() {
        return Ok(None);
    }
    if raw.len() > crate::wire::register::MAX_FORMAT_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "{field} is {} bytes serialized; the limit is {}",
            raw.len(),
            crate::wire::register::MAX_FORMAT_BYTES
        )));
    }
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| JsErrorBox::type_error(format!("{field} is not valid JSON: {e}")))?;
    crate::wire::register::validate_format(field, &value).map_err(JsErrorBox::type_error)?;
    Ok(Some(value))
}

/// The refusal every registration kind — function, observer, observer
/// hooks — gives once this runtime's shared `MAX_REGISTRATIONS_PER_RUNTIME`
/// budget is exhausted. One function so the call sites cannot drift into
/// different wordings for what is, from a tenant's perspective, one cap.
fn registration_cap_exceeded() -> JsErrorBox {
    JsErrorBox::generic(format!(
        "this runtime already holds {MAX_REGISTRATIONS_PER_RUNTIME} registrations \
         (functions, observers, and observer hooks share one budget); reuse an id or \
         tear the runtime down"
    ))
}

/// The one authorization check that gates both adding and removing a
/// registration: a runtime may only touch ids inside its own namespace
/// prefix. Shared by `op_iii_register` and `op_iii_unregister` rather than
/// copied, so the two can never drift into refusing differently — see
/// `op_iii_unregister`'s doc comment for why "not yours" and "not there"
/// must be the exact same refusal.
fn require_own_namespace(namespace: &str, fn_id: String) -> Result<String, JsErrorBox> {
    if fn_id.starts_with(namespace) {
        Ok(fn_id)
    } else {
        Err(JsErrorBox::type_error(
            NodeEngineError::NamespaceDenied {
                id: fn_id,
                namespace: namespace.to_string(),
            }
            .message(),
        ))
    }
}

/// Publish a JS handler as a real engine function.
///
/// The proxy runs on the engine's thread: it pushes an `Invoke` into this
/// isolate's command channel and awaits the reply, so JS never runs anywhere
/// but its own thread.
///
/// `args_format` is a third format argument with the same "" sentinel and
/// the same validation as the other two; `private` is the guest's
/// `options.private`, `false` when omitted.
#[op2(fast)]
fn op_iii_register(
    state: &mut OpState,
    #[string] fn_id: String,
    #[string] description: String,
    #[string] request_format: String,
    #[string] response_format: String,
    #[string] args_format: String,
    private: bool,
) -> Result<(), JsErrorBox> {
    let ops = state.borrow_mut::<OpsState>();

    if fn_id.len() > MAX_FUNCTION_ID_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "function id is {} bytes; the limit is {MAX_FUNCTION_ID_BYTES}",
            fn_id.len()
        )));
    }

    // Tenant-supplied text that leaves this worker and lands in the engine's
    // function catalog, where every caller of `engine::functions::info` reads
    // it back. Capped for the same reason the id is.
    if description.len() > MAX_DESCRIPTION_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "description is {} bytes; the limit is {MAX_DESCRIPTION_BYTES}",
            description.len()
        )));
    }

    let request_format = parse_format("request_format", &request_format)?;
    let response_format = parse_format("response_format", &response_format)?;
    let options = GuestFunctionOptions {
        args_format: parse_format("args_format", &args_format)?,
        private,
    };
    // Empty means "not supplied": the prelude sends "" when the caller omits
    // the argument, and `Engine::register` substitutes its default.
    let description = (!description.is_empty()).then_some(description);

    let fn_id = require_own_namespace(&ops.namespace, fn_id)?;

    let weak_tx = ops.command_tx.clone();
    let last_activity = ops.last_activity.clone();
    let timeout = std::time::Duration::from_millis(ops.invoke_timeout_ms);
    let proxy_id = fn_id.clone();
    let handler: ProxyHandler = Arc::new(move |payload: serde_json::Value| {
        let weak_tx = weak_tx.clone();
        let last_activity = last_activity.clone();
        let fn_id = proxy_id.clone();
        Box::pin(async move {
            let Some(tx) = weak_tx.upgrade() else {
                return Err(NodeEngineError::RuntimeGone(fn_id).to_string());
            };
            let (reply, rx) = tokio::sync::oneshot::channel();
            if tx
                .send(Command::Invoke {
                    fn_id: fn_id.clone(),
                    payload,
                    timeout,
                    reply,
                    method: None,
                })
                .is_err()
            {
                return Err(NodeEngineError::RuntimeGone(fn_id).to_string());
            }
            // Bump at SEND, not at reply. `invoke_timeout_ms` (always the
            // configured `default_timeout_ms`, typically 5000 ms) is roughly
            // 180x below `idle_ttl_secs` (900s default), so a handler that just
            // opened this window cannot possibly outlive it — bumping here
            // is enough to keep the runtime alive for the whole call. Note: no
            // validation ensures `idle_ttl_secs` and `default_timeout_ms` stay
            // separated; a misconfiguration could sweep the runtime mid-invoke,
            // but the caller gets `node-engine::runtime_gone` (bounded, clean,
            // no hang or orphan). This is the ONLY place an INVOKE proxied
            // straight from the bus touches activity at all: it never reaches
            // `RuntimeManager::run`, so without this a runtime doing exactly
            // what `register_function` exists for — serving
            // calls — would still be swept as idle at `idle_ttl_secs`.
            *last_activity.lock().unwrap() = Instant::now();
            match rx.await {
                Ok(result) => result.map_err(|e| e.to_string()),
                Err(_) => Err(NodeEngineError::RuntimeGone(fn_id).to_string()),
            }
        }) as BoxFuture<'static, CallResult>
    });

    // ONE critical section: de-dup, cap, claim, bus write, record. `destroy`
    // holds this same guard across its drain AND `release_owner`, so a
    // registration can never land on the bus after its claim was released —
    // that id would be live, unclaimed and impossible to unregister, reopening
    // the duplicate-id abort. Lock order is `unregisters` → `ids` on both
    // sides; neither `Engine::register` nor the unregister closure re-enters
    // `unregisters`, and there is no `.await` in this path.
    {
        let mut registrations = ops.unregisters.lock().unwrap();

        // Re-registering an id swaps the JS handler (the prelude's Map does
        // that) and the bus registration already exists, so it normally needs
        // no bus write. Checked before the cap so a re-registration never
        // trips it.
        //
        // The exception is NEW metadata: a description, a format, an
        // `args_format` or `private`. All of it is published — `engine::
        // functions::info` is where the next caller reads what this function
        // does, and the engine enforces `args_format` and `private` — and the
        // SDK only carries it at registration time, so keeping the existing
        // registration would silently discard it: the call reports success
        // and the engine still holds the old values. Drop the old
        // registration and make a new one carrying them. Safe to do here: the
        // claim is already ours, so no other runtime can take the id in the
        // gap, and `destroy` cannot interleave because it holds this same
        // guard.
        //
        // Scoped to `RegistrationKind::Function` alone, and not additionally
        // gated on `ops.registered` — that list is the current EVAL's delta
        // (cleared at the start of every eval, see `runtime.rs`) while
        // `unregisters` is every live registration for the life of the
        // runtime, and a kind-blind lookup there once treated a function as
        // "already registered" because ANOTHER kind held the same literal id,
        // silently skipping the bus write while still reporting success.
        // `held` (kind-tagged, checked against `unregisters`) is the reliable
        // source of truth for "is a FUNCTION already registered under this
        // id". The two lists agree only on the one thing
        // `OpsState::unregister` keeps aligned: a removal drops the matching
        // `(kind, id)` from both.
        let held = registrations
            .iter()
            .position(|(k, id, _)| *k == RegistrationKind::Function && *id == fn_id);
        if let Some(index) = held {
            if description.is_none()
                && request_format.is_none()
                && response_format.is_none()
                && options == GuestFunctionOptions::default()
            {
                return Ok(());
            }
            let (_, _, unregister) = registrations.swap_remove(index);
            unregister();
            // Wholesale replacement, never a merge: the SDK only carries
            // metadata at registration time, so a re-registration supplying
            // formats but no description falls back to the default
            // description rather than keeping the previous one.
            let fresh = ops.engine.register(
                fn_id.clone(),
                description,
                request_format,
                response_format,
                options,
                handler,
            );
            registrations.push((RegistrationKind::Function, fn_id, fresh));
            return Ok(());
        }

        // Functions share the one budget with observers and observer hooks.
        if registrations.len() >= MAX_REGISTRATIONS_PER_RUNTIME {
            return Err(registration_cap_exceeded());
        }

        // Claim before touching the bus. `Engine::register` on an id another
        // runtime already holds reaches a `panic!` inside this `extern "C"`
        // callback, which aborts the process rather than unwinding.
        if !ops.ids.claim(&fn_id, &ops.runtime_id) {
            return Err(JsErrorBox::generic(
                NodeEngineError::IdTaken(fn_id).message(),
            ));
        }

        let unregister = ops.engine.register(
            fn_id.clone(),
            description,
            request_format,
            response_format,
            options,
            handler,
        );
        registrations.push((RegistrationKind::Function, fn_id.clone(), unregister));
    }
    ops.registered.push((RegistrationKind::Function, fn_id));
    Ok(())
}

/// Remove a function this runtime registered.
///
/// The namespace check is the authorization: a runtime may only unpublish
/// inside its own prefix, and the refusal names the id without revealing
/// whether it exists — "not yours" and "not there" must be one answer, or
/// this op becomes a way to enumerate other tenants' registrations.
///
/// Split out from the `#[op2]` function below so it can be exercised
/// directly in a test with a hand-built `OpsState`. That seam matters here
/// specifically: the prelude's `registerFunction` only ever hands
/// `op_iii_unregister` an id that already passed THIS runtime's own
/// `op_iii_register` namespace check (`unregister()` closes over the id from
/// a successful registration), so there is no way to hand this op a foreign
/// id through the guest-visible surface at all. The check below is
/// defense-in-depth against that surface changing later, and a JS-level test
/// can only prove the surface still refuses early — not that this line does
/// its job. Calling it directly is the only way to do that.
///
/// `kind` is which registration list entry this id refers to — see
/// `RegistrationKind`'s doc comment for why a bare id is not enough:
/// `op_iii_unregister` and `op_iii_clear_observer_hooks` both route through
/// this, and a function and its hooks share the identical literal id.
fn unregister_checked(
    ops: &mut OpsState,
    kind: RegistrationKind,
    fn_id: String,
) -> Result<(), JsErrorBox> {
    let fn_id = require_own_namespace(&ops.namespace, fn_id)?;
    ops.unregister(kind, &fn_id);
    Ok(())
}

#[op2(fast)]
fn op_iii_unregister(state: &mut OpState, #[string] fn_id: String) -> Result<(), JsErrorBox> {
    unregister_checked(
        state.borrow_mut::<OpsState>(),
        RegistrationKind::Function,
        fn_id,
    )
}

/// The engine-side half of a guest's observer hooks: one proxy, set as both
/// the added and the removed hook of `fn_id`.
///
/// The engine calls it with `{ method, observer }`
/// (`engine::observer_hook_payload`). `method` picks which guest hook
/// `__iii.invoke` runs and is stripped; `observer` is forwarded as that
/// hook's whole argument. A function whose guest set only one of the two
/// hooks still gets both proxies, so this runtime's books
/// (`hooked_observers`) see every observer arrive AND leave; the prelude
/// answers a callback with no guest hook behind it the way iii does by
/// default — accept, nothing to tear down.
fn hook_proxy(ops: &OpsState, fn_id: String) -> ProxyHandler {
    let weak_tx = ops.command_tx.clone();
    let last_activity = ops.last_activity.clone();
    let hooked = ops.hooked_observers.clone();
    let timeout = std::time::Duration::from_millis(ops.invoke_timeout_ms);
    Arc::new(move |payload: serde_json::Value| {
        let weak_tx = weak_tx.clone();
        let last_activity = last_activity.clone();
        let hooked = hooked.clone();
        let fn_id = fn_id.clone();
        Box::pin(async move {
            let Some(obj) = payload.as_object() else {
                return Err("observer hook payload must be an object".to_string());
            };
            let Some(method) = obj
                .get("method")
                .and_then(serde_json::Value::as_str)
                .and_then(HookMethod::parse)
            else {
                return Err("observer hook payload names no hook method".to_string());
            };
            let Some(observer) = obj.get("observer").filter(|o| o.is_object()).cloned() else {
                return Err("observer hook payload carries no observer".to_string());
            };
            let Some(observer_id) = observer
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
            else {
                return Err("observer hook payload's observer has no id".to_string());
            };

            // Book the observer BEFORE the guest's added hook runs, so the
            // hook itself may call through it; unbook it before the removed
            // hook runs, since a removed observer is no longer callable.
            match method {
                HookMethod::ObserverAdded => {
                    hooked
                        .lock()
                        .unwrap()
                        .insert(observer_id.clone(), fn_id.clone());
                }
                HookMethod::ObserverRemoved => {
                    hooked.lock().unwrap().remove(&observer_id);
                }
            }

            let Some(tx) = weak_tx.upgrade() else {
                hooked.lock().unwrap().remove(&observer_id);
                return Err(NodeEngineError::RuntimeGone(fn_id).to_string());
            };
            let (reply, rx) = tokio::sync::oneshot::channel();
            if tx
                .send(Command::Invoke {
                    fn_id: fn_id.clone(),
                    payload: observer,
                    timeout,
                    reply,
                    method: Some(method.as_str().to_string()),
                })
                .is_err()
            {
                hooked.lock().unwrap().remove(&observer_id);
                return Err(NodeEngineError::RuntimeGone(fn_id).to_string());
            }
            // Same rationale as `op_iii_register`'s proxy: this is the only
            // place an INVOKE dispatched straight from the bus touches
            // activity at all, and `invoke_timeout_ms` is far below
            // `idle_ttl_secs`, so bumping at send is enough to cover the
            // whole call.
            *last_activity.lock().unwrap() = Instant::now();
            let result = match rx.await {
                Ok(result) => result.map_err(|e| e.to_string()),
                Err(_) => Err(NodeEngineError::RuntimeGone(fn_id).to_string()),
            };
            // A rejected observer never becomes active and never gets a
            // removed callback, so it leaves the books here.
            if method == HookMethod::ObserverAdded && hook_outcome(result.clone()).is_err() {
                hooked.lock().unwrap().remove(&observer_id);
            }
            result
        }) as BoxFuture<'static, CallResult>
    })
}

/// Give `fn_id` observer hooks: `iii.onObserverAdded(fn_id, hook)` /
/// `iii.onObserverRemoved(fn_id, hook)`, the first time either is set for
/// that id.
///
/// Mirrors `op_iii_register` closely — same namespace check, same shared
/// cap, same claim-before-bus-write ordering — because hooks decide who may
/// observe a function and hand out its observers: a second runtime setting
/// hooks on an id another runtime holds could reject every observer of that
/// runtime's function, or collect them and call through them. The claim
/// makes hooks and the function they belong to one runtime's.
///
/// Later calls for the same id are no-ops here: the prelude swaps the JS
/// hook in its own map, and the proxy built below looks that map up FRESH
/// on every callback (never a captured function reference — see
/// `prelude.js`'s `invoke`), so the existing engine hooks keep routing to
/// whichever guest hook is current without a second engine write.
#[op2(fast)]
fn op_iii_set_observer_hooks(
    state: &mut OpState,
    #[string] fn_id: String,
) -> Result<(), JsErrorBox> {
    let ops = state.borrow_mut::<OpsState>();

    if fn_id.len() > MAX_FUNCTION_ID_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "function id is {} bytes; the limit is {MAX_FUNCTION_ID_BYTES}",
            fn_id.len()
        )));
    }

    let fn_id = require_own_namespace(&ops.namespace, fn_id)?;
    let handler = hook_proxy(ops, fn_id.clone());

    let mut registrations = ops.unregisters.lock().unwrap();

    if registrations
        .iter()
        .any(|(k, id, _)| *k == RegistrationKind::ObserverHooks && *id == fn_id)
    {
        return Ok(());
    }

    if registrations.len() >= MAX_REGISTRATIONS_PER_RUNTIME {
        return Err(registration_cap_exceeded());
    }

    if !ops.ids.claim(&fn_id, &ops.runtime_id) {
        return Err(JsErrorBox::generic(
            NodeEngineError::IdTaken(fn_id).message(),
        ));
    }

    ops.engine
        .on_observer_added(fn_id.clone(), Some(handler.clone()));
    ops.engine.on_observer_removed(fn_id.clone(), Some(handler));
    let engine = ops.engine.clone();
    let hooked = ops.hooked_observers.clone();
    let hooks_id = fn_id.clone();
    let unregister: UnregisterFn = Box::new(move || {
        engine.on_observer_added(hooks_id.clone(), None);
        engine.on_observer_removed(hooks_id.clone(), None);
        // With the hooks gone no removed callback will arrive for these, so
        // they leave the books now.
        hooked.lock().unwrap().retain(|_, f| *f != hooks_id);
    });
    registrations.push((RegistrationKind::ObserverHooks, fn_id, unregister));
    // Not echoed in `registered`: that delta reports functions, and hooks
    // publish nothing a caller could call.
    Ok(())
}

/// Remove `fn_id`'s observer hooks: the prelude calls this once the guest
/// has set both of them back to `null`. Namespace-checked like
/// `op_iii_unregister`, since `fn_id` is tenant-chosen.
#[op2(fast)]
fn op_iii_clear_observer_hooks(
    state: &mut OpState,
    #[string] fn_id: String,
) -> Result<(), JsErrorBox> {
    if fn_id.len() > MAX_FUNCTION_ID_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "function id is {} bytes; the limit is {MAX_FUNCTION_ID_BYTES}",
            fn_id.len()
        )));
    }
    unregister_checked(
        state.borrow_mut::<OpsState>(),
        RegistrationKind::ObserverHooks,
        fn_id,
    )
}

/// Create an observer on the caller's behalf: `iii.observe(source, target,
/// options?)`. Resolves to the observer's id.
///
/// The id is settled HERE, before the engine sees the request. A guest that
/// names its observer (`options.id`) names it inside its own namespace, the
/// same rule as a function id, and the id is claimed in the worker-global
/// registry: every guest shares the worker's one connection, so an observer
/// id another tenant chose would otherwise address — and replace — that
/// tenant's observer. Without `options.id`, a fresh UUID is minted.
///
/// Knowing the id up front is also what lets the registration land in
/// `unregisters` in the same critical section as the cap check, BEFORE the
/// `.await`: a tenant firing many un-awaited `observe()` calls cannot have
/// them all pass one "under the cap" snapshot, and a teardown that drains
/// the runtime mid-await disposes the observer like any other.
#[op2]
#[string]
async fn op_iii_observe(
    state: Rc<RefCell<OpState>>,
    #[string] input_json: String,
) -> Result<String, JsErrorBox> {
    // Same hazard `MAX_CALL_PAYLOAD_BYTES`/`MAX_ACTION_BYTES` guard in
    // `op_iii_call`: deno_core byte-copies a `#[string]` argument to the Rust
    // heap at dispatch time, before this body runs at all.
    if input_json.len() > MAX_CALL_PAYLOAD_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "observe request is {} bytes; the limit is {MAX_CALL_PAYLOAD_BYTES}",
            input_json.len()
        )));
    }
    let mut input: serde_json::Value = serde_json::from_str(&input_json)
        .map_err(|e| JsErrorBox::type_error(format!("observe request is not JSON: {e}")))?;
    // The text copy is dead once parsed; free it before parking on the await.
    drop(input_json);
    let parsed = parse_guest_observe(input.clone()).map_err(JsErrorBox::type_error)?;

    let (engine, observer_id) = {
        let mut borrowed = state.borrow_mut();
        let ops = borrowed.borrow_mut::<OpsState>();
        let observer_id = match parsed.options.id {
            Some(id) => {
                if id.len() > MAX_FUNCTION_ID_BYTES {
                    return Err(JsErrorBox::type_error(format!(
                        "observer id is {} bytes; the limit is {MAX_FUNCTION_ID_BYTES}",
                        id.len()
                    )));
                }
                require_own_namespace(&ops.namespace, id)?
            }
            None => uuid::Uuid::new_v4().to_string(),
        };

        let mut registrations = ops.unregisters.lock().unwrap();
        if registrations
            .iter()
            .any(|(k, id, _)| *k == RegistrationKind::Observer && *id == observer_id)
        {
            return Err(JsErrorBox::type_error(format!(
                "observer {observer_id:?} already exists in this runtime; dispose it before \
                 observing again under the same id"
            )));
        }
        if registrations.len() >= MAX_REGISTRATIONS_PER_RUNTIME {
            return Err(registration_cap_exceeded());
        }
        if !ops.ids.claim(&observer_id, &ops.runtime_id) {
            return Err(JsErrorBox::generic(
                NodeEngineError::IdTaken(observer_id).message(),
            ));
        }

        let dispose_engine = ops.engine.clone();
        let dispose_id = observer_id.clone();
        let unregister: UnregisterFn = Box::new(move || {
            // Teardown runs this from the manager's thread, which is not
            // inside an async fn, and the engine's dispose waits on a reply
            // from the bus. Hand it to the runtime this is called on rather
            // than blocking teardown on the network.
            let dispose = dispose_engine.dispose_observer(dispose_id.clone());
            let id = dispose_id.clone();
            match tokio::runtime::Handle::try_current() {
                Ok(rt) => {
                    rt.spawn(async move {
                        if let Err(e) = dispose.await {
                            tracing::debug!(observer_id = %id, error = %e, "observer dispose failed");
                        }
                    });
                }
                Err(_) => {
                    if let Err(e) = futures::executor::block_on(dispose) {
                        tracing::debug!(observer_id = %id, error = %e, "observer dispose failed");
                    }
                }
            }
        });
        registrations.push((RegistrationKind::Observer, observer_id.clone(), unregister));
        (ops.engine.clone(), observer_id)
    };

    // The engine receives the id settled above, never the guest's raw one.
    if let Some(obj) = input.as_object_mut() {
        let options = obj
            .entry("options")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(options) = options.as_object_mut() {
            options.insert("id".into(), serde_json::Value::String(observer_id.clone()));
        }
    }

    match engine.observe(input).await {
        Ok(_) => Ok(observer_id),
        Err(message) => {
            // Nothing reached the engine to dispose: take the entry off the
            // books without running it.
            let mut borrowed = state.borrow_mut();
            let ops = borrowed.borrow_mut::<OpsState>();
            drop(ops.take(RegistrationKind::Observer, &observer_id));
            Err(JsErrorBox::generic(message))
        }
    }
}

/// Remove an observer this runtime created: the handle's `dispose()`.
/// Ownership is entirely "does THIS runtime's own `unregisters` list hold
/// it" — an id it does not hold is the same quiet no-op as unregistering an
/// unknown function, so the op cannot be used to probe or remove another
/// tenant's observers. Resolves once the engine has removed it.
#[op2]
async fn op_iii_dispose_observer(
    state: Rc<RefCell<OpState>>,
    #[string] observer_id: String,
) -> Result<(), JsErrorBox> {
    if observer_id.len() > MAX_FUNCTION_ID_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "observer id is {} bytes; the limit is {MAX_FUNCTION_ID_BYTES}",
            observer_id.len()
        )));
    }
    let engine = {
        let mut borrowed = state.borrow_mut();
        let ops = borrowed.borrow_mut::<OpsState>();
        if ops.take(RegistrationKind::Observer, &observer_id).is_none() {
            return Ok(());
        }
        ops.engine.clone()
    };
    engine
        .dispose_observer(observer_id)
        .await
        .map_err(JsErrorBox::generic)
}

// ---------------------------------------------------------------------------
// iii.files — this runtime's private scratch directory
// ---------------------------------------------------------------------------

/// Cap on the tenant-supplied file NAME.
///
/// This is a `#[string]`, so deno_core byte-copies it into an owned Rust
/// `String` at dispatch, before the op body runs — the hazard documented at
/// `MAX_ACTION_BYTES`. The check is still the first statement in every body
/// that takes one, matching `op_iii_call` and `op_iii_register`.
///
/// Honest accounting of what this cap buys, because the `op_iii_call`
/// argument does NOT transfer: these ops are SYNCHRONOUS, so at most one copy
/// is ever live and it is dropped at the end of the body — unlike
/// `op_iii_call`, where 32 un-awaited calls pin 32 copies for a whole timeout
/// window. What it actually buys is a STABLE refusal: 255 is `NAME_MAX` on
/// every filesystem this ships to, so a name that passes here cannot be
/// refused later by the OS with a platform-dependent `ENAMETOOLONG` the guest
/// cannot act on.
const MAX_SCRATCH_NAME_BYTES: usize = 255;

/// Windows device names, which the OS resolves as devices regardless of any
/// extension — `CON.txt` is still `CON`.
///
/// Checked UNCONDITIONALLY, not behind `cfg(windows)`. This worker ships
/// `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`, but CI runs Linux:
/// a `cfg(windows)` rule would be untested code on the only platform it
/// protects. The cost is that a Linux tenant cannot name a file `con.txt`.
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Turn a tenant-supplied name into a host path inside this runtime's
/// directory, or refuse.
///
/// The containment argument is STRUCTURAL, not a denylist: an accepted name is
/// exactly ONE path component drawn from a closed charset, so `root.join(name)`
/// has exactly one component below `root` and there is no intermediate
/// component for a symlink to redirect through. That is what makes the
/// leaf-only `O_NOFOLLOW` / `symlink_metadata` check at each open COMPLETE
/// rather than partial — `O_NOFOLLOW` refuses only the FINAL component, which
/// would be nearly worthless if `a/b/c` were expressible.
///
/// The sibling python sandbox's escape came from the opposite posture:
/// cap-primitives rejects only *rooted* symlink targets, i.e. a denylist with
/// a hole. Do not loosen the charset.
fn scratch_path(root: &std::path::Path, name: &str) -> Result<std::path::PathBuf, JsErrorBox> {
    if name.len() > MAX_SCRATCH_NAME_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "file name is {} bytes; max {MAX_SCRATCH_NAME_BYTES}",
            name.len()
        )));
    }
    if name.is_empty() {
        return Err(JsErrorBox::type_error("file name must not be empty"));
    }
    // One rule refuses, simultaneously: `/` and `\` (traversal and Windows
    // separators), `\0` (C-string truncation), `:` (Windows alternate data
    // streams and drive letters), whitespace and control characters (log
    // forging), and every non-ASCII byte (Unicode normalization — on APFS and
    // NTFS two different byte sequences can be the SAME file, which is both an
    // aliasing bug and a way to exceed the entry cap with names that look
    // distinct).
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
    {
        return Err(JsErrorBox::type_error(format!(
            "file name {name:?} may only contain letters, digits, '.', '_' and '-' — \
             it names one file in this runtime's private directory, not a path"
        )));
    }
    // `.` and `..` are entirely within the charset above, so the rule before
    // this one does not catch them.
    //
    // This DELIBERATELY overlaps the trailing-dot rule below, which also
    // refuses both (they end in `.`). Verified by mutation: disabling either
    // rule alone leaves `..` refused; only disabling BOTH accepts it, which is
    // what `traversal_names_are_refused` actually pins. The overlap is kept
    // because the two rules guard different things and could plausibly be
    // edited apart — the trailing-dot rule reads as a Windows nicety somebody
    // might one day put behind `cfg(windows)`, at which point this rule is the
    // only thing standing between a tenant and `..`.
    if name == "." || name == ".." {
        return Err(JsErrorBox::type_error(format!(
            "file name {name:?} is a directory reference, not a file"
        )));
    }
    // Windows silently strips a trailing dot or space, so `a.` and `a` alias.
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(JsErrorBox::type_error(format!(
            "file name {name:?} must not end with '.' or a space — on Windows those are \
             stripped, so two different names would be the same file"
        )));
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        return Err(JsErrorBox::type_error(format!(
            "file name {name:?} is a reserved device name on Windows"
        )));
    }
    // `Path::join` REPLACES the base when its argument is absolute:
    // `root.join("/etc/passwd")` is `/etc/passwd`. The charset rule makes that
    // unreachable — which is exactly why anyone loosening it must read this.
    Ok(root.join(name))
}

/// What this runtime's directory currently holds, plus the size of the file a
/// write is about to replace.
struct ScratchUsage {
    bytes: u64,
    files: usize,
    /// `Some` when `target` already exists — its current size, so an overwrite
    /// does not double-count.
    target_bytes: Option<u64>,
}

/// Measure the directory. Derived per call rather than cached on `OpsState`:
/// a stored counter has to be maintained across create, overwrite, remove, and
/// a `write_all` that fails halfway leaving a partial file — each of those a
/// place to leak budget, and a leaked counter refuses every later write for
/// the life of the runtime. Cheap here specifically because the directory is
/// flat and capped at `scratch_max_files`, so this is a bounded stat scan.
///
/// `DirEntry::metadata` deliberately: it does NOT traverse symlinks, so a
/// planted link contributes its own size rather than its target's.
///
/// ponytail: O(entries) stat scan per write; cache a counter on `OpsState` if
/// a write-heavy workload ever shows it hot.
fn scan_scratch(root: &std::path::Path, target: &str) -> Result<ScratchUsage, JsErrorBox> {
    let mut usage = ScratchUsage {
        bytes: 0,
        files: 0,
        target_bytes: None,
    };
    let entries = std::fs::read_dir(root)
        .map_err(|e| JsErrorBox::generic(format!("scratch directory is unreadable: {e}")))?;
    for entry in entries.flatten() {
        let Ok(md) = entry.metadata() else { continue };
        let len = md.len();
        usage.bytes += len;
        usage.files += 1;
        if entry.file_name() == std::ffi::OsStr::new(target) {
            usage.target_bytes = Some(len);
        }
    }
    Ok(usage)
}

/// The refusal every mutating op gives once this runtime's directory is at its
/// cap. One function so the call sites cannot drift into different wordings
/// for what is, to a tenant, one cap — the same rationale
/// `registration_cap_exceeded` gives.
///
/// Names the guest's own file name and the caps; NEVER the host path, which
/// would leak the temp token and the host layout into tenant code.
fn scratch_quota_exceeded(name: &str, max_bytes: u64, max_files: usize) -> JsErrorBox {
    JsErrorBox::generic(format!(
        "writing {name:?} would exceed this runtime's scratch quota \
         ({max_bytes} bytes across at most {max_files} files); remove a file first"
    ))
}

/// Borrow the scratch root, or refuse when the feature is off.
///
/// Defence in depth: with `scratch_mb: 0` the guest surface is removed
/// entirely (see `runtime.rs`), so this refusal should be unreachable.
fn scratch_root(ops: &OpsState) -> Result<&std::path::Path, JsErrorBox> {
    ops.scratch
        .as_ref()
        .map(|d| d.path())
        .ok_or_else(|| JsErrorBox::generic("iii.files is disabled on this deployment"))
}

/// Open a file for reading, refusing anything that is not a regular file.
///
/// `std::fs::File::open` follows symlinks and `std::fs::metadata` resolves
/// them, so both are wrong here: `symlink_metadata` answers "what is AT this
/// path". Refusing a non-regular file is not tidiness — `File::open` on a FIFO
/// blocks forever, the watchdog terminates at JS boundaries and cannot
/// interrupt a blocking syscall, and a wedged isolate thread then wedges
/// `RuntimeThread::Drop`'s join and every teardown queued behind it.
fn open_regular(path: &std::path::Path) -> Result<std::fs::File, JsErrorBox> {
    let md = std::fs::symlink_metadata(path)
        .map_err(|_| JsErrorBox::type_error("no such file in this runtime's scratch directory"))?;
    if !md.is_file() {
        return Err(JsErrorBox::type_error(
            "that scratch entry is not a regular file",
        ));
    }
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW);
    }
    open.open(path)
        .map_err(|e| JsErrorBox::generic(format!("scratch read failed: {e}")))
}

/// Write (or overwrite) one file in this runtime's private directory.
///
/// `contents` is `#[buffer] &[u8]`: a ZERO-COPY view of the `Uint8Array`'s
/// backing store, NOT an owned Rust copy. Those bytes were allocated by
/// `crate::allocator`'s capped `v8::Allocator` and are already charged against
/// `external_mb`, so there is no Rust-heap byte cap to add here — the quota
/// below is about DISK. Do NOT "simplify" this to `#[buffer(copy)] Vec<u8>` or
/// `#[string] String`: either reintroduces the exact off-heap copy
/// `MAX_CALL_PAYLOAD_BYTES` exists to bound, at a far larger size.
///
/// Sync, and it must stay sync. An `.await` between the scan and the write is
/// precisely the TOCTOU `op_iii_observe` avoids by booking its slot before
/// it awaits: a tenant firing 100 un-awaited writes would have all 100
/// observe the same under-quota snapshot. Sync makes that unrepresentable
/// rather than defended.
///
/// ponytail: the watchdog's `terminate_execution` cannot interrupt a blocking
/// syscall, so a pathologically slow `$TMPDIR` extends a runtime past its
/// deadline by one write. Bounded because the write is size-capped and
/// `open_regular` refuses the two things that block unboundedly. Upgrade path
/// is `spawn_blocking` plus a reservation counter; not before a slow-storage
/// deployment actually appears.
#[op2(fast)]
fn op_iii_fs_write(
    state: &mut OpState,
    #[string] name: String,
    #[buffer] contents: &[u8],
) -> Result<(), JsErrorBox> {
    let ops = state.borrow_mut::<OpsState>();
    if name.len() > MAX_SCRATCH_NAME_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "file name is {} bytes; max {MAX_SCRATCH_NAME_BYTES}",
            name.len()
        )));
    }
    let max_bytes = ops.scratch_max_bytes;
    let max_files = ops.scratch_max_files;
    let root = scratch_root(ops)?.to_path_buf();
    let path = scratch_path(&root, &name)?;

    let usage = scan_scratch(&root, &name)?;
    let projected_bytes = usage.bytes - usage.target_bytes.unwrap_or(0) + contents.len() as u64;
    let projected_files = usage.files + usize::from(usage.target_bytes.is_none());
    // Checked BEFORE the truncating open, so a refusal is non-destructive: the
    // previous contents are still there.
    if projected_bytes > max_bytes || projected_files > max_files {
        return Err(scratch_quota_exceeded(&name, max_bytes, max_files));
    }

    let mut open = std::fs::OpenOptions::new();
    open.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW);
    }
    let mut f = open
        .open(&path)
        .map_err(|e| JsErrorBox::generic(format!("scratch write failed: {e}")))?;
    std::io::Write::write_all(&mut f, contents)
        .map_err(|e| JsErrorBox::generic(format!("scratch write failed: {e}")))
}

/// Read one file back.
///
/// Plain `#[op2]`, not `(fast)`: a fastcall cannot allocate a return buffer.
/// `#[buffer] Vec<u8>` copies into a fresh `Uint8Array` through the capped
/// allocator, so the result is charged against `external_mb` like any other
/// guest buffer, and it is bounded by the total-bytes quota — nothing in the
/// directory can exceed it.
#[op2]
#[buffer]
fn op_iii_fs_read(state: &mut OpState, #[string] name: String) -> Result<Vec<u8>, JsErrorBox> {
    let ops = state.borrow_mut::<OpsState>();
    if name.len() > MAX_SCRATCH_NAME_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "file name is {} bytes; max {MAX_SCRATCH_NAME_BYTES}",
            name.len()
        )));
    }
    let cap = ops.scratch_max_bytes;
    let root = scratch_root(ops)?.to_path_buf();
    let path = scratch_path(&root, &name)?;
    let f = open_regular(&path)?;
    let mut buf = Vec::new();
    // Belt-and-braces against a file grown out-of-band between the stat and
    // the open; the quota already bounds anything this runtime wrote.
    std::io::Read::read_to_end(&mut std::io::Read::take(f, cap + 1), &mut buf)
        .map_err(|e| JsErrorBox::generic(format!("scratch read failed: {e}")))?;
    Ok(buf)
}

/// Every file and its size, as JSON text — the same
/// one-representation-across-the-boundary rule `op_iii_call` states. Bounded
/// by construction: at most `scratch_max_files` entries of at most
/// `MAX_SCRATCH_NAME_BYTES` plus a number each.
///
/// This is the one DISCRETIONARY op. `keep` works without it. It is here
/// because it is the only way for a guest to observe its own quota state,
/// which turns "write refused" from a mystery into something a tenant can
/// handle — and because it is the scan the write path already computes.
#[op2]
#[string]
fn op_iii_fs_list(state: &mut OpState) -> Result<String, JsErrorBox> {
    let ops = state.borrow_mut::<OpsState>();
    let root = scratch_root(ops)?.to_path_buf();
    let entries = std::fs::read_dir(&root)
        .map_err(|e| JsErrorBox::generic(format!("scratch directory is unreadable: {e}")))?;
    let mut out: Vec<serde_json::Value> = Vec::new();
    for entry in entries.flatten() {
        let Ok(md) = entry.metadata() else { continue };
        if !md.is_file() {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        out.push(serde_json::json!({ "name": name, "bytes": md.len() }));
    }
    out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    serde_json::to_string(&out)
        .map_err(|e| JsErrorBox::generic(format!("scratch listing failed: {e}")))
}

/// Remove one file. Idempotent: removing a name that is not there is a quiet
/// success, the same shape `OpsState::unregister` chose.
///
/// NOT optional. Without it a long-lived runtime — which is the entire point
/// of `keep` — can only ever approach its cap and never retreat from it. The
/// first tenant to fill the quota would have permanently bricked its own
/// runtime with no recovery short of teardown: a self-inflicted denial of
/// service shipped by omission.
#[op2(fast)]
fn op_iii_fs_remove(state: &mut OpState, #[string] name: String) -> Result<(), JsErrorBox> {
    let ops = state.borrow_mut::<OpsState>();
    if name.len() > MAX_SCRATCH_NAME_BYTES {
        return Err(JsErrorBox::type_error(format!(
            "file name is {} bytes; max {MAX_SCRATCH_NAME_BYTES}",
            name.len()
        )));
    }
    let root = scratch_root(ops)?.to_path_buf();
    let path = scratch_path(&root, &name)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(JsErrorBox::generic(format!("scratch remove failed: {e}"))),
    }
}

extension!(
    node_engine_ext,
    ops = [
        op_iii_log,
        op_iii_call,
        op_iii_call_observer,
        op_iii_register,
        op_iii_unregister,
        op_iii_set_observer_hooks,
        op_iii_clear_observer_hooks,
        op_iii_observe,
        op_iii_dispose_observer,
        op_iii_fs_write,
        op_iii_fs_read,
        op_iii_fs_list,
        op_iii_fs_remove,
    ],
    options = { state: OpsState },
    state = |op_state: &mut OpState, options| {
        op_state.put::<OpsState>(options.state);
    },
);

/// Reads the ops state out of a runtime's `OpState`. Used by the isolate loop
/// to take the per-eval log buffer and registration delta.
pub fn with_ops_state<R>(op_state: &Rc<RefCell<OpState>>, f: impl FnOnce(&mut OpsState) -> R) -> R {
    let mut borrowed = op_state.borrow_mut();
    f(borrowed.borrow_mut::<OpsState>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FakeEngine;

    /// A minimal but real `OpsState` for one runtime — no isolate involved.
    /// `unregisters`/`ids` are fresh per call, matching `RuntimeThread::spawn`
    /// (see the module doc on `OpsState::unregister`): two calls with the
    /// same `runtime_id` do NOT share state, exactly as two real runtimes
    /// never do either.
    fn test_ops(namespace: &str, runtime_id: &str, engine: Arc<dyn Engine>) -> OpsState {
        let (tx, _rx) = mpsc::unbounded_channel();
        OpsState {
            engine,
            namespace: namespace.to_string(),
            logs: Vec::new(),
            log_bytes: 0,
            log_truncated: false,
            detached_log_bytes: 0,
            capturing: false,
            registered: Vec::new(),
            unregisters: Arc::new(Mutex::new(Vec::new())),
            hooked_observers: Arc::new(Mutex::new(HashMap::new())),
            ids: crate::ids::IdRegistry::default(),
            runtime_id: runtime_id.to_string(),
            call_timeout_ms: 2_000,
            max_timeout_ms: 10_000,
            inflight_calls: 0,
            command_tx: tx.downgrade(),
            invoke_timeout_ms: 2_000,
            last_activity: Arc::new(Mutex::new(Instant::now())),
            scratch: None,
            scratch_max_bytes: 0,
            scratch_max_files: 0,
        }
    }

    fn noop_handler() -> ProxyHandler {
        Arc::new(|_| Box::pin(async { Ok(serde_json::json!(null)) }))
    }

    /// The mutation this guards against: deleting the `require_own_namespace`
    /// call from `unregister_checked`. See that function's doc comment for
    /// why a JS-level eval test cannot reach this line — the guest surface
    /// never hands `op_iii_unregister` a foreign id, so only a direct call
    /// can prove the check itself refuses one.
    #[test]
    fn a_runtime_cannot_unregister_another_runtimes_function() {
        let fake = FakeEngine::new();
        // Stands in for "victim::" having registered it — going straight to
        // `Engine::register` rather than through `op_iii_register`, since
        // only the removal path is under test here.
        let _ = fake.register(
            "victim::secret".to_string(),
            None,
            None,
            None,
            GuestFunctionOptions::default(),
            noop_handler(),
        );
        assert!(fake
            .registered_ids()
            .contains(&"victim::secret".to_string()));

        let mut attacker = test_ops("attacker::", "rt-attacker", fake.clone());
        let err = unregister_checked(
            &mut attacker,
            RegistrationKind::Function,
            "victim::secret".to_string(),
        )
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

        // "Not yours" and "not there" must be indistinguishable: an id that
        // was never registered anywhere gets the identical refusal shape —
        // derived independently here, not by editing the string above — so
        // the op cannot be used to probe which ids are live elsewhere.
        let missing = unregister_checked(
            &mut attacker,
            RegistrationKind::Function,
            "victim::never-existed".to_string(),
        )
        .unwrap_err();
        let expected = |id: &str| {
            NodeEngineError::NamespaceDenied {
                id: id.to_string(),
                namespace: "attacker::".to_string(),
            }
            .message()
        };
        assert_eq!(err.to_string(), expected("victim::secret"));
        assert_eq!(missing.to_string(), expected("victim::never-existed"));
    }

    /// A runtime unregistering its own, actually-live id succeeds and the
    /// engine forgets it.
    #[test]
    fn unregister_checked_removes_a_registration_the_runtime_owns() {
        let fake = FakeEngine::new();
        let mut ops = test_ops("app::", "rt-owner", fake.clone());
        let unregister = fake.register(
            "app::mine".to_string(),
            None,
            None,
            None,
            GuestFunctionOptions::default(),
            noop_handler(),
        );
        ops.unregisters.lock().unwrap().push((
            RegistrationKind::Function,
            "app::mine".to_string(),
            unregister,
        ));
        ops.ids.claim("app::mine", "rt-owner");

        unregister_checked(
            &mut ops,
            RegistrationKind::Function,
            "app::mine".to_string(),
        )
        .unwrap();

        assert!(!fake.registered_ids().contains(&"app::mine".to_string()));
        assert!(
            ops.ids.claim("app::mine", "someone-else"),
            "the id should be free again after unregister"
        );
    }

    /// Unregistering an id inside your own namespace that was never
    /// registered is a quiet no-op, not an error — the same idempotent shape
    /// `op_iii_register` gives a re-registration.
    #[test]
    fn unregister_checked_is_a_no_op_for_an_unknown_id_in_your_own_namespace() {
        let fake = FakeEngine::new();
        let mut ops = test_ops("app::", "rt-owner", fake.clone());
        unregister_checked(
            &mut ops,
            RegistrationKind::Function,
            "app::never-registered".to_string(),
        )
        .unwrap();
    }

    /// CRITICAL (Task 12 review, round 2): a cross-kind unregister — asking
    /// to remove OBSERVER HOOKS at an id that is actually a live FUNCTION —
    /// must leave that function's bus registration AND its worker-global
    /// `ids` claim alone. Before this fix, `OpsState::unregister` found
    /// nothing to remove from the bus (correct) but still released the
    /// claim unconditionally (wrong): a second runtime could then claim the
    /// same id and reach `Engine::register` on a duplicate, which aborts the
    /// process (see `ids.rs`'s module doc).
    #[test]
    fn cross_kind_unregister_does_not_release_a_live_registrations_claim() {
        let fake = FakeEngine::new();
        let mut ops = test_ops("app::", "rt-owner", fake.clone());
        let unregister = fake.register(
            "app::x".to_string(),
            None,
            None,
            None,
            GuestFunctionOptions::default(),
            noop_handler(),
        );
        ops.unregisters.lock().unwrap().push((
            RegistrationKind::Function,
            "app::x".to_string(),
            unregister,
        ));
        ops.ids.claim("app::x", "rt-owner");

        // No hooks were ever set at this id — this must be a pure no-op.
        ops.unregister(RegistrationKind::ObserverHooks, "app::x");

        assert!(
            fake.registered_ids().contains(&"app::x".to_string()),
            "the live function was removed from the bus by an unrelated unregister"
        );
        assert!(
            !ops.ids.claim("app::x", "rt-other"),
            "a live registration's claim was released by an unrelated unregister"
        );
    }

    /// A function and its observer hooks share the identical literal id —
    /// that is the normal case, not an edge. Clearing the hooks must not
    /// release the claim while the function is still live; only once
    /// NOTHING remains under that id does the claim actually free up.
    #[test]
    fn unregistering_one_kind_does_not_release_the_claim_while_another_kind_shares_the_id() {
        let fake = FakeEngine::new();
        let mut ops = test_ops("app::", "rt-owner", fake.clone());
        let unregister_fn = fake.register(
            "app::x".to_string(),
            None,
            None,
            None,
            GuestFunctionOptions::default(),
            noop_handler(),
        );
        fake.on_observer_added("app::x".to_string(), Some(noop_handler()));
        let hooks_engine = fake.clone();
        let unregister_hooks: UnregisterFn =
            Box::new(move || hooks_engine.on_observer_added("app::x".to_string(), None));
        {
            let mut regs = ops.unregisters.lock().unwrap();
            regs.push((
                RegistrationKind::Function,
                "app::x".to_string(),
                unregister_fn,
            ));
            regs.push((
                RegistrationKind::ObserverHooks,
                "app::x".to_string(),
                unregister_hooks,
            ));
        }
        ops.ids.claim("app::x", "rt-owner");

        ops.unregister(RegistrationKind::ObserverHooks, "app::x");
        assert!(fake.hooked_ids().0.is_empty(), "the hooks must be cleared");
        assert!(
            !ops.ids.claim("app::x", "rt-other"),
            "the claim was released while the function under the same id is still live"
        );

        ops.unregister(RegistrationKind::Function, "app::x");
        assert!(
            ops.ids.claim("app::x", "rt-other"),
            "the claim should free once nothing remains registered under this id"
        );
    }

    /// `take` is `unregister` without running the undo: the books lose the
    /// entry and the claim, the engine keeps the registration until whoever
    /// took the undo decides. `op_iii_dispose_observer` relies on exactly
    /// that split.
    #[test]
    fn take_removes_the_entry_without_running_its_undo() {
        let fake = FakeEngine::new();
        let mut ops = test_ops("app::", "rt-owner", fake.clone());
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ran.clone();
        ops.unregisters.lock().unwrap().push((
            RegistrationKind::Observer,
            "app::watch".to_string(),
            Box::new(move || flag.store(true, std::sync::atomic::Ordering::SeqCst)),
        ));
        ops.ids.claim("app::watch", "rt-owner");

        let undo = ops
            .take(RegistrationKind::Observer, "app::watch")
            .expect("the entry was there to take");
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
        assert!(ops.unregisters.lock().unwrap().is_empty());
        assert!(ops.ids.claim("app::watch", "rt-other"));
        undo();
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
        assert!(ops.take(RegistrationKind::Observer, "app::watch").is_none());
    }
}

#[cfg(test)]
mod scratch_tests {
    use super::*;

    fn root() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("node-core-test-")
            .tempdir()
            .expect("tempdir")
    }

    /// Every accepted name must be exactly one component.
    ///
    /// Mutation, established by running it: deleting the `.`/`..` rule ALONE
    /// leaves this green, because `.` and `..` both end in `.` and the
    /// trailing-dot rule catches them too. The mutation this actually
    /// discriminates is deleting BOTH — which is the honest claim, and the
    /// reason the overlap is documented at the rule rather than left to look
    /// accidental. The assertion on `root.parent()` is what proves a mutant
    /// escapes rather than merely differing.
    #[test]
    fn traversal_names_are_refused() {
        let d = root();
        for name in ["..", ".", "../etc/passwd", "a/../b"] {
            assert!(
                scratch_path(d.path(), name).is_err(),
                "accepted traversal name {name:?}"
            );
        }
        assert!(
            !d.path().parent().unwrap().join("etc").exists(),
            "nothing may have been created outside the scratch root"
        );
    }

    /// Mutation: replace the charset allowlist with `!name.contains("..")` —
    /// the reasonable-looking rewrite. `foo/bar` then passes and `root.join`
    /// yields a path one directory down.
    #[test]
    fn separators_are_refused() {
        let d = root();
        for name in ["foo/bar", "foo\\bar", "a:b", "a\u{0}b", "a b", "a\nb"] {
            assert!(
                scratch_path(d.path(), name).is_err(),
                "accepted name containing a separator or control char: {name:?}"
            );
        }
    }

    /// `Path::join` REPLACES the base when its argument is absolute, so a
    /// mutant that allows `/` reads the host's passwd file rather than a file
    /// in the scratch directory. Mutation: add `b'/'` to the charset.
    #[test]
    fn absolute_names_are_refused() {
        let d = root();
        for name in ["/etc/passwd", "C:\\Windows", "\\\\server\\share"] {
            assert!(
                scratch_path(d.path(), name).is_err(),
                "accepted absolute name {name:?}"
            );
        }
    }

    /// Unconditional, not `cfg(windows)`: CI runs Linux, and a
    /// `cfg(windows)` rule would be untested code on the only platform it
    /// protects. Mutation: delete the reserved-name rule.
    #[test]
    fn windows_device_names_are_refused() {
        let d = root();
        for name in ["CON", "con.txt", "COM1", "nul", "LPT9.log", "a.", "a "] {
            assert!(
                scratch_path(d.path(), name).is_err(),
                "accepted reserved or alias-prone name {name:?}"
            );
        }
        assert!(scratch_path(d.path(), "console.txt").is_ok());
    }

    /// Non-ASCII is refused because on APFS and NTFS two different byte
    /// sequences can be the SAME file — an aliasing bug and a way to exceed
    /// the entry cap with names that look distinct. Mutation: allow
    /// `!b.is_ascii()`.
    #[test]
    fn non_ascii_names_are_refused() {
        let d = root();
        assert!(scratch_path(d.path(), "café.txt").is_err());
        assert!(scratch_path(d.path(), "cafe.txt").is_ok());
    }

    /// Asserting only "it threw" would pass against a mutant with no length
    /// check at all, because the OS answers `ENAMETOOLONG`. The message must
    /// name the limit. Mutation: delete the length check.
    #[test]
    fn an_oversized_name_names_the_limit() {
        let d = root();
        let long = "x".repeat(MAX_SCRATCH_NAME_BYTES + 1);
        let err = scratch_path(d.path(), &long).unwrap_err().to_string();
        assert!(
            err.contains(&MAX_SCRATCH_NAME_BYTES.to_string()),
            "refusal must name the limit, got: {err}"
        );
    }

    /// Host-side, because no guest op can plant a link — which is exactly why
    /// the guard looks like dead code and gets deleted without this test.
    /// Mutation: replace `open_regular`'s `symlink_metadata`/`is_file` guard
    /// with a bare `File::open`; the host then reads the link's target.
    #[cfg(unix)]
    #[test]
    fn a_planted_symlink_is_not_read_through() {
        let d = root();
        let bait = d.path().parent().unwrap().join("node-core-bait.txt");
        std::fs::write(&bait, b"ESCAPED").expect("write bait");
        std::os::unix::fs::symlink(&bait, d.path().join("link.txt")).expect("plant link");

        let err = open_regular(&d.path().join("link.txt"))
            .expect_err("a symlink must be refused, not followed");
        assert!(!err.to_string().contains("ESCAPED"));
        let _ = std::fs::remove_file(&bait);
    }

    /// `File::open` on a FIFO blocks forever, the watchdog terminates only at
    /// JS boundaries, and a wedged isolate thread then wedges every teardown
    /// queued behind it. Run on its own thread with a bounded wait, because
    /// the mutant HANGS rather than failing — and a hanging test is a bad
    /// test. Mutation: weaken the guard to "not a symlink".
    #[cfg(unix)]
    #[test]
    fn a_fifo_is_refused_rather_than_opened() {
        let d = root();
        let fifo = d.path().join("pipe");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0, "mkfifo");

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(open_regular(&fifo).is_err());
        });
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(refused) => assert!(refused, "a FIFO must be refused"),
            Err(_) => panic!("open_regular blocked on a FIFO instead of refusing"),
        }
    }

    /// The `- target_bytes` term is the most easily-omitted line in the quota
    /// projection. Mutation: drop it, and the second same-size overwrite of a
    /// nearly-full directory is refused.
    #[test]
    fn scanning_reports_the_target_size_for_an_overwrite() {
        let d = root();
        std::fs::write(d.path().join("a.txt"), b"0123456789").unwrap();
        std::fs::write(d.path().join("b.txt"), b"012").unwrap();

        let u = scan_scratch(d.path(), "a.txt").unwrap();
        assert_eq!(u.bytes, 13);
        assert_eq!(u.files, 2);
        assert_eq!(u.target_bytes, Some(10));

        let fresh = scan_scratch(d.path(), "new.txt").unwrap();
        assert_eq!(fresh.target_bytes, None, "an absent target has no size");
    }
}
