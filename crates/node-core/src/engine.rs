//! The seam between this engine and whatever hosts it.
//!
//! Everything that talks to the iii bus goes through [`Engine`], so ops,
//! runtimes and handlers are all testable without a live engine — and this
//! crate carries no SDK dependency at all. The production implementation
//! lives in the hosting worker, which is the only thing that owns a
//! connection; tests here use [`FakeEngine`].

use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::Value;

pub type CallResult = Result<Value, String>;

/// A JS handler exposed to the bus. Dynamic registrations are `Value`-in /
/// `Value`-out by nature: the schema lives in the evaluated JavaScript, not in
/// Rust types.
pub type ProxyHandler = Arc<dyn Fn(Value) -> BoxFuture<'static, CallResult> + Send + Sync>;

pub type UnregisterFn = Box<dyn Fn() + Send + Sync>;

/// What a guest's `registerFunction` options tell the engine about the
/// function beyond its catalog text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuestFunctionOptions {
    /// JSON Schema for observers' `args`; the engine validates them against
    /// it. `None`: the function takes no args, and an observer that passes
    /// some is refused with `INVALID_ARGS`.
    pub args_format: Option<Value>,
    /// Only this worker's connection may call the function; any worker may
    /// observe it.
    pub private: bool,
}

pub trait Engine: Send + Sync + 'static {
    /// Invoke any engine function. Unrestricted by design — the deployment's
    /// `iii-permissions.yaml` is the gate (see the spec's accepted risks).
    ///
    /// `action` routes the call the way the iii-sdk's own `call()` does
    /// (omit for plain request/response). It crosses as the JSON text of the
    /// guest's action object (`{"type":"void"}`) because the isolate
    /// boundary has exactly one representation to reason about — see
    /// `op_iii_call`, whose `iii.call` caller is what sends a non-`None`
    /// value.
    fn call(
        &self,
        fn_id: String,
        payload: Value,
        timeout_ms: u64,
        action: Option<String>,
    ) -> BoxFuture<'static, CallResult>;

    /// Call through an observer of a function this worker hosts: the engine
    /// runs the observer's own target with `payload`, under the observer's
    /// policy and metadata. `observer_id` is one an observer hook received
    /// (`op_iii_call_observer` refuses any other); `action` and `timeout_ms`
    /// mean what they mean for [`call`](Self::call).
    fn call_observer(
        &self,
        observer_id: String,
        payload: Value,
        timeout_ms: u64,
        action: Option<String>,
    ) -> BoxFuture<'static, CallResult>;

    /// Publish a dynamically-created function. The returned closure removes it.
    ///
    /// `description` is what `engine::functions::info` shows a caller. `None`
    /// falls back to a generic line naming node-engine (see
    /// [`DEFAULT_DYNAMIC_DESC`]) — a registration with no description at all
    /// is indistinguishable from a missing function in the catalog, which is
    /// worse than a generic line.
    ///
    /// `request_format`/`response_format` are caller-supplied JSON Schemas
    /// for the catalog: when present they replace the auto-extracted "any"
    /// schema a dynamic `Value → Value` registration otherwise carries, so
    /// `engine::functions::info` can show the real shapes. Validated before
    /// they get here (`wire::register::validate_format`), as is
    /// `options.args_format`.
    fn register(
        &self,
        fn_id: String,
        description: Option<String>,
        request_format: Option<Value>,
        response_format: Option<Value>,
        options: GuestFunctionOptions,
        handler: ProxyHandler,
    ) -> UnregisterFn;

    /// Run `hook` when an observer is added to `fn_id`, before the engine
    /// accepts it; `None` removes the hook. The hook is called with
    /// [`observer_hook_payload`]`(HookMethod::ObserverAdded, ..)`, and what it
    /// returns is read with [`hook_outcome`]: an error rejects the observer.
    fn on_observer_added(&self, fn_id: String, hook: Option<ProxyHandler>);

    /// Run `hook` when an accepted observer of `fn_id` is removed; `None`
    /// removes the hook. Called with
    /// [`observer_hook_payload`]`(HookMethod::ObserverRemoved, ..)`; what it
    /// returns is only logged.
    fn on_observer_removed(&self, fn_id: String, hook: Option<ProxyHandler>);

    /// Create an observer on the caller's behalf from a
    /// [`GuestObserveInput`]-shaped value, parsed with
    /// [`parse_guest_observe`]. `options.id` is always set by the time it
    /// gets here (`op_iii_observe` mints one when the guest gave none).
    /// Resolves to the observer id, which is what `dispose_observer` needs.
    /// Resolves before the observer is active: it stays `pending` until its
    /// function is registered and the function's hooks accept it.
    fn observe(&self, input: Value) -> BoxFuture<'static, Result<String, String>>;

    /// Remove an observer by the id `observe` resolved to.
    fn dispose_observer(&self, observer_id: String) -> BoxFuture<'static, Result<(), String>>;
}

/// Stands in when a caller registers without a description, on node-engine's
/// own connection.
pub const DEFAULT_DYNAMIC_DESC: &str =
    "Registered at runtime by node-engine; implemented in JavaScript.";

/// The code a hook rejection carries when the guest's error named none.
pub const OBSERVE_REJECTED: &str = "OBSERVE_REJECTED";

/// The longest rejection code a guest may choose. Codes are identifiers
/// (`UNKNOWN_KEY`), and the engine stores the code on the observer.
pub const MAX_REJECTION_CODE_BYTES: usize = 64;

/// The longest rejection message forwarded to the engine. The message is
/// tenant text the observer's creator reads back from `ready()`, so it is
/// capped the way a registration's description is.
pub const MAX_REJECTION_MESSAGE_BYTES: usize = 4 * 1024;

/// An observer as the guest's hooks see it: the record of what was asked
/// for. The engine keeps who asked, the observer's metadata and its policy.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuestObserver {
    pub id: String,
    pub function_id: String,
    /// Validated against the function's `args_format` (`{}` when none).
    pub args: Value,
    /// In the engine's normal form (`{}` matches every call).
    pub filters: Value,
    /// The function each call through this observer runs.
    pub target: String,
}

/// Which of a function's two observer hooks a callback is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookMethod {
    ObserverAdded,
    ObserverRemoved,
}

impl HookMethod {
    /// The method name as it crosses into the isolate, where the prelude
    /// picks the guest's `onObserverAdded` / `onObserverRemoved` hook by it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObserverAdded => "observerAdded",
            Self::ObserverRemoved => "observerRemoved",
        }
    }

    pub fn parse(method: &str) -> Option<Self> {
        match method {
            "observerAdded" => Some(Self::ObserverAdded),
            "observerRemoved" => Some(Self::ObserverRemoved),
            _ => None,
        }
    }
}

/// The payload a hook proxy is called with: `{ method, observer }`.
///
/// ONE function builds it, used by both the hosting worker's live hook
/// adapters and [`FakeEngine::fire_observer_added`] /
/// [`FakeEngine::fire_observer_removed`], so the shape a guest hook is tested
/// against is the shape it receives in production. A fake that sends a
/// narrower payload than the live adapter makes the test evidence about the
/// fake.
pub fn observer_hook_payload(method: HookMethod, observer: &GuestObserver) -> Value {
    serde_json::json!({
        "method": method.as_str(),
        "observer": observer,
    })
}

/// Why a guest's `onObserverAdded` refused an observer, in the engine's
/// vocabulary: the hosting worker turns it into the SDK's rejection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRejection {
    pub code: String,
    pub message: String,
}

/// Read what a hook proxy returned.
///
/// The prelude resolves a guest hook that threw as
/// `{ "rejected": { "code": <the error's code or null>, "message": … } }`
/// rather than rejecting: only JavaScript can read the thrown value's
/// `code`, and an `Err` crosses the isolate boundary as one formatted
/// string. Any other value is acceptance. An `Err` is a hook that never ran
/// to completion — it timed out, or its runtime is gone — and rejects with
/// [`OBSERVE_REJECTED`].
///
/// The code and message are tenant text bound for engine state, so a code
/// that is not an identifier (`A-Z`, `0-9`, `_`, at most
/// [`MAX_REJECTION_CODE_BYTES`]) becomes [`OBSERVE_REJECTED`], and the
/// message is cut at [`MAX_REJECTION_MESSAGE_BYTES`].
pub fn hook_outcome(result: CallResult) -> Result<(), HookRejection> {
    let value = match result {
        Ok(value) => value,
        Err(message) => {
            return Err(HookRejection {
                code: OBSERVE_REJECTED.to_string(),
                message: cap_message(message),
            })
        }
    };
    let Some(rejected) = value.get("rejected").filter(|r| r.is_object()) else {
        return Ok(());
    };
    let code = rejected
        .get("code")
        .and_then(Value::as_str)
        .filter(|code| {
            !code.is_empty()
                && code.len() <= MAX_REJECTION_CODE_BYTES
                && code
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        })
        .unwrap_or(OBSERVE_REJECTED);
    let message = rejected
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("observer rejected")
        .to_string();
    Err(HookRejection {
        code: code.to_string(),
        message: cap_message(message),
    })
}

fn cap_message(mut message: String) -> String {
    if message.len() > MAX_REJECTION_MESSAGE_BYTES {
        let mut end = MAX_REJECTION_MESSAGE_BYTES;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    message
}

/// An observer as the GUEST asks for it: `iii.observe(source, target,
/// options?)`, with a string `source` already turned into
/// `{ function_id }` by the prelude.
///
/// Unknown fields are refused rather than dropped, so a misspelled
/// `filter` is an error naming the fields that exist instead of an observer
/// that silently matches every call.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestObserveInput {
    pub source: GuestObserveSource,
    pub target: String,
    #[serde(default)]
    pub options: GuestObserverOptions,
}

/// The left side of a guest observer: the function observed, and which of
/// its calls reach the target.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestObserveSource {
    pub function_id: String,
    /// The observed function's namespace; unset resolves like a call.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Read by the worker that hosts the function, validated against its
    /// `args_format`.
    #[serde(default)]
    pub args: Option<serde_json::Map<String, Value>>,
    /// Read by iii: which calls reach the target.
    #[serde(default)]
    pub filters: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub deliver: Option<GuestDeliver>,
}

/// What the target receives when iii delivers a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuestDeliver {
    /// The `Observed` envelope (the default).
    Envelope,
    /// The call's input.
    Input,
    /// The call's output.
    Output,
}

/// The observer itself: its id, policy and metadata sidecar.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestObserverOptions {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub max_fires: Option<u64>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
    #[serde(default)]
    pub max_in_flight: Option<u32>,
}

/// Parse and validate a guest observer, in the guest's own vocabulary. The
/// hosting worker maps it onto the SDK's builders; `FakeEngine` and
/// `op_iii_observe` call this directly, so all three refuse identically.
pub fn parse_guest_observe(input: Value) -> Result<GuestObserveInput, String> {
    let parsed: GuestObserveInput =
        serde_json::from_value(input).map_err(|e| format!("invalid observe request: {e}"))?;
    if parsed.source.function_id.is_empty() {
        return Err("invalid observe request: source.function_id must be non-empty".into());
    }
    if parsed.target.is_empty() {
        return Err("invalid observe request: target must be non-empty".into());
    }
    Ok(parsed)
}

/// What each id was published with: `(id, description, handler)`.
#[cfg(test)]
type RegisteredHandlers = Arc<std::sync::Mutex<Vec<(String, Option<String>, ProxyHandler)>>>;
#[cfg(test)]
type RegisteredFormats =
    Arc<std::sync::Mutex<std::collections::HashMap<String, (Option<Value>, Option<Value>)>>>;
#[cfg(test)]
type RegisteredOptions =
    Arc<std::sync::Mutex<std::collections::HashMap<String, GuestFunctionOptions>>>;
#[cfg(test)]
type Hooks = std::sync::Mutex<std::collections::HashMap<String, ProxyHandler>>;

#[cfg(test)]
#[derive(Default)]
pub struct FakeEngine {
    responses: std::sync::Mutex<std::collections::HashMap<String, CallResult>>,
    /// Per-id queue of responses, indexed by how many times that id has
    /// already been called (pinned at the last entry once exhausted, rather
    /// than falling off the end) — models an answer that changes across
    /// repeated polls, e.g. a readiness check that comes back `not_found`
    /// once and resolved after.
    sequenced_responses:
        std::sync::Mutex<std::collections::HashMap<String, (Vec<CallResult>, usize)>>,
    calls: std::sync::Mutex<Vec<(String, Value)>>,
    /// `timeout_ms` for every call, in the same order as `calls` — a
    /// separate list rather than widening `calls`'s tuple, so the many
    /// existing `assert_eq!(fake.calls(), vec![(...)])` call sites across the
    /// test suite don't all need a third field they don't care about.
    timeouts: std::sync::Mutex<Vec<u64>>,
    /// `action` for every call, same order as `calls`, for the same reason.
    actions: std::sync::Mutex<Vec<Option<String>>>,
    /// `Arc` so the `'static` unregister closure can remove its own entry —
    /// a fake whose unregister only counted would let a later task assert
    /// "torn-down functions are uncallable" and pass without teardown ever
    /// removing anything.
    handlers: RegisteredHandlers,
    /// `(request_format, response_format)` per registered id — a separate
    /// map rather than widening the `handlers` tuple, for the same reason
    /// `timeouts` is separate from `calls`.
    formats: RegisteredFormats,
    /// `GuestFunctionOptions` per registered id, separate for the same reason.
    options: RegisteredOptions,
    unregisters: Arc<std::sync::atomic::AtomicUsize>,
    hang: std::sync::atomic::AtomicBool,
    route_back: std::sync::atomic::AtomicBool,
    delay: std::sync::Mutex<Option<std::time::Duration>>,
    /// `(id, input)` for every observer currently live.
    observers: std::sync::Mutex<Vec<(String, Value)>>,
    /// Observer hooks, keyed by function id.
    added_hooks: Hooks,
    removed_hooks: Hooks,
    /// `(observer_id, payload)` for every call through an observer.
    observer_calls: std::sync::Mutex<Vec<(String, Value)>>,
    observer_responses: std::sync::Mutex<std::collections::HashMap<String, CallResult>>,
}

#[cfg(test)]
impl FakeEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The `(request_format, response_format)` pair `register` received for
    /// `fn_id` — `None` when the id is not currently registered, so an
    /// unregistered id and a never-registered one read the same.
    pub fn formats_of(&self, fn_id: &str) -> Option<(Option<Value>, Option<Value>)> {
        self.formats.lock().unwrap().get(fn_id).cloned()
    }

    /// The `GuestFunctionOptions` `register` received for `fn_id`, `None`
    /// when it is not currently registered.
    pub fn options_of(&self, fn_id: &str) -> Option<GuestFunctionOptions> {
        self.options.lock().unwrap().get(fn_id).cloned()
    }

    pub fn with_response(&self, fn_id: &str, result: CallResult) {
        self.responses
            .lock()
            .unwrap()
            .insert(fn_id.to_string(), result);
    }

    /// Queue `results` for `fn_id`, one per call to it, so a test can model a
    /// value that changes across repeated polls of the SAME id (a readiness
    /// wait that sees `not_found` once, then resolved). Once every entry has
    /// been served once, later calls keep getting the LAST entry rather than
    /// falling through to `with_response`'s single value or the default
    /// "no such function" error — a queue that quietly stopped answering
    /// would make a caller that polls past it look like it timed out.
    pub fn with_response_sequence(&self, fn_id: &str, results: Vec<CallResult>) {
        assert!(!results.is_empty(), "an empty sequence answers nothing");
        self.sequenced_responses
            .lock()
            .unwrap()
            .insert(fn_id.to_string(), (results, 0));
    }

    /// What a call through `observer_id` resolves to. Unset, the call fails
    /// the way the engine fails one for an observer it does not hold.
    pub fn with_observer_response(&self, observer_id: &str, result: CallResult) {
        self.observer_responses
            .lock()
            .unwrap()
            .insert(observer_id.to_string(), result);
    }

    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    /// `timeout_ms` as received by `Engine::call`, one per entry in
    /// `calls()`, same order. What `op_iii_call`'s `iii.call` timeout
    /// clamp is verified against — the fake doesn't enforce or interpret the
    /// value, only records it.
    pub fn call_timeouts(&self) -> Vec<u64> {
        self.timeouts.lock().unwrap().clone()
    }

    /// `action` as received by `Engine::call`, one per entry in `calls()`.
    pub fn call_actions(&self) -> Vec<Option<String>> {
        self.actions.lock().unwrap().clone()
    }

    /// `(observer_id, payload)` for every `Engine::call_observer`.
    pub fn observer_calls(&self) -> Vec<(String, Value)> {
        self.observer_calls.lock().unwrap().clone()
    }

    pub fn registered_ids(&self) -> Vec<String> {
        self.handlers
            .lock()
            .unwrap()
            .iter()
            .map(|(id, _, _)| id.clone())
            .collect()
    }

    /// What each id was published with — the fake's only view of the
    /// description reaching the bus, so a test can prove it is not dropped.
    pub fn registered_descriptions(&self) -> Vec<(String, Option<String>)> {
        self.handlers
            .lock()
            .unwrap()
            .iter()
            .map(|(id, desc, _)| (id.clone(), desc.clone()))
            .collect()
    }

    pub fn unregister_count(&self) -> usize {
        self.unregisters.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Make `call` return a future that never resolves. A real engine call is
    /// pending for a while; the default fake resolves instantly, which would
    /// make in-flight accounting untestable because the counter would return
    /// to zero before the next call started.
    pub fn hang_calls(&self) {
        self.hang.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Make `call` resolve only after `delay`, so its settlement lands in a
    /// LATER event-loop pump than the eval that started it.
    ///
    /// This distinction is load-bearing for the detached-work test. The
    /// default fake resolves synchronously, so deno_core dispatches the op,
    /// resolves it, and runs its `.then`/`.catch` all inside ONE
    /// `poll_event_loop` call — while the initiating eval is still pending and
    /// its own deadline still covers everything. A real engine call crosses a
    /// socket and cannot do that; this models the real timing.
    pub fn delay_calls(&self, delay: std::time::Duration) {
        *self.delay.lock().unwrap() = Some(delay);
    }

    /// Make `call(fn_id, …)` dispatch to the handler registered under the same
    /// id, so an in-isolate call re-enters the isolate the way the engine does.
    pub fn route_calls_to_registrations(&self) {
        self.route_back
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Drive a registered proxy the way the engine would.
    pub async fn invoke(&self, fn_id: &str, payload: Value) -> CallResult {
        let handler = self
            .handlers
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _, _)| id == fn_id)
            .map(|(_, _, h)| h.clone());
        match handler {
            Some(h) => h(payload).await,
            None => Err(format!("no such registered function: {fn_id}")),
        }
    }

    /// The input each currently-live observer was created with.
    pub fn registered_observers(&self) -> Vec<Value> {
        self.observers
            .lock()
            .unwrap()
            .iter()
            .map(|(_, input)| input.clone())
            .collect()
    }

    /// Function ids that currently have an added hook / a removed hook.
    pub fn hooked_ids(&self) -> (Vec<String>, Vec<String>) {
        let mut added: Vec<String> = self.added_hooks.lock().unwrap().keys().cloned().collect();
        let mut removed: Vec<String> = self.removed_hooks.lock().unwrap().keys().cloned().collect();
        added.sort();
        removed.sort();
        (added, removed)
    }

    /// Add `observer` to its function the way the engine would: run the
    /// function's added hook with [`observer_hook_payload`] and read the
    /// answer with [`hook_outcome`], the same two functions the live adapter
    /// uses. A function with no added hook accepts, which is iii's default.
    pub async fn fire_observer_added(&self, observer: &GuestObserver) -> Result<(), HookRejection> {
        let hook = self
            .added_hooks
            .lock()
            .unwrap()
            .get(&observer.function_id)
            .cloned();
        match hook {
            Some(h) => {
                hook_outcome(h(observer_hook_payload(HookMethod::ObserverAdded, observer)).await)
            }
            None => Ok(()),
        }
    }

    /// Remove an accepted `observer` the way the engine would. `Err` is the
    /// hook's failure, which the live adapter only logs.
    pub async fn fire_observer_removed(&self, observer: &GuestObserver) -> CallResult {
        let hook = self
            .removed_hooks
            .lock()
            .unwrap()
            .get(&observer.function_id)
            .cloned();
        match hook {
            Some(h) => h(observer_hook_payload(HookMethod::ObserverRemoved, observer)).await,
            None => Ok(Value::Null),
        }
    }

    fn respond(&self, out: CallResult) -> BoxFuture<'static, CallResult> {
        let delay = *self.delay.lock().unwrap();
        Box::pin(async move {
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            out
        })
    }
}

#[cfg(test)]
impl Engine for FakeEngine {
    fn call(
        &self,
        fn_id: String,
        payload: Value,
        timeout_ms: u64,
        action: Option<String>,
    ) -> BoxFuture<'static, CallResult> {
        self.calls
            .lock()
            .unwrap()
            .push((fn_id.clone(), payload.clone()));
        self.timeouts.lock().unwrap().push(timeout_ms);
        self.actions.lock().unwrap().push(action);

        if self.hang.load(std::sync::atomic::Ordering::SeqCst) {
            return Box::pin(std::future::pending());
        }

        if self.route_back.load(std::sync::atomic::Ordering::SeqCst) {
            let handler = self
                .handlers
                .lock()
                .unwrap()
                .iter()
                .find(|(id, _, _)| *id == fn_id)
                .map(|(_, _, h)| h.clone());
            if let Some(h) = handler {
                return Box::pin(async move { h(payload).await });
            }
        }

        {
            let mut sequenced = self.sequenced_responses.lock().unwrap();
            if let Some((seq, next)) = sequenced.get_mut(&fn_id) {
                let i = (*next).min(seq.len() - 1);
                *next += 1;
                let out = seq[i].clone();
                drop(sequenced);
                return self.respond(out);
            }
        }

        let out = self
            .responses
            .lock()
            .unwrap()
            .get(&fn_id)
            .cloned()
            .unwrap_or_else(|| Err(format!("no such function: {fn_id}")));
        self.respond(out)
    }

    fn call_observer(
        &self,
        observer_id: String,
        payload: Value,
        _timeout_ms: u64,
        _action: Option<String>,
    ) -> BoxFuture<'static, CallResult> {
        self.observer_calls
            .lock()
            .unwrap()
            .push((observer_id.clone(), payload));
        let out = self
            .observer_responses
            .lock()
            .unwrap()
            .get(&observer_id)
            .cloned()
            .unwrap_or_else(|| Err(format!("OBSERVER_NOT_FOUND: no observer `{observer_id}`")));
        self.respond(out)
    }

    fn register(
        &self,
        fn_id: String,
        description: Option<String>,
        request_format: Option<Value>,
        response_format: Option<Value>,
        options: GuestFunctionOptions,
        handler: ProxyHandler,
    ) -> UnregisterFn {
        self.formats
            .lock()
            .unwrap()
            .insert(fn_id.clone(), (request_format, response_format));
        self.options.lock().unwrap().insert(fn_id.clone(), options);
        self.handlers
            .lock()
            .unwrap()
            .push((fn_id.clone(), description, handler));
        let counter = self.unregisters.clone();
        let handlers = self.handlers.clone();
        let formats = self.formats.clone();
        let options = self.options.clone();
        Box::new(move || {
            // Remove, not just count: the fake must be able to show that an
            // unregistered id is genuinely gone.
            formats.lock().unwrap().remove(&fn_id);
            options.lock().unwrap().remove(&fn_id);
            handlers.lock().unwrap().retain(|(id, _, _)| *id != fn_id);
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
    }

    fn on_observer_added(&self, fn_id: String, hook: Option<ProxyHandler>) {
        let mut hooks = self.added_hooks.lock().unwrap();
        match hook {
            Some(h) => {
                hooks.insert(fn_id, h);
            }
            None => {
                hooks.remove(&fn_id);
            }
        }
    }

    fn on_observer_removed(&self, fn_id: String, hook: Option<ProxyHandler>) {
        let mut hooks = self.removed_hooks.lock().unwrap();
        match hook {
            Some(h) => {
                hooks.insert(fn_id, h);
            }
            None => {
                hooks.remove(&fn_id);
            }
        }
    }

    fn observe(&self, input: Value) -> BoxFuture<'static, Result<String, String>> {
        // Parse through the SAME `parse_guest_observe` the live engine uses,
        // and refuse what it refuses: a fake that accepts what production
        // rejects is not a stand-in, it is a second implementation. It still
        // STORES the guest's own object, so `registered_observers()` is
        // asserted against the shape callers wrote.
        let parsed = match parse_guest_observe(input.clone()) {
            Ok(parsed) => parsed,
            Err(e) => return Box::pin(async move { Err(e) }),
        };
        let id = parsed
            .options
            .id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.observers.lock().unwrap().push((id.clone(), input));
        Box::pin(async move { Ok(id) })
    }

    fn dispose_observer(&self, observer_id: String) -> BoxFuture<'static, Result<(), String>> {
        let existed = {
            let mut observers = self.observers.lock().unwrap();
            let before = observers.len();
            observers.retain(|(id, _)| *id != observer_id);
            observers.len() != before
        };
        Box::pin(async move {
            if existed {
                Ok(())
            } else {
                Err(format!("no such observer: {observer_id}"))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn observer(function_id: &str) -> GuestObserver {
        GuestObserver {
            id: "obs-1".into(),
            function_id: function_id.into(),
            args: json!({ "key": "k" }),
            filters: json!({}),
            target: "app::react".into(),
        }
    }

    #[tokio::test]
    async fn fake_records_calls_and_returns_canned_responses() {
        let fake = FakeEngine::new();
        fake.with_response("state::get", Ok(json!({ "value": 7 })));
        let out = fake
            .call("state::get".into(), json!({ "key": "k" }), 1_000, None)
            .await;
        assert_eq!(out, Ok(json!({ "value": 7 })));
        assert_eq!(
            fake.calls(),
            vec![("state::get".to_string(), json!({ "key": "k" }))]
        );
    }

    #[tokio::test]
    async fn fake_returns_error_for_unconfigured_ids() {
        let fake = FakeEngine::new();
        let out = fake
            .call("nope::missing".into(), json!({}), 1_000, None)
            .await;
        assert_eq!(out, Err("no such function: nope::missing".to_string()));
    }

    #[tokio::test]
    async fn fake_register_exposes_the_handler_and_counts_unregisters() {
        let fake = FakeEngine::new();
        let handler: ProxyHandler = Arc::new(|p: serde_json::Value| {
            Box::pin(async move { Ok(json!({ "echo": p })) }) as BoxFuture<'static, CallResult>
        });
        let options = GuestFunctionOptions {
            args_format: Some(json!({ "type": "object" })),
            private: true,
        };
        let un = fake.register(
            "ns::hello".into(),
            None,
            None,
            None,
            options.clone(),
            handler,
        );
        assert_eq!(fake.registered_ids(), vec!["ns::hello".to_string()]);
        assert_eq!(fake.options_of("ns::hello"), Some(options));
        assert_eq!(
            fake.invoke("ns::hello", json!({ "a": 1 })).await,
            Ok(json!({ "echo": { "a": 1 } }))
        );
        un();
        assert_eq!(fake.unregister_count(), 1);
        // The fake must actually forget it, so a later task cannot assert
        // "teardown makes functions uncallable" and pass spuriously.
        assert!(fake.registered_ids().is_empty());
        assert_eq!(fake.options_of("ns::hello"), None);
        assert!(fake.invoke("ns::hello", json!({ "a": 1 })).await.is_err());
    }

    #[tokio::test]
    async fn the_fake_engine_records_observers() {
        let engine = FakeEngine::new();
        let id = engine
            .observe(json!({
                "source": { "function_id": "state::on-changed", "filters": { "input.key": { "$eq": "k" } } },
                "target": "app::react",
            }))
            .await
            .unwrap();
        assert!(!id.is_empty(), "an observer must get an id");
        assert_eq!(engine.registered_observers().len(), 1);

        engine.dispose_observer(id.clone()).await.unwrap();
        assert!(engine.registered_observers().is_empty());
    }

    #[tokio::test]
    async fn the_fake_engine_keeps_a_given_observer_id() {
        let engine = FakeEngine::new();
        let id = engine
            .observe(json!({
                "source": { "function_id": "cron::on-tick", "args": { "expression": "0 * * * * * *" } },
                "target": "app::tick",
                "options": { "id": "app::nightly", "max_in_flight": 1 },
            }))
            .await
            .unwrap();
        assert_eq!(id, "app::nightly");
    }

    #[tokio::test]
    async fn dispose_observer_reports_an_unknown_id() {
        let engine = FakeEngine::new();
        let err = engine
            .dispose_observer("no-such-observer".to_string())
            .await
            .unwrap_err();
        assert!(err.contains("no-such-observer"));
    }

    /// The fake refuses a malformed observer exactly where the live engine
    /// does, because both parse with `parse_guest_observe`.
    #[tokio::test]
    async fn the_fake_engine_refuses_what_the_parse_refuses() {
        let engine = FakeEngine::new();
        let err = engine
            .observe(json!({
                "source": { "function_id": "state::on-changed", "filter": {} },
                "target": "app::react",
            }))
            .await
            .unwrap_err();
        assert!(
            err.contains("invalid observe request") && err.contains("filter"),
            "a misspelled field must be named: {err}"
        );
        assert!(engine.registered_observers().is_empty());
    }

    #[test]
    fn a_guest_observer_parses_every_documented_field() {
        let parsed = parse_guest_observe(json!({
            "source": {
                "function_id": "database::on-row-changed",
                "namespace": "default",
                "args": { "db": "primary", "table": "orders" },
                "filters": { "input.op": { "$eq": "insert" } },
                "deliver": "output",
            },
            "target": "app::project",
            "options": {
                "id": "app::orders",
                "metadata": { "owner": "app" },
                "max_fires": 3,
                "expires_in_ms": 60000,
                "max_in_flight": 1,
            },
        }))
        .unwrap();
        assert_eq!(parsed.source.function_id, "database::on-row-changed");
        assert_eq!(parsed.source.namespace.as_deref(), Some("default"));
        assert_eq!(parsed.source.deliver, Some(GuestDeliver::Output));
        assert_eq!(parsed.target, "app::project");
        assert_eq!(parsed.options.id.as_deref(), Some("app::orders"));
        assert_eq!(parsed.options.max_in_flight, Some(1));
    }

    #[test]
    fn a_guest_observer_needs_a_function_and_a_target() {
        for input in [
            json!({ "source": {}, "target": "app::react" }),
            json!({ "source": { "function_id": "" }, "target": "app::react" }),
            json!({ "source": { "function_id": "state::on-changed" } }),
            json!({ "source": { "function_id": "state::on-changed" }, "target": "" }),
            json!({ "source": { "function_id": "state::on-changed", "args": [] }, "target": "app::react" }),
            json!({ "source": { "function_id": "state::on-changed", "deliver": "everything" }, "target": "app::react" }),
        ] {
            assert!(
                parse_guest_observe(input.clone()).is_err(),
                "accepted {input}"
            );
        }
    }

    #[tokio::test]
    async fn fake_hooks_run_with_the_shared_payload_and_outcome() {
        let fake = FakeEngine::new();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = seen.clone();
        let hook: ProxyHandler = Arc::new(move |p: Value| {
            record.lock().unwrap().push(p);
            Box::pin(async move { Ok(json!(null)) }) as BoxFuture<'static, CallResult>
        });
        fake.on_observer_added("app::x".into(), Some(hook.clone()));
        fake.on_observer_removed("app::x".into(), Some(hook));
        assert_eq!(
            fake.hooked_ids(),
            (vec!["app::x".to_string()], vec!["app::x".to_string()])
        );

        let obs = observer("app::x");
        fake.fire_observer_added(&obs).await.unwrap();
        fake.fire_observer_removed(&obs).await.unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                json!({ "method": "observerAdded", "observer": obs }),
                json!({ "method": "observerRemoved", "observer": obs }),
            ]
        );

        fake.on_observer_added("app::x".into(), None);
        fake.on_observer_removed("app::x".into(), None);
        assert_eq!(fake.hooked_ids(), (vec![], vec![]));
    }

    /// No hook is iii's default: the observer is accepted.
    #[tokio::test]
    async fn a_function_without_an_added_hook_accepts_its_observers() {
        let fake = FakeEngine::new();
        assert_eq!(fake.fire_observer_added(&observer("app::x")).await, Ok(()));
    }

    #[test]
    fn a_hook_that_resolved_accepts_the_observer() {
        assert_eq!(hook_outcome(Ok(json!(null))), Ok(()));
        assert_eq!(hook_outcome(Ok(json!({ "anything": 1 }))), Ok(()));
    }

    #[test]
    fn a_hook_that_threw_rejects_with_its_code() {
        let out = hook_outcome(Ok(json!({
            "rejected": { "code": "UNKNOWN_KEY", "message": "no key k" }
        })));
        assert_eq!(
            out,
            Err(HookRejection {
                code: "UNKNOWN_KEY".into(),
                message: "no key k".into()
            })
        );
    }

    #[test]
    fn a_hook_that_threw_without_a_code_is_observe_rejected() {
        let out = hook_outcome(Ok(
            json!({ "rejected": { "code": null, "message": "nope" } }),
        ))
        .unwrap_err();
        assert_eq!(out.code, OBSERVE_REJECTED);
        assert_eq!(out.message, "nope");
    }

    /// The code lands in engine state and in another worker's `ready()`
    /// error, so only identifier-shaped codes pass.
    #[test]
    fn a_code_that_is_not_an_identifier_is_observe_rejected() {
        for code in [
            json!(""),
            json!("lower_case"),
            json!("HAS SPACE"),
            json!("X".repeat(MAX_REJECTION_CODE_BYTES + 1)),
            json!(42),
        ] {
            let out = hook_outcome(Ok(json!({ "rejected": { "code": code, "message": "m" } })))
                .unwrap_err();
            assert_eq!(out.code, OBSERVE_REJECTED, "accepted code {code}");
        }
    }

    #[test]
    fn a_hook_that_never_finished_is_observe_rejected() {
        let out = hook_outcome(Err("node-engine::timeout: deadline".into())).unwrap_err();
        assert_eq!(out.code, OBSERVE_REJECTED);
        assert!(out.message.contains("timeout"));
    }

    #[test]
    fn a_rejection_message_is_capped_on_a_char_boundary() {
        let long = "é".repeat(MAX_REJECTION_MESSAGE_BYTES);
        let out =
            hook_outcome(Ok(json!({ "rejected": { "code": "X", "message": long } }))).unwrap_err();
        assert!(out.message.len() <= MAX_REJECTION_MESSAGE_BYTES);
        assert!(out.message.chars().all(|c| c == 'é'));
    }

    #[test]
    fn hook_methods_round_trip_their_wire_names() {
        for method in [HookMethod::ObserverAdded, HookMethod::ObserverRemoved] {
            assert_eq!(HookMethod::parse(method.as_str()), Some(method));
        }
        assert_eq!(HookMethod::parse("constructor"), None);
    }

    #[tokio::test]
    async fn the_fake_calls_through_an_observer() {
        let fake = FakeEngine::new();
        assert!(fake
            .call_observer("obs-1".into(), json!({}), 1_000, None)
            .await
            .unwrap_err()
            .contains("OBSERVER_NOT_FOUND"));
        fake.with_observer_response("obs-1", Ok(json!({ "status_code": 200 })));
        assert_eq!(
            fake.call_observer("obs-1".into(), json!({ "path": "/" }), 1_000, None)
                .await,
            Ok(json!({ "status_code": 200 }))
        );
        assert_eq!(
            fake.observer_calls(),
            vec![
                ("obs-1".to_string(), json!({})),
                ("obs-1".to_string(), json!({ "path": "/" })),
            ]
        );
    }
}
